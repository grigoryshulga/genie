//! Observation HTTP API: routes, status codes and request bodies (§14, §15). The local operator
//! acts through the router without a token until the first user exists; the viewer, team-member
//! and project-manager tests use tokens. Refusals and messages are asserted, not only statuses.

use axum::http::StatusCode;
use chrono::Duration;
use genie_core::server_db::ProjectRole;
use genie_core::{Actor, CreateInput, Role};
use serde_json::{Value, json};

use crate::common::Harness;
use crate::common::call;
use crate::common::fakemetrics;

const PROJECT: &str = "shop";

/// A project with one open task, created by a person.
fn project() -> (Harness, String) {
    let h = Harness::new();
    h.project(PROJECT);
    let task = h
        .app
        .with_tracker(PROJECT, |t| {
            t.create(&Actor::new("anna", Role::Human), CreateInput { title: "Faster checkout".into(), ..Default::default() })
        })
        .unwrap()
        .id;
    (h, task)
}

/// A manual guardrail: `checkout_errors` must stay at or below `max`.
fn guard(max: f64) -> Value {
    json!({ "guardrails": [{ "id": "checkout_errors", "source": "manual", "max": max }], "observation": { "minimumWindow": "24h" } })
}

/// A token of a team member (an agent of the analyst class), for one hour.
fn team_member(h: &Harness) -> String {
    h.app.with_server(|db| db.create_agent_token(PROJECT, Role::Analyst, "dana", Some("G-1"), None, Duration::hours(1))).unwrap()
}

/// A token of the project manager, the orchestrator of the project.
fn project_manager(h: &Harness) -> String {
    h.app
        .with_server(|db| {
            db.create_role_token(PROJECT, Role::Orchestrator, Some("project-manager"), "orchestrator", None, None, Duration::hours(1))
        })
        .unwrap()
}

/// A person who may only read the project. Creating users ends the local operator's access, so
/// tests that need it create the case first.
fn viewer(h: &Harness) -> String {
    h.app
        .with_server(|db| {
            let vic = db.create_user("vic", "Vic", None, None, false)?;
            db.set_membership(PROJECT, vic.id, ProjectRole::Viewer)?;
            db.create_user_token(vic.id, "viewer")
        })
        .unwrap()
}

/// The case of the local operator, created with its contract (`O-1`, proposed).
async fn create_case(h: &Harness, task: &str) -> Value {
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases")
        .json(json!({ "title": "Checkout latency", "policy": "guardrails", "tasks": [task], "contract": guard(50.0) }))
        .send()
        .await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

#[tokio::test]
async fn a_case_runs_from_creation_through_arming_start_evaluation_and_cancellation() {
    let (h, task) = project();
    let created = create_case(&h, &task).await;
    assert_eq!(created["case"]["key"], "O-1");
    assert_eq!(created["case"]["status"], "proposed");
    assert_eq!(created["case"]["contractVersion"], 1);
    assert_eq!(created["armProblems"], json!([]));
    assert_eq!(created["contracts"].as_array().unwrap().len(), 1);

    let (s, shown, _) = call(&h.router, "GET", "/api/observation/cases/O-1").send().await;
    assert_eq!(s, StatusCode::OK, "{shown}");
    assert_eq!(shown["case"]["tasks"], json!([task]));
    assert_eq!(shown["contract"]["version"], 1);

    // A revision before start needs no reason; the version moves on.
    let (s, revised, _) = call(&h.router, "PUT", "/api/observation/cases/1/contract").json(json!({ "contract": guard(40.0) })).send().await;
    assert_eq!(s, StatusCode::OK, "{revised}");
    assert_eq!(revised["case"]["contractVersion"], 2);
    assert_eq!(revised["contract"]["contract"]["guardrails"][0]["max"], 40.0);

    let (s, armed, _) = call(&h.router, "POST", "/api/observation/cases/1/arm").json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{armed}");
    assert_eq!(armed["case"]["status"], "waiting");
    assert!(armed["case"]["armedAt"].is_string());

    let (s, started, _) = call(&h.router, "POST", "/api/observation/cases/1/start").json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{started}");
    assert_eq!(started["case"]["status"], "observing");
    assert!(started["case"]["observationStartedAt"].is_string());

    let (s, asked, _) = call(&h.router, "POST", "/api/observation/cases/1/evaluate").json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{asked}");
    assert_eq!(asked["case"]["evaluationRequested"], "manual");

    let (s, cancelled, _) = call(&h.router, "POST", "/api/observation/cases/1/cancel").json(json!({ "reason": "superseded" })).send().await;
    assert_eq!(s, StatusCode::OK, "{cancelled}");
    assert_eq!(cancelled["case"]["status"], "cancelled");
    assert_eq!(cancelled["case"]["conclusion"], "superseded");

    let (s, listed, _) = call(&h.router, "GET", "/api/observation/cases?status=cancelled").send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let (_, none, _) = call(&h.router, "GET", "/api/observation/cases?status=observing").send().await;
    assert_eq!(none, json!([]));
}

#[tokio::test]
async fn unknown_cases_and_malformed_ids_are_refused_with_their_messages() {
    let (h, task) = project();
    create_case(&h, &task).await;

    let (s, v, _) = call(&h.router, "GET", "/api/observation/cases/O-99").send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"], "observation O-99 not found");

    let (s, v, _) = call(&h.router, "GET", "/api/observation/cases/abc").send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["error"], "unknown observation id abc");

    let (s, v, _) = call(&h.router, "POST", "/api/observation/incidents/x/resolve").json(json!({})).send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["error"], "unknown incident id x");

    let (s, v, _) = call(&h.router, "GET", "/api/observation/cases?limit=many").send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["error"], "limit must be a whole number, not many");

    let (s, v, _) = call(&h.router, "GET", "/api/observation/cases?limit=500").send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "the store checks the range of a limit: {v}");
    assert_eq!(v["error"], "limit must be between 1 and 200");

    let (s, v, _) = call(&h.router, "GET", "/api/observation/cases?status=sleeping").send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("sleeping"), "{v}");
}

#[tokio::test]
async fn an_invalid_contract_is_refused_with_its_problems_and_unknown_body_keys_are_bad_requests() {
    let (h, task) = project();
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases")
        .json(json!({
            "title": "Checkout latency",
            "policy": "guardrails",
            "tasks": [task],
            "contract": { "guardrails": [], "observation": { "minimumWindow": "24h" } }
        }))
        .send()
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"], "contract is invalid: guardrails: the guardrails policy needs at least one guardrail");

    // The body types refuse keys they do not know, with the extractor's bad request.
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases")
        .json(json!({ "title": "Checkout latency", "policy": "guardrails", "tasks": [task], "bogus": 1 }))
        .send()
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("unknown field `bogus`"), "{v}");

    // Nothing was created by either refusal.
    let (_, listed, _) = call(&h.router, "GET", "/api/observation/cases").send().await;
    assert_eq!(listed, json!([]));
}

#[tokio::test]
async fn arming_with_an_unknown_source_is_a_conflict_listing_the_problems() {
    let (h, task) = project();
    let metrics = json!({
        "guardrails": [{ "id": "errors", "source": "metrics", "query": "sum(rate(errors[$__range]))", "max": 50 }],
        "observation": { "minimumWindow": "24h" }
    });
    let (s, _, _) = call(&h.router, "POST", "/api/observation/cases")
        .json(json!({ "title": "Errors", "policy": "guardrails", "tasks": [task], "contract": metrics }))
        .send()
        .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/arm").json(json!({})).send().await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], "unknown source metrics");
    let (_, shown, _) = call(&h.router, "GET", "/api/observation/cases/1").send().await;
    assert_eq!(shown["armProblems"], json!(["unknown source metrics"]));

    let (s, _, _) = call(&h.router, "POST", "/api/observation/sources")
        .json(json!({ "name": "metrics", "kind": "prometheus", "url": "https://prom.example.com", "auth": "bearer" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/arm").json(json!({})).send().await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    assert_eq!(v["error"], "source metrics needs a secret");

    let (s, _, _) =
        call(&h.router, "PUT", "/api/observation/sources/metrics/secret").json(json!({ "value": "sk-test-0123456789" })).send().await;
    assert_eq!(s, StatusCode::OK);
    let (s, armed, _) = call(&h.router, "POST", "/api/observation/cases/1/arm").json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{armed}");
    assert_eq!(armed["case"]["status"], "waiting");
}

#[tokio::test]
async fn a_viewer_reads_and_every_mutation_is_forbidden() {
    let (h, task) = project();
    create_case(&h, &task).await;
    let token = viewer(&h);

    let (s, listed, _) = call(&h.router, "GET", "/api/observation/cases").bearer(&token).send().await;
    assert_eq!(s, StatusCode::OK, "{listed}");
    assert_eq!(listed.as_array().unwrap().len(), 1);
    let (s, shown, _) = call(&h.router, "GET", "/api/observation/cases/O-1").bearer(&token).send().await;
    assert_eq!(s, StatusCode::OK, "{shown}");
    let (s, evidence, _) = call(&h.router, "GET", "/api/observation/cases/O-1/evidence").bearer(&token).send().await;
    assert_eq!(s, StatusCode::OK, "{evidence}");
    let (s, incidents, _) = call(&h.router, "GET", "/api/observation/incidents").bearer(&token).send().await;
    assert_eq!(s, StatusCode::OK, "{incidents}");

    let refused = "read-only access to this project";
    for (method, path, body) in [
        ("POST", "/api/observation/cases", json!({ "title": "Other", "policy": "guardrails", "tasks": [task] })),
        ("PATCH", "/api/observation/cases/1", json!({ "title": "Renamed" })),
        ("POST", "/api/observation/cases/1/arm", json!({})),
        ("POST", "/api/observation/cases/1/evidence", json!({ "criterion": "checkout_errors", "value": 61.0 })),
    ] {
        let (s, v, _) = call(&h.router, method, path).bearer(&token).json(body).send().await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {path}: {v}");
        assert_eq!(v["error"], refused, "{method} {path}");
    }
    let (s, v, _) = call(&h.router, "GET", "/api/observation/sources").bearer(&token).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"], "project admin rights required");

    // Nothing changed.
    let (_, after, _) = call(&h.router, "GET", "/api/observation/cases/O-1").bearer(&token).send().await;
    assert_eq!(after["case"]["status"], "proposed");
    assert_eq!(after["case"]["title"], "Checkout latency");
}

#[tokio::test]
async fn a_team_member_reads_but_may_not_manage_observation() {
    let (h, task) = project();
    create_case(&h, &task).await;
    let token = team_member(&h);

    let (s, shown, _) = call(&h.router, "GET", "/api/observation/cases/O-1").bearer(&token).send().await;
    assert_eq!(s, StatusCode::OK, "{shown}");
    let manage = "only people and the project manager manage observation";
    for (method, path, body) in [
        ("POST", "/api/observation/cases", json!({ "title": "Other", "policy": "guardrails", "tasks": [task] })),
        ("POST", "/api/observation/cases/1/arm", json!({})),
        ("POST", "/api/observation/cases/1/cancel", json!({ "reason": "stop" })),
        ("POST", "/api/observation/cases/1/evidence", json!({ "criterion": "checkout_errors", "value": 61.0 })),
        ("POST", "/api/observation/cases/1/followup", json!({})),
    ] {
        let (s, v, _) = call(&h.router, method, path).bearer(&token).json(body).send().await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{method} {path}: {v}");
        assert_eq!(v["error"], manage, "{method} {path}");
    }
    let (s, v, _) = call(&h.router, "POST", "/api/observation/sources")
        .bearer(&token)
        .json(json!({ "name": "m", "kind": "prometheus", "url": "https://x.example.com" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"], "project admin rights required");
}

#[tokio::test]
async fn the_project_manager_creates_and_runs_a_case_but_only_people_finish_it() {
    let (h, task) = project();
    let token = project_manager(&h);
    let (s, created, _) = call(&h.router, "POST", "/api/observation/cases")
        .bearer(&token)
        .json(json!({ "title": "Checkout latency", "policy": "guardrails", "tasks": [task], "contract": guard(50.0) }))
        .send()
        .await;
    assert_eq!(s, StatusCode::CREATED, "{created}");
    assert_eq!(created["case"]["author"], "orchestrator");
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/arm").bearer(&token).json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/start").bearer(&token).json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/evaluate").bearer(&token).json(json!({ "final": true })).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"], "only a person may request a final evaluation");
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/evaluate").bearer(&token).json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["case"]["evaluationRequested"], "manual");

    // The manager's violating record is `agent` evidence: the evaluator ignores it, so no incident.
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/evidence")
        .bearer(&token)
        .json(json!({ "criterion": "checkout_errors", "value": 61.0 }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["evidence"][0]["trust"], "agent");
    assert_eq!(v["incidents"], json!([]));
}

#[tokio::test]
async fn a_person_reports_a_violation_and_only_a_person_dismisses_the_incident() {
    let (h, task) = project();
    create_case(&h, &task).await;
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/arm").json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/start").json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");

    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/evidence")
        .json(json!({ "criterion": "checkout_errors", "value": 61.0, "note": "p95 spike" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["evidence"][0]["trust"], "reported");
    assert_eq!(v["incidents"][0]["key"], "I-1");
    assert_eq!(v["incidents"][0]["status"], "open");

    // A criterion that the current contract does not name is refused.
    let (s, v, _) =
        call(&h.router, "POST", "/api/observation/cases/1/evidence").json(json!({ "criterion": "nope", "value": 1.0 })).send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"], "criterion nope is not in the current contract of O-1");

    // A list of evidence, oldest first, with its limits checked.
    let (_, all, _) = call(&h.router, "GET", "/api/observation/cases/O-1/evidence").send().await;
    assert_eq!(all.as_array().unwrap().len(), 1);
    let (s, v, _) = call(&h.router, "GET", "/api/observation/cases/O-1/evidence?limit=0").send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"], "limit must be between 1 and 500");
    let (_, none, _) = call(&h.router, "GET", "/api/observation/cases/O-1/evidence?criterion=other").send().await;
    assert_eq!(none, json!([]));

    // The incident: listed by status, dismissed by a person with a note.
    let (_, open, _) = call(&h.router, "GET", "/api/observation/incidents?status=open").send().await;
    assert_eq!(open.as_array().unwrap().len(), 1);
    let (_, resolved, _) = call(&h.router, "GET", "/api/observation/incidents?status=resolved").send().await;
    assert_eq!(resolved, json!([]));
    let (s, v, _) =
        call(&h.router, "POST", "/api/observation/incidents/I-1/dismiss").json(json!({ "note": "a false positive" })).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "dismissed");
    assert_eq!(v["note"], "a false positive");
}

#[tokio::test]
async fn the_manager_resolves_an_incident_and_files_a_follow_up_task_linked_from_it() {
    let (h, task) = project();
    let token = project_manager(&h);
    let (s, _, _) = call(&h.router, "POST", "/api/observation/cases")
        .bearer(&token)
        .json(json!({ "title": "Checkout latency", "policy": "guardrails", "tasks": [task], "contract": guard(50.0) }))
        .send()
        .await;
    assert_eq!(s, StatusCode::CREATED);
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/arm").bearer(&token).json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/start").bearer(&token).json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    // The local operator is a person: this violation is `reported` and opens the incident that the
    // manager then handles.
    let (s, v, _) = call(&h.router, "POST", "/api/observation/cases/1/evidence")
        .json(json!({ "criterion": "checkout_errors", "value": 61.0 }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["incidents"].as_array().unwrap().len(), 1);

    let (s, v, _) =
        call(&h.router, "POST", "/api/observation/incidents/I-1/dismiss").bearer(&token).json(json!({ "note": "noise" })).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(v["error"], "only a person may dismiss an incident; resolve it instead, or ask a person to dismiss it");

    let (s, v, _) = call(&h.router, "POST", "/api/observation/incidents/1/resolve").bearer(&token).json(json!({})).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["status"], "resolved");

    let (s, v, _) =
        call(&h.router, "POST", "/api/observation/cases/O-1/followup").bearer(&token).json(json!({ "incident": 1 })).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let followup = v["followups"][0]["task"].as_str().unwrap().to_string();
    assert_eq!(v["followups"][0]["incident"], 1);
    assert_eq!(v["incidents"][0]["followups"], json!([followup]));

    let (s, filed, _) = call(&h.router, "GET", &format!("/api/tasks/{followup}")).send().await;
    assert_eq!(s, StatusCode::OK, "{filed}");
    assert_eq!(filed["title"], "Regression: checkout_errors (O-1)");

    let (s, cases, _) = call(&h.router, "GET", &format!("/api/observation/tasks/{task}")).send().await;
    assert_eq!(s, StatusCode::OK, "{cases}");
    assert_eq!(cases[0]["key"], "O-1");
    let (_, none, _) = call(&h.router, "GET", "/api/observation/tasks/GNE-404").send().await;
    assert_eq!(none, json!([]));
}

#[tokio::test]
async fn sources_are_managed_by_the_project_admin_and_their_secrets_never_leave_the_server() {
    let (h, _task) = project();
    let (s, added, _) = call(&h.router, "POST", "/api/observation/sources")
        .json(json!({ "name": "metrics", "kind": "prometheus", "url": "https://prom.example.com", "auth": "bearer" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::CREATED, "{added}");
    assert_eq!(added["name"], "metrics");
    assert_eq!(added["kind"], "prometheus");
    assert!(added.get("secret").is_none(), "{added}");

    let (s, with_secret, _) =
        call(&h.router, "PUT", "/api/observation/sources/metrics/secret").json(json!({ "value": "sk-test-0123456789" })).send().await;
    assert_eq!(s, StatusCode::OK, "{with_secret}");
    assert_eq!(with_secret["secret"]["hint"], "…6789");
    assert!(!with_secret.to_string().contains("sk-test"), "{with_secret}");

    let (s, listed, _) = call(&h.router, "GET", "/api/observation/sources").send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["secret"]["hint"], "…6789");
    assert!(!listed.to_string().contains("sk-test"), "{listed}");

    let (s, updated, _) = call(&h.router, "PUT", "/api/observation/sources/metrics")
        .json(json!({ "name": "metrics", "kind": "prometheus", "url": "https://prom2.example.com", "auth": "bearer", "timeoutSecs": 10 }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{updated}");
    assert_eq!(updated["url"], "https://prom2.example.com");
    assert_eq!(updated["secret"]["hint"], "…6789", "updating the configuration keeps the secret");

    let (s, v, _) = call(&h.router, "PUT", "/api/observation/sources/metrics")
        .json(json!({ "name": "other", "kind": "prometheus", "url": "https://prom2.example.com" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"], "source metrics cannot be renamed; add other and remove metrics instead");

    let (s, v, _) = call(&h.router, "POST", "/api/observation/sources")
        .json(json!({ "name": "metrics", "kind": "prometheus", "url": "https://prom.example.com", "bogus": true }))
        .send()
        .await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["error"].as_str().unwrap().contains("unknown field `bogus`"), "{v}");

    let (s, cleared, _) = call(&h.router, "DELETE", "/api/observation/sources/metrics/secret").send().await;
    assert_eq!(s, StatusCode::OK, "{cleared}");
    assert!(cleared.get("secret").is_none(), "{cleared}");

    let (s, v, _) = call(&h.router, "DELETE", "/api/observation/sources/metrics").send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v, json!({ "ok": true }));
    let (_, listed, _) = call(&h.router, "GET", "/api/observation/sources").send().await;
    assert_eq!(listed, json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_source_test_returns_the_value_the_fake_server_gave_and_refusals_carry_the_adapter_message() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond(
        "api_calls",
        json!({ "status": "success", "data": { "resultType": "vector", "result": [{ "metric": {}, "value": [1760702400, "0.25"] }] } }),
    );
    let (h, _task) = project();
    let (s, _, _) = call(&h.router, "POST", "/api/observation/sources")
        .json(json!({ "name": "metrics", "kind": "prometheus", "url": fake.url }))
        .send()
        .await;
    assert_eq!(s, StatusCode::CREATED);

    let (s, tested, _) = call(&h.router, "POST", "/api/observation/sources/metrics/test")
        .json(json!({ "query": "sum(rate(api_calls[5m]))", "window": "30m" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{tested}");
    assert_eq!(tested["value"], 0.25);
    assert_eq!(tested["query"], "sum(rate(api_calls[5m]))");
    assert_eq!(tested["provenance"]["adapter"], "prometheus");

    let (s, v, _) =
        call(&h.router, "POST", "/api/observation/sources/metrics/test").json(json!({ "query": "missing_metric" })).send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"], "the source answered HTTP 404: no scripted answer");

    let (s, v, _) =
        call(&h.router, "POST", "/api/observation/sources/metrics/test").json(json!({ "query": "up", "window": "soon" })).send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert_eq!(v["error"], "window soon must be a positive duration of at most 90 days, such as 30m, 1h or 2d");

    let (s, v, _) = call(&h.router, "POST", "/api/observation/sources/nope/test").json(json!({ "query": "up" })).send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"], "source nope not found");
}
