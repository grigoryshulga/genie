//! Behaviour of both managers through the public application and HTTP entrances.
use crate::common::{Harness, call};
use axum::http::StatusCode;
use genie::{
    management,
    tasks::{self, Caller, CreateBody, StatusBody},
};
use genie_core::management::manager_name;
use genie_core::team::{NewMember, NewTeam, ORCHESTRATOR, SendMail};
use genie_core::{Actor, CreateInput, Role, Status, StatusOptions, UpdateInput};
use serde_json::json;

fn harness() -> Harness {
    let h = Harness::new();
    h.project("shop");
    h
}
fn ready(h: &Harness) -> String {
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
fn manager(h: &Harness, id: &str) -> String {
    management::dispatch(&h.app, "shop", &Actor::new("anna", Role::Human), id).unwrap();
    h.app
        .with_server(|db| {
            db.create_role_token(
                "shop",
                Role::Orchestrator,
                Some("orchestrator"),
                &manager_name(id),
                None,
                None,
                chrono::Duration::hours(1),
            )
        })
        .unwrap()
}
fn team(h: &Harness, id: &str) {
    h.app
        .with_tracker("shop", |t| {
            t.bus().create(
                "anna",
                "human",
                NewTeam {
                    id: id.into(),
                    task: id.into(),
                    cwd: "/tmp".into(),
                    members: vec![NewMember { name: "worker".into(), role: "executor".into(), ..Default::default() }],
                    ..Default::default()
                },
            )?;
            t.assign_team(&Actor::new("anna", Role::Human), id, Some(id), None, None)
        })
        .unwrap();
}

#[test]
fn dispatch_is_idempotent_and_rechecks_readiness_and_dependencies() {
    let h = harness();
    let id = ready(&h);
    let anna = Actor::new("anna", Role::Human);
    let first = management::dispatch(&h.app, "shop", &anna, &id).unwrap();
    h.app.with_tracker("shop", |t| t.set_status(&anna, &id, Status::InProgress, StatusOptions::default())).unwrap();
    let second = management::dispatch(&h.app, "shop", &anna, &id).unwrap();
    assert_eq!(first.created, second.created);
    assert_eq!(h.app.with_tracker("shop", |t| t.management().managers()).unwrap().len(), 1);
    let blocked = ready(&h);
    h.app.with_tracker("shop", |t| t.update(&anna, &blocked, UpdateInput { add_deps: vec![id.clone()], ..Default::default() })).unwrap();
    assert!(management::dispatch(&h.app, "shop", &anna, &blocked).is_err());
    let empty =
        tasks::create(&h.app, &Caller::person("shop", "anna"), CreateBody { title: "Incomplete".into(), ..Default::default() }).unwrap();
    tasks::set_status(&h.app, &Caller::person("shop", "anna"), &empty.id, StatusBody::to(Status::Ready, None)).unwrap();
    assert!(management::dispatch(&h.app, "shop", &anna, &empty.id).is_err());
}

#[tokio::test]
async fn task_managers_have_separate_mail_and_cannot_mutate_other_tasks_or_teams() {
    let h = harness();
    let a = ready(&h);
    let b = ready(&h);
    let token = manager(&h, &a);
    manager(&h, &b);
    team(&h, &a);
    team(&h, &b);
    h.app
        .with_tracker("shop", |t| {
            for id in [&a, &b] {
                t.bus().send(SendMail {
                    team: id,
                    from: "worker",
                    from_role: "executor",
                    to: ORCHESTRATOR,
                    text: id,
                    kind: "message",
                    ..Default::default()
                })?;
            }
            Ok(())
        })
        .unwrap();
    let (s, v, _) = call(&h.router, "POST", "/api/agent/inbox/lease").bearer(&token).json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert!(v["text"].as_str().unwrap().contains(&a));
    assert!(!v["text"].as_str().unwrap().contains(&b));
    for (method, path, body) in [
        ("PATCH", format!("/api/tasks/{b}"), json!({"notes":"stolen"})),
        ("POST", format!("/api/teams/{b}/stop"), json!({})),
        ("POST", format!("/api/teams/{b}/members"), json!({"role":"analyst"})),
        ("POST", format!("/api/management/tasks/{b}/retry"), json!({})),
    ] {
        let (s, v, _) = call(&h.router, method, &path).bearer(&token).json(body).send().await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{path}: {v}");
    }
    let (s, _, _) =
        call(&h.router, "PATCH", &format!("/api/tasks/{a}")).bearer(&token).json(json!({"description":"new requirements"})).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    // A routed ask can be answered by its actual manager address.
    h.app
        .with_tracker("shop", |t| {
            let mail = t.bus().send(SendMail {
                team: &a,
                from: "worker",
                from_role: "executor",
                to: ORCHESTRATOR,
                text: "Question",
                kind: "message",
                awaits: true,
                ..Default::default()
            })?;
            let replies = t.bus().reply(&manager_name(&a), "orchestrator", mail[0].id, "Answer")?;
            assert_eq!(replies[0].to, "worker");
            let answer = t.bus().send(SendMail {
                team: &a,
                from: &manager_name(&a),
                from_role: "orchestrator",
                to: "worker",
                text: "Question back",
                kind: "message",
                ..Default::default()
            })?;
            assert_eq!(t.bus().reply("worker", "executor", answer[0].id, "Answer back")?[0].to, manager_name(&a));
            Ok(())
        })
        .unwrap();
}

#[tokio::test]
async fn clarification_is_routed_through_project_manager_and_old_answers_are_refused() {
    let h = harness();
    let id = ready(&h);
    let token = manager(&h, &id);
    let path = format!("/api/management/tasks/{id}");
    let (s, q, _) = call(&h.router, "POST", &format!("{path}/clarify")).bearer(&token).json(json!({"question":"Retention?"})).send().await;
    assert_eq!(s, StatusCode::OK, "{q}");
    let qid = q["id"].as_i64().unwrap();
    let project_mail = h.app.with_tracker("shop", |t| t.bus().pending(None, ORCHESTRATOR)).unwrap();
    assert!(project_mail.iter().any(|m| m.text.contains("Retention?")));
    let (s, v, _) = call(&h.router, "POST", &format!("{path}/answer"))
        .bearer(&token)
        .json(json!({"id":qid,"text":"30 days","source":"policy"}))
        .send()
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let pm = h
        .app
        .with_server(|db| {
            db.create_role_token(
                "shop",
                Role::Orchestrator,
                Some("project-manager"),
                "orchestrator",
                None,
                None,
                chrono::Duration::hours(1),
            )
        })
        .unwrap();
    let (s, v, _) = call(&h.router, "POST", &format!("{path}/answer")).bearer(&pm).json(json!({"id":qid,"text":"30 days"})).send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    let (s, v, _) = call(&h.router, "POST", &format!("{path}/escalate")).json(json!({"id":qid})).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(h.app.with_tracker("shop", |t| Ok(t.get(&id)?.status)).unwrap(), Status::NeedsOwner);
    let (s, v, _) =
        call(&h.router, "POST", &format!("{path}/answer")).json(json!({"id":qid,"text":"30 days","source":"Owner response"})).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["clarifications"][0]["state"], "answered");
    assert_eq!(v["task"]["status"], "ready");
    let (_, q, _) =
        call(&h.router, "POST", &format!("{path}/clarify")).bearer(&token).json(json!({"question":"Which encoding?"})).send().await;
    h.app
        .with_tracker("shop", |t| {
            t.update(&Actor::new("anna", Role::Human), &id, UpdateInput { description: Some("New format".into()), ..Default::default() })
        })
        .unwrap();
    let (s, v, _) =
        call(&h.router, "POST", &format!("{path}/answer")).json(json!({"id":q["id"],"text":"UTF-8","source":"old decision"})).send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(h.app.with_tracker("shop", |t| Ok(t.management().brief(&id, None)?["version"].clone())).unwrap(), 2);
}

#[tokio::test]
async fn closure_unblocks_dependants_and_retrospective_survives_restart_separately() {
    let h = harness();
    let id = ready(&h);
    manager(&h, &id);
    team(&h, &id);
    let child = ready(&h);
    h.app
        .with_tracker("shop", |t| {
            t.update(&Actor::new("anna", Role::Human), &child, UpdateInput { add_deps: vec![id.clone()], ..Default::default() })
        })
        .unwrap();
    tasks::set_status(&h.app, &Caller::person("shop", "anna"), &id, StatusBody::to(Status::Done, None)).unwrap();
    let state = management::show(&h.app, "shop", &id).unwrap();
    assert_eq!(state["manager"]["state"], "retrospective");
    assert_eq!(state["task"]["status"], "done");
    assert!(management::dispatch(&h.app, "shop", &Actor::new("anna", Role::Human), &child).is_ok());
    assert_eq!(h.app.with_tracker("shop", |t| Ok(t.bus().get(&id)?.state)).unwrap(), genie_core::TeamState::Stopped);
    let cfg = genie::config::Config::load(h.dir.path()).unwrap();
    let restarted = genie::state::App::open(h.dir.path(), cfg, std::path::PathBuf::from("/nonexistent")).unwrap();
    management::reconcile(&restarted, "shop").unwrap();
    assert_eq!(management::show(&restarted, "shop", &id).unwrap()["manager"]["state"], "retrospective");
    let report = management::retrospective(
        &restarted,
        "shop",
        &Actor::new(manager_name(&id), Role::Orchestrator),
        &id,
        "No execution occurred; closed by owner. No prompt changes proposed.",
        management::Coverage::Reviewed(&[]),
    )
    .unwrap();
    assert_eq!(report.state, "completed");
    let again = management::retrospective(
        &restarted,
        "shop",
        &Actor::new(manager_name(&id), Role::Orchestrator),
        &id,
        "retry",
        management::Coverage::Reviewed(&[]),
    )
    .unwrap();
    assert_eq!(report.report, again.report);
    assert_eq!(restarted.with_tracker("shop", |t| Ok(t.get(&id)?.artifacts.len())).unwrap(), 1);
}

#[test]
fn retrospective_requires_source_coverage_and_keeps_failed_and_removed_attempts() {
    let h = harness();
    let id = ready(&h);
    manager(&h, &id);
    team(&h, &id);
    let turn = h
        .app
        .with_server(|db| {
            let id = db.start_turn("shop", "former-worker", Some(&id), Some("former-worker"), None)?;
            db.finish_turn(id, "failed", Some(1), Some("error"), Some("full failed attempt"))?;
            Ok(id)
        })
        .unwrap();
    tasks::set_status(&h.app, &Caller::person("shop", "anna"), &id, StatusBody::to(Status::Done, None)).unwrap();
    h.app.with_tracker("shop", |t| t.bus().delete(&id)).unwrap();
    let actor = Actor::new(manager_name(&id), Role::Orchestrator);
    assert_eq!(management::sources(&h.app, "shop", &id).unwrap()["sources"][0]["id"], turn.to_string());
    assert!(management::retrospective(&h.app, "shop", &actor, &id, "Report", management::Coverage::Reviewed(&[])).is_err());
    let partial =
        management::retrospective(&h.app, "shop", &actor, &id, "Missing source review", management::Coverage::Incomplete(&[])).unwrap();
    assert_eq!(partial.state, "retrospective");
    let source = management::source(&h.app, "shop", &id, &turn.to_string(), 0).unwrap();
    assert_eq!(source["text"], "full failed attempt");
    let complete = management::retrospective(
        &h.app,
        "shop",
        &actor,
        &id,
        "Observed failure; proposed retry change; verify on next task.",
        management::Coverage::Reviewed(&[turn.to_string()]),
    )
    .unwrap();
    assert_eq!(complete.state, "completed");
    assert!(management::source(&h.app, "shop", &ready(&h), &turn.to_string(), 0).is_err());
}

#[test]
fn project_memory_preserves_sources_and_replacement_history() {
    let h = harness();
    let anna = Actor::new("anna", Role::Human);
    h.app
        .with_tracker("shop", |t| {
            assert!(t.management().remember(&anna, "CSV", "", None).is_err());
            let first = t.management().remember(&anna, "CSV export", "task:G-1", None)?;
            t.management().remember(&anna, "XLSX export", "task:G-2", Some(first))?;
            assert!(t.management().memory("CSV")?.is_empty());
            let rows = t.management().memory("export")?;
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["source"], "task:G-2");
            Ok(())
        })
        .unwrap();
}

#[test]
fn manual_mode_prevents_automatic_dispatch_but_allows_explicit_human_release() {
    let h = harness();
    let id = ready(&h);
    h.app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    assert!(management::dispatch(&h.app, "shop", &Actor::new("orchestrator", Role::Orchestrator), &id).is_err());
    assert!(management::dispatch(&h.app, "shop", &Actor::new("anna", Role::Human), &id).is_ok());
}

#[test]
fn a_manager_that_stops_without_its_retrospective_is_retried() {
    let h = harness();
    let id = ready(&h);
    manager(&h, &id);
    tasks::set_status(&h.app, &Caller::person("shop", "anna"), &id, StatusBody::to(Status::Done, None)).unwrap();
    h.app
        .with_tracker("shop", |t| {
            t.bus().lease(None, &manager_name(&id), 99)?;
            t.bus().complete_lease(99)?;
            Ok(())
        })
        .unwrap();
    let key = genie::runtime::AgentKey::TaskManager { project: "shop".into(), task: id.clone() };
    let verdict = genie::outcome::record(&h.app, &key, Some(&id), genie::outcome::Outcome::Ran);
    assert_eq!(verdict.failures, 1);
    assert!(!h.app.with_tracker("shop", |t| t.bus().pending(None, &manager_name(&id))).unwrap().is_empty());
}

#[test]
fn an_existing_team_is_adopted_without_restarting_it_or_losing_mail() {
    let h = harness();
    let id = ready(&h);
    team(&h, &id);
    h.app.with_tracker("shop", |t| t.bus().notify_orchestrator("worker", "executor", "message", "Need a decision", Some(&id))).unwrap();
    management::reconcile(&h.app, "shop").unwrap();
    management::reconcile(&h.app, "shop").unwrap();
    h.app
        .with_tracker("shop", |t| {
            assert_eq!(t.management().managers()?.len(), 1);
            assert_eq!(t.bus().list(false)?.len(), 1);
            let mail = t.bus().pending(None, &manager_name(&id))?;
            assert_eq!(mail.iter().filter(|m| m.text == "Need a decision").count(), 1);
            assert_eq!(mail.iter().filter(|m| m.text.contains("You own this task")).count(), 1);
            Ok(())
        })
        .unwrap();
}

#[test]
fn all_archived_messages_remain_readable_after_the_team_stops() {
    let h = harness();
    let id = ready(&h);
    manager(&h, &id);
    team(&h, &id);
    let agent = format!("{id}/former-worker");
    h.app
        .with_tracker("shop", |t| {
            for n in 0..25 {
                t.management().record_execution(&id, &agent, &json!({ "type": "message_end", "message": format!("Message {n}") }))?;
            }
            Ok(())
        })
        .unwrap();
    tasks::set_status(&h.app, &Caller::person("shop", "anna"), &id, StatusBody::to(Status::Done, None)).unwrap();
    let source_id = format!("session:{agent}");
    let first = management::source(&h.app, "shop", &id, &source_id, 0).unwrap();
    assert_eq!(first["events"].as_array().unwrap().len(), 20);
    let next = first["nextOffset"].as_u64().unwrap() as usize;
    let second = management::source(&h.app, "shop", &id, &source_id, next).unwrap();
    assert_eq!(second["events"].as_array().unwrap().len(), 5);
    assert!(second["events"][4]["content"].as_str().unwrap().contains("Message 24"));
}

#[test]
fn project_memory_search_handles_russian_case() {
    let h = harness();
    h.app
        .with_tracker("shop", |t| {
            t.management().remember(&Actor::new("anna", Role::Human), "Экспорт в CSV", "task:G-1", None)?;
            assert_eq!(t.management().memory("экспорт")?.len(), 1);
            Ok(())
        })
        .unwrap();
}

#[test]
fn project_manager_inherits_the_configured_model_and_can_be_configured_separately() {
    let h = harness();
    assert_eq!(h.app.agents().roles["project-manager"].model.as_deref(), Some("fake/orchestrator"));
    std::fs::write(h.dir.path().join("agents/project-manager.md"), "---\nmodel: fake/project\n---\n").unwrap();
    h.app.reload_agents();
    assert_eq!(h.app.agents().roles["project-manager"].model.as_deref(), Some("fake/project"));
    assert_eq!(h.app.agents().roles["orchestrator"].model.as_deref(), Some("fake/orchestrator"));
}
