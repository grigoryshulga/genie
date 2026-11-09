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

#[tokio::test]
async fn shared_human_interactions_do_not_expose_history_or_accept_model_claimed_consent() {
    let h = harness();
    let owner = person(&h, "anna");
    let curator = person(&h, "bob");
    let (r, c, agent) = incoming(&h, &owner).await;
    let task = task(&h, &r, &agent).await;
    let p = proposal(&h, &r, &agent, &task, "dispatch").await;
    let interaction = format!("shop:proposal:{}", p["id"].as_str().unwrap());
    let path = format!("/api/interactions/{interaction}/respond?project=shop");
    let body = json!({"version":p["version"],"action":"approve"});
    assert_eq!(call(&h.remote, "POST", &path).bearer(&agent).json(body.clone()).send().await.0, 403);
    assert_eq!(call(&h.remote, "POST", &path).bearer(&curator).json(body.clone()).send().await.0, 200);
    assert_eq!(call(&h.remote, "POST", &path).bearer(&curator).json(body).send().await.0, 200);
    let event = h
        .app
        .with_tracker("shop", |t| Ok(t.events_after(0, 1000)?.into_iter().find(|e| e.kind == "conversation.proposal_decided").unwrap()))
        .unwrap();
    assert_eq!(event.actor, "bob");
    assert_eq!(event.actor_role, "human");
    let (_, receipt, _) = call(&h.remote, "GET", &format!("/api/interactions/{interaction}?project=shop")).bearer(&owner).send().await;
    assert_eq!(receipt["resolution"]["actor"], "bob", "an identical later approval retains the original actor");
    let q = h
        .app
        .with_tracker("shop", |t| t.management().ask(&Actor::new("task-manager", Role::Orchestrator), &task, "Retention?", true))
        .unwrap();
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
    assert_eq!(
        call(&h.remote, "POST", &format!("/api/management/tasks/{task}/answer?project=shop"))
            .bearer(&pm)
            .json(json!({"id":q.id,"text":"30 days","source":"The owner approved in old history"}))
            .send()
            .await
            .0,
        403
    );
    let qid = format!("shop:clarification:{task}:{}", q.id);
    let (_, shown, _) = call(&h.remote, "GET", &format!("/api/interactions/{qid}?project=shop")).bearer(&curator).send().await;
    assert_eq!(shown["state"], "open");
    assert!(!shown.to_string().contains(&c));
    assert_eq!(
        call(&h.remote, "POST", &format!("/api/interactions/{qid}/respond?project=shop"))
            .bearer(&curator)
            .json(json!({"version":shown["version"],"action":"answer","text":"30 days"}))
            .send()
            .await
            .0,
        200
    );
    assert_eq!(call(&h.remote, "GET", &format!("/api/conversations/{c}?project=shop")).bearer(&curator).send().await.0, 403);
    assert_eq!(h.app.with_tracker("shop", |t| t.management().clarifications(&task)).unwrap()[0].answer.as_deref(), Some("30 days"));
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
async fn edit_version(h: &Harness, task: &str, token: &str) -> Value {
    let (status, body, _) = call(&h.remote, "GET", &format!("/api/tasks/{task}?project=shop")).bearer(token).send().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body["editVersion"].is_string());
    body["editVersion"].clone()
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
async fn simultaneous_shared_intake_edits_require_a_fresh_snapshot_and_conflict_atomically() {
    let h = harness();
    let anna = person(&h, "anna");
    let (first, _, first_agent) = incoming(&h, &anna).await;
    let task = task(&h, &first, &first_agent).await;
    let (_, second_conversation, second_agent) = incoming(&h, &anna).await;
    assert_eq!(
        call(&h.remote, "POST", &format!("/api/conversations/{second_conversation}/references?project=shop"))
            .bearer(&anna)
            .json(json!({"task":task}))
            .send()
            .await
            .0,
        StatusCode::OK
    );
    let path = format!("/api/tasks/{task}?project=shop");
    assert_eq!(
        call(&h.remote, "PATCH", &path).bearer(&first_agent).json(json!({"description":"missing version"})).send().await.0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        call(&h.remote, "PATCH", &path).bearer(&anna).json(json!({"description":"unversioned native replacement"})).send().await.0,
        StatusCode::BAD_REQUEST
    );
    let version = edit_version(&h, &task, &first_agent).await;
    assert_eq!(version, edit_version(&h, &task, &second_agent).await);
    let (left, right) = tokio::join!(
        call(&h.remote, "PATCH", &path)
            .bearer(&first_agent)
            .json(json!({"description":"left requirements","expectedVersion":version}))
            .send(),
        call(&h.remote, "PATCH", &path)
            .bearer(&second_agent)
            .json(json!({"description":"right requirements","expectedVersion":version}))
            .send()
    );
    assert!(
        matches!((left.0, right.0), (StatusCode::OK, StatusCode::CONFLICT) | (StatusCode::CONFLICT, StatusCode::OK)),
        "{left:?} {right:?}"
    );
    let stored = h.app.with_tracker("shop", |t| t.get(&task)).unwrap();
    assert_eq!(stored.description, if left.0 == StatusCode::OK { "left requirements" } else { "right requirements" });
    let fresh = edit_version(&h, &task, &second_agent).await;
    assert_ne!(fresh, version);
    let (status, _, _) = call(&h.remote, "PATCH", &path)
        .bearer(&second_agent)
        .json(json!({"notes":"reconciled on the latest task","expectedVersion":fresh}))
        .send()
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        call(&h.remote, "PATCH", &path)
            .bearer(&anna)
            .json(json!({"notes":"stale human replacement","expectedVersion":fresh}))
            .send()
            .await
            .0,
        StatusCode::CONFLICT
    );
    let latest = edit_version(&h, &task, &second_agent).await;
    let (status, result, _) = call(&h.remote, "POST", "/mcp")
        .bearer(&second_agent)
        .json(json!({
            "jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"genie_task","arguments":{
                "action":"update","task":task,"expectedVersion":latest,"notes":"MCP reconciled edit","project":"shop"
            }}
        }))
        .send()
        .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["result"]["isError"], false, "{result}");
}

#[tokio::test]
async fn recovered_controls_bypass_execution_gates_and_commit_their_reply_with_the_generation() {
    use genie_core::conversations::{Message, ResponseStatus, Submission};
    let h = harness();
    h.app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    let anna = person(&h, "anna");
    let uid = h.app.with_server(|db| Ok(db.user_by_login("anna")?.unwrap().id)).unwrap();
    let owner = Actor::new("anna", Role::Human);
    let task = h
        .app
        .with_tracker("shop", |t| t.create(&owner, genie_core::CreateInput { title: "Controlled task".into(), ..Default::default() }))
        .unwrap();
    genie::tasks::pause_execution(&h.app, &genie::tasks::Caller::person("shop", "anna"), &task.id, "owner", "Initial pause").unwrap();
    let command = h
        .app
        .with_tracker("shop", |t| {
            t.tx(|| {
                let r = t.conversations().submit(Submission {
                    principal: format!("user:{uid}"),
                    initiator: Some("anna".into()),
                    conversation_id: None,
                    previous_response_id: None,
                    input: vec![Message { role: "user".into(), content: format!("/resume {}", task.id) }],
                    idempotency_key: None,
                    fingerprint: "resume".into(),
                })?;
                t.conversations().reference_task(&r.conversation_id, &task.id)?;
                t.conversations().claim(&r.id)?;
                Ok(r)
            })
        })
        .unwrap();
    genie::runtime::recover(&h.app).unwrap();
    genie::runtime::tick(&h.app).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if h.app.with_tracker("shop", |t| t.conversations().get(&command.id)).unwrap().status.is_terminal() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("a recovered resume cannot remain queued behind its own pause");
    assert_eq!(h.app.with_tracker("shop", |t| t.conversations().get(&command.id)).unwrap().status, ResponseStatus::Completed);
    assert!(h.app.with_tracker("shop", |t| t.execution().blocking(&task.id)).unwrap().is_none());
    h.app.with_tracker("shop", |t| {t.conn().execute_batch(
        "CREATE TRIGGER fail_control_reply BEFORE UPDATE ON conversation_responses WHEN NEW.status='completed' BEGIN SELECT RAISE(ABORT,'reply storage fault'); END;")?;Ok(())}).unwrap();
    let before = h.app.with_tracker("shop", |t| t.execution().state(&task.id)).unwrap();
    let (status, _, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/responses")
        .bearer(&anna)
        .json(json!({"model":"genie","input":format!("/stop {}",task.id)}))
        .send()
        .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        h.app.with_tracker("shop", |t| t.execution().state(&task.id)).unwrap(),
        before,
        "a failed reply commit rolls back the stop too"
    );
    h.app
        .with_tracker("shop", |t| {
            t.conn().execute_batch("DROP TRIGGER fail_control_reply;")?;
            Ok(())
        })
        .unwrap();
    let (status, stopped, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/responses")
        .bearer(&anna)
        .json(json!({"model":"genie","input":format!("/stop {}",task.id)}))
        .send()
        .await;
    assert_eq!(status, StatusCode::OK, "{stopped}");
    assert_eq!(stopped["status"], "completed");
    genie::tasks::resume_execution(&h.app, &genie::tasks::Caller::person("shop", "anna"), &task.id).unwrap();
    let resumed = h.app.with_tracker("shop", |t| t.execution().state(&task.id)).unwrap();
    genie::runtime::recover(&h.app).unwrap();
    genie::runtime::tick(&h.app).await.unwrap();
    assert_eq!(
        h.app.with_tracker("shop", |t| t.execution().state(&task.id)).unwrap(),
        resumed,
        "an already applied stop cannot replay after a later resume"
    );
    assert!(h.app.with_tracker("shop", |t| t.conversations().queued()).unwrap().is_empty());
}

#[tokio::test]
async fn intake_creation_retries_are_stable_and_unselected_work_is_forbidden() {
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
        .json(json!({"description":"Export all current orders","expectedVersion":edit_version(&h,&t,&a).await}))
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
    h.app
        .with_server(|db| {
            let user = db.user_by_login("boris")?.unwrap();
            db.set_membership("shop", user.id, ProjectRole::Viewer)
        })
        .unwrap();
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
    assert_eq!(
        call(&h.remote, "PATCH", &format!("/api/tasks/{t}?project=shop"))
            .bearer(&a)
            .json(json!({"description":"Changed scope","expectedVersion":edit_version(&h,&t,&a).await}))
            .send()
            .await
            .0,
        StatusCode::OK
    );
    assert!(!decide(&h, &anna, old["id"].as_str().unwrap()).await.0.is_success());
    let (s, receipt, _) = call(&h.remote, "GET", &format!("/api/conversations/{r}?project=shop")).bearer(&anna).send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(receipt["proposals"][0]["stale"], true);
    let (s, reply, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/chat/completions")
        .bearer(&anna)
        .json(json!({"model":"genie","messages":[{"role":"user","content":format!("/status {t}")}]}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK);
    assert!(!reply["choices"][0]["message"]["content"].as_str().unwrap().contains(old["id"].as_str().unwrap()));

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
    h.app
        .with_server(|db| {
            let user = db.user_by_login("boris")?.unwrap();
            db.set_membership("shop", user.id, ProjectRole::Viewer)
        })
        .unwrap();
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
async fn explicit_chat_approval_requires_the_real_proposal_and_current_project_rights() {
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
    assert!(v["choices"][0]["message"]["content"].as_str().unwrap().contains(&t));
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

#[tokio::test]
async fn telegram_reply_resumes_the_same_question_once_and_rechecks_identity() {
    let (base, sent, _) = super::telegram::fake().await;
    let h = Harness::with_config(|cfg| cfg.telegram = Some(genie::config::TelegramConfig { token: "LEGACY".into(), api_base: Some(base) }));
    h.project("shop");
    let anna = person(&h, "anna");
    let (r, c, a) = incoming(&h, &anna).await;
    let t = task(&h, &r, &a).await;
    let p = proposal(&h, &r, &a, &t, "dispatch").await;
    assert_eq!(decide(&h, &anna, p["id"].as_str().unwrap()).await.0, StatusCode::OK);
    let bot = h
        .app
        .with_server(|db| {
            db.set_project_bot("shop", 123, "project_bot", "123:test")?;
            let bot = db.project_bot("shop")?.unwrap();
            let user = db.user_by_login("anna")?.unwrap();
            db.link_channel(user.id, &bot.scope, "42")?;
            Ok(bot)
        })
        .unwrap();
    let q = h
        .app
        .with_tracker("shop", |tr| tr.management().ask(&Actor::new("task-manager", Role::Orchestrator), &t, "Which timezone?", true))
        .unwrap();
    let event = h
        .app
        .with_tracker("shop", |tr| Ok(tr.events_after(0, 1000)?.into_iter().find(|e| e.kind == "conversation.question").unwrap()))
        .unwrap();
    // Replay of the event after a crash cannot enqueue another copy.
    genie::interactions::notify_event(&h.app, "shop", &event).unwrap();
    genie::engine::tick(&h.app).unwrap();
    genie::channels::dispatch(&h.app).await;
    assert_eq!(sent.lock().unwrap().len(), 1);
    assert!(sent.lock().unwrap()[0]["body"]["text"].as_str().unwrap().contains("Which timezone?"));
    assert!(!genie::interactions::telegram_reply(&h.app, &bot, 43, 42, "UTC").unwrap());
    genie::telegram::handle_update(
        &h.app,
        &bot,
        &json!({"message": {
            "chat":{"id":42,"type":"private"},"text":"UTC","reply_to_message":{"message_id":42}
        }}),
    )
    .await
    .unwrap();
    let pending = || h.app.with_tracker("shop", |tr| tr.bus().pending(None, &genie_core::management::manager_name(&t))).unwrap();
    let before = pending().len();
    assert!(genie::interactions::telegram_reply(&h.app, &bot, 42, 42, "UTC").unwrap());
    assert_eq!(pending().len(), before);
    assert!(genie::interactions::telegram_reply(&h.app, &bot, 42, 42, "Europe/Paris").is_err());
    let (_, receipt, _) = call(&h.remote, "GET", &format!("/api/conversations/{c}?project=shop")).bearer(&anna).send().await;
    assert_eq!(receipt["tasks"][0]["clarifications"][0]["id"], q.id);
    assert_eq!(receipt["tasks"][0]["clarifications"][0]["answer"], "UTC");
    h.app
        .with_server(|db| {
            let user = db.user_by_login("anna")?.unwrap();
            db.set_membership("shop", user.id, ProjectRole::Viewer)
        })
        .unwrap();
    assert!(!genie::interactions::telegram_reply(&h.app, &bot, 42, 42, "UTC").unwrap());
}

#[tokio::test]
async fn telegram_proposal_callback_is_bound_to_the_message_and_current_user() {
    let (base, sent, _) = super::telegram::fake().await;
    let h = Harness::with_config(|cfg| cfg.telegram = Some(genie::config::TelegramConfig { token: "LEGACY".into(), api_base: Some(base) }));
    h.project("shop");
    h.app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    let anna = person(&h, "anna");
    let (r, _, a) = incoming(&h, &anna).await;
    let t = task(&h, &r, &a).await;
    let p = proposal(&h, &r, &a, &t, "dispatch").await;
    let pid = p["id"].as_str().unwrap();
    let bot = h
        .app
        .with_server(|db| {
            db.set_project_bot("shop", 123, "project_bot", "123:test")?;
            let bot = db.project_bot("shop")?.unwrap();
            let user = db.user_by_login("anna")?.unwrap();
            db.link_channel(user.id, &bot.scope, "42")?;
            db.rename_user(user.id, "anna-renamed")?;
            Ok(bot)
        })
        .unwrap();
    genie::engine::tick(&h.app).unwrap();
    genie::channels::dispatch(&h.app).await;
    assert_eq!(sent.lock().unwrap().len(), 1);
    assert_eq!(sent.lock().unwrap()[0]["body"]["reply_markup"]["inline_keyboard"][0][0]["callback_data"], "interaction:approve");
    assert!(genie::interactions::telegram_reply(&h.app, &bot, 42, 42, "да").is_err());
    let callback = |from| {
        json!({"callback_query":{"id":"callback-test","from":{"id":from},"data":"interaction:approve",
        "message":{"message_id":42,"chat":{"id":42,"type":"private"}}}})
    };
    genie::telegram::handle_update(&h.app, &bot, &callback(43)).await.unwrap();
    assert!(h.app.with_tracker("shop", |tr| tr.management().manager(&t)).unwrap().is_none());
    genie::telegram::handle_update(&h.app, &bot, &callback(42)).await.unwrap();
    assert!(h.app.with_tracker("shop", |tr| tr.management().manager(&t)).unwrap().is_some());
    genie::telegram::handle_update(&h.app, &bot, &callback(42)).await.unwrap();
    assert_eq!(
        h.app.with_tracker("shop", |tr| tr.conversations().proposal(pid)).unwrap().status,
        genie_core::conversations::ProposalStatus::Approved
    );
    assert!(genie::interactions::telegram_reply(&h.app, &bot, 42, 42, "отклоняю").is_err());
}

#[tokio::test]
async fn selected_tasks_cross_private_conversations_without_granting_chat_access() {
    let h = harness();
    let anna = person(&h, "anna");
    let boris = person(&h, "boris");
    let (r, c, a) = incoming(&h, &anna).await;
    let t = task(&h, &r, &a).await;
    let (_, new_c, new_agent) = incoming(&h, &anna).await;
    let (br, bc, ba) = incoming(&h, &boris).await;
    let select = format!("/api/conversations/{bc}/references?project=shop");
    assert_eq!(call(&h.remote, "POST", &select).bearer(&ba).json(json!({"task":t})).send().await.0, StatusCode::FORBIDDEN);
    assert_eq!(
        call(&h.remote, "PATCH", &format!("/api/tasks/{t}?project=shop")).bearer(&ba).json(json!({"plan":"Not selected"})).send().await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(call(&h.remote, "POST", &select).bearer(&boris).json(json!({"task":t})).send().await.0, StatusCode::OK);
    let (s, v, _) = call(&h.remote, "PATCH", &format!("/api/tasks/{t}?project=shop"))
        .bearer(&ba)
        .json(json!({"plan":"Selected by current user","expectedVersion":edit_version(&h,&t,&ba).await}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(
        call(&h.remote, "GET", &format!("/api/conversations/{c}?project=shop")).bearer(&boris).send().await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(call(&h.remote, "GET", &format!("/api/conversations/{c}?project=shop")).bearer(&ba).send().await.0, StatusCode::FORBIDDEN);
    let p = proposal(&h, &br, &ba, &t, "dispatch").await;
    assert_eq!(decide(&h, &boris, p["id"].as_str().unwrap()).await.0, StatusCode::OK);
    let q = h
        .app
        .with_tracker("shop", |tr| tr.management().ask(&Actor::new("task-manager", Role::Orchestrator), &t, "Timezone?", true))
        .unwrap();
    for conv in [&new_c, &c] {
        let (s, v, _) = call(&h.remote, "POST", &format!("/api/conversations/{conv}/answer?project=shop"))
            .bearer(&anna)
            .json(json!({"task":t,"question":q.id,"text":"UTC"}))
            .send()
            .await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    assert_eq!(
        call(&h.remote, "PATCH", &format!("/api/tasks/{t}?project=shop"))
            .bearer(&new_agent)
            .json(json!({"notes":"Continued","expectedVersion":edit_version(&h,&t,&new_agent).await}))
            .send()
            .await
            .0,
        StatusCode::OK
    );
    h.app
        .with_server(|db| {
            let u = db.user_by_login("boris")?.unwrap();
            db.set_membership("shop", u.id, ProjectRole::Viewer)
        })
        .unwrap();
    assert_eq!(
        call(&h.remote, "PATCH", &format!("/api/tasks/{t}?project=shop")).bearer(&ba).json(json!({"plan":"Revoked"})).send().await.0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn explicit_continuation_selects_shared_work_and_creation_keys_allow_multiple_tasks() {
    let h = harness();
    let anna = person(&h, "anna");
    let boris = person(&h, "boris");
    let (r, c, a) = incoming(&h, &anna).await;
    let first = task(&h, &r, &a).await;
    let create = format!("/api/conversations/{r}/tasks?project=shop");
    let body = json!({"title":"Independent second task","requestKey":"second"});
    let (s, second, _) = call(&h.remote, "POST", &create).bearer(&a).json(body.clone()).send().await;
    assert_eq!(s, StatusCode::OK, "{second}");
    assert_ne!(second["id"], first);
    assert_eq!(call(&h.remote, "POST", &create).bearer(&a).json(body).send().await.1["id"], second["id"]);
    assert_eq!(h.app.with_tracker("shop", |tr| tr.conversations().tasks(&c)).unwrap().len(), 2);
    let (s, v, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/responses")
        .bearer(&boris)
        .json(json!({"model":"genie","input":format!("/continue {first} Review current work"),"background":true}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let id = v["id"].as_str().unwrap();
    let context = genie::conversations::prepare_run(&h.app, "shop", id).unwrap().unwrap();
    assert!(context.message.contains(&first));
    let rc = h.app.with_tracker("shop", |tr| Ok(tr.conversations().get(id)?.conversation_id)).unwrap();
    assert_ne!(rc, c);
    assert_eq!(h.app.with_tracker("shop", |tr| tr.conversations().tasks(&rc)).unwrap(), vec![first]);
}

#[tokio::test]
async fn cross_channel_cycle_accepts_result_after_switching_conversations() {
    let (base, sent, _) = super::telegram::fake().await;
    let h = Harness::with_config(|cfg| cfg.telegram = Some(genie::config::TelegramConfig { token: "LEGACY".into(), api_base: Some(base) }));
    h.project("shop");
    h.app.with_server(|db| db.set_autonomy("shop", "manual")).unwrap();
    let anna = person(&h, "anna");
    let (r, c, a) = incoming(&h, &anna).await;
    let t = task(&h, &r, &a).await;
    let p = proposal(&h, &r, &a, &t, "dispatch").await;
    assert_eq!(decide(&h, &anna, p["id"].as_str().unwrap()).await.0, StatusCode::OK);
    h.app.with_tracker("shop", |tr| tr.conversations().complete(&r, "Task launched")).unwrap();
    let bot = h
        .app
        .with_server(|db| {
            db.set_project_bot("shop", 123, "project_bot", "123:test")?;
            let bot = db.project_bot("shop")?.unwrap();
            let user = db.user_by_login("anna")?.unwrap();
            db.link_channel(user.id, &bot.scope, "42")?;
            Ok(bot)
        })
        .unwrap();
    // No active response/client request is needed for delivery or durable continuation.
    let q = h
        .app
        .with_tracker("shop", |tr| tr.management().ask(&Actor::new("task-manager", Role::Orchestrator), &t, "Which timezone?", true))
        .unwrap();
    genie::engine::tick(&h.app).unwrap();
    genie::channels::dispatch(&h.app).await;
    assert_eq!(sent.lock().unwrap().len(), 1);
    let (_, new_c, new_agent) = incoming(&h, &anna).await;
    genie::telegram::handle_update(
        &h.app,
        &bot,
        &json!({"message":{
            "chat":{"id":42,"type":"private"},"text":"UTC","reply_to_message":{"message_id":42}
        }}),
    )
    .await
    .unwrap();
    let (s, error, _) = call(&h.remote, "POST", &format!("/api/conversations/{new_c}/answer?project=shop"))
        .bearer(&anna)
        .json(json!({"task":t,"question":q.id,"text":"UTC"}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{error}");
    let mail = h.app.with_tracker("shop", |tr| tr.bus().pending(None, &genie_core::management::manager_name(&t))).unwrap();
    assert!(mail.iter().any(|m| m.text.contains("UTC")));
    let n = mail.len();
    assert!(genie::interactions::telegram_reply(&h.app, &bot, 42, 42, "UTC").unwrap());
    assert_eq!(h.app.with_tracker("shop", |tr| tr.bus().pending(None, &genie_core::management::manager_name(&t))).unwrap().len(), n);
    // Explicit fixture result, not an LLM-generated artifact.
    h.app
        .with_tracker("shop", |tr| {
            tr.set_status(
                &Actor::new("anna", Role::Human),
                &t,
                Status::NeedsOwner,
                StatusOptions { force: true, note: Some("Inspect fixture CSV".into()), ..Default::default() },
            )
        })
        .unwrap();
    result_evidence(&h, &t);
    let (s, error, _) = call(&h.remote, "PATCH", &format!("/api/tasks/{t}?project=shop"))
        .bearer(&new_agent)
        .json(json!({"notes":"Continue in new session","expectedVersion":edit_version(&h,&t,&new_agent).await}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{error}");
    let (s, response, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/responses")
        .bearer(&anna)
        .json(json!({"model":"genie","input":"Prepare result for acceptance","previous_response_id":r,"background":true}))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK);
    let r = response["id"].as_str().unwrap();
    h.app.with_tracker("shop", |tr| tr.conversations().claim(r)).unwrap();
    let a = h
        .app
        .with_server(|db| {
            db.create_role_token("shop", Role::Analyst, Some("intake"), &format!("request:{r}"), None, None, chrono::Duration::hours(1))
        })
        .unwrap();
    let p = proposal(&h, r, &a, &t, "accept").await;
    genie::engine::tick(&h.app).unwrap();
    genie::channels::dispatch(&h.app).await;
    let idx =
        sent.lock().unwrap().iter().position(|m| m["body"]["text"].as_str().unwrap_or("").contains(p["id"].as_str().unwrap())).unwrap();
    let message_id = 42 + idx as i64;
    let update = json!({"callback_query":{"id":"accept-result","from":{"id":42},"data":"interaction:approve",
        "message":{"message_id":message_id,"chat":{"id":42,"type":"private"}}}});
    genie::telegram::handle_update(&h.app, &bot, &update).await.unwrap();
    genie::telegram::handle_update(&h.app, &bot, &update).await.unwrap();
    assert_eq!(h.app.with_tracker("shop", |tr| Ok(tr.get(&t)?.status)).unwrap(), Status::Done);
    let (s, v, _) = call(&h.remote, "GET", &format!("/api/conversations/{c}?project=shop")).bearer(&anna).send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["tasks"][0]["task"]["status"], "done");
}
