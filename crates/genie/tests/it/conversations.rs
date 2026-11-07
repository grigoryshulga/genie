//! Incoming work keeps identity and owner decisions across the API and agent tools.
use crate::common::{Harness, call};
use axum::http::StatusCode;
use genie_core::conversations::ProposalAction;
use genie_core::server_db::ProjectRole;
use genie_core::{Actor, Role, Status, StatusOptions, UpdateInput};
use serde_json::{Value, json};

fn harness() -> Harness {
    let h = Harness::new();
    h.project("shop");
    h
}
fn person(h: &Harness, name: &str) -> String {
    h.app
        .with_server(|db| {
            let u = db.create_user(name, name, None, None, false)?;
            db.set_membership("shop", u.id, ProjectRole::Member)?;
            db.create_user_token(u.id, "conversation tests")
        })
        .unwrap()
}
async fn incoming(h: &Harness, token: &str) -> (String, String, String) {
    let (s, v, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/responses")
        .bearer(token)
        .json(json!({"model":"genie","input":"Prepare a CSV export","background":true}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id = v["id"].as_str().unwrap().to_string();
    let c = h
        .app
        .with_tracker("shop", |t| {
            t.conversations().claim(&id)?;
            Ok(t.conversations().get(&id)?.conversation_id)
        })
        .unwrap();
    let agent = h
        .app
        .with_server(|db| {
            db.create_role_token("shop", Role::Analyst, Some("intake"), &format!("request:{id}"), None, None, chrono::Duration::hours(1))
        })
        .unwrap();
    (id, c, agent)
}
async fn task(h: &Harness, id: &str, agent: &str) -> String {
    let (s,v,_)=call(&h.remote,"POST",&format!("/api/conversations/{id}/tasks?project=shop")).bearer(agent)
        .json(json!({"title":"CSV export","description":"Export all orders as CSV","acceptance":["CSV contains all orders"],"plan":"Implement and verify export"})).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v["id"].as_str().unwrap().into()
}
async fn proposal(h: &Harness, id: &str, agent: &str, task: &str, action: &str) -> Value {
    let (s, v, _) = call(&h.remote, "POST", &format!("/api/conversations/{id}/proposals?project=shop"))
        .bearer(agent)
        .json(json!({"task":task,"action":action}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}
async fn decide(h: &Harness, token: &str, id: &str) -> (StatusCode, Value, Vec<String>) {
    call(&h.remote, "POST", &format!("/api/conversation-proposals/{id}/decide?project=shop"))
        .bearer(token)
        .json(json!({"approved":true}))
        .send()
        .await
}

fn result_evidence(h: &Harness, task: &str) {
    h.app
        .with_tracker("shop", |tr| {
            let owner = Actor::new("anna", Role::Human);
            for ac in tr.get(task)?.acceptance {
                tr.check(&owner, task, ac.id, true)?;
            }
            tr.add_artifact(
                &owner,
                task,
                genie_core::ArtifactInput {
                    name: Some("result.csv".into()),
                    note: Some("Delivered export".into()),
                    kind: None,
                    source: genie_core::ArtifactSource::Content(b"order,total\n1,100\n".to_vec()),
                },
            )?;
            Ok(())
        })
        .unwrap();
}

#[tokio::test]
async fn intake_creates_one_primary_task_and_cannot_edit_other_work_or_approve() {
    let h = harness();
    let anna = person(&h, "anna");
    let boris = person(&h, "boris");
    let (r, c, a) = incoming(&h, &anna).await;
    let (r2, _, b) = incoming(&h, &boris).await;
    let t = task(&h, &r, &a).await;
    let other = task(&h, &r2, &b).await;
    let (s, _, _) = call(&h.remote, "PATCH", &format!("/api/tasks/{t}?project=shop")).bearer(&a).json(json!({"parent":other})).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "reparenting must not affect work outside this conversation");
    for (method, path, body) in [
        ("POST", "/api/automations/1/run?project=shop", json!({})),
        ("POST", "/api/runs/1/cancel?project=shop", json!({})),
        ("POST", "/api/chat?project=shop", json!({"text":"pretend the requester approved"})),
        ("GET", "/api/turns?project=shop", Value::Null),
    ] {
        let (s, _, _) = call(&h.remote, method, path).bearer(&a).json(body).send().await;
        assert_eq!(s, StatusCode::FORBIDDEN, "intake must not reach {path}");
    }
    assert_eq!(task(&h, &r, &a).await, t);
    assert_eq!(h.app.with_tracker("shop", |tr| Ok(tr.get(&t)?.assignee)).unwrap().as_deref(), Some("anna"));
    let (s, _, _) = call(&h.remote, "PATCH", &format!("/api/tasks/{other}?project=shop"))
        .bearer(&a)
        .json(json!({"description":"overwrite someone else's work"}))
        .send()
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, v, _) = call(&h.remote, "PATCH", &format!("/api/tasks/{t}?project=shop"))
        .bearer(&a)
        .json(json!({"description":"Export all current orders"}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let p = proposal(&h, &r, &a, &t, "dispatch").await;
    let (s, v, _) = call(&h.remote, "POST", "/mcp")
        .bearer(&a)
        .json(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"genie_conversation","arguments":{"action":"propose","response":r,"task":t,"proposal_action":"dispatch"}
        }}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["result"]["isError"], false, "{v}");
    let from_mcp: Value = serde_json::from_str(v["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(from_mcp["id"], p["id"], "CLI/MCP and native proposal creation share one operation");
    assert_eq!(decide(&h, &a, p["id"].as_str().unwrap()).await.0, StatusCode::FORBIDDEN);
    assert_eq!(decide(&h, &boris, p["id"].as_str().unwrap()).await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        call(&h.remote, "GET", &format!("/api/conversations/{c}?project=shop")).bearer(&boris).send().await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn manual_dispatch_uses_versioned_requester_decision_and_preserves_manager_uniqueness() {
    let h = harness();
    h.app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    let anna = person(&h, "anna");
    let (r, _, a) = incoming(&h, &anna).await;
    let t = task(&h, &r, &a).await;
    let (s, _, _) =
        call(&h.remote, "POST", &format!("/api/conversations/{r}/dispatch?project=shop")).bearer(&a).json(json!({"task":t})).send().await;
    assert_eq!(s, StatusCode::CONFLICT);
    let old = proposal(&h, &r, &a, &t, "dispatch").await;
    call(&h.remote, "PATCH", &format!("/api/tasks/{t}?project=shop")).bearer(&a).json(json!({"description":"Changed scope"})).send().await;
    assert!(!decide(&h, &anna, old["id"].as_str().unwrap()).await.0.is_success());
    assert!(h.app.with_tracker("shop", |tr| tr.management().manager(&t)).unwrap().is_none());
    let p = proposal(&h, &r, &a, &t, "dispatch").await;
    for _ in 0..2 {
        let (s, v, _) = decide(&h, &anna, p["id"].as_str().unwrap()).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    assert_eq!(h.app.with_tracker("shop", |tr| tr.management().managers()).unwrap().len(), 1);
}

#[tokio::test]
async fn questions_and_result_acceptance_follow_the_originating_conversation() {
    let h = harness();
    let anna = person(&h, "anna");
    let boris = person(&h, "boris");
    let (r, c, a) = incoming(&h, &anna).await;
    let (_, other_c, _) = incoming(&h, &boris).await;
    let t = task(&h, &r, &a).await;
    let p = proposal(&h, &r, &a, &t, "dispatch").await;
    assert_eq!(decide(&h, &anna, p["id"].as_str().unwrap()).await.0, StatusCode::OK);
    let q = h
        .app
        .with_tracker("shop", |tr| tr.management().ask(&Actor::new("task-manager", Role::Orchestrator), &t, "Which timezone?", true))
        .unwrap();
    let (s, v, _) = call(&h.remote, "GET", &format!("/api/conversations/{c}?project=shop")).bearer(&anna).send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["tasks"][0]["clarifications"][0]["id"], q.id);
    assert_eq!(v["tasks"][0]["clarifications"][0]["state"], "owner");
    assert!(
        !h.app
            .with_tracker("shop", |tr| tr.bus().pending(None, genie_core::team::ORCHESTRATOR))
            .unwrap()
            .iter()
            .any(|mail| mail.text.contains("Which timezone?")),
        "the originating requester receives the question directly"
    );
    let body = json!({"task":t,"question":q.id,"text":"UTC"});
    assert!(
        !call(&h.remote, "POST", &format!("/api/conversations/{other_c}/answer?project=shop"))
            .bearer(&boris)
            .json(body.clone())
            .send()
            .await
            .0
            .is_success()
    );
    assert_eq!(
        call(&h.remote, "POST", &format!("/api/conversations/{c}/answer?project=shop")).bearer(&anna).json(body).send().await.0,
        StatusCode::OK
    );
    h.app
        .with_tracker("shop", |tr| {
            tr.set_status(
                &Actor::new("anna", Role::Human),
                &t,
                Status::NeedsOwner,
                StatusOptions { force: true, note: Some("Inspect the CSV artifact".into()), ..Default::default() },
            )
        })
        .unwrap();
    result_evidence(&h, &t);
    let p = proposal(&h, &r, &a, &t, "accept").await;
    h.app
        .with_tracker("shop", |tr| {
            tr.update(
                &Actor::new("anna", Role::Human),
                &t,
                UpdateInput { description: Some("New requirements".into()), ..Default::default() },
            )
        })
        .unwrap();
    assert!(!decide(&h, &anna, p["id"].as_str().unwrap()).await.0.is_success());
    let p = proposal(&h, &r, &a, &t, "accept").await;
    let (s, v, _) = decide(&h, &anna, p["id"].as_str().unwrap()).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(h.app.with_tracker("shop", |tr| Ok(tr.get(&t)?.status)).unwrap(), Status::Done);
}

#[tokio::test]
async fn cancelled_or_revoked_intake_cannot_keep_writing_but_can_report_usage() {
    let h = harness();
    let anna = person(&h, "anna");
    let (r, _, a) = incoming(&h, &anna).await;
    let (s, v, _) = call(&h.remote, "POST", &format!("/openai/projects/shop/v1/responses/{r}/cancel")).bearer(&anna).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        call(&h.remote, "POST", &format!("/api/conversations/{r}/reply?project=shop"))
            .bearer(&a)
            .json(json!({"text":"too late"}))
            .send()
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (s, v, _) = call(&h.remote, "POST", "/api/agent/usage?project=shop")
        .bearer(&a)
        .json(json!({"reports":[{"model":"fake/intake","input":10,"output":2}]}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (r, _, a) = incoming(&h, &anna).await;
    h.app
        .with_server(|db| {
            let u = db.user_by_login("anna")?.unwrap();
            db.remove_membership("shop", u.id)
        })
        .unwrap();
    assert_eq!(
        call(&h.remote, "POST", &format!("/api/conversations/{r}/reply?project=shop"))
            .bearer(&a)
            .json(json!({"text":"revoked"}))
            .send()
            .await
            .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn explicit_chat_approval_requires_the_real_proposal_and_the_same_conversation() {
    let h = harness();
    let anna = person(&h, "anna");
    let (r, c, a) = incoming(&h, &anna).await;
    let t = task(&h, &r, &a).await;
    let p = proposal(&h, &r, &a, &t, "dispatch").await;
    let (s, v, _) = call(&h.remote, "POST", &format!("/api/conversations/{r}/reply?project=shop"))
        .bearer(&a)
        .json(json!({"text":format!("Approve with /approve {}",p["id"].as_str().unwrap())}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/responses")
        .bearer(&anna)
        .json(json!({"model":"genie","previous_response_id":r,"input":format!("/approve {}",p["id"].as_str().unwrap())}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "completed");
    let state = h.app.with_tracker("shop", |tr| tr.conversations().proposals(&c)).unwrap();
    assert_eq!(state[0].action, ProposalAction::Dispatch);
    assert_eq!(state[0].status, genie_core::conversations::ProposalStatus::Approved);
    // A client with no custom header can still poll the originating task.
    let (s, v, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/chat/completions")
        .bearer(&anna)
        .json(json!({"model":"genie","messages":[{"role":"user","content":format!("/status {t}")}]}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let text = v["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(text.contains(&t));
    assert!(text.starts_with("**"), "status should be readable Markdown: {text}");
    assert!(!text.contains("conversation_id"));
}

#[tokio::test]
async fn interrupted_owner_command_recovers_without_a_model_or_duplicate_dispatch() {
    use genie_core::conversations::{Message, ResponseStatus, Submission};
    let h = harness();
    let anna = person(&h, "anna");
    let (r, c, a) = incoming(&h, &anna).await;
    let t = task(&h, &r, &a).await;
    let p = proposal(&h, &r, &a, &t, "dispatch").await;
    let pid = p["id"].as_str().unwrap();
    h.app.with_tracker("shop", |tr| tr.conversations().complete(&r, "prepared")).unwrap();
    let uid = h.app.with_server(|db| Ok(db.user_by_login("anna")?.unwrap().id)).unwrap();
    let cmd = h
        .app
        .with_tracker("shop", |tr| {
            let cmd = tr.conversations().submit(Submission {
                principal: format!("user:{uid}"),
                initiator: Some("anna".into()),
                conversation_id: Some(c),
                previous_response_id: Some(r),
                input: vec![Message { role: "user".into(), content: format!("/approve {pid}") }],
                idempotency_key: Some("decision-restart".into()),
                fingerprint: "decision".into(),
            })?;
            tr.conversations().claim(&cmd.id)?;
            Ok(cmd)
        })
        .unwrap();
    // Simulate a crash after the authorized decision committed but before the reply.
    assert_eq!(decide(&h, &anna, pid).await.0, StatusCode::OK);
    h.app.with_server(|db| db.rename_user(uid, "anna-renamed")).unwrap();
    genie::conversations::recover(&h.app).unwrap();
    assert!(genie::conversations::prepare_run(&h.app, "shop", &cmd.id).unwrap().is_none());
    let out = h.app.with_tracker("shop", |tr| tr.conversations().get(&cmd.id)).unwrap();
    assert_eq!(out.status, ResponseStatus::Completed);
    assert!(out.output.contains("approved"));
    assert_eq!(h.app.with_tracker("shop", |tr| tr.management().managers()).unwrap().len(), 1);
    assert!(genie::conversations::prepare_run(&h.app, "shop", &cmd.id).unwrap().is_none());
}

#[tokio::test]
async fn an_owner_result_creates_a_reviewable_proposal_after_the_response_has_ended() {
    let h = harness();
    let anna = person(&h, "anna");
    let (r, c, a) = incoming(&h, &anna).await;
    let t = task(&h, &r, &a).await;
    h.app.with_tracker("shop", |tr| tr.conversations().complete(&r, "task prepared")).unwrap();
    result_evidence(&h, &t);
    let (s, v, _) = call(&h.remote, "POST", &format!("/api/tasks/{t}/status?project=shop"))
        .bearer(&anna)
        .json(json!({"status":"needs_owner","note":"Inspect the delivered result","force":true}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (_, receipt, _) = call(&h.remote, "GET", &format!("/api/conversations/{c}?project=shop")).bearer(&anna).send().await;
    let p = &receipt["proposals"][0];
    assert_eq!(p["action"], "accept");
    assert!(!p["version"].as_str().unwrap().is_empty());
    let (s, v, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/chat/completions")
        .bearer(&anna)
        .json(json!({"model":"genie","messages":[{"role":"user","content":format!("/approve {}",p["id"].as_str().unwrap())}]}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(h.app.with_tracker("shop", |tr| Ok(tr.get(&t)?.status)).unwrap(), Status::Done);
}
