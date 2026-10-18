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
