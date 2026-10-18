use crate::common::{Harness, call};
use genie::decisions::Policy;
use genie_core::{
    Actor, CreateInput, Role, Status, StatusOptions,
    intelligence::{IntelligencePolicy, ProtectedComponent},
};
use serde_json::{Value, json};
fn ready(h: &Harness) -> String {
    h.app
        .with_tracker("shop", |t| {
            let a = Actor::new("owner", Role::Human);
            let task = t.create(
                &a,
                CreateInput {
                    title: "Scoped task".into(),
                    description: Some("Deliver an artifact".into()),
                    acceptance: vec!["Artifact verified".into()],
                    merge_strategy: Some("Artifact delivery".into()),
                    ..Default::default()
                },
            )?;
            t.set_status(&a, &task.id, Status::Ready, StatusOptions { force: true, ..Default::default() })?;
            Ok(task.id)
        })
        .unwrap()
}
fn policy(h: &Harness, protected: bool) {
    let p = Policy {
        intelligence: IntelligencePolicy {
            enabled: true,
            components: if protected {
                vec![ProtectedComponent {
                    id: "auth".into(),
                    paths: vec!["src/auth".into()],
                    sensitivity: "security".into(),
                    required_checks: vec!["security_review".into()],
                }]
            } else {
                vec![]
            },
            checks: std::collections::BTreeMap::from([("security_review".into(), "reviewer".into())]),
            ..Default::default()
        },
        ..Default::default()
    };
    genie::decisions::validate_policy(&h.app, "shop", &p).unwrap();
    h.app.with_server(|db| db.set_decision_policy("shop", "owner", &json!(p))).unwrap();
}
async fn history(h: &Harness, task: &str) -> Value {
    call(&h.router, "GET", &format!("/api/tasks/{task}/intelligence?project=shop")).send().await.1
}
#[tokio::test]
async fn protected_policy_applies_to_explicit_human_dispatch_and_history_is_project_scoped() {
    let h = Harness::new();
    h.project("shop");
    h.project("other");
    let id = ready(&h);
    policy(&h, true);
    let (status, v, _) = call(&h.router, "POST", "/api/teams?project=shop").json(json!({"task":id,"template":"solo"})).send().await;
    assert_eq!(status, 409, "{v}");
    assert!(v.to_string().contains("preflight"));
    let (status, _, _) = call(&h.router, "POST", &format!("/api/tasks/{id}/intelligence/assess?project=shop"))
        .json(json!({"affectedPaths":["src/auth/login.rs"]}))
        .send()
        .await;
    assert_eq!(status, 200);
    let (status, v, _) = call(&h.router, "POST", "/api/teams?project=shop").json(json!({"task":id,"template":"solo"})).send().await;
    assert_eq!(status, 409, "{v}");
    assert!(v.to_string().contains("mandatory"));
    let (status, team, _) = call(&h.router, "POST", "/api/teams?project=shop").json(json!({"task":id,"template":"pair"})).send().await;
    assert_eq!(status, 201, "{team}");
    let history = history(&h, &id).await;
    assert_eq!(history["attempts"].as_array().unwrap().len(), 1);
    assert_eq!(history["routing"].as_array().unwrap().len(), 1);
    assert_eq!(history["routing"][0]["plan"]["requiredChecks"], json!(["security_review"]));
    assert_eq!(call(&h.router, "GET", &format!("/api/tasks/{id}/intelligence?project=other")).send().await.0, 422);
}
#[tokio::test]
async fn solo_discovery_adds_reviewer_without_replacing_workspace_or_attempt_and_replay_is_idempotent() {
    let h = Harness::new();
    h.project("shop");
    let id = ready(&h);
    policy(&h, false);
    let (status, team, _) = call(&h.router, "POST", "/api/teams?project=shop").json(json!({"task":id,"template":"solo"})).send().await;
    assert_eq!(status, 201, "{team}");
    let body = json!({"requestId":"scope-v1","reason":"scope_expansion","evidenceRefs":["file:src/service.rs"]});
    let uri = format!("/api/tasks/{id}/intelligence/reassess?project=shop");
    let (status, result, _) = call(&h.router, "POST", &uri).json(body.clone()).send().await;
    assert_eq!(status, 200, "{result}");
    assert_eq!(result["team"]["id"], team["id"]);
    assert_eq!(result["team"]["cwd"], team["cwd"]);
    assert_eq!(result["team"]["members"].as_array().unwrap().len(), 2);
    let before = history(&h, &id).await;
    let (status, replay, _) = call(&h.router, "POST", &uri).json(body).send().await;
    assert_eq!(status, 200, "{replay}");
    assert_eq!(replay["reused"], true);
    assert_eq!(history(&h, &id).await, before);
    assert_eq!(before["attempts"].as_array().unwrap().len(), 1);
}
#[tokio::test]
async fn shadow_evaluation_freezes_candidate_without_execution_or_policy_mutation() {
    let h = Harness::new();
    h.project("shop");
    let id = ready(&h);
    policy(&h, false);
    let (_, snapshot, _) =
        call(&h.router, "POST", &format!("/api/tasks/{id}/intelligence/assess?project=shop")).json(json!({})).send().await;
    let before = h.app.with_tracker("shop", |t| t.get(&id)).unwrap();
    let policy_before = h.app.with_server(|db| db.decision_policy("shop")).unwrap();
    let candidate = Policy { templates: vec!["solo".into(), "pair".into()], ..Default::default() };
    let (status, v, _) = call(&h.router, "POST", &format!("/api/tasks/{id}/intelligence/shadow?project=shop"))
        .json(json!({"snapshotId":snapshot["id"],"candidate":candidate}))
        .send()
        .await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["evidence"]["mode"], "shadow");
    assert_eq!(v["evidence"]["status"], "unavailable");
    assert_eq!(v["evidence"]["productionPolicyChanged"], false);
    assert_eq!(serde_json::to_value(h.app.with_tracker("shop", |t| t.get(&id)).unwrap()).unwrap(), json!(before));
    assert_eq!(h.app.with_server(|db| db.decision_policy("shop")).unwrap(), policy_before);
    let history = history(&h, &id).await;
    assert!(history["attempts"].as_array().unwrap().is_empty());
    assert_eq!(history["evaluations"].as_array().unwrap().len(), 1);
}
#[tokio::test]
async fn unknown_sensitivity_discovery_without_policy_pauses_existing_members() {
    let h = Harness::new();
    h.project("shop");
    let id = ready(&h);
    policy(&h, false);
    let (_, team, _) = call(&h.router, "POST", "/api/teams?project=shop").json(json!({"task":id,"template":"solo"})).send().await;
    let (status, v, _) = call(&h.router, "POST", &format!("/api/tasks/{id}/intelligence/reassess?project=shop"))
        .json(json!({"requestId":"sensitive","reason":"sensitive_change","evidenceRefs":["file:credentials.rs"]}))
        .send()
        .await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["requiresOwner"], true);
    let id = team["id"].as_str().unwrap();
    assert!(
        h.app.with_tracker("shop", |t| Ok(t.bus().get(id)?.members.iter().all(|m| m.state == genie_core::MemberState::Paused))).unwrap()
    );
}

#[tokio::test]
async fn runtime_constraint_persists_until_explicit_human_resolution() {
    let h = Harness::new();
    h.project("shop");
    let id = ready(&h);
    policy(&h, false);
    let (_, team, _) = call(&h.router, "POST", "/api/teams?project=shop").json(json!({"task":id,"template":"solo"})).send().await;
    let uri = format!("/api/tasks/{id}/intelligence");
    let (status, v, _) = call(&h.router, "POST", &format!("{uri}/reassess?project=shop"))
        .json(json!({"requestId":"conflict-1","reason":"requirement_conflict","evidenceRefs":["brief:contradiction"]}))
        .send()
        .await;
    assert_eq!(status, 200, "{v}");
    let (status, assessed, _) = call(&h.router, "POST", &format!("{uri}/assess?project=shop")).json(json!({})).send().await;
    assert_eq!(status, 200, "{assessed}");
    assert!(
        assessed["profile"]["signals"].as_array().unwrap().iter().any(|s| s["name"] == "ambiguity" && s["value"] == "needs_owner_decision")
    );
    let team_id = team["id"].as_str().unwrap();
    let name = team["members"][0]["name"].as_str().unwrap();
    let resume = format!("/api/agents/{team_id}/{name}/resume?project=shop");
    assert_eq!(call(&h.router, "POST", &resume).send().await.0, 422);
    let (status,resolved,_)=call(&h.router,"POST",&format!("{uri}/resolve?project=shop")).json(json!({"snapshotId":assessed["id"],"reason":"Owner clarified the conflicting acceptance condition","evidenceRefs":["owner:decision-1"]})).send().await;
    assert_eq!(status, 200, "{resolved}");
    assert_eq!(call(&h.router, "POST", &resume).send().await.0, 200);
}
#[tokio::test]
async fn failed_routing_commit_rolls_back_topology_and_retry_retains_upgrade_evidence() {
    let h = Harness::new();
    h.project("shop");
    let id = ready(&h);
    policy(&h, false);
    let (_, team, _) = call(&h.router, "POST", "/api/teams?project=shop").json(json!({"task":id,"template":"solo"})).send().await;
    h.app.with_tracker("shop",|t|{t.conn().execute_batch("CREATE TRIGGER reject_routing BEFORE INSERT ON routing_decisions BEGIN SELECT RAISE(ABORT,'injected routing failure'); END;")?;Ok(())}).unwrap();
    let body = json!({"requestId":"atomic-upgrade","reason":"scope_expansion","evidenceRefs":["file:second-component.rs"]});
    let uri = format!("/api/tasks/{id}/intelligence/reassess?project=shop");
    assert_ne!(call(&h.router, "POST", &uri).json(body.clone()).send().await.0, 200);
    let current = h.app.with_tracker("shop", |t| t.bus().get(team["id"].as_str().unwrap())).unwrap();
    assert_eq!(json!(current), team);
    h.app
        .with_tracker("shop", |t| {
            t.conn().execute_batch("DROP TRIGGER reject_routing")?;
            Ok(())
        })
        .unwrap();
    let (status, v, _) = call(&h.router, "POST", &uri).json(body).send().await;
    assert_eq!(status, 200, "{v}");
    assert!(v["routing"]["plan"]["reasonCodes"].as_array().unwrap().contains(&json!("topology_upgrade")));
}
#[tokio::test]
async fn refinement_discoveries_do_not_create_delivery_attempts() {
    let h = Harness::new();
    h.project("shop");
    let id = ready(&h);
    policy(&h, false);
    h.app
        .with_tracker("shop", |t| {
            t.set_status(&Actor::new("owner", Role::Human), &id, Status::Refining, StatusOptions { force: true, ..Default::default() })
        })
        .unwrap();
    let (status, team, _) =
        call(&h.router, "POST", "/api/teams?project=shop").json(json!({"task":id,"members":[{"role":"analyst"}]})).send().await;
    assert_eq!(status, 201, "{team}");
    let (status, v, _) = call(&h.router, "POST", &format!("/api/tasks/{id}/intelligence/reassess?project=shop"))
        .json(json!({"requestId":"refinement","reason":"requirement_conflict","evidenceRefs":["brief:question"]}))
        .send()
        .await;
    assert_eq!(status, 409, "{v}");
    assert!(history(&h, &id).await["attempts"].as_array().unwrap().is_empty());
}
#[tokio::test]
async fn valid_large_policy_has_bounded_provenance_without_losing_checks() {
    let h = Harness::new();
    h.project("shop");
    let id = ready(&h);
    let mut p = Policy::default();
    p.intelligence.enabled = true;
    p.intelligence.checks.insert("review".into(), "reviewer".into());
    p.intelligence.components = (0..64)
        .map(|i| ProtectedComponent {
            id: format!("c{i}"),
            paths: vec![format!("src/c{i}")],
            sensitivity: "security".into(),
            required_checks: vec!["review".into()],
        })
        .collect();
    genie::decisions::validate_policy(&h.app, "shop", &p).unwrap();
    h.app.with_server(|db| db.set_decision_policy("shop", "owner", &json!(p))).unwrap();
    let (status, v, _) = call(&h.router, "POST", &format!("/api/tasks/{id}/intelligence/assess?project=shop")).json(json!({})).send().await;
    assert_eq!(status, 200, "{v}");
    assert!(v["profile"]["signals"].as_array().unwrap().iter().all(|s| s["evidenceRefs"].as_array().unwrap().len() <= 32));
}
#[tokio::test]
async fn shadow_intersects_frozen_choices_and_preserves_original_definitions() {
    let h = Harness::new();
    h.project("shop");
    let id = ready(&h);
    let p = Policy { templates: vec!["pair".into()], ..Default::default() };
    h.app.with_server(|db| db.set_decision_policy("shop", "owner", &json!(p))).unwrap();
    let (status, team, _) = call(&h.router, "POST", "/api/teams?project=shop").json(json!({"task":id,"template":"standard"})).send().await;
    assert_eq!(status, 201, "{team}");
    let history = history(&h, &id).await;
    let snapshot_id = history["routing"][0]["snapshotId"].clone();
    let frozen = history["routing"][0]["plan"]["templateEvidence"].clone();
    let (_, template, _) = call(&h.router, "GET", "/api/templates/pair?project=shop").send().await;
    let mut changed: Value = serde_json::from_str(template["builtin"].as_str().unwrap()).unwrap();
    changed["description"] = json!("A later incompatible description");
    let (status, v, _) =
        call(&h.router, "PUT", "/api/templates/pair?project=shop").json(json!({"template":changed,"baseHash":""})).send().await;
    assert_eq!(status, 200, "{v}");
    let candidate = Policy { templates: vec!["pair".into(), "full".into()], ..Default::default() };
    let (status, v, _) = call(&h.router, "POST", &format!("/api/tasks/{id}/intelligence/shadow?project=shop"))
        .json(json!({"snapshotId":snapshot_id,"candidate":candidate}))
        .send()
        .await;
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["evidence"]["frozenInput"]["eligibleTemplates"], json!(["pair"]));
    assert_eq!(v["evidence"]["frozenInput"]["frozenTemplates"], frozen);
}

#[tokio::test]
async fn intelligence_respects_read_only_membership_and_agent_task_scope() {
    let h = Harness::new();
    h.project("shop");
    let first = ready(&h);
    let other = ready(&h);
    policy(&h, false);
    let (_, team, _) = call(&h.router, "POST", "/api/teams?project=shop").json(json!({"task":first,"template":"solo"})).send().await;
    let agent = h
        .app
        .with_server(|db| {
            db.create_role_token("shop", Role::Executor, Some("executor"), "agent", team["id"].as_str(), None, chrono::Duration::hours(1))
        })
        .unwrap();
    assert_eq!(call(&h.router, "GET", &format!("/api/tasks/{first}/intelligence?project=shop")).bearer(&agent).send().await.0, 200);
    assert_eq!(call(&h.router, "GET", &format!("/api/tasks/{other}/intelligence?project=shop")).bearer(&agent).send().await.0, 403);
    assert_eq!(
        call(&h.router, "POST", &format!("/api/tasks/{first}/intelligence/assess?project=shop"))
            .bearer(&agent)
            .json(json!({"affectedPaths":["src/safe.rs"]}))
            .send()
            .await
            .0,
        403
    );
    let user = h.app.with_server(|db| db.create_user("viewer", "Viewer", None, Some("password-1"), false)).unwrap();
    h.app.with_server(|db| db.set_membership("shop", user.id, genie_core::server_db::ProjectRole::Viewer)).unwrap();
    let key = h.app.with_server(|db| db.create_user_token(user.id, "test")).unwrap();
    assert_eq!(call(&h.router, "GET", &format!("/api/tasks/{first}/intelligence?project=shop")).bearer(&key).send().await.0, 200);
    assert_eq!(
        call(&h.router, "POST", &format!("/api/tasks/{first}/intelligence/assess?project=shop"))
            .bearer(&key)
            .json(json!({}))
            .send()
            .await
            .0,
        403
    );
    assert_eq!(
        call(&h.router, "POST", &format!("/api/tasks/{first}/intelligence/shadow?project=shop"))
            .bearer(&key)
            .json(json!({"snapshotId":1,"candidate":{}}))
            .send()
            .await
            .0,
        403
    );
}
