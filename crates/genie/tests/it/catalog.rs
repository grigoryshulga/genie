//! The catalog of operations end to end: a command line and JSON arguments (as
//! the MCP server passes them) reach the server's API as the caller — the
//! operator of the server's machine, a person with a token or an agent — and
//! what comes back is rendered for people and models.

use crate::common;

use common::Harness;
use genie::ops::{self, Auth, Cx, InProcess, Out};
use genie_core::Role;
use genie_core::inbox::NewQuestion;
use genie_core::server_db::ProjectRole;
use serde_json::{Value, json};

fn cx(h: &Harness, auth: Auth, project: Option<&str>) -> Cx {
    let api = InProcess::new(genie::http::router(h.app.clone()), h.app.cfg.port, auth, project.map(str::to_string));
    Cx { api: Box::new(api), project: project.map(str::to_string), task: None, team: None, local: false }
}

/// A person of the project with a personal token.
fn person(h: &Harness, login: &str, role: ProjectRole) -> (i64, Cx) {
    let u = h.app.with_server(|db| db.create_user(login, login, None, Some("password-1"), false)).unwrap();
    h.app.with_server(|db| db.set_membership("shop", u.id, role)).unwrap();
    let token = h.app.with_server(|db| db.create_user_token(u.id, "test")).unwrap();
    (u.id, cx(h, Auth::Bearer(token), Some("shop")))
}

fn agent(h: &Harness, role: Role, name: &str) -> Cx {
    let token = h.app.with_server(|db| db.create_agent_token("shop", role, name, None, None, chrono::Duration::hours(1))).unwrap();
    cx(h, Auth::Bearer(token), Some("shop"))
}

/// An operation with JSON arguments, as the MCP server runs it.
async fn op(cx: &Cx, group: &str, name: &str, args: Value) -> Result<Out, String> {
    ops::find(group, name).unwrap_or_else(|| panic!("no operation {group} {name}")).run_json(args, cx).await
}

/// A command line, as `genie …` runs it.
async fn cli(cx: &Cx, line: &[&str]) -> Result<Out, String> {
    let m = genie::cli::command().try_get_matches_from(std::iter::once("genie").chain(line.iter().copied())).map_err(|e| e.to_string())?;
    let (entry, args) = ops::chosen(&m).expect("an operation of the catalog");
    entry.run_cli(args, cx).await
}

#[tokio::test]
async fn admins_change_the_agent_configuration_and_everyone_reads_it() {
    let h = Harness::new();
    h.project("shop");
    let admin = cx(&h, Auth::Operator, None);

    let role = "---\nextends: reviewer\ntitle: Security reviewer\n---\nLook for injections.\n";
    let out = cli(&admin, &["agents", "save", "role:security-reviewer", "--text", role]).await.unwrap();
    assert!(out.text.starts_with("saved agents/security-reviewer.md · hash "), "{}", out.text);
    let shown = op(&admin, "agents", "show", json!({ "item": "role:security-reviewer" })).await.unwrap();
    assert!(
        shown.text.starts_with("role security-reviewer — Security reviewer\nclass reviewer · custom · extends reviewer"),
        "{}",
        shown.text
    );
    assert!(shown.text.ends_with("Look for injections.\n"), "the file itself comes last: {}", shown.text);
    let hash = shown.data["file"]["hash"].as_str().unwrap().to_string();

    let stale = op(&admin, "agents", "save", json!({ "item": "role:security-reviewer", "text": "---\n---\nv2", "baseHash": "0000" })).await;
    assert!(stale.unwrap_err().contains("changed since you opened it"));
    let v2 = "---\nextends: reviewer\n---\nv2\n";
    op(&admin, "agents", "save", json!({ "item": "role:security-reviewer", "text": v2, "baseHash": hash })).await.unwrap();
    let broken = op(&admin, "agents", "save", json!({ "item": "role:ghost", "text": "---\nextends: nobody\n---\n" })).await;
    assert!(broken.unwrap_err().contains("not saved: role:ghost"), "a broken file is refused before it is written");
    let history = cli(&admin, &["agents", "history", "--item", "role:security-reviewer"]).await.unwrap();
    let lines: Vec<&str> = history.text.lines().collect();
    assert!(lines[0].contains("changed agents/security-reviewer.md") && lines[1].contains("created"), "{}", history.text);

    // A skill and one of its files; a file keeps its bytes.
    std::fs::create_dir_all(h.app.data.join("skills/deploy")).unwrap();
    std::fs::write(h.app.data.join("skills/deploy/SKILL.md"), "---\nname: deploy\ndescription: Deploy the shop\n---\nRun it.\n").unwrap();
    h.app.reload_agents();
    let saved =
        op(&admin, "agents", "save", json!({ "item": "skill:deploy/scripts/run.sh", "text": "#!/bin/sh\necho hi\n" })).await.unwrap();
    assert_eq!(saved.text, "saved skills/deploy/scripts/run.sh (18 bytes)");
    let skill = cli(&admin, &["agents", "show", "skill:deploy"]).await.unwrap();
    assert!(skill.text.contains("files: SKILL.md, scripts/run.sh"), "{}", skill.text);
    let file = cli(&admin, &["agents", "show", "skill:deploy/scripts/run.sh"]).await.unwrap();
    assert!(file.text.ends_with("echo hi\n"), "{}", file.text);

    // A person of the project reads the configuration; only server admins change it.
    let (_, vic) = person(&h, "vic", ProjectRole::Admin);
    let reviewer = cli(&vic, &["agents", "show", "role:reviewer"]).await.unwrap();
    assert!(reviewer.text.contains("built in, no file yet"), "{}", reviewer.text);
    let refused = cli(&vic, &["agents", "save", "role:reviewer", "--text", "---\n---\nmine"]).await;
    assert!(refused.unwrap_err().contains("server admin rights required"));
    assert!(cli(&vic, &["agents", "check"]).await.is_err(), "the check shows the whole server");

    // The check reads the files as they are now and fails on errors.
    let check = cli(&admin, &["agents", "check"]).await.unwrap();
    assert!(check.failed.is_none() && check.text.contains("no problems"), "{}", check.text);
    std::fs::write(h.app.data.join("agents/broken.md"), "---\nextends: nobody\n---\nReview.\n").unwrap();
    // Not yet seen by the server (its watcher reloads within seconds), already by the check.
    let check = cli(&admin, &["agents", "check"]).await.unwrap();
    assert!(check.text.contains("role:broken"), "{}", check.text);
    assert!(check.failed.as_deref().unwrap_or_default().starts_with("1 error(s) in the agent configuration"), "{:?}", check.failed);
    let list = cli(&admin, &["agents", "ls"]).await.unwrap();
    assert!(list.text.contains("security-reviewer") && list.text.contains("Team templates:\n  abap"), "{}", list.text);
    assert!(list.text.contains("Skills: deploy") && list.text.ends_with("problem(s): genie agents check"), "{}", list.text);

    h.app.reload_agents();
    let deleted = cli(&admin, &["agents", "delete", "role:security-reviewer"]).await.unwrap();
    assert!(deleted.text.starts_with("deleted role:security-reviewer"));
    assert!(!h.app.data.join("agents/security-reviewer.md").exists());
    assert!(cli(&admin, &["agents", "delete", "mcp"]).await.unwrap_err().contains("save it without the connection"));
}

#[tokio::test]
async fn people_answer_the_questions_they_were_asked_and_read_their_notifications() {
    let h = Harness::new();
    h.project("shop");
    let admin = cx(&h, Auth::Operator, Some("shop"));
    cli(&admin, &["task", "create", "CSV export"]).await.unwrap();
    let (anna_id, anna) = person(&h, "anna", ProjectRole::Member);
    let (_, bob) = person(&h, "bob", ProjectRole::Member);
    let questions = [
        NewQuestion { text: "CSV or XLSX?".into(), why: "the library".into(), options: vec!["CSV".into(), "XLSX".into()] },
        NewQuestion { text: "Who gets the file?".into(), ..Default::default() },
    ];
    let (qn, _) = h
        .app
        .with_server(|db| db.create_questionnaire("shop", Some("G-1"), "analyst", anna_id, "web", &questions, None, None, None))
        .unwrap();

    let waiting = cli(&anna, &["me", "questions"]).await.unwrap();
    assert!(waiting.text.contains("1. CSV or XLSX?\n     why: the library\n     options: CSV | XLSX"), "{}", waiting.text);
    let id = qn.id.to_string();
    let first = cli(&anna, &["me", "answer", &id, "1=XLSX"]).await.unwrap();
    assert!(first.text.contains("→ XLSX") && first.text.ends_with("some questions are still open"), "{}", first.text);
    assert!(cli(&bob, &["me", "answer", &id, "2=me"]).await.unwrap_err().contains("no such questionnaire for you"));
    let done = op(&anna, "me", "answer", json!({ "questionnaire": qn.id, "answers": ["2=Accounting"] })).await.unwrap();
    assert!(done.text.ends_with("the answers went to the task"), "{}", done.text);
    let task = h.app.with_tracker("shop", |t| t.get("G-1")).unwrap();
    let answers = task.comments.iter().find(|c| c.text.starts_with("Ответы на вопросы")).expect("the answers reach the task");
    assert!(answers.author == "anna" && answers.text.contains("XLSX") && answers.text.contains("Accounting"));
    assert!(cli(&anna, &["me", "answer", &id, "1=CSV"]).await.unwrap_err().contains("is answered"), "a closed questionnaire stays closed");

    h.app
        .with_server(|db| db.add_notification(anna_id, Some("shop"), Some("G-1"), "task", "G-1 готова", "Описан экспорт", None, None))
        .unwrap();
    let unread = cli(&anna, &["me", "notifications"]).await.unwrap();
    assert!(unread.text.contains("● ") && unread.text.contains("[shop] G-1 готова\n       Описан экспорт"), "{}", unread.text);
    assert!(unread.text.ends_with("1 unread"));
    cli(&anna, &["me", "read"]).await.unwrap();
    assert_eq!(cli(&anna, &["me", "notifications"]).await.unwrap().text, "nothing unread");
    let me = cli(&anna, &["me", "show"]).await.unwrap();
    assert!(me.text.starts_with("anna (anna)\n  * shop "), "{}", me.text);

    // Agents have no inbox of a person.
    let orch = agent(&h, Role::Orchestrator, "orchestrator");
    assert_eq!(cli(&orch, &["me", "show"]).await.unwrap().text, "agent orchestrator (orchestrator) of project shop");
    assert!(cli(&orch, &["me", "notifications"]).await.unwrap_err().contains("agents cannot do this"));
}

#[tokio::test]
async fn automations_jobs_and_the_journal_through_the_catalog() {
    let h = Harness::new();
    h.project("shop");
    let admin = cx(&h, Auth::Operator, Some("shop"));

    assert!(cli(&admin, &["automation", "playbooks"]).await.unwrap().text.contains("task-done-knowledge"));
    let installed = cli(&admin, &["automation", "install", "task-done-knowledge"]).await.unwrap();
    assert!(installed.text.starts_with("installed #1    enabled"), "{}", installed.text);
    // The spec as JSON itself (MCP) or as its text (the command line).
    let nightly = json!({ "name": "Nightly", "on": { "schedule": "0 3 * * *", "tz": "Europe/Moscow" }, "steps": [{ "id": "n", "notify": { "to": "owners", "text": "hi" } }] });
    let created = op(&admin, "automation", "create", json!({ "spec": nightly })).await.unwrap();
    assert!(created.text.ends_with("Nightly  on schedule 0 3 * * * Europe/Moscow"), "{}", created.text);
    let hook = json!({ "name": "Hook", "on": { "webhook": { "secret": "0123456789abcdef0123" } }, "steps": [{ "id": "n", "notify": { "to": "owners", "text": "hi" } }] });
    let created = cli(&admin, &["automation", "create", "--spec", &hook.to_string()]).await.unwrap();
    assert!(created.text.ends_with("Hook  on webhook"), "{}", created.text);
    cli(&admin, &["automation", "disable", "1"]).await.unwrap();
    let list = cli(&admin, &["automation", "list"]).await.unwrap();
    let lines: Vec<&str> = list.text.lines().collect();
    assert_eq!(lines.len(), 3, "{}", list.text);
    assert!(lines[0].starts_with("#1    off     Задача закрыта"), "{}", list.text);
    let (_, vic) = person(&h, "vic", ProjectRole::Member);
    assert!(cli(&vic, &["automation", "enable", "1"]).await.is_err(), "project admins change automations");

    let started = cli(&admin, &["job", "start", "--role", "documenter", "Describe the export", "--input", "page=export"]).await.unwrap();
    assert!(started.text.starts_with("started #1     queued     documenter"), "{}", started.text);
    let shown = cli(&admin, &["job", "show", "1"]).await.unwrap();
    assert!(shown.text.contains("Describe the export") && shown.text.contains("\"page\": \"export\""), "{}", shown.text);
    assert_eq!(cli(&admin, &["job", "ls"]).await.unwrap().text.lines().count(), 1);
    let executor = agent(&h, Role::Executor, "bender");
    let refused = op(&executor, "job", "start", json!({ "role": "documenter", "goal": "x" })).await;
    assert!(refused.unwrap_err().contains("only the orchestrator (or a person) starts jobs"));

    cli(&admin, &["task", "create", "CSV export"]).await.unwrap();
    let events = cli(&admin, &["project", "events", "--limit", "2"]).await.unwrap();
    let lines: Vec<&str> = events.text.lines().collect();
    assert_eq!(lines.len(), 2, "the most recent ones: {}", events.text);
    assert!(lines[0].contains("task.created") && lines[0].contains("G-1 "), "{}", events.text);
    let first = events.data["events"][0]["id"].as_i64().unwrap();
    let after = op(&admin, "project", "events", json!({ "after": first })).await.unwrap();
    assert_eq!(after.data["events"][0]["id"].as_i64(), Some(first + 1), "paging goes on after the last one printed");
}

#[tokio::test]
async fn owners_decide_on_the_pages_agents_propose() {
    let h = Harness::new();
    h.project("shop");
    let admin = cx(&h, Auth::Operator, Some("shop"));
    let documenter = agent(&h, Role::Documenter, "documenter");

    let page = "---\ntitle: Export\ntype: guide\nstatus: current\n---\n# Export\n\nOrders go out as CSV.\n";
    let proposed =
        op(&documenter, "docs", "write", json!({ "path": "shop/features/export.md", "text": page, "note": "export docs" })).await.unwrap();
    assert!(proposed.text.starts_with("proposal #1 created"), "agents' pages go to review by default: {}", proposed.text);
    let open = cli(&admin, &["docs", "proposals"]).await.unwrap();
    assert!(open.text.starts_with("#1    open      shop/features/export.md  by documenter (agent) — export docs"), "{}", open.text);
    let one = cli(&admin, &["docs", "proposal", "1"]).await.unwrap();
    assert!(one.text.contains("## Proposed\n---\ntitle: Export") && one.text.contains("(the page does not exist yet)"), "{}", one.text);
    assert!(cli(&documenter, &["docs", "approve", "1"]).await.is_err(), "agents do not decide");
    let approved = cli(&admin, &["docs", "approve", "1", "--note", "good"]).await.unwrap();
    assert_eq!(approved.text, "proposal #1 approved: shop/features/export.md changed");
    assert!(cli(&admin, &["docs", "read", "shop/features/export.md"]).await.unwrap().text.contains("Orders go out as CSV."));

    op(&documenter, "docs", "write", json!({ "path": "shop/features/export.md", "text": page.replace("CSV", "XLSX") })).await.unwrap();
    let rejected = op(&admin, "docs", "reject", json!({ "id": 2, "note": "we stay with CSV" })).await.unwrap();
    assert_eq!(rejected.text, "proposal #2 rejected");
    assert!(cli(&admin, &["docs", "proposals", "--status", "all"]).await.unwrap().text.contains("#2    rejected"));

    let space = cli(&admin, &["docs", "space", "shop", "--owner", "anna", "--agents", "direct"]).await.unwrap();
    assert_eq!(space.text, "shop/  project shop · owners anna · people direct, agents direct");
    let direct = op(&documenter, "docs", "write", json!({ "path": "shop/features/import.md", "text": page })).await.unwrap();
    assert_eq!(direct.text, "shop/features/import.md saved", "the space now lets agents publish directly");
    assert!(cli(&admin, &["docs", "spaces"]).await.unwrap().text.starts_with("shop/  project shop · owners anna"));
    assert!(cli(&admin, &["docs", "changelog"]).await.unwrap().text.starts_with("shop/changelog.md"));
}
