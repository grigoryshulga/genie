//! The project chat: who may post, what a mention wakes, the manager's state in the page, which
//! manual-project wake-ups it allows, and when the manager may dispatch on behalf of a person.

use std::sync::Arc;
use std::time::Duration;

use crate::common;

use axum::http::StatusCode;
use common::{Harness, call};
use genie::ops::{self, Auth, Cx, InProcess, Out};
use genie::runtime;
use genie::runtime::manager_starts;
use genie_core::management::manager_name;
use genie_core::server_db::{Project, ProjectRole};
use genie_core::team::{CHAT_MAIL, ORCHESTRATOR};
use genie_core::{Actor, CreateInput, Role, Status, StatusOptions};
use serde_json::{Value, json};

/// A person of the project with a personal token.
fn person(h: &Harness, login: &str, role: ProjectRole) -> (i64, String) {
    let u = h.app.with_server(|db| db.create_user(login, login, None, None, false)).unwrap();
    h.app.with_server(|db| db.set_membership("shop", u.id, role)).unwrap();
    let token = h.app.with_server(|db| db.create_user_token(u.id, "test")).unwrap();
    (u.id, token)
}

/// A user of the server without access to the project.
fn user(h: &Harness, login: &str) -> i64 {
    h.app.with_server(|db| db.create_user(login, login, None, None, false)).unwrap().id
}

/// An agent token of the project.
fn agent(h: &Harness, role: Role, name: &str, job: Option<i64>) -> String {
    h.app.with_server(|db| db.create_agent_token("shop", role, name, None, job, chrono::Duration::hours(1))).unwrap()
}

fn project_manager(h: &Harness) -> String {
    agent(h, Role::Orchestrator, ORCHESTRATOR, None)
}

fn project_of(h: &Harness) -> Project {
    h.app.with_server(|db| db.project("shop")).unwrap()
}

/// Chat mail waiting for the project manager, as (kind, text).
fn manager_mail(h: &Harness) -> Vec<(String, String)> {
    h.app.with_tracker("shop", |t| t.bus().pending(None, ORCHESTRATOR)).unwrap().into_iter().map(|m| (m.kind, m.text)).collect()
}

async fn post(h: &Harness, token: &str, text: &str) -> (StatusCode, Value) {
    let (s, v, _) = call(&h.router, "POST", "/api/chat").bearer(token).json(json!({ "text": text })).send().await;
    (s, v)
}

fn set_autonomy(h: &Harness, autonomy: &str) {
    h.app.with_server(|db| db.set_autonomy("shop", autonomy)).unwrap();
}

fn ready_task(h: &Harness) -> String {
    h.app
        .with_tracker("shop", |t| {
            let anna = Actor::new("anna", Role::Human);
            let task = t.create(
                &anna,
                CreateInput {
                    title: "Export".into(),
                    description: Some("Download orders as CSV".into()),
                    acceptance: vec!["CSV downloads".into()],
                    merge_strategy: Some("deliver an artifact".into()),
                    status: Some(Status::Ready),
                    ..Default::default()
                },
            )?;
            t.set_status(&anna, &task.id, Status::Ready, StatusOptions { force: true, ..Default::default() })?;
            Ok(task.id)
        })
        .unwrap()
}

fn comments_of(h: &Harness, task: &str) -> Vec<String> {
    h.app.with_tracker("shop", |t| Ok(t.get(task)?.comments)).unwrap().into_iter().map(|c| c.text).collect()
}

fn ops_cx(h: &Harness, token: &str) -> Cx {
    let api = InProcess::new(genie::http::router(h.app.clone()), h.app.cfg.port, Auth::Bearer(token.to_string()), Some("shop".into()));
    Cx { api: Box::new(api), project: Some("shop".into()), task: None, team: None, local: false }
}

async fn run(cx: &Cx, group: &str, name: &str, args: Value) -> Result<Out, String> {
    ops::find(group, name).unwrap_or_else(|| panic!("no operation {group} {name}")).run_json(args, cx).await
}

#[tokio::test]
async fn people_post_and_read_the_chat_and_the_manager_state_comes_with_it() {
    let h = Harness::new();
    h.project("shop");
    let (_, anna) = person(&h, "anna", ProjectRole::Member);

    let (s, v) = post(&h, &anna, "  Is the export done?  ").await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!((v["id"].as_i64(), v["author"].as_str(), v["authorRole"].as_str()), (Some(1), Some("anna"), Some("human")));
    assert_eq!(v["text"], "Is the export done?", "the text is stored trimmed");

    let (s, page, _) = call(&h.router, "GET", "/api/chat").bearer(&anna).send().await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert_eq!(page["messages"][0]["text"], "Is the export done?");
    assert_eq!(page["manager"], json!({ "state": null, "pending": 0 }), "no manager session runs yet");

    let (s, e) = post(&h, &anna, "x").await;
    assert_eq!(s, StatusCode::CREATED, "{e}");
    let (s, e, _) = call(&h.router, "POST", "/api/chat").bearer(&anna).json(json!({ "text": "reply", "replyTo": 99 })).send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    assert!(e["error"].as_str().unwrap().contains("#99"), "{e}");
    let (s, _, _) = call(&h.router, "POST", "/api/chat").bearer(&anna).json(json!({ "text": "x", "urgent": true })).send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "unknown fields are refused");
    let (s, _, _) = call(&h.router, "GET", "/api/chat?limit=0").bearer(&anna).send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);

    let (s, page, _) = call(&h.router, "GET", "/api/chat?limit=1").bearer(&anna).send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(page["messages"].as_array().unwrap().len(), 1, "a page of one: the newest");
    assert_eq!(page["messages"][0]["text"], "x");
    let (_, older, _) = call(&h.router, "GET", "/api/chat?limit=1&before=2").bearer(&anna).send().await;
    assert_eq!(older["messages"][0]["id"], 1);

    let (_, viewer) = person(&h, "vera", ProjectRole::Viewer);
    let (s, e) = post(&h, &viewer, "hello").await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{e}");
    let (s, page, _) = call(&h.router, "GET", "/api/chat").bearer(&viewer).send().await;
    assert_eq!(s, StatusCode::OK, "readers read the chat: {page}");
}

#[tokio::test]
async fn among_agents_only_the_project_manager_speaks_in_the_chat() {
    let h = Harness::new();
    h.project("shop");
    let token = project_manager(&h);
    let (s, v) = post(&h, &token, "Working on it.").await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    assert_eq!((v["author"].as_str(), v["authorRole"].as_str()), (Some(ORCHESTRATOR), Some("orchestrator")));

    let task_manager = agent(&h, Role::Orchestrator, &manager_name("G-1"), None);
    let member = agent(&h, Role::Executor, "bender", None);
    let job = agent(&h, Role::Analyst, "job-1", Some(1));
    for token in [task_manager, member, job] {
        let (s, v) = post(&h, &token, "me too").await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    }
    let (s, page, _) = call(&h.router, "GET", "/api/chat").bearer(&project_manager(&h)).send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(page["messages"].as_array().unwrap().len(), 1, "the refused posts left nothing behind");
    assert_eq!(page["manager"]["state"], json!(null));
}

#[tokio::test]
async fn a_tag_of_the_manager_is_mail_for_it_with_the_context_and_a_member_tag_notifies() {
    let h = Harness::new();
    h.project("shop");
    let (_, anna) = person(&h, "anna", ProjectRole::Member);
    let (bob_id, _) = person(&h, "bob", ProjectRole::Member);
    let carol_id = user(&h, "carol");
    let (s, _) = post(&h, &anna, "Let's export the orders as CSV").await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, _) = post(&h, &anna, "Plan it, @bob will look, @carol and @anna too. @PM can you make a task?").await;
    assert_eq!(s, StatusCode::CREATED);

    let mail = manager_mail(&h);
    assert_eq!(mail.len(), 1, "one tag, one mail: {mail:?}");
    let (kind, text) = &mail[0];
    assert_eq!(kind, CHAT_MAIL);
    assert!(text.contains("#2 from @anna"), "{text}");
    assert!(text.contains("Plan it, @bob will look"), "the message itself: {text}");
    assert!(text.contains("@anna: Let's export the orders as CSV"), "the message before it is context: {text}");
    assert!(text.contains("genie chat post --reply-to 2 \"…\""), "{text}");

    let bob = h.app.with_server(|db| db.notifications(bob_id, false, 10)).unwrap();
    assert_eq!(bob.len(), 1, "@bob is told on the web");
    assert_eq!((bob[0].kind.as_str(), bob[0].link.as_deref()), ("chat", Some("/chat")));
    assert!(bob[0].title.contains("@anna"), "{}", bob[0].title);
    assert!(h.app.with_server(|db| db.notifications(carol_id, false, 10)).unwrap().is_empty(), "not a member, not told");
    let own = h.app.with_server(|db| db.user_by_login("anna")).unwrap().unwrap();
    assert!(h.app.with_server(|db| db.notifications(own.id, false, 10)).unwrap().is_empty(), "the author is not told");

    // The manager's own messages name people too, and they are told; its tag of itself makes no mail.
    let token = project_manager(&h);
    let (s, _) = post(&h, &token, "@bob the export is in progress; @manager thanks").await;
    assert_eq!(s, StatusCode::CREATED);
    assert_eq!(manager_mail(&h).len(), 1, "the manager does not mail itself");
    assert_eq!(h.app.with_server(|db| db.notifications(bob_id, false, 10)).unwrap().len(), 2);
}

#[tokio::test]
async fn the_manager_mail_carries_the_last_ten_messages_before_the_tag() {
    let h = Harness::new();
    h.project("shop");
    let (_, anna) = person(&h, "anna", ProjectRole::Member);
    for n in 1..=12 {
        post(&h, &anna, &format!("note {n}")).await;
    }
    post(&h, &anna, "@pm status?").await;
    let (_, text) = &manager_mail(&h)[0];
    assert!(text.contains("\n#3 ") && text.contains("\n#12 "), "{text}");
    assert!(!text.contains("\n#2 "), "only the ten messages before the tag: {text}");
}

#[tokio::test]
async fn a_manual_projects_manager_runs_for_chat_and_never_for_other_mail() {
    // The manager's window (managerIdleStopSecs) is the one a manual project's manager keeps for a tag.
    let h = Harness::with_config(|cfg| cfg.runtime.manager_idle_stop_secs = 1);
    h.project("shop");
    set_autonomy(&h, "manual");
    let (_, anna) = person(&h, "anna", ProjectRole::Member);

    h.app.with_tracker("shop", |t| t.bus().notify_orchestrator("anna", "human", "owner", "decide something", None)).unwrap();
    assert!(!manager_starts(&h.app, &project_of(&h)), "owner mail waits for a person in a manual project");
    assert!(!runtime::manager_wanted_in(&h.app, "shop").unwrap());

    post(&h, &anna, "@pm can you look at the export?").await;
    assert!(manager_starts(&h.app, &project_of(&h)), "a tag of the manager starts it");
    assert!(runtime::manager_wanted_in(&h.app, "shop").unwrap());

    // Once it has read the chat, it stays wanted only for the idle window after the last tag.
    h.app.with_tracker("shop", |t| t.bus().lease(None, ORCHESTRATOR, 1).and_then(|_| t.bus().complete_lease(1))).unwrap();
    assert!(runtime::manager_wanted_in(&h.app, "shop").unwrap(), "a tag within the window keeps it");
    tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
    assert!(!runtime::manager_wanted_in(&h.app, "shop").unwrap(), "the window closed: no new step starts");

    set_autonomy(&h, "autonomous");
    assert!(manager_starts(&h.app, &project_of(&h)), "other projects run the manager for any mail");
}

#[tokio::test]
async fn a_person_at_the_console_keeps_the_manual_manager_away() {
    let h = Harness::new();
    h.project("shop");
    set_autonomy(&h, "manual");
    let (_, anna) = person(&h, "anna", ProjectRole::Member);
    post(&h, &anna, "@pm hello").await;
    assert!(manager_starts(&h.app, &project_of(&h)));
    h.app.with_server(|db| db.take_console("shop", "anna", None, chrono::Duration::hours(1), false)).unwrap();
    assert!(!manager_starts(&h.app, &project_of(&h)), "the console is a person's");
    assert!(!runtime::manager_wanted_in(&h.app, "shop").unwrap());
}

#[tokio::test]
async fn in_a_manual_project_the_manager_dispatches_only_on_behalf_of_a_chat_that_asked() {
    let h = Harness::new();
    h.project("shop");
    set_autonomy(&h, "manual");
    let task = ready_task(&h);
    let (_, anna) = person(&h, "anna", ProjectRole::Member);
    let pm = project_manager(&h);
    let dispatch = |body: Value| call(&h.router, "POST", &format!("/api/management/tasks/{task}/dispatch")).bearer(&pm).json(body);

    let (s, e, _) = dispatch(json!({})).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{e}");
    assert!(e["error"].as_str().unwrap().contains("--on-behalf"), "{e}");

    // Missing: the message does not exist.
    let (s, e, _) = dispatch(json!({ "onBehalf": 99 })).send().await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::FORBIDDEN, Some("chat message #99 does not exist, so it cannot release a dispatch")));

    // The task id must be named as a word of its own.
    let (_, no_task) = post(&h, &anna, "@pm please start the export").await;
    let (s, e, _) = dispatch(json!({ "onBehalf": no_task["id"] })).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{e}");
    assert!(e["error"].as_str().unwrap().contains("does not name the task G-1"), "{e}");
    let (_, longer) = post(&h, &anna, "@pm start G-10").await;
    let (s, _, _) = dispatch(json!({ "onBehalf": longer["id"] })).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "G-10 is not G-1");

    // The tag of the manager is what asks.
    let (_, untagged) = post(&h, &anna, "please start G-1").await;
    let (s, e, _) = dispatch(json!({ "onBehalf": untagged["id"] })).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{e}");
    assert!(e["error"].as_str().unwrap().contains("does not tag the project manager"), "{e}");

    // Agent-written messages do not release anything, even when they tag the manager.
    let (_, own) = post(&h, &pm, "@manager start G-1").await;
    let (s, e, _) = dispatch(json!({ "onBehalf": own["id"] })).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{e}");
    assert!(e["error"].as_str().unwrap().contains("not by a person"), "{e}");

    // Older than a day.
    let (_, old) = post(&h, &anna, "@pm start G-1").await;
    let id = old["id"].as_i64().unwrap();
    let stale = (chrono::Utc::now() - chrono::Duration::hours(25)).to_rfc3339();
    h.app
        .with_tracker("shop", |t| Ok(t.conn().execute("UPDATE chat_messages SET at = ?1 WHERE id = ?2", rusqlite::params![stale, id])?))
        .unwrap();
    let (s, e, _) = dispatch(json!({ "onBehalf": id })).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{e}");
    assert!(e["error"].as_str().unwrap().contains("older than 24 hours"), "{e}");

    // A person who lost write access since writing does not release it.
    let (carol_id, carol) = person(&h, "carol", ProjectRole::Member);
    let (_, lost) = post(&h, &carol, "@pm start G-1").await;
    h.app.with_server(|db| db.remove_membership("shop", carol_id)).unwrap();
    let (s, e, _) = dispatch(json!({ "onBehalf": lost["id"] })).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{e}");
    assert!(e["error"].as_str().unwrap().contains("no write access"), "{e}");
    assert!(h.app.with_tracker("shop", |t| t.management().manager(&task)).unwrap().is_none(), "nothing was dispatched");

    // The request that names the task releases the dispatch, and the note records who asked.
    let (_, asked) = post(&h, &anna, "@pm please start G-1").await;
    let (s, v, _) = dispatch(json!({ "onBehalf": asked["id"] })).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["task"], task);
    let note = format!("Dispatched on behalf of @anna (chat #{}).", asked["id"]);
    assert!(comments_of(&h, &task).contains(&note), "{:?}", comments_of(&h, &task));
    let again = dispatch(json!({ "onBehalf": asked["id"] })).send().await;
    assert_eq!(again.0, StatusCode::OK);
    assert_eq!(comments_of(&h, &task).iter().filter(|c| **c == note).count(), 1, "a repeated dispatch adds no note");
}

#[tokio::test]
async fn outside_manual_projects_the_on_behalf_chat_message_is_recorded_not_checked() {
    let h = Harness::new();
    h.project("shop");
    let task = ready_task(&h);
    let (_, anna) = person(&h, "anna", ProjectRole::Member);
    let (_, unrelated) = post(&h, &anna, "lunch?").await;
    let pm = project_manager(&h);
    let (s, v, _) = call(&h.router, "POST", &format!("/api/management/tasks/{task}/dispatch"))
        .bearer(&pm)
        .json(json!({ "onBehalf": unrelated["id"] }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(comments_of(&h, &task).contains(&format!("Dispatched on behalf of @anna (chat #{}).", unrelated["id"])));

    let (s, e, _) =
        call(&h.router, "POST", &format!("/api/management/tasks/{task}/dispatch")).bearer(&pm).json(json!({ "onBehalf": 42 })).send().await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("chat message #42 not found")));
}

#[tokio::test]
async fn the_chat_and_its_manager_dispatch_run_through_the_catalog() {
    let h = Harness::new();
    h.project("shop");
    set_autonomy(&h, "manual");
    let task = ready_task(&h);
    let (_, anna) = person(&h, "anna", ProjectRole::Member);
    let pm = ops_cx(&h, &project_manager(&h));
    let person_cx = ops_cx(&h, &anna);

    let out = run(&person_cx, "chat", "post", json!({ "text": "@pm start G-1", "replyTo": null })).await.unwrap();
    assert_eq!(out.text, "posted #1");
    let out = run(&pm, "chat", "post", json!({ "text": "On it", "replyTo": 1 })).await.unwrap();
    assert_eq!(out.text, "posted #2");
    let page = run(&person_cx, "chat", "read", json!({})).await.unwrap();
    let at = |i: usize| page.data["messages"][i]["at"].as_str().unwrap()[11..16].to_string();
    assert_eq!(
        page.text,
        format!(
            "#1 {} @anna: @pm start G-1\n#2 {} project manager ↩ #1: On it\nproject manager: not running · 1 chat message(s) not yet read by it",
            at(0),
            at(1)
        ),
    );

    let out = run(&pm, "manager", "dispatch", json!({ "task": task, "onBehalf": 1 })).await.unwrap();
    assert_eq!(out.data["task"], task);
    let refused = run(&pm, "manager", "dispatch", json!({ "task": task.clone() })).await.unwrap_err();
    assert!(refused.contains("--on-behalf"), "{refused}");

    // The command line parses the same flags.
    let m = genie::cli::command().try_get_matches_from(["genie", "chat", "post", "hi there", "--reply-to", "2"]).unwrap();
    let (entry, args) = ops::chosen(&m).unwrap();
    let out = entry.run_cli(args, &pm).await.unwrap();
    assert_eq!(out.text, "posted #3");
    let m = genie::cli::command().try_get_matches_from(["genie", "chat", "read", "--limit", "2", "--before", "3"]).unwrap();
    let (entry, args) = ops::chosen(&m).unwrap();
    assert_eq!(entry.run_cli(args, &pm).await.unwrap().data["messages"].as_array().unwrap().len(), 2);
    let m = genie::cli::command().try_get_matches_from(["genie", "manager", "dispatch", &task, "--on-behalf", "1"]).unwrap();
    let (entry, args) = ops::chosen(&m).unwrap();
    assert!(entry.run_cli(args, &pm).await.is_ok(), "the flag reaches the server");
}

/// `POST /api/chat/warm` as `token` calls it: the status and the body.
async fn warm(h: &Harness, token: &str) -> (StatusCode, Value) {
    let (s, v, _) = call(&h.router, "POST", "/api/chat/warm").bearer(token).send().await;
    (s, v)
}

#[tokio::test]
async fn only_people_warm_the_project_chat() {
    let h = Harness::new();
    h.project("shop");
    let (s, v) = warm(&h, &project_manager(&h)).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "the project manager does not warm itself: {v}");
    let member = agent(&h, Role::Reviewer, "yoda", None);
    assert_eq!(warm(&h, &member).await.0, StatusCode::FORBIDDEN, "a member does not warm the chat");
    let task_manager = agent(&h, Role::Orchestrator, &manager_name("SHOP-1"), None);
    assert_eq!(warm(&h, &task_manager).await.0, StatusCode::FORBIDDEN, "a task manager does not warm the chat");
}

#[tokio::test]
async fn a_warm_up_opens_a_manual_managers_window_for_the_manager_idle_time() {
    // `turns` mode starts no session: the warm-up records its time and starts nothing, so the window is all there is to check.
    let clock = Arc::new(common::ManualClock::default());
    let h = Harness::with_clock(|cfg| cfg.runtime.mode = "turns".into(), clock.clone());
    h.project("shop");
    set_autonomy(&h, "manual");
    let (_, anna) = person(&h, "anna", ProjectRole::Member);
    assert!(!runtime::manager_wanted_in(&h.app, "shop").unwrap(), "a manual project's manager waits for chat");

    assert_eq!(warm(&h, &anna).await, (StatusCode::OK, json!({ "started": false })));
    assert!(runtime::manager_wanted_in(&h.app, "shop").unwrap(), "a warm-up is the same ask as a message that tags the manager");
    clock.advance(Duration::from_secs(3599));
    assert!(runtime::manager_wanted_in(&h.app, "shop").unwrap(), "the window is managerIdleStopSecs (3600 s) long");
    clock.advance(Duration::from_secs(2));
    assert!(!runtime::manager_wanted_in(&h.app, "shop").unwrap(), "the window has ended");
}

#[tokio::test]
async fn a_warm_up_is_limited_to_one_per_project_every_30_seconds() {
    let clock = Arc::new(common::ManualClock::default());
    let h = Harness::with_clock(|cfg| cfg.runtime.mode = "turns".into(), clock.clone());
    h.project("shop");
    set_autonomy(&h, "manual");
    let (_, anna) = person(&h, "anna", ProjectRole::Member);
    assert_eq!(warm(&h, &anna).await.1["started"], false);
    assert!(runtime::manager_wanted_in(&h.app, "shop").unwrap());

    // Ten seconds later the request is refused, and a refused request changes nothing: the window
    // still ends 3600 s after the first warm-up.
    clock.advance(Duration::from_secs(10));
    assert_eq!(warm(&h, &anna).await, (StatusCode::OK, json!({ "started": false })));
    clock.advance(Duration::from_secs(3591));
    assert!(!runtime::manager_wanted_in(&h.app, "shop").unwrap(), "the refused warm-up did not extend the window");

    // Thirty seconds after the first one, the next warm-up is accepted again.
    assert_eq!(warm(&h, &anna).await.0, StatusCode::OK);
    assert!(runtime::manager_wanted_in(&h.app, "shop").unwrap(), "an accepted warm-up opens the window again");
}

#[tokio::test]
async fn a_tagged_message_keeps_a_manual_manager_for_the_manager_idle_window() {
    let h = Harness::with_config(|cfg| {
        cfg.runtime.idle_stop_secs = 1;
        cfg.runtime.manager_idle_stop_secs = 3600;
    });
    h.project("shop");
    set_autonomy(&h, "manual");
    let (_, anna) = person(&h, "anna", ProjectRole::Member);
    let (s, v) = post(&h, &anna, "@pm can you look at the export?").await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    tokio::time::sleep(Duration::from_millis(1200)).await;
    assert!(runtime::manager_wanted_in(&h.app, "shop").unwrap(), "the window is the manager's, an hour long, not the member's second");
}

#[tokio::test]
async fn a_tagged_message_and_its_manager_mail_are_visible_together() {
    // The client refetches the chat on `chat.posted`: by then the manager already counts the mail.
    let h = Harness::new();
    h.project("shop");
    let (_, anna) = person(&h, "anna", ProjectRole::Member);
    let (s, v) = post(&h, &anna, "@pm can you look at the export?").await;
    assert_eq!(s, StatusCode::CREATED, "{v}");

    let (s, page, _) = call(&h.router, "GET", "/api/chat").bearer(&anna).send().await;
    assert_eq!(s, StatusCode::OK, "{page}");
    assert_eq!(page["messages"].as_array().map(Vec::len), Some(1), "{page}");
    assert!(page["manager"]["pending"].as_i64().unwrap() >= 1, "the manager's mail is pending at once: {page}");
    assert_eq!(manager_mail(&h).iter().filter(|(kind, _)| kind == CHAT_MAIL).count(), 1);
}

#[tokio::test]
async fn a_message_whose_manager_mail_fails_is_not_posted_and_a_retry_posts_it_once() {
    let h = Harness::new();
    h.project("shop");
    let (_, anna) = person(&h, "anna", ProjectRole::Member);
    // Refuse the manager's mail: the post and its mail are one transaction, so the message goes too.
    h.app
        .with_tracker("shop", |t| {
            Ok(t.conn().execute_batch(
                "CREATE TRIGGER refuse_chat BEFORE INSERT ON mail WHEN NEW.kind = 'chat' BEGIN SELECT RAISE(ABORT, 'refused'); END;",
            )?)
        })
        .unwrap();

    let (s, e) = post(&h, &anna, "@pm status?").await;
    assert!(s.is_server_error(), "the refused mail fails the post: {s} {e}");
    let (_, page, _) = call(&h.router, "GET", "/api/chat").bearer(&anna).send().await;
    assert!(page["messages"].as_array().unwrap().is_empty(), "nothing is half posted: {page}");
    assert_eq!(page["manager"]["pending"], 0);

    h.app.with_tracker("shop", |t| Ok(t.conn().execute_batch("DROP TRIGGER refuse_chat;")?)).unwrap();
    let (s, v) = post(&h, &anna, "@pm status?").await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    let (_, page, _) = call(&h.router, "GET", "/api/chat").bearer(&anna).send().await;
    assert_eq!(page["messages"].as_array().map(Vec::len), Some(1), "the retry posts the message once: {page}");
    assert_eq!(page["manager"]["pending"], 1);
}
