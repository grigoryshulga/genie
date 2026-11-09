use crate::common::{Harness, call};
use axum::http::StatusCode;
use genie_core::server_db::ProjectRole;
use genie_core::{Actor, CreateInput, Role};
use serde_json::json;

#[tokio::test]
async fn restart_revokes_every_old_worker_before_a_paused_task_can_resume() {
    let h = Harness::new();
    h.project("shop");
    let owner = h
        .app
        .with_server(|db| {
            let user = db.create_user("owner", "", None, None, true)?;
            db.create_user_token(user.id, "owner")
        })
        .unwrap();
    let task = h
        .app
        .with_tracker("shop", |t| {
            t.create(&Actor::new("owner", Role::Human), CreateInput { title: "Interrupted stop".into(), ..Default::default() })
        })
        .unwrap();
    let stale = h
        .app
        .with_server(|db| {
            ["task-manager:G-1", "old-member", "job-1", "not-yet-registered"]
                .into_iter()
                .map(|name| db.create_agent_token("shop", Role::Orchestrator, name, None, None, chrono::Duration::hours(1)))
                .collect::<genie_core::Result<Vec<_>>>()
        })
        .unwrap();
    // Crash exactly after the durable fence, before halt_scope can revoke tokens.
    h.app.with_tracker("shop", |t| t.execution().pause(&Actor::new("owner", Role::Human), &task.id, "owner", "Keep progress")).unwrap();
    assert!(stale.iter().all(|token| h.app.with_server(|db| db.resolve_token(token)).unwrap().is_some()));
    genie::runtime::recover(&h.app).unwrap();
    genie::tasks::resume_execution(&h.app, &genie::tasks::Caller::person("shop", "owner"), &task.id).unwrap();
    for token in stale {
        assert!(h.app.with_server(|db| db.resolve_token(&token)).unwrap().is_none());
        assert_eq!(
            call(&h.remote, "POST", &format!("/api/tasks/{}/comments?project=shop", task.id))
                .bearer(&token)
                .json(json!({"text":"stale mutation"}))
                .send()
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    assert_eq!(
        call(&h.remote, "POST", &format!("/api/tasks/{}/comments?project=shop", task.id))
            .bearer(&owner)
            .json(json!({"text":"current owner"}))
            .send()
            .await
            .0,
        StatusCode::CREATED
    );
    let fresh = h
        .app
        .with_server(|db| db.create_agent_token("shop", Role::Orchestrator, "replacement", None, None, chrono::Duration::hours(1)))
        .unwrap();
    assert_eq!(
        call(&h.remote, "POST", &format!("/api/tasks/{}/comments?project=shop", task.id))
            .bearer(&fresh)
            .json(json!({"text":"current worker"}))
            .send()
            .await
            .0,
        StatusCode::CREATED
    );
}

#[tokio::test]
async fn only_people_with_write_access_control_execution_and_response_cancel_is_separate() {
    let h = Harness::new();
    h.project("shop");
    let (owner, viewer, agent) = h
        .app
        .with_server(|db| {
            let owner = db.create_user("owner", "", None, None, true)?;
            let viewer = db.create_user("viewer", "", None, None, false)?;
            db.set_membership("shop", viewer.id, ProjectRole::Viewer)?;
            Ok((
                db.create_user_token(owner.id, "test")?,
                db.create_user_token(viewer.id, "test")?,
                db.create_agent_token("shop", Role::Orchestrator, "worker", None, None, chrono::Duration::hours(1))?,
            ))
        })
        .unwrap();
    let (task, child) = h
        .app
        .with_tracker("shop", |t| {
            let actor = Actor::new("owner", Role::Human);
            let task = t.create(&actor, CreateInput { title: "Retained task".into(), ..Default::default() })?;
            let child =
                t.create(&actor, CreateInput { title: "Retained child".into(), parent: Some(task.id.clone()), ..Default::default() })?;
            Ok((task, child))
        })
        .unwrap();
    let stop = format!("/api/tasks/{}/stop?project=shop", task.id);
    let resume = format!("/api/tasks/{}/resume?project=shop", task.id);
    for token in [&viewer, &agent] {
        assert_eq!(call(&h.remote, "POST", &stop).bearer(token).json(json!({"note":"try"})).send().await.0, StatusCode::FORBIDDEN);
        assert_eq!(call(&h.remote, "POST", &resume).bearer(token).send().await.0, StatusCode::FORBIDDEN);
    }
    let (s, r, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/responses")
        .bearer(&owner)
        .json(json!({"model":"genie","input":"Continue retained task","background":true}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK);
    let rid = r["id"].as_str().unwrap();
    h.app
        .with_tracker("shop", |t| t.conversations().link_task(r["metadata"]["genie_conversation_id"].as_str().unwrap(), &task.id))
        .unwrap();
    assert_eq!(
        call(&h.remote, "POST", &format!("/openai/projects/shop/v1/responses/{rid}/cancel")).bearer(&owner).send().await.0,
        StatusCode::OK
    );
    assert!(h.app.with_tracker("shop", |t| t.execution().blocking(&task.id)).unwrap().is_none());
    let (s, paused, _) = call(&h.remote, "POST", &stop).bearer(&owner).json(json!({"note":"Keep this progress"})).send().await;
    assert_eq!(s, StatusCode::OK, "{paused}");
    let (s, again, _) = call(&h.remote, "POST", &stop).bearer(&owner).json(json!({"note":"Retry"})).send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(paused["execution"], again["execution"]);
    let (s, out, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/chat/completions")
        .bearer(&owner)
        .json(json!({"model":"genie","messages":[{"role":"user","content":format!("/status {}",task.id)}]}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert!(out["choices"][0]["message"]["content"].as_str().unwrap().contains("/resume"));
    let (s, out, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/chat/completions")
        .bearer(&owner)
        .json(json!({"model":"genie","messages":[{"role":"user","content":format!("/status {}",child.id)}]}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{out}");
    let text = out["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(text.contains(&format!("/resume {}", task.id)), "the command names the blocking ancestor: {text}");
    assert!(!text.contains(&format!("/resume {}", child.id)), "a child cannot clear its ancestor's pause");
    assert!(!call(&h.remote, "POST", &format!("/api/tasks/{}/resume?project=shop", child.id)).bearer(&owner).send().await.0.is_success());
    assert_eq!(call(&h.remote, "POST", &resume).bearer(&owner).send().await.0, StatusCode::OK);
    let last = h.app.with_tracker("shop", |t| t.last_event_id()).unwrap();
    assert_eq!(call(&h.remote, "POST", &resume).bearer(&owner).send().await.0, StatusCode::OK);
    assert_eq!(h.app.with_tracker("shop", |t| t.last_event_id()).unwrap(), last);
    assert_eq!(h.app.with_tracker("shop", |t| t.get(&task.id)).unwrap().status, task.status);
}

#[test]
fn unknown_prices_and_tiny_amounts_never_become_free_budget() {
    let h = Harness::with_config(|cfg| {
        cfg.budgets.per_task = 0.00001;
        cfg.model_prices.insert(
            "another-provider/model".into(),
            genie::config::ModelPrice { input: 0.0, output: 0.0, cache_read: Some(0.0), cache_write: Some(0.0) },
        );
        cfg.model_prices.insert(
            "fake/known".into(),
            genie::config::ModelPrice { input: 1.0, output: 0.0, cache_read: Some(0.0), cache_write: Some(0.0) },
        );
    });
    h.project("shop");
    let owner = h
        .app
        .with_server(|db| {
            let owner = db.create_user("owner", "", None, None, true)?;
            db.set_membership("shop", owner.id, ProjectRole::Owner)?;
            Ok(owner.id)
        })
        .unwrap();
    let (known, unknown) = h
        .app
        .with_tracker("shop", |t| {
            let a = Actor::new("owner", Role::Human);
            let known = t.create(&a, CreateInput { title: "Tiny but nonzero".into(), ..Default::default() })?;
            let unknown = t.create(&a, CreateInput { title: "Unpriced".into(), ..Default::default() })?;
            t.record_usage("worker", Some(&known.id), "fake/known", &genie_core::usage::Tokens { input: 11, ..Default::default() })?;
            t.record_usage(
                "worker",
                Some(&unknown.id),
                "unconfigured/model",
                &genie_core::usage::Tokens { input: 1, ..Default::default() },
            )?;
            Ok((known.id, unknown.id))
        })
        .unwrap();
    assert!(genie::budget::check(&h.app, "shop", Some(&known)).unwrap().is_some());
    let over = genie::budget::check(&h.app, "shop", Some(&unknown)).unwrap().unwrap();
    assert_eq!(over.unpriced_tokens, 1);
    assert!(over.explain().contains("without valid prices"));
    genie::budget::enforce(&h.app, "shop", Some(&unknown)).unwrap();
    assert!(h.app.with_tracker("shop", |t| t.execution().blocking(&unknown)).unwrap().is_some());
    assert!(genie::tasks::resume_execution(&h.app, &genie::tasks::Caller::person("shop", "owner"), &unknown).is_err());
    genie::interactions::tick(&h.app, "shop").unwrap();
    genie::interactions::tick(&h.app, "shop").unwrap();
    assert_eq!(
        h.app.with_server(|db| db.notifications(owner, true, 50)).unwrap().len(),
        1,
        "replay delivers one durable owner notification"
    );
}

#[test]
fn subtask_work_respects_its_parent_task_budget() {
    let h = Harness::with_config(|cfg| {
        cfg.budgets.per_task = 1.0;
        cfg.model_prices.insert("fake/known".into(), genie::config::ModelPrice { input: 1.0, ..Default::default() });
    });
    h.project("shop");
    let (parent, left) = h
        .app
        .with_tracker("shop", |t| {
            let a = Actor::new("owner", Role::Human);
            let parent = t.create(&a, CreateInput { title: "Parent".into(), ..Default::default() })?;
            let left = t.create(&a, CreateInput { title: "Left".into(), parent: Some(parent.id.clone()), ..Default::default() })?;
            let right = t.create(&a, CreateInput { title: "Right".into(), parent: Some(parent.id.clone()), ..Default::default() })?;
            for task in [&left.id, &right.id] {
                t.record_usage("worker", Some(task), "fake/known", &genie_core::usage::Tokens { input: 600_000, ..Default::default() })?;
            }
            Ok((parent.id, left.id))
        })
        .unwrap();
    let over = genie::budget::check(&h.app, "shop", Some(&left)).unwrap().unwrap();
    assert_eq!(over.scope, genie::budget::Scope::Task(parent));
}

#[tokio::test]
async fn paused_teams_release_capacity_and_resume_cannot_overbook_it() {
    let h = Harness::with_config(|cfg| cfg.limits.max_active_teams = 1);
    h.project("shop");
    let mut ids = Vec::new();
    for title in ["A", "B"] {
        let (_, task, _) = call(&h.router, "POST", "/api/tasks?project=shop").json(json!({"title":title})).send().await;
        let id = task["id"].as_str().unwrap().to_owned();
        assert!(
            call(&h.router, "POST", &format!("/api/tasks/{id}/status?project=shop"))
                .json(json!({"status":"ready"}))
                .send()
                .await
                .0
                .is_success()
        );
        ids.push(id);
    }
    let spawn =
        |id: &str| call(&h.router, "POST", "/api/teams?project=shop").json(json!({"task":id,"members":[{"role":"executor"}]})).send();
    assert_eq!(spawn(&ids[0]).await.0, StatusCode::CREATED);
    let control = |id: &str, action: &str| {
        call(&h.router, "POST", &format!("/api/tasks/{id}/{action}?project=shop")).json(json!({"note":"Capacity test"})).send()
    };
    assert_eq!(control(&ids[0], "stop").await.0, StatusCode::OK);
    assert_eq!(h.app.with_tracker("shop", |t| t.bus().active_count()).unwrap(), 0);
    assert_eq!(spawn(&ids[1]).await.0, StatusCode::CREATED, "unrelated work can use released capacity");
    let (s, v, _) = control(&ids[0], "resume").await;
    assert!(!s.is_success(), "{v}");
    assert!(
        h.app.with_tracker("shop", |t| t.execution().blocking(&ids[0])).unwrap().is_some(),
        "failed resume leaves the durable fence intact"
    );
    assert_eq!(control(&ids[1], "stop").await.0, StatusCode::OK);
    assert_eq!(control(&ids[0], "resume").await.0, StatusCode::OK);
    assert_eq!(h.app.with_tracker("shop", |t| t.bus().active_count()).unwrap(), 1);
}

#[tokio::test]
async fn simultaneous_launches_and_resume_share_the_last_team_slot() {
    let h = Harness::with_config(|cfg| cfg.limits.max_active_teams = 1);
    h.project("shop");
    let mut ids = Vec::new();
    for n in 0..8 {
        let (_, task, _) = call(&h.router, "POST", "/api/tasks?project=shop").json(json!({"title":format!("Contender {n}")})).send().await;
        let id = task["id"].as_str().unwrap().to_owned();
        assert!(
            call(&h.router, "POST", &format!("/api/tasks/{id}/status?project=shop"))
                .json(json!({"status":"ready"}))
                .send()
                .await
                .0
                .is_success()
        );
        ids.push(id);
    }
    assert_eq!(
        call(&h.router, "POST", "/api/teams?project=shop").json(json!({"task":ids[0],"members":[{"role":"executor"}]})).send().await.0,
        StatusCode::CREATED
    );
    let caller = genie::tasks::Caller::person("shop", "owner");
    genie::tasks::pause_execution(&h.app, &caller, &ids[0], "owner", "Release last slot").unwrap();
    let start = std::sync::Barrier::new(ids.len());
    let won = std::thread::scope(|scope| {
        let workers = ids
            .iter()
            .enumerate()
            .map(|(n, id)| {
                let (app, start, caller) = (&h.app, &start, &caller);
                scope.spawn(move || {
                    start.wait();
                    if n == 0 {
                        genie::tasks::resume_execution(app, caller, id).is_ok()
                    } else {
                        genie::runtime::spawn_team(
                            app,
                            "shop",
                            genie::runtime::SpawnRequest {
                                task: id.clone(),
                                template: None,
                                members: vec![genie::config::MemberSpec { role: "executor".into(), ..Default::default() }],
                                models: Default::default(),
                                note: None,
                                by: Actor::new("owner", Role::Human),
                                initiator: None,
                                decision_override: None,
                            },
                        )
                        .is_ok()
                    }
                })
            })
            .collect::<Vec<_>>();
        workers.into_iter().map(|worker| worker.join().unwrap()).filter(|won| *won).count()
    });
    assert_eq!(won, 1, "only one contender can take the last slot");
    assert_eq!(h.app.with_tracker("shop", |t| t.bus().active_count()).unwrap(), 1);
}
