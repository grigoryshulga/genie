//! A task's delivery end to end: an agent pushes through the proxy, opens a request on the
//! (fake) host, the workflow's gates hold until the request is in order, a person merges,
//! the watcher notices what happens on the host, and `auto` merges by itself.

use crate::common;

use genie_core::{CheckState, DeliveryState, RequestState};
use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use common::fakehost::{self, FakeHost};
use common::githost::{sh, try_sh, upstream};
use common::*;
use genie::runtime::{SpawnRequest, spawn_team};
use genie_core::{Actor, CreateInput, Role, Status, StatusOptions};
use serde_json::{Value, json};

struct Rig {
    h: Harness,
    fake: FakeHost,
    port: u16,
    orchestrator: String,
}

struct Team {
    task: String,
    ws: PathBuf,
    executor: String,
    reviewer: String,
}

async fn rig(kind: &'static str, policy: Value) -> Rig {
    rig_with(kind, policy, |_| {}).await
}

async fn rig_with(kind: &'static str, policy: Value, cfg: impl FnOnce(&mut genie::config::Config)) -> Rig {
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let h = Harness::with_config(|c| {
        c.port = port;
        cfg(c);
    });
    let hosts = h.dir.path().join("hosts");
    std::fs::create_dir_all(&hosts).unwrap();
    upstream(&hosts, "acme/api", &[("README.md", "api\n")]);
    let fake = fakehost::spawn(kind, "secret").await;
    let cfg = json!({ "hosts": { "h": {
        "kind": kind, "url": fake.url,
        "clone_urls": { "https": format!("file://{}/{{remote}}.git", hosts.display()) }
    } } });
    std::fs::write(h.dir.path().join("git.json"), cfg.to_string()).unwrap();
    h.project("shop");
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port)).await.unwrap();
    tokio::spawn(genie::serve_on(h.app.clone(), listener, std::future::pending()));
    let orchestrator = h
        .app
        .with_server(|db| {
            db.create_role_token("shop", Role::Orchestrator, Some("orchestrator"), "orchestrator", None, None, chrono::Duration::hours(1))
        })
        .unwrap();
    let r = Rig { h, fake, port, orchestrator };
    let (s, b, _) = call(&r.h.router, "POST", "/api/repos")
        .json(json!({ "name": "api", "host": "h", "remote": "acme/api", "mount": ".", "policy": policy, "token": "secret" }))
        .header("x-genie-project", "shop")
        .header("host", &format!("127.0.0.1:{port}"))
        .send()
        .await;
    assert_eq!(s, 201, "{b}");
    r
}

async fn http(r: &Rig, method: &str, path: &str, token: Option<&str>, body: Option<Value>) -> (StatusCode, Value) {
    let mut c = call(&r.h.router, method, path).header("x-genie-project", "shop").header("host", &format!("127.0.0.1:{}", r.port));
    if let Some(t) = token {
        c = c.bearer(t);
    }
    if let Some(b) = body {
        c = c.json(b);
    }
    let (s, b, _) = c.send().await;
    (s, b)
}

async fn team(r: &Rig, title: &str) -> Team {
    let task =
        r.h.app
            .with_tracker("shop", |t| {
                let orch = Actor::new("orchestrator", Role::Orchestrator);
                let task = t.create(
                    &orch,
                    CreateInput {
                        title: title.into(),
                        description: Some("do it".into()),
                        acceptance: vec!["it works".into()],
                        ..Default::default()
                    },
                )?;
                t.set_status(&orch, &task.id, Status::Ready, StatusOptions::default())?;
                Ok(task.id)
            })
            .unwrap();
    let (app, t) = (r.h.app.clone(), task.clone());
    let team = tokio::task::spawn_blocking(move || {
        spawn_team(
            &app,
            "shop",
            SpawnRequest {
                task: t,
                template: None,
                members: vec![
                    genie::config::MemberSpec { role: "executor".into(), ..Default::default() },
                    genie::config::MemberSpec { role: "reviewer".into(), ..Default::default() },
                ],
                models: Default::default(),
                note: None,
                by: Actor::new("orchestrator", Role::Orchestrator),
                initiator: None,
            },
        )
        .unwrap()
    })
    .await
    .unwrap();
    let token = |class: Role, role: &str, name: &str| {
        r.h.app
            .with_server(|db| db.create_role_token("shop", class, Some(role), name, Some(&team.id), None, chrono::Duration::hours(1)))
            .unwrap()
    };
    let exec = team.members.iter().find(|m| m.role == "executor").unwrap().name.clone();
    let rev = team.members.iter().find(|m| m.role == "reviewer").unwrap().name.clone();
    Team {
        task,
        ws: PathBuf::from(&team.cwd),
        executor: token(Role::Executor, "executor", &exec),
        reviewer: token(Role::Reviewer, "reviewer", &rev),
    }
}

fn git(dir: &Path, token: &str, args: &[&str]) -> (bool, String) {
    try_sh(dir, &[("GENIE_TOKEN", token)], args)
}

fn commit_and_push(t: &Team) -> String {
    std::fs::write(t.ws.join("feature.txt"), "feature\n").unwrap();
    sh(&t.ws, &["add", "."]);
    sh(&t.ws, &["commit", "-q", "-m", "feature"]);
    let (ok, out) = git(&t.ws, &t.executor, &["push", "origin", "HEAD"]);
    assert!(ok, "{out}");
    sh(&t.ws, &["branch", "--show-current"])
}

fn row(r: &Rig, task: &str) -> genie_core::repos::TaskRepo {
    r.h.app.with_server(|db| db.task_repo("shop", task, "api")).unwrap().unwrap()
}

fn journal(r: &Rig, kind: &str) -> Vec<genie_core::Event> {
    r.h.app.with_tracker("shop", |t| genie_core::events::latest_of(t.conn(), kind, 50)).unwrap()
}

async fn status(r: &Rig, task: &str, token: &str, to: &str) -> (StatusCode, Value) {
    http(r, "POST", &format!("/api/tasks/{task}/status"), Some(token), Some(json!({ "status": to }))).await
}

/// What the task's team was told by the git host.
fn team_mail(r: &Rig, task: &str) -> Vec<genie_core::team::Mail> {
    let team = r.h.app.with_tracker("shop", |t| Ok(t.get(task)?.team.unwrap())).unwrap();
    r.h.app.with_tracker("shop", |t| t.bus().history(&team, 50)).unwrap()
}

async fn each_provider(f: impl AsyncFn(&'static str)) {
    f("github").await;
    f("gitlab").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_workflow_waits_for_the_request_and_a_person_merges_it() {
    each_provider(async |kind| {
        let r = rig(kind, json!({})).await;
        let t = team(&r, "Add a feature").await;
        let id = t.task.clone();

        assert_eq!(status(&r, &id, &t.executor, "in_progress").await.0, StatusCode::OK);
        let branch = commit_and_push(&t);
        assert_eq!(row(&r, &id).state, DeliveryState::Published);

        // Review needs the request: the branch is on the host, the policy says requests.
        let (s, b) = status(&r, &id, &t.executor, "review").await;
        assert_eq!(s, StatusCode::CONFLICT, "{kind}: {b}");
        assert!(b["error"].as_str().unwrap().contains("pr open"), "{b}");

        // Opening the request through genie.
        let (s, b) = http(
            &r,
            "POST",
            &format!("/api/tasks/{id}/repos/api/cr"),
            Some(&t.executor),
            Some(json!({ "title": "Add a feature", "body": "what and why" })),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "{kind}: {b}");
        assert_eq!(b["request"]["number"], 1);
        assert_eq!(b["request"]["head"], branch.as_str());
        {
            let f = r.fake.lock();
            assert_eq!(f.prs.len(), 1);
            assert!(f.prs[0].title.starts_with(&format!("[{id}]")), "the task is named in the title: {}", f.prs[0].title);
            assert!(f.prs[0].body.contains("what and why") && f.prs[0].body.contains(&format!("Task {id}")), "{}", f.prs[0].body);
        }
        assert_eq!((row(&r, &id).cr_number, row(&r, &id).cr_state), (Some(1), Some(RequestState::Open)), "{kind}");
        // Again: the same request, not a second one, and one announcement.
        let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
        assert_eq!((s, b["request"]["number"].clone()), (StatusCode::OK, json!(1)), "{kind}: {b}");
        assert_eq!(journal(&r, "cr.opened").len(), 1);

        assert_eq!(status(&r, &id, &t.executor, "review").await.0, StatusCode::OK);
        assert_eq!(http(&r, "POST", &format!("/api/tasks/{id}/acceptance/1"), Some(&t.reviewer), None).await.0, StatusCode::OK);
        assert_eq!(status(&r, &id, &t.reviewer, "approved").await.0, StatusCode::OK);

        // The task is approved, but its request is open: it cannot be closed yet.
        let (s, b) = status(&r, &id, &r.orchestrator, "done").await;
        assert_eq!(s, StatusCode::CONFLICT, "{kind}: {b}");
        assert!(b["error"].as_str().unwrap().contains("not merged"), "{b}");

        // The policy says a person merges: the executor may not.
        let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr/merge"), Some(&t.executor), None).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{kind}: {b}");
        assert!(b["error"].as_str().unwrap().contains("person merges"), "{b}");
        assert_eq!(r.fake.lock().prs[0].state, "open");

        // The owner merges it (through genie here; on the host itself works as well: see the watcher test).
        let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr/merge"), None, None).await;
        assert_eq!(s, StatusCode::OK, "{kind}: {b}");
        assert_eq!(b["request"]["state"], "merged");
        assert_eq!(row(&r, &id).state, DeliveryState::Merged);
        assert_eq!(journal(&r, "cr.merged").len(), 1);

        assert_eq!(status(&r, &id, &r.orchestrator, "done").await.0, StatusCode::OK, "{kind}");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_watcher_follows_the_host_and_tells_the_team_and_the_orchestrator() {
    let r = rig("gitlab", json!({})).await;
    let t = team(&r, "Fix the export").await;
    let id = t.task.clone();
    commit_and_push(&t);
    let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let watch = || async {
        genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    };

    // The checks fail on the host: an event, and the team hears of it.
    r.fake.lock().ci = "failed".into();
    watch().await;
    assert_eq!(row(&r, &id).ci_state, Some(CheckState::Failed));
    assert_eq!(journal(&r, "ci.failed").len(), 1);
    let team_id = r.h.app.with_tracker("shop", |t| Ok(t.get(&id)?.team.unwrap())).unwrap();
    let mail = r.h.app.with_tracker("shop", |t| t.bus().history(&team_id, 50)).unwrap();
    assert!(mail.iter().any(|m| m.from == "git-host" && m.text.contains("checks") && m.text.contains("failed")), "{mail:?}");
    // The team gets which job failed, a link to it and the end of its log (without colour codes).
    let m = mail.iter().find(|m| m.from == "git-host" && m.text.contains("checks")).unwrap();
    assert!(m.text.contains("- test (https://gitlab.example/acme/api/-/jobs/41)"), "{}", m.text);
    assert!(m.text.contains("test export::csv ... FAILED") && !m.text.contains('\u{1b}'), "{}", m.text);
    watch().await;
    assert_eq!(journal(&r, "ci.failed").len(), 1, "one announcement per change");

    // Fixed; and somebody merges it on the host itself.
    r.fake.lock().ci = "passed".into();
    watch().await;
    assert_eq!(journal(&r, "ci.passed").len(), 1);
    r.fake.lock().prs[0].state = "merged".into();
    watch().await;
    assert_eq!((row(&r, &id).state, row(&r, &id).cr_state), (DeliveryState::Merged, Some(RequestState::Merged)));
    assert_eq!(journal(&r, "cr.merged").len(), 1);
    let task = r.h.app.with_tracker("shop", |t| t.get(&id)).unwrap();
    assert!(task.comments.iter().any(|c| c.text.contains("was merged")), "the task says so");
    assert!(r.h.app.with_server(|db| db.open_deliveries()).unwrap().is_empty(), "a merged request is not watched any more");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_closed_on_the_host_is_recorded_as_abandoned() {
    let r = rig("github", json!({})).await;
    let t = team(&r, "Try something").await;
    let id = t.task.clone();
    commit_and_push(&t);
    http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    r.fake.lock().prs[0].state = "closed".into();
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    assert_eq!((row(&r, &id).state, row(&r, &id).cr_state), (DeliveryState::Abandoned, Some(RequestState::Closed)));
    assert_eq!(journal(&r, "cr.closed").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn with_auto_the_server_merges_once_the_task_is_approved_and_the_host_agrees() {
    let policy = json!({ "change_request": { "merge": "auto", "approvals": 1, "require_ci": true } });
    let r = rig("github", policy).await;
    let t = team(&r, "Ship it").await;
    let id = t.task.clone();
    commit_and_push(&t);
    http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    let watch = || async { genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap() };

    // Not approved as a task: nothing happens, however green the host is.
    {
        let mut f = r.fake.lock();
        f.ci = "passed".into();
        f.approvals = 1;
    }
    watch().await;
    assert_eq!(r.fake.lock().prs[0].state, "open");

    // The task is approved, but the host's conditions are not met yet: it waits.
    r.h.app
        .with_tracker("shop", |tr| {
            tr.set_status(&Actor::new("boss", Role::Human), &id, Status::Approved, StatusOptions { force: true, ..Default::default() })
        })
        .unwrap();
    r.fake.lock().approvals = 0;
    watch().await;
    assert_eq!(r.fake.lock().prs[0].state, "open");
    r.fake.lock().ci = "pending".into();
    r.fake.lock().approvals = 1;
    watch().await;
    assert_eq!(r.fake.lock().prs[0].state, "open", "the checks are still running");

    // Everything holds: the server merges.
    r.fake.lock().ci = "passed".into();
    watch().await;
    assert_eq!(r.fake.lock().prs[0].state, "merged");
    assert_eq!(row(&r, &id).state, DeliveryState::Merged);
    assert_eq!(journal(&r, "cr.merged").len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_agent_may_merge_after_approval_when_the_policy_lets_it_and_the_host_refusing_is_reported() {
    let policy = json!({ "change_request": { "merge": "agent_after_approval", "approvals": 0, "require_ci": false } });
    let r = rig("gitlab", policy).await;
    let t = team(&r, "Small fix").await;
    let id = t.task.clone();
    commit_and_push(&t);
    http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    let merge = |token: String| {
        let (r, id) = (&r, id.clone());
        async move { http(r, "POST", &format!("/api/tasks/{id}/repos/api/cr/merge"), Some(&token), None).await }
    };
    // Before the reviewer approves the task: no.
    let (s, b) = merge(t.executor.clone()).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert!(b["error"].as_str().unwrap().contains("approved"));
    assert_eq!(status(&r, &id, &t.executor, "in_progress").await.0, StatusCode::OK);
    assert_eq!(status(&r, &id, &t.executor, "review").await.0, StatusCode::OK);
    assert_eq!(status(&r, &id, &t.reviewer, "approved").await.0, StatusCode::OK);
    // The host has conflicts: its refusal comes back as it is.
    r.fake.lock().mergeable = false;
    let (s, b) = merge(t.executor.clone()).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert!(b["error"].as_str().unwrap().contains("cannot be merged"), "{b}");
    r.fake.lock().mergeable = true;
    let (s, b) = merge(t.executor.clone()).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(r.fake.lock().prs[0].state, "merged");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn requests_follow_the_policy_and_the_branch() {
    let r = rig("github", json!({ "change_request": { "open": false }, "push": "branches" })).await;
    let t = team(&r, "No requests here").await;
    let id = t.task.clone();
    commit_and_push(&t);
    let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    assert!(b["error"].as_str().unwrap().contains("not allowed"), "{b}");
    // Review is not held up by a request nobody may open.
    assert_eq!(status(&r, &id, &t.executor, "in_progress").await.0, StatusCode::OK);
    assert_eq!(status(&r, &id, &t.executor, "review").await.0, StatusCode::OK);
    assert!(r.fake.lock().prs.is_empty());

    // A reviewer cannot open one either, and a branch with nothing new has nothing to deliver.
    let r2 = rig("github", json!({})).await;
    let t2 = team(&r2, "Nothing yet").await;
    let (s, b) = http(&r2, "POST", &format!("/api/tasks/{}/repos/api/cr", t2.task), Some(&t2.reviewer), Some(json!({}))).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
    let (s, b) = http(&r2, "POST", &format!("/api/tasks/{}/repos/api/cr", t2.task), Some(&t2.executor), Some(json!({}))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert!(b["error"].as_str().unwrap().contains("not on the git host"), "{b}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn deleting_a_task_removes_its_workspace_and_stops_watching_its_request() {
    let r = rig("github", json!({})).await;
    let t = team(&r, "Throwaway").await;
    let id = t.task.clone();
    commit_and_push(&t);
    http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    assert_eq!(r.h.app.with_server(|db| db.open_deliveries()).unwrap().len(), 1);
    assert!(t.ws.join("feature.txt").is_file());

    let (s, b) = http(&r, "DELETE", &format!("/api/tasks/{id}"), None, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert!(r.h.app.with_server(|db| db.task_repos("shop", &id)).unwrap().is_empty(), "the delivery rows are gone");
    assert!(r.h.app.with_server(|db| db.open_deliveries()).unwrap().is_empty(), "nobody watches the request any more");
    assert!(!t.ws.exists(), "the team's clones are removed: the work is on the host");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn what_a_person_writes_on_the_request_reaches_the_task_and_the_team_once() {
    each_provider(async |kind| {
        let r = rig(kind, json!({})).await;
        let t = team(&r, "Review me").await;
        let id = t.task.clone();
        commit_and_push(&t);
        http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
        let watch = || async { genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap() };
        watch().await;
        let reviews = |task: &genie_core::Task| task.comments.iter().filter(|c| c.kind == genie_core::CommentKind::Review).count();
        assert_eq!(reviews(&r.h.app.with_tracker("shop", |t| t.get(&id)).unwrap()), 0);

        // The agent's own comment (signed) is not echoed back; a person's is passed on.
        let (s, _) = http(
            &r,
            "POST",
            &format!("/api/tasks/{id}/repos/api/cr/comments"),
            Some(&t.executor),
            Some(json!({ "text": "ready for a look" })),
        )
        .await;
        assert_eq!(s, StatusCode::OK);
        r.fake.lock().comments.push(("alice".into(), "please rename the flag".into()));
        watch().await;
        let task = r.h.app.with_tracker("shop", |t| t.get(&id)).unwrap();
        assert_eq!(reviews(&task), 1, "{kind}: {:?}", task.comments);
        let review = task.comments.iter().find(|c| c.kind == genie_core::CommentKind::Review).unwrap();
        assert!(review.text.contains("alice") && review.text.contains("please rename the flag"), "{}", review.text);
        let team_id = task.team.clone().unwrap();
        let mail = r.h.app.with_tracker("shop", |t| t.bus().history(&team_id, 50)).unwrap();
        assert!(mail.iter().any(|m| m.from == "git-host" && m.text.contains("please rename the flag")), "{kind}: {mail:?}");

        // The same comment is not announced again.
        watch().await;
        assert_eq!(reviews(&r.h.app.with_tracker("shop", |t| t.get(&id)).unwrap()), 1, "{kind}");
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_owner_merges_from_the_agents_request_by_the_policy() {
    let r = rig("github", json!({})).await;
    let t = team(&r, "Ship the report").await;
    let id = t.task.clone();
    assert_eq!(status(&r, &id, &t.executor, "in_progress").await.0, StatusCode::OK);
    commit_and_push(&t);

    // Before there is a request, asking to merge one says so.
    let ask = json!({ "status": "needs_owner", "note": "Слейте, пожалуйста", "action": { "kind": "ask-for-merge-pr" } });
    let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/status"), Some(&r.orchestrator), Some(ask.clone())).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b["error"].as_str().unwrap().contains("no open request"), "{b}");

    let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({ "title": "Report" }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    // The task's only open request is the one to merge: the server names it.
    let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/status"), Some(&r.orchestrator), Some(ask)).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let action = &b["needsOwner"]["action"];
    assert_eq!(
        (action["kind"].as_str(), action["repo"].as_str(), action["number"].as_i64()),
        (Some("ask-for-merge-pr"), Some("api"), Some(1)),
        "{b}"
    );
    assert!(action["url"].as_str().is_some_and(|u| !u.is_empty()), "{b}");

    // A repository without a request of the task is refused by name.
    let wrong = json!({ "status": "needs_owner", "note": "?", "action": { "kind": "ask-for-merge-pr", "repo": "web" } });
    let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/status"), Some(&r.orchestrator), Some(wrong)).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{b}");
    assert!(b["error"].as_str().unwrap().contains("open in api"), "{b}");

    // The button holds the person to the policy: the checks must pass (the person counts as the approval).
    r.fake.lock().ci = "failed".into();
    let merge = format!("/api/tasks/{id}/repos/api/cr/merge");
    let (s, b) = http(&r, "POST", &merge, None, Some(json!({ "policy": true }))).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert!(b["error"].as_str().unwrap().contains("checks"), "{b}");
    assert_eq!(r.fake.lock().prs[0].state, "open");

    r.fake.lock().ci = "passed".into();
    let (s, b) = http(&r, "POST", &merge, None, Some(json!({ "policy": true }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["request"]["state"], "merged");
}

/// G-88: on GitHub the team hears which check failed, where to look and what the host says.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_github_check_reaches_the_team_with_its_name_link_and_summary() {
    let r = rig("github", json!({})).await;
    let t = team(&r, "Fix the export").await;
    let id = t.task.clone();
    commit_and_push(&t);
    let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{b}");

    r.fake.lock().ci = "failed".into();
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    let team_id = r.h.app.with_tracker("shop", |t| Ok(t.get(&id)?.team.unwrap())).unwrap();
    let mail = r.h.app.with_tracker("shop", |t| t.bus().history(&team_id, 50)).unwrap();
    let m = mail.iter().find(|m| m.from == "git-host" && m.text.contains("checks")).expect("the team is told");
    assert!(m.text.contains("- build (https://github.example/acme/api/runs/9)"), "{}", m.text);
    assert!(m.text.contains("Build failed") && m.text.contains("unresolved import `orders`"), "{}", m.text);
    let event = journal(&r, "ci.failed");
    assert_eq!(event[0].payload["failures"][0]["name"], "build", "{event:?}");
}

/// AC1: an agent may not hand the work over for review while the checks of its branch have failed.
/// The decision is taken from the host, with the recorded state as the fallback.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn review_is_closed_when_the_checks_failed() {
    each_provider(async |kind| {
        let r = rig(kind, json!({})).await;
        let t = team(&r, "Add a feature").await;
        let id = t.task.clone();
        assert_eq!(status(&r, &id, &t.executor, "in_progress").await.0, StatusCode::OK);
        commit_and_push(&t);
        let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
        assert_eq!(s, StatusCode::OK, "{kind}: {b}");

        // The checks are red on the host: the move is refused and says what is in the way.
        r.fake.lock().ci = "failed".into();
        let (s, b) = status(&r, &id, &t.executor, "review").await;
        assert_eq!(s, StatusCode::CONFLICT, "{kind}: {b}");
        let why = b["error"].as_str().unwrap();
        assert!(why.contains("checks") && why.contains("failed") && why.contains("api"), "{kind}: {b}");

        // Still running is not a failure: nothing wakes a session when checks turn green, so a
        // `pending` CI must not strand the executor (the plan's decision 1).
        r.fake.lock().ci = "pending".into();
        assert_eq!(status(&r, &id, &t.executor, "review").await.0, StatusCode::OK, "{kind}");
        assert_eq!(
            http(&r, "POST", &format!("/api/tasks/{id}/status"), None, Some(json!({ "status": "in_progress" }))).await.0,
            StatusCode::OK
        );

        // Back to a red CI, and a host that cannot be reached: the recorded state decides.
        r.fake.lock().ci = "failed".into();
        genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
        assert_eq!(row(&r, &id).ci_state, Some(CheckState::Failed));
        r.fake.lock().broken = 5;
        let (s, b) = status(&r, &id, &t.executor, "review").await;
        assert_eq!(s, StatusCode::CONFLICT, "{kind}: the recorded state is used when the host is down: {b}");

        // A person is not held to it: their moves are authoritative.
        let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/status"), None, Some(json!({ "status": "review" }))).await;
        assert_eq!(s, StatusCode::OK, "{kind}: a person decides: {b}");
        assert_eq!(
            http(&r, "POST", &format!("/api/tasks/{id}/status"), None, Some(json!({ "status": "in_progress" }))).await.0,
            StatusCode::OK
        );

        // Green: the agent may hand it over.
        r.fake.lock().ci = "passed".into();
        r.fake.lock().broken = 0;
        let (s, b) = status(&r, &id, &t.executor, "review").await;
        assert_eq!(s, StatusCode::OK, "{kind}: {b}");
    })
    .await;
}

/// AC2: with a policy that asks for no requests, the branch's checks are watched all the same and
/// the team hears about a failure (there is no request to watch).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pushed_branch_is_watched_without_a_request() {
    let policy = json!({ "push": "branches", "change_request": { "open": false } });
    let r = rig("github", policy).await;
    let t = team(&r, "Deliver on the branch").await;
    let id = t.task.clone();
    assert_eq!(status(&r, &id, &t.executor, "in_progress").await.0, StatusCode::OK);
    let branch = commit_and_push(&t);

    // The push itself arms the watch: the branch and its commit are recorded, no request exists.
    let pushed = row(&r, &id);
    assert_eq!(pushed.cr_number, None, "no request was opened");
    assert_eq!(pushed.ci_ref, branch);
    assert!(pushed.ci_sha.is_some(), "the watched commit comes from the push");
    assert_eq!(pushed.ci_state, None, "nothing has been looked at yet");
    assert_eq!(r.h.app.with_server(|db| db.watched_deliveries()).unwrap().len(), 1);

    r.fake.lock().ci = "failed".into();
    genie::git::delivery::watch_one(&r.h.app, &pushed).await.unwrap();
    assert_eq!(row(&r, &id).ci_state, Some(CheckState::Failed));
    assert_eq!(journal(&r, "ci.failed").len(), 1);
    let mail = team_mail(&r, &id);
    let m = mail.iter().find(|m| m.from == "git-host" && m.text.contains("failed")).expect("the team is told");
    assert!(m.text.contains(&format!("branch `{branch}`")), "the branch is named: {}", m.text);
    assert!(m.text.contains("- build (https://github.example/acme/api/runs/9)"), "{}", m.text);

    // The same failure is not announced twice.
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    assert_eq!(journal(&r, "ci.failed").len(), 1);

    // The review gate reads it too — and with no request to open, nothing else holds the move.
    let (s, b) = status(&r, &id, &t.executor, "review").await;
    assert_eq!(s, StatusCode::CONFLICT, "{b}");

    // The checks are still visible to whoever asks for the delivery (AC2: without asking for CI).
    let (s, b) = http(&r, "GET", &format!("/api/tasks/{id}/repos"), Some(&t.executor), None).await;
    assert_eq!(s, StatusCode::OK);
    let row_json = b["repos"].as_array().unwrap().iter().find(|x| x["repo"] == "api").unwrap().clone();
    assert_eq!((row_json["ciState"].as_str(), row_json["ciRef"].as_str()), (Some("failed"), Some(branch.as_str())));
}

/// AC2: a person may merge a request past its checks; the commit that lands on the target branch is
/// then watched, and its failure reaches the team.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_target_branch_is_checked_after_a_merge_past_the_checks() {
    each_provider(async |kind| {
        let r = rig(kind, json!({})).await;
        let t = team(&r, "Merge it as it is").await;
        let id = t.task.clone();
        commit_and_push(&t);
        let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
        assert_eq!(s, StatusCode::OK, "{kind}: {b}");

        // A person merges on the host while the checks are red.
        r.fake.lock().ci = "failed".into();
        r.fake.lock().prs[0].state = "merged".into();
        genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
        let merged = row(&r, &id);
        assert_eq!(merged.state, DeliveryState::Merged, "{kind}");
        assert_eq!(journal(&r, "cr.merged").len(), 1);
        assert_eq!(merged.ci_ref, "main", "{kind}: the target branch is what is watched now");
        assert_eq!(merged.ci_sha.as_deref(), Some("2222222222222222222222222222222222222222"), "{kind}");
        assert_eq!(merged.ci_state, None, "the merge commit has not been looked at yet");
        assert!(journal(&r, "ci.failed").is_empty(), "{kind}: the head's red checks are history, not news");

        // The next look reports the failure of the commit that landed on the target branch.
        genie::git::delivery::watch_one(&r.h.app, &merged).await.unwrap();
        assert_eq!(row(&r, &id).ci_state, Some(CheckState::Failed));
        assert_eq!(journal(&r, "ci.failed").len(), 1);
        let mail = team_mail(&r, &id);
        assert!(mail.iter().any(|m| m.text.contains("branch `main`")), "{kind}: {mail:?}");

        // Green again: the watch settles and says so.
        r.fake.lock().ci = "passed".into();
        genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
        assert_eq!(row(&r, &id).ci_state, Some(CheckState::Passed));
        assert_eq!(journal(&r, "ci.passed").len(), 1, "{kind}");
    })
    .await;
}

/// AC3: checks that never settle are reported once, and the watch stops taking them as running.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn checks_pending_too_long_are_reported_once() {
    let policy = json!({ "push": "branches", "change_request": { "open": false } });
    let r = rig_with("gitlab", policy, |c| c.runtime.ci_pending_secs = 1).await;
    let t = team(&r, "Checks that never end").await;
    let id = t.task.clone();
    assert_eq!(status(&r, &id, &t.executor, "in_progress").await.0, StatusCode::OK);
    commit_and_push(&t);
    r.fake.lock().ci = "pending".into();

    // A first look records that the checks are running and since when.
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    let running = row(&r, &id);
    assert_eq!(running.ci_state, Some(CheckState::Pending));
    assert_ne!(running.ci_since, "", "the clock is running");
    assert!(journal(&r, "ci.stalled").is_empty());

    // Longer than `ciPendingSecs`: one event, one letter, and the state stops pretending.
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    let stalled = row(&r, &id);
    assert_eq!(stalled.ci_state, Some(CheckState::Stalled));
    assert_eq!(journal(&r, "ci.stalled").len(), 1);
    let letters = team_mail(&r, &id).into_iter().filter(|m| m.from == "git-host").count();
    assert!(letters > 0, "the team is told");
    assert!(team_mail(&r, &id).iter().any(|m| m.from == "git-host" && m.text.contains(&format!("branch `{}`", stalled.ci_ref))));

    // Reported once: looking again neither repeats the letter nor the event, and the row leaves the
    // watch set (nothing keeps asking the host about checks nobody waits for any more).
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    assert_eq!(journal(&r, "ci.stalled").len(), 1);
    assert_eq!(team_mail(&r, &id).into_iter().filter(|m| m.from == "git-host").count(), letters, "one letter, not two");
    assert!(r.h.app.with_server(|db| db.watched_deliveries()).unwrap().is_empty(), "stalled is terminal");

    // `stalled` is not a failure: it does not close the review either.
    let (s, b) = status(&r, &id, &t.executor, "review").await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

/// A `task_repos` row as it looked before the CI watch existed: nothing watched, only `head_sha`
/// (or nothing at all) and the state the host last gave.
fn unarm(r: &Rig, task: &str, head_sha: Option<&str>, ci_state: &str) {
    r.h.app
        .with_server(|db| {
            db.conn().execute(
                "UPDATE task_repos SET ci_state = ?1, ci_ref = '', ci_sha = NULL, ci_since = '', head_sha = ?2
                 WHERE project = 'shop' AND task = ?3 AND repo = 'api'",
                rusqlite::params![ci_state, head_sha, task],
            )?;
            Ok(())
        })
        .unwrap();
}

/// F1 (round 1): a delivery whose watched commit was never armed — a row from before this change, or
/// a request whose host named no head commit — must not slip past a failed CI. F4: a head that moved
/// without a push through the proxy must not leave the gate and the watcher on different commits.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_delivery_from_before_the_watch_does_not_slip_past_a_failed_check() {
    let r = rig("github", json!({})).await;
    let t = team(&r, "An old delivery").await;
    let id = t.task.clone();
    assert_eq!(status(&r, &id, &t.executor, "in_progress").await.0, StatusCode::OK);
    let branch = commit_and_push(&t);
    let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let watched = row(&r, &id).ci_sha.clone().expect("the push armed the watch");

    // (a) Nothing watched, `head_sha` and the failure recorded, the host down: the state decides.
    unarm(&r, &id, Some(&watched), "failed");
    r.fake.lock().broken = 5;
    let (s, b) = status(&r, &id, &t.executor, "review").await;
    assert_eq!(s, StatusCode::CONFLICT, "the recorded state decides when the host is down: {b}");
    assert!(b["error"].as_str().unwrap().contains("checks"), "{b}");
    r.fake.lock().broken = 0;

    // (b) The host answers: with nothing armed the gate asks about the request's head.
    r.fake.lock().ci = "passed".into();
    let (s, b) = status(&r, &id, &t.executor, "review").await;
    assert_eq!(s, StatusCode::OK, "the checks of the request's head are read live: {b}");
    assert_eq!(
        http(&r, "POST", &format!("/api/tasks/{id}/status"), None, Some(json!({ "status": "in_progress" }))).await.0,
        StatusCode::OK
    );

    // (c) No commit to ask about at all (a request the host opened without a head sha): the recorded
    // failure still closes the review.
    unarm(&r, &id, None, "failed");
    let (s, b) = status(&r, &id, &t.executor, "review").await;
    assert_eq!(s, StatusCode::CONFLICT, "nothing to ask about, the state decides: {b}");

    // The next look arms the watch from the request: the row from before the watch gets one too, and
    // the checks become visible (AC2).
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    let armed = row(&r, &id);
    assert_eq!(armed.ci_sha.as_deref(), Some(r.fake.lock().sha.as_str()), "the request's head is watched");
    assert_eq!(armed.ci_ref, branch);
    assert_eq!(armed.ci_state, Some(CheckState::Passed), "and its checks are read");

    // (d) F4: the branch moved on the host, not through the proxy. The next look follows the request.
    r.fake.lock().prs[0].sha = "3333333333333333333333333333333333333333".into();
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    assert_eq!(row(&r, &id).ci_sha.as_deref(), Some("3333333333333333333333333333333333333333"), "the gate and the watcher agree");
}

/// F2 (round 1): a repository whose host reports no checks is looked at once more — they may start a
/// moment after the push — and then left alone, so nothing is polled for the life of the row.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_branch_without_checks_is_watched_once_more_and_then_left_alone() {
    let policy = json!({ "push": "branches", "change_request": { "open": false } });
    let r = rig("github", policy).await;
    let t = team(&r, "No checks here").await;
    let id = t.task.clone();
    assert_eq!(status(&r, &id, &t.executor, "in_progress").await.0, StatusCode::OK);
    let branch = commit_and_push(&t);
    r.fake.lock().ci = "none".into();

    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    let first = row(&r, &id);
    assert_eq!(first.ci_state, Some(CheckState::None));
    assert!(first.ci_sha.is_some(), "the branch stays watched for one more look");
    assert_eq!(r.h.app.with_server(|db| db.watched_deliveries()).unwrap().len(), 1);

    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    let second = row(&r, &id);
    assert_eq!(second.ci_state, Some(CheckState::None), "the answer stands");
    assert_eq!(second.ci_sha, None, "and nothing is watched any more");
    assert!(r.h.app.with_server(|db| db.watched_deliveries()).unwrap().is_empty(), "no row is polled for ever");
    assert!(journal(&r, "ci.failed").is_empty());

    // The branch's delivery still says what was found, and it is still visible (AC2).
    let (s, b) = http(&r, "GET", &format!("/api/tasks/{id}/repos"), Some(&t.executor), None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let found = b["repos"].as_array().unwrap().iter().find(|x| x["repo"] == "api").unwrap().clone();
    assert_eq!(found["ciState"].as_str(), Some("none"));
    assert_eq!(found["branch"].as_str(), Some(branch.as_str()));
}

// --- G-115: rerunning failed checks -----------------------------------------------------

async fn rerun(r: &Rig, task: &str, token: &str) -> (StatusCode, Value) {
    http(r, "POST", &format!("/api/tasks/{task}/repos/api/cr/rerun"), Some(token), Some(json!({}))).await
}

/// Add a second repository to the project and put it on the task (the per-task limit is summed
/// over a task's deliveries, so the test needs two rows).
async fn add_second_repo(r: &Rig, task: &str) {
    let (s, b) = http(
        r,
        "POST",
        "/api/repos",
        None,
        Some(json!({ "name": "web", "host": "h", "remote": "acme/api", "mount": "web", "policy": {}, "token": "secret" })),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let (s, b) = http(r, "PUT", &format!("/api/tasks/{task}/repos"), None, Some(json!({ "repos": ["api:write", "web:write"] }))).await;
    assert_eq!(s, StatusCode::OK, "{b}");
}

/// AC1: a failed check is rerun on the host, the limit is spent, the watch is re-armed to `pending`
/// (a `failed` row is not watched), the event is journalled, and the watcher follows the new run.
/// AC2: the same commit cannot be rerun twice; a moved head is a new commit.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_check_is_rerun_once_and_the_watch_follows_it() {
    each_provider(async |kind| {
        let r = rig(kind, json!({})).await;
        let t = team(&r, "Fix the build").await;
        let id = t.task.clone();
        commit_and_push(&t);
        let (s, b) = http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
        assert_eq!(s, StatusCode::OK, "{kind}: {b}");

        // The checks fail, and the watcher records it.
        r.fake.lock().ci = "failed".into();
        genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
        assert_eq!(row(&r, &id).ci_state, Some(CheckState::Failed), "{kind}");

        // The executor reruns them: the host restarts the run, the row goes back to `pending`.
        let (s, b) = rerun(&r, &id, &t.executor).await;
        assert_eq!(s, StatusCode::OK, "{kind}: {b}");
        assert!(b["runs"].as_u64().unwrap_or(0) >= 1, "{kind}: {b}");
        assert_eq!((b["used"]["request"].as_i64(), b["limits"]["request"].as_i64()), (Some(1), Some(2)), "{kind}: {b}");
        assert_eq!(r.fake.lock().reruns, 1, "{kind}");
        let rearmed = row(&r, &id);
        assert_eq!(rearmed.ci_state, Some(CheckState::Pending), "{kind}: re-armed, or the watcher would never look again");
        assert_ne!(rearmed.ci_since, "", "{kind}: the stalled clock starts over");
        assert_eq!(rearmed.ci_reruns, 1, "{kind}");
        assert_eq!(rearmed.ci_rerun_sha, rearmed.ci_sha.clone().unwrap_or_default(), "{kind}");
        assert_eq!(journal(&r, "ci.rerun").len(), 1, "{kind}");
        assert_eq!(journal(&r, "ci.rerun")[0].payload["runs"].as_u64(), Some(1), "{kind}");
        // The row is watched again, and the restarted run is followed to its end.
        assert_eq!(r.h.app.with_server(|db| db.watched_deliveries()).unwrap().len(), 1, "{kind}");
        r.fake.lock().ci = "passed".into();
        genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
        assert_eq!(row(&r, &id).ci_state, Some(CheckState::Passed), "{kind}");
        assert_eq!(journal(&r, "ci.passed").len(), 1, "{kind}");

        // The same commit failed again: the owner's one-rerun-per-commit rule refuses a second one.
        r.fake.lock().ci = "failed".into();
        genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
        let (s, b) = rerun(&r, &id, &t.executor).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{kind}: {b}");
        assert!(b["error"].as_str().unwrap().contains("already rerun"), "{kind}: {b}");
        assert_eq!(r.fake.lock().reruns, 1, "{kind}: the host was not called again");

        // A moved head is a new commit: the per-commit rule allows it, the request limit counts it.
        r.fake.lock().prs[0].sha = "4444444444444444444444444444444444444444".into();
        genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
        let (s, b) = rerun(&r, &id, &t.executor).await;
        assert_eq!(s, StatusCode::OK, "{kind}: {b}");
        assert_eq!(b["used"]["request"].as_i64(), Some(2), "{kind}: {b}");
        assert_eq!(r.fake.lock().reruns, 2, "{kind}");
    })
    .await;
}

/// AC1/AC2: only a commit whose checks have actually failed can be rerun, and only once a commit is
/// watched at all. `stalled` keeps its own letter: a run nobody waits for is not restarted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_rerun_needs_a_failed_check_and_a_watched_commit() {
    let policy = json!({ "push": "branches", "change_request": { "open": false } });
    let r = rig_with("github", policy, |c| c.runtime.ci_pending_secs = 1).await;
    let t = team(&r, "Only when red").await;
    let id = t.task.clone();

    // Nothing pushed yet: there is no commit to rerun.
    let (s, b) = rerun(&r, &id, &t.executor).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert!(b["error"].as_str().unwrap().contains("push the branch first"), "{b}");

    assert_eq!(status(&r, &id, &t.executor, "in_progress").await.0, StatusCode::OK);
    commit_and_push(&t);
    for state in ["none", "pending", "passed"] {
        r.fake.lock().ci = state.into();
        genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
        let (s, b) = rerun(&r, &id, &t.executor).await;
        assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{state}: {b}");
        assert!(b["error"].as_str().unwrap().contains("not failed"), "{state}: {b}");
    }
    assert_eq!(r.fake.lock().reruns, 0, "nothing failed, nothing restarted");

    // Checks that stay running past `ciPendingSecs` become `stalled`: their letter says to settle
    // them on the host, and a rerun is refused (nothing may restart a run nobody is waiting for).
    r.fake.lock().ci = "pending".into();
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    assert_eq!(row(&r, &id).ci_state, Some(CheckState::Stalled));
    let (s, b) = rerun(&r, &id, &t.executor).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    // The host still says the run is going: only a failed one may be restarted.
    assert!(b["error"].as_str().unwrap().contains("not failed"), "{b}");
    assert_eq!(r.fake.lock().reruns, 0);
}

/// AC2: `runtime.ciRerunsPerRequest` caps one delivery, `runtime.ciRerunsPerTask` the whole task
/// (summed over its repositories).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_rerun_limits_stop_a_second_rerun() {
    // Per request: the second rerun of the same delivery is refused after the first.
    let r = rig_with("github", json!({}), |c| {
        c.runtime.ci_reruns_per_request = 1;
        c.runtime.ci_reruns_per_task = 5;
    })
    .await;
    let t = team(&r, "One rerun only").await;
    let id = t.task.clone();
    commit_and_push(&t);
    http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    r.fake.lock().ci = "failed".into();
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    assert_eq!(rerun(&r, &id, &t.executor).await.0, StatusCode::OK);

    r.fake.lock().ci = "failed".into();
    r.fake.lock().prs[0].sha = "5555555555555555555555555555555555555555".into();
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    let (s, b) = rerun(&r, &id, &t.executor).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert!(b["error"].as_str().unwrap().contains("ciRerunsPerRequest"), "{b}");
    assert_eq!(r.fake.lock().reruns, 1, "the host was not called again");

    // Per task: another delivery of the same task has spent the task's only rerun.
    let r = rig_with("github", json!({}), |c| {
        c.runtime.ci_reruns_per_request = 5;
        c.runtime.ci_reruns_per_task = 1;
    })
    .await;
    let t = team(&r, "One budget").await;
    let id = t.task.clone();
    commit_and_push(&t);
    http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    add_second_repo(&r, &id).await;
    r.h.app
        .with_server(|db| {
            Ok(db.conn().execute("UPDATE task_repos SET ci_reruns = 1 WHERE project = 'shop' AND task = ?1 AND repo = 'web'", [&id])?)
        })
        .unwrap();
    r.fake.lock().ci = "failed".into();
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    let (s, b) = rerun(&r, &id, &t.executor).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert!(b["error"].as_str().unwrap().contains("ciRerunsPerTask"), "{b}");
    assert_eq!(r.fake.lock().reruns, 0);
}

/// AC2: a host that refuses the rerun is reported, and the refusal must not eat the limit — the
/// reservation is given back so a fixed token can try again.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_host_refusing_a_rerun_does_not_burn_the_limit() {
    let r = rig("github", json!({})).await;
    let t = team(&r, "Bad token").await;
    let id = t.task.clone();
    commit_and_push(&t);
    http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    r.fake.lock().ci = "failed".into();
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();

    r.fake.lock().refuse_rerun = Some("Resource not accessible by personal access token".into());
    let (s, b) = rerun(&r, &id, &t.executor).await;
    assert_ne!(s, StatusCode::OK, "{b}");
    assert!(b["error"].as_str().unwrap().contains("token"), "the token's right is named: {b}");
    assert_eq!(r.fake.lock().reruns, 0);
    let released = row(&r, &id);
    assert_eq!((released.ci_reruns, released.ci_rerun_sha.as_str()), (0, ""), "the reservation was given back");

    // The token is fixed: the rerun goes through and counts once.
    let (s, b) = rerun(&r, &id, &t.executor).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!((r.fake.lock().reruns, row(&r, &id).ci_reruns), (1, 1));
}

/// AC2: `runtime.ciRerunsPerRequest: 0` switches reruns off: refused before the host is asked.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reruns_are_switched_off_by_the_config() {
    let r = rig_with("github", json!({}), |c| c.runtime.ci_reruns_per_request = 0).await;
    let t = team(&r, "No reruns").await;
    let id = t.task.clone();
    commit_and_push(&t);
    http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    r.fake.lock().ci = "failed".into();
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();

    let (s, b) = rerun(&r, &id, &t.executor).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert!(b["error"].as_str().unwrap().contains("switched off"), "{b}");
    assert_eq!(r.fake.lock().reruns, 0);
    assert!(!r.fake.lock().calls.iter().any(|c| c == "rerun"), "no rerun reached the host");
}

/// AC3: a plain git server has no rerun API; the command explains it instead of failing oddly.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_plain_host_cannot_rerun_checks() {
    let r = rig("github", json!({})).await;
    let t = team(&r, "Plain host").await;
    let id = t.task.clone();
    commit_and_push(&t);
    http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    // The host is a bare git server: no pull/merge request API, so no rerun either.
    let hosts = r.h.dir.path().join("hosts");
    let cfg = json!({ "hosts": { "h": { "kind": "plain", "url": r.fake.url,
        "clone_urls": { "https": format!("file://{}/{{remote}}.git", hosts.display()) } } } });
    std::fs::write(r.h.dir.path().join("git.json"), cfg.to_string()).unwrap();
    let (s, b) = rerun(&r, &id, &t.executor).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert!(b["error"].as_str().unwrap().contains("no pull/merge request API"), "{b}");
}

/// AC3: the GitHub failure that is a commit status or another app's check run has no rerun API; the
/// command says so, and nothing was restarted.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_github_failure_without_actions_runs_is_explained() {
    let r = rig("github", json!({})).await;
    let t = team(&r, "Status only").await;
    let id = t.task.clone();
    commit_and_push(&t);
    http(&r, "POST", &format!("/api/tasks/{id}/repos/api/cr"), Some(&t.executor), Some(json!({}))).await;
    {
        let mut f = r.fake.lock();
        f.ci = "failed".into();
        f.action_runs = false;
    }
    genie::git::delivery::watch_one(&r.h.app, &row(&r, &id)).await.unwrap();
    let (s, b) = rerun(&r, &id, &t.executor).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert!(b["error"].as_str().unwrap().contains("commit statuses"), "{b}");
    assert_eq!(r.fake.lock().reruns, 0);
    assert_eq!(row(&r, &id).ci_reruns, 0, "a host that cannot rerun spends no limit");
}
