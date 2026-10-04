//! Task commands hold the server's rules for every caller: an automation is held to the
//! orchestrator's rules, and its comments notify the people they name.

use std::path::PathBuf;
use std::sync::Arc;

use genie::config::Config;
use genie::state::{App, AppError};
use genie::tasks::{self, Caller, CommentBody, CreateBody, StatusBody};
use genie_core::server_db::ProjectRole;
use genie_core::{Actor, Role, Status};
use serde_json::json;

fn app() -> (tempfile::TempDir, Arc<App>) {
    let dir = tempfile::tempdir().unwrap();
    let app = App::open(dir.path(), Config::load(dir.path()).unwrap(), PathBuf::from("/nonexistent")).unwrap();
    app.create_project("shop", "Shop", None, None, None).unwrap();
    (dir, app)
}

fn automation() -> Caller {
    Caller::automation("shop", Actor::new("automation:close:1", Role::Orchestrator))
}

/// A task a person has put in progress.
fn working_task(app: &App) -> String {
    let anna = Caller::person("shop", "anna");
    let task = tasks::create(app, &anna, CreateBody { title: "CSV export".into(), ..Default::default() }).unwrap();
    tasks::set_status(app, &anna, &task.id, StatusBody::to(Status::InProgress, None)).unwrap();
    task.id
}

#[test]
fn an_automation_does_not_close_a_task_in_an_assisted_project() {
    let (_d, app) = app();
    app.with_server(|db| db.set_autonomy("shop", "assisted")).unwrap();
    let id = working_task(&app);
    let done = StatusBody { force: Some(true), ..StatusBody::to(Status::Done, Some("finished".into())) };
    match tasks::set_status(&app, &automation(), &id, done) {
        Err(AppError::Conflict(m)) => assert!(m.contains("assisted"), "{m}"),
        other => panic!("an automation closed the task: {other:?}"),
    }
    // A person closes it.
    let t = tasks::set_status(&app, &Caller::person("shop", "anna"), &id, StatusBody::to(Status::Done, None)).unwrap();
    assert_eq!(t.status, Status::Done);
}

#[test]
fn an_automation_does_not_move_a_task_to_review_past_failed_checks() {
    let (_d, app) = app();
    let id = working_task(&app);
    app.with_server(|db| {
        db.conn().execute(
            "INSERT INTO task_repos(project, task, repo, access, branch, state, ci_state, updated) VALUES ('shop', ?1, 'app', 'write', 'genie/S-1', 'published', 'failed', '')",
            [&id],
        )?;
        Ok(())
    })
    .unwrap();
    let review = StatusBody { force: Some(true), ..StatusBody::to(Status::Review, None) };
    match tasks::set_status(&app, &automation(), &id, review) {
        Err(AppError::Conflict(m)) => assert!(m.contains("checks of `genie/S-1` in app failed"), "{m}"),
        other => panic!("the task went to review past a red CI: {other:?}"),
    }
    // People decide for themselves.
    let t = tasks::set_status(&app, &Caller::person("shop", "anna"), &id, StatusBody::to(Status::Review, None)).unwrap();
    assert_eq!(t.status, Status::Review);
}

#[test]
fn an_automations_comment_notifies_the_people_it_names() {
    let (_d, app) = app();
    let bob = app
        .with_server(|db| {
            let bob = db.create_user("bob", "Bob", None, Some("bob-secret-1"), false)?;
            db.set_membership("shop", bob.id, ProjectRole::Member)?;
            Ok(bob)
        })
        .unwrap();
    let id = working_task(&app);
    std::thread::sleep(std::time::Duration::from_millis(5));
    // Through the engine: a rule comments on every new task.
    let spec = json!({ "name": "Ping bob", "on": { "event": "task.created" }, "steps": [{ "id": "c", "task.comment": { "text": "@bob, a new task" } }] });
    app.with_server(|db| db.create_automation("shop", &spec, "anna")).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    let second = tasks::create(&app, &Caller::person("shop", "anna"), CreateBody { title: "Import".into(), ..Default::default() }).unwrap();
    genie::engine::tick(&app).unwrap();
    // And directly.
    tasks::comment(&app, &automation(), &id, CommentBody { text: "@bob, look".into(), kind: None }).unwrap();
    let mut tasks_named: Vec<Option<String>> = app
        .with_server(|db| db.notifications(bob.id, false, 10))
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == "mention")
        .map(|n| n.task)
        .collect();
    tasks_named.sort();
    assert_eq!(tasks_named, vec![Some(id), Some(second.id)]);
}

#[test]
fn a_request_with_a_key_nobody_knows_is_refused() {
    let err = serde_json::from_value::<CreateBody>(json!({ "title": "x", "plann": "typo" })).unwrap_err().to_string();
    assert!(err.contains("plann"), "{err}");
    let body: CreateBody = serde_json::from_value(json!({ "title": "x", "plan": "1. do it", "priority": "1", "labels": "ui" })).unwrap();
    assert_eq!((body.plan.as_deref(), body.priority, body.labels), (Some("1. do it"), Some(1), Some(vec!["ui".to_string()])));
}
