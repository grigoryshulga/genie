//! End-to-end observation scenarios: a change is delivered, observed, guarded and concluded. The people
//! and agents act through the HTTP API and the operation catalog, and time moves only through
//! `scheduler::pass_at` with explicit times. A case starts at the pass that finds its task done, so its
//! anchor is that pass's time and no wall clock enters a scenario's schedule. Each scenario reads the
//! case's journal events in order (§12) and checks the states, the tasks, the notifications and the mail
//! the project manager receives.

use axum::http::StatusCode;
use chrono::{DateTime, Duration, Utc};
use genie::observation::scheduler::{self, Plan};
use genie::ops::{self, Auth, Cx, InProcess};
use genie_core::inbox::Notification;
use genie_core::observation::{parse_ts, ts};
use genie_core::server_db::ProjectRole;
use genie_core::team::{Mail, ORCHESTRATOR};
use genie_core::{Role, TaskType};
use serde_json::{Value, json};

use crate::common::fakemetrics;
use crate::common::{Harness, call};

const PROJECT: &str = "shop";
/// The first pass of every scenario, and the anchor of its observation.
const START: &str = "2026-10-17T12:00:00.000Z";

/// The time `minutes` after `START`: the only clock the scenarios use.
fn at(minutes: i64) -> DateTime<Utc> {
    parse_ts(START).unwrap() + Duration::minutes(minutes)
}

/// The people and agents of the project. `root` is a server administrator, which makes it a project
/// admin too; `ann` is a person with write access and the author of the cases; `manager` is the
/// project manager. Every request of a scenario carries one of these tokens, so the local operator
/// never acts.
struct Team {
    h: Harness,
    root: String,
    ann: String,
    manager: String,
}

fn team() -> Team {
    let h = Harness::new();
    h.project(PROJECT);
    let (root, ann) = h
        .app
        .with_server(|db| {
            let root = db.create_user("root", "Root", None, None, true)?;
            let ann = db.create_user("ann", "Ann", None, None, false)?;
            db.set_membership(PROJECT, ann.id, ProjectRole::Member)?;
            Ok((db.create_user_token(root.id, "root")?, db.create_user_token(ann.id, "ann")?))
        })
        .unwrap();
    let manager = h
        .app
        .with_server(|db| {
            db.create_role_token(PROJECT, Role::Orchestrator, Some("project-manager"), "orchestrator", None, None, Duration::hours(1))
        })
        .unwrap();
    Team { h, root, ann, manager }
}

/// A request as `token`; the status must be `status`. The answer's body is returned.
async fn send(h: &Harness, token: &str, method: &str, path: &str, body: Option<Value>, status: StatusCode) -> Value {
    let request = call(&h.router, method, path).bearer(token);
    let request = match body {
        Some(body) => request.json(body),
        None => request,
    };
    let (got, answer, _) = request.send().await;
    assert_eq!(got, status, "{method} {path}: {answer}");
    answer
}

/// A task created by `ann`, with the id the tracker gave it.
async fn create_task(t: &Team, title: &str) -> String {
    let created = send(&t.h, &t.ann, "POST", "/api/tasks", Some(json!({ "title": title })), StatusCode::CREATED).await;
    created["id"].as_str().unwrap().to_string()
}

/// `ann` moves the task to done: the delivery the observation waits for.
async fn deliver(t: &Team, task: &str) {
    send(&t.h, &t.ann, "POST", &format!("/api/tasks/{task}/status"), Some(json!({ "status": "done" })), StatusCode::OK).await;
}

/// `root` adds the Prometheus-compatible source `metrics`, served by the fake at `url`.
async fn add_metrics_source(t: &Team, url: &str) {
    let body = json!({ "name": "metrics", "kind": "prometheus", "url": url });
    send(&t.h, &t.root, "POST", "/api/observation/sources", Some(body), StatusCode::CREATED).await;
}

async fn create_case(t: &Team, body: Value) -> Value {
    send(&t.h, &t.ann, "POST", "/api/observation/cases", Some(body), StatusCode::CREATED).await
}

/// `ann` arms the case: it waits for its tasks.
async fn arm(t: &Team, key: &str) -> Value {
    send(&t.h, &t.ann, "POST", &format!("/api/observation/cases/{key}/arm"), Some(json!({})), StatusCode::OK).await
}

/// The case page, as `ann` reads it.
async fn view(t: &Team, key: &str) -> Value {
    send(&t.h, &t.ann, "GET", &format!("/api/observation/cases/{key}"), None, StatusCode::OK).await
}

/// The evidence records of the case, oldest first.
async fn evidence(t: &Team, key: &str) -> Vec<Value> {
    let listed = send(&t.h, &t.ann, "GET", &format!("/api/observation/cases/{key}/evidence"), None, StatusCode::OK).await;
    listed.as_array().unwrap().clone()
}

/// One scheduler pass at `now`, on the blocking pool as the server runs it.
async fn pass(h: &Harness, now: DateTime<Utc>) -> Plan {
    h.app.blocking(move |app| scheduler::pass_at(app, now)).await.unwrap()
}

/// The journal events of one case, oldest first, as (type, payload).
fn journal(h: &Harness, id: i64) -> Vec<(String, Value)> {
    let subject = format!("observation:{id}");
    h.app
        .with_tracker(PROJECT, |t| t.events_after(0, 1000))
        .unwrap()
        .into_iter()
        .filter(|e| e.subject.as_deref() == Some(subject.as_str()))
        .map(|e| (e.kind, e.payload))
        .collect()
}

fn kinds(journal: &[(String, Value)]) -> Vec<&str> {
    journal.iter().map(|(kind, _)| kind.as_str()).collect()
}

/// The payload of the first event of `kind`.
fn payload<'a>(journal: &'a [(String, Value)], kind: &str) -> &'a Value {
    &journal.iter().find(|(k, _)| k.as_str() == kind).unwrap_or_else(|| panic!("no {kind} in {journal:?}")).1
}

/// The web notifications of one kind for a login.
fn notifications(h: &Harness, login: &str, kind: &str) -> Vec<Notification> {
    h.app
        .with_server(|db| {
            let user = db.user_by_login(login)?.ok_or_else(|| genie_core::GenieError::not_found(format!("user {login} not found")))?;
            db.notifications(user.id, false, 100)
        })
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == kind)
        .collect()
}

/// The mail of the observation plane waiting for the project manager, oldest first. The same mailbox
/// also holds the owner's task notices, which are not part of this plane and are left out.
fn manager_mail(h: &Harness) -> Vec<Mail> {
    h.app.with_tracker(PROJECT, |t| t.bus().pending(None, ORCHESTRATOR)).unwrap().into_iter().filter(|m| m.from == "observation").collect()
}

/// The operation catalog as a caller with `token`, the way the CLI and MCP reach the API.
fn catalog(h: &Harness, token: &str) -> Cx {
    let api = InProcess::new(genie::http::router(h.app.clone()), h.app.cfg.port, Auth::Bearer(token.into()), Some(PROJECT.into()));
    Cx { api: Box::new(api), project: Some(PROJECT.into()), task: None, team: None, local: false }
}

/// A Prometheus-compatible answer with one series.
fn vector(sample: &str) -> Value {
    json!({ "status": "success", "data": { "resultType": "vector", "result": [{ "metric": { "job": "api" }, "value": [1760702400, sample] }] } })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guardrail_regression_opens_an_incident_files_a_bug_and_concludes_negative() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("checkout_errors", vector("1.0"));
    let t = team();
    let task = create_task(&t, "Faster checkout").await;
    assert_eq!(task, "G-1");
    let case = create_case(
        &t,
        json!({
            "title": "Checkout error rate",
            "policy": "guardrails",
            "tasks": [task],
            "contract": {
                "guardrails": [{ "id": "error_rate", "source": "metrics", "query": "sum(rate(checkout_errors_total[$__range]))", "maxChange": "+0.1pp" }],
                "observation": { "minimumWindow": "24h" },
                "followup": "auto"
            }
        }),
    )
    .await;
    assert_eq!(case["case"]["key"], "O-1");

    // The contract names a source the project does not have yet: arming waits for it.
    let refused = send(&t.h, &t.ann, "POST", "/api/observation/cases/O-1/arm", Some(json!({})), StatusCode::CONFLICT).await;
    assert_eq!(refused["error"], "unknown source metrics");
    add_metrics_source(&t, &fake.url).await;
    assert_eq!(arm(&t, "O-1").await["case"]["status"], "waiting");

    // G-1 is delivered. The first pass starts the observation at its own time, takes the baseline (the
    // 7 days before the anchor), and asks for the first guardrail check 15 minutes on. The observed
    // window at the anchor is empty, so nothing else is asked.
    deliver(&t, &task).await;
    let plan = pass(&t.h, at(0)).await;
    let started = view(&t, "O-1").await;
    assert_eq!(started["case"]["status"], "observing");
    assert_eq!(started["case"]["observationStartedAt"], ts(at(0)));
    assert_eq!(plan.at, Some(at(15)));
    let calls = fake.state.lock().calls.clone();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].url.contains("604800s"), "the baseline window is 7 days: {}", calls[0].url);
    let records = evidence(&t, "O-1").await;
    assert_eq!(records.len(), 1);
    assert_eq!((records[0]["phase"].as_str(), records[0]["value"].as_f64()), (Some("baseline"), Some(1.0)));
    assert_eq!(records[0]["windowStart"], ts(at(0) - Duration::days(7)));

    // At 15 minutes the error rate is unchanged: an observed record, and no incident.
    let plan = pass(&t.h, at(15)).await;
    assert_eq!(plan.at, Some(at(30)));
    assert!(view(&t, "O-1").await["incidents"].as_array().unwrap().is_empty());

    // At 30 minutes it is 1.2 against the baseline 1.0: +0.2pp, over the +0.1pp the contract allows. The
    // violation opens I-1 in the pass that stores the record, and the auto policy files its bug.
    fake.state.lock().respond("checkout_errors", vector("1.2"));
    let plan = pass(&t.h, at(30)).await;
    assert_eq!(plan.at, Some(at(45)), "the next check; the evaluation waits for the minimum window");
    let violated = view(&t, "O-1").await;
    let records = evidence(&t, "O-1").await;
    assert_eq!(records.len(), 3);
    let incident = &violated["incidents"][0];
    assert_eq!(
        (incident["key"].as_str(), incident["guardrail"].as_str(), incident["status"].as_str()),
        (Some("I-1"), Some("error_rate"), Some("open"))
    );
    assert_eq!(incident["summary"], "change +0.2pp exceeds +0.1pp");
    assert_eq!(incident["evidence"], records[2]["id"], "the incident names the record that violated the limit");
    assert_eq!(incident["followups"], json!(["G-2"]));
    assert_eq!(violated["followups"].as_array().unwrap().len(), 1);
    assert_eq!(violated["followups"][0]["task"], "G-2");
    assert_eq!(violated["followups"][0]["incident"], 1);
    assert_eq!(violated["followups"][0]["actor"], "observation");

    // The follow-up is a bug, labelled `observation`, and its description links the case and the incident.
    let filed = t.h.app.with_tracker(PROJECT, |tr| tr.get("G-2")).unwrap();
    assert_eq!(filed.task_type, TaskType::Bug);
    assert_eq!(filed.labels, ["observation"]);
    assert_eq!(filed.title, "Regression: error_rate (O-1)");
    assert!(filed.description.contains("/observation/1") && filed.description.contains("I-1"), "{}", filed.description);

    // The admin (root) and the author (ann, a person) are told, once each, with a link to the case.
    for login in ["root", "ann"] {
        let told = notifications(&t.h, login, "observation.incident");
        assert_eq!(told.len(), 1, "{login}");
        assert_eq!(told[0].link.as_deref(), Some("/observation/1"));
        assert_eq!(told[0].body, "change +0.2pp exceeds +0.1pp");
    }
    // The project manager is mailed the incident, with the command that answers it.
    let mail = manager_mail(&t.h);
    assert_eq!(mail.len(), 1);
    assert_eq!((mail[0].from.as_str(), mail[0].task.as_deref()), ("observation", Some("G-1")));
    assert!(mail[0].text.contains("is violated") && mail[0].text.contains("genie observe resolve I-1"), "{}", mail[0].text);

    let so_far = journal(&t.h, 1);
    assert_eq!(
        kinds(&so_far),
        [
            "outcome.created",
            "outcome.updated",
            "observation.started",
            "evidence.collected",
            "evidence.collected",
            "guardrail.violated",
            "incident.created",
            "evidence.collected",
            "followup_task.created",
        ]
    );
    assert_eq!(payload(&so_far, "incident.created")["incidentKey"], "I-1");
    assert_eq!(payload(&so_far, "followup_task.created")["task"], "G-2");

    // A person asks for a final evaluation before the minimum window. The violated guardrail concludes
    // the case as negative: the repeated violation of the open incident files nothing new.
    let asked = send(&t.h, &t.ann, "POST", "/api/observation/cases/O-1/evaluate", Some(json!({ "final": true })), StatusCode::OK).await;
    assert_eq!(asked["case"]["evaluationRequested"], "final");
    let plan = pass(&t.h, at(45)).await;
    assert_eq!(plan.at, None, "a concluded case is not due");

    let concluded = view(&t, "O-1").await;
    assert_eq!(concluded["case"]["status"], "concluded");
    assert_eq!(concluded["case"]["result"], "negative");
    assert_eq!(concluded["case"]["confidence"], "high", "a violated guardrail is measured evidence");
    assert_eq!(concluded["case"]["conclusion"], "negative: guardrails: 0/1 ok, 1 violated; attribution: not assessed");
    assert!(concluded["case"]["nextGuardrailAt"].is_null());
    assert!(concluded["case"]["nextEvaluationAt"].is_null());
    assert_eq!(concluded["evaluations"].as_array().unwrap().len(), 1);
    assert_eq!(concluded["evaluations"][0]["trigger"], "final");
    assert_eq!(concluded["evaluations"][0]["verdict"], "negative");
    assert_eq!(concluded["evaluations"][0]["details"][0]["state"], "violated");
    assert_eq!(concluded["followups"].as_array().unwrap().len(), 1);

    for login in ["root", "ann"] {
        let told = notifications(&t.h, login, "observation.concluded");
        assert_eq!(told.len(), 1, "{login}");
        assert_eq!(told[0].link.as_deref(), Some("/observation/1"));
    }
    let mail = manager_mail(&t.h);
    assert_eq!(mail.len(), 2);
    assert_eq!(mail[1].from, "observation");
    assert!(mail[1].text.contains("concluded as negative") && mail[1].text.contains("genie observe followup O-1"), "{}", mail[1].text);

    let whole = journal(&t.h, 1);
    assert_eq!(
        kinds(&whole),
        [
            "outcome.created",
            "outcome.updated",
            "observation.started",
            "evidence.collected",
            "evidence.collected",
            "guardrail.violated",
            "incident.created",
            "evidence.collected",
            "followup_task.created",
            "outcome.ready_for_evaluation",
            "evidence.collected",
            "outcome.negative",
        ]
    );
    assert_eq!(payload(&whole, "outcome.ready_for_evaluation")["trigger"], "final");
    assert_eq!(payload(&whole, "outcome.negative")["confidence"], "high");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verified_improvement_waits_for_its_samples_and_is_confirmed_with_high_confidence() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("checkout_latency", vector("100"));
    fake.state.lock().respond("checkout_requests", vector("4200"));
    let t = team();
    add_metrics_source(&t, &fake.url).await;
    let task = create_task(&t, "Faster checkout").await;
    create_case(
        &t,
        json!({
            "title": "Checkout latency",
            "policy": "verify",
            "tasks": [task],
            "contract": {
                "hypotheses": [{
                    "id": "checkout_latency_p95",
                    "source": "metrics",
                    "query": "histogram_quantile(0.95, sum(rate(checkout_latency_bucket[$__range])) by (le))",
                    "samplesQuery": "sum(increase(checkout_requests_total[$__range]))",
                    "expect": { "change": "-20%" }
                }],
                "observation": { "minimumWindow": "24h", "preferredWindow": "72h", "minimumSamples": 10000 }
            }
        }),
    )
    .await;
    assert_eq!(arm(&t, "O-1").await["case"]["status"], "waiting");

    // Delivery starts the observation at the first pass; the baseline is the latency of the week before.
    deliver(&t, &task).await;
    pass(&t.h, at(0)).await;
    assert_eq!(view(&t, "O-1").await["case"]["status"], "observing");
    let baseline = evidence(&t, "O-1").await;
    assert_eq!((baseline[0]["phase"].as_str(), baseline[0]["value"].as_f64()), (Some("baseline"), Some(100.0)));

    // At the minimum window latency has fallen by 30%, but only 4200 requests were counted: the verdict is
    // insufficient, the case keeps observing, and the next evaluation is due 6 hours on.
    fake.state.lock().respond("checkout_latency", vector("70"));
    let plan = pass(&t.h, at(24 * 60)).await;
    assert_eq!(plan.at, Some(at(24 * 60 + 15)), "the next guardrail check (none here) comes before the evaluation");
    let waiting = view(&t, "O-1").await;
    assert_eq!(waiting["case"]["status"], "observing");
    assert_eq!(waiting["case"]["nextEvaluationAt"], ts(at(30 * 60)));
    let first = &waiting["evaluations"][0];
    assert_eq!((first["verdict"].as_str(), first["trigger"].as_str()), (Some("insufficient"), Some("window")));
    assert_eq!(first["details"][0]["reason"], "samples 4200 below minimum 10000");
    assert!(first["confidence"].is_null());
    let measured = evidence(&t, "O-1").await;
    assert_eq!(measured[1]["samples"], 4200);
    let relative = measured[1]["deltaRel"].as_f64().unwrap();
    assert!((relative + 30.0).abs() < 1e-9, "{relative}");

    // The catalog shows the same observing case to its callers.
    let shown = ops::find("observe", "show").unwrap().run_json(json!({ "id": "O-1" }), &catalog(&t.h, &t.ann)).await.unwrap();
    assert_eq!(shown.data["case"]["status"], "observing", "{}", shown.text);

    // At 30 hours the samples are above the minimum and the latency change still holds: confirmed.
    fake.state.lock().respond("checkout_requests", vector("12000"));
    pass(&t.h, at(30 * 60)).await;
    let done = view(&t, "O-1").await;
    assert_eq!(done["case"]["status"], "concluded");
    assert_eq!(done["case"]["result"], "confirmed");
    assert_eq!(done["case"]["confidence"], "high", "measured records only");
    assert_eq!(done["case"]["conclusion"], "confirmed: 1/1 hypotheses met; guardrails: 0/0 ok, 0 violated; attribution: not assessed");
    assert_eq!(done["evaluations"].as_array().unwrap().len(), 2);
    assert_eq!(done["evaluations"][0]["verdict"], "confirmed");
    assert_eq!(done["evaluations"][0]["trigger"], "window");
    assert_eq!(done["evaluations"][0]["details"][0]["reason"], "delta -30% meets target -20%");
    assert_eq!(done["evaluations"][1]["verdict"], "insufficient");
    assert!(done["incidents"].as_array().unwrap().is_empty());
    assert!(done["followups"].as_array().unwrap().is_empty(), "a confirmed case files nothing by itself");

    for login in ["root", "ann"] {
        let told = notifications(&t.h, login, "observation.concluded");
        assert_eq!(told.len(), 1, "{login}");
        assert_eq!(told[0].link.as_deref(), Some("/observation/1"));
    }
    let mail = manager_mail(&t.h);
    assert_eq!(mail.len(), 1);
    assert!(mail[0].text.contains("concluded as confirmed") && mail[0].text.contains("genie observe followup O-1"), "{}", mail[0].text);

    let whole = journal(&t.h, 1);
    assert_eq!(
        kinds(&whole),
        [
            "outcome.created",
            "outcome.updated",
            "observation.started",
            "evidence.collected",
            "evidence.collected",
            "outcome.evaluated",
            "evidence.collected",
            "outcome.confirmed",
        ]
    );
    assert_eq!(payload(&whole, "outcome.evaluated")["verdict"], "insufficient");
    assert_eq!(payload(&whole, "outcome.evaluated")["nextEvaluationAt"], ts(at(30 * 60)));
    assert_eq!(payload(&whole, "outcome.confirmed")["confidence"], "high");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_false_positive_is_dismissed_by_a_person_and_the_final_evaluation_confirms() {
    let t = team();
    let task = create_task(&t, "Faster checkout").await;
    let dana =
        t.h.app
            .with_server(|db| db.create_agent_token(PROJECT, Role::Analyst, "dana", Some(task.as_str()), None, Duration::hours(1)))
            .unwrap();
    create_case(
        &t,
        json!({
            "title": "Checkout errors",
            "policy": "guardrails",
            "tasks": [task],
            "contract": {
                "guardrails": [{ "id": "checkout_errors", "source": "manual", "max": 50 }],
                "observation": { "minimumWindow": "24h" }
            }
        }),
    )
    .await;
    arm(&t, "O-1").await;
    deliver(&t, &task).await;
    pass(&t.h, at(0)).await;
    assert_eq!(view(&t, "O-1").await["case"]["status"], "observing");

    // A person reports 61 against the limit of 50: the violation opens I-1 at once. Manual cases file
    // nothing by themselves.
    let reported = send(
        &t.h,
        &t.ann,
        "POST",
        "/api/observation/cases/O-1/evidence",
        Some(json!({ "criterion": "checkout_errors", "value": 61.0, "windowEnd": ts(at(30)), "note": "dashboard spike" })),
        StatusCode::OK,
    )
    .await;
    assert_eq!(reported["evidence"][0]["trust"], "reported");
    assert_eq!((reported["incidents"][0]["key"].as_str(), reported["incidents"][0]["status"].as_str()), (Some("I-1"), Some("open")));
    assert!(reported["followups"].as_array().unwrap().is_empty());
    for login in ["root", "ann"] {
        assert_eq!(notifications(&t.h, login, "observation.incident").len(), 1, "{login}");
    }
    assert_eq!(manager_mail(&t.h).len(), 1);

    // An analyst agent may not dismiss the incident, and neither may the project manager, through the
    // catalog: only a person dismisses.
    let refused =
        send(&t.h, &dana, "POST", "/api/observation/incidents/I-1/dismiss", Some(json!({ "note": "noise" })), StatusCode::FORBIDDEN).await;
    assert_eq!(refused["error"], "only people and the project manager manage observation");
    let refused = ops::find("observe", "dismiss")
        .unwrap()
        .run_json(json!({ "id": "I-1", "note": "noise" }), &catalog(&t.h, &t.manager))
        .await
        .unwrap_err();
    assert!(refused.contains("only a person may dismiss an incident; resolve it instead, or ask a person to dismiss it"), "{refused}");
    let open = send(&t.h, &t.ann, "GET", "/api/observation/incidents?status=open", None, StatusCode::OK).await;
    assert_eq!(open.as_array().unwrap().len(), 1, "both refusals left the incident open: {open}");

    // The person dismisses it as a false positive, with the note that says why.
    let dismissed = send(
        &t.h,
        &t.ann,
        "POST",
        "/api/observation/incidents/I-1/dismiss",
        Some(json!({ "note": "a load test, not a regression" })),
        StatusCode::OK,
    )
    .await;
    assert_eq!(dismissed["status"], "dismissed");
    assert_eq!(dismissed["note"], "a load test, not a regression");

    // The final evaluation no longer counts the dismissed violation: the guardrail is ok, so the case is
    // confirmed. The record behind that state is a person's report, which makes the confidence medium.
    let asked = send(&t.h, &t.ann, "POST", "/api/observation/cases/O-1/evaluate", Some(json!({ "final": true })), StatusCode::OK).await;
    assert_eq!(asked["case"]["evaluationRequested"], "final");
    pass(&t.h, at(60)).await;
    let concluded = view(&t, "O-1").await;
    assert_eq!(concluded["case"]["status"], "concluded");
    assert_eq!(concluded["case"]["result"], "confirmed");
    assert_eq!(concluded["case"]["confidence"], "medium", "a reported record is behind the decided state");
    assert_eq!(concluded["case"]["conclusion"], "confirmed: guardrails: 1/1 ok, 0 violated; attribution: not assessed");
    assert_eq!(concluded["evaluations"][0]["trigger"], "final");
    assert_eq!(concluded["evaluations"][0]["verdict"], "confirmed");
    assert_eq!(concluded["incidents"][0]["status"], "dismissed");
    assert!(concluded["followups"].as_array().unwrap().is_empty());

    for login in ["root", "ann"] {
        assert_eq!(notifications(&t.h, login, "observation.concluded").len(), 1, "{login}");
    }
    let mail = manager_mail(&t.h);
    assert_eq!(mail.len(), 2);
    assert!(mail[1].text.contains("concluded as confirmed"), "{}", mail[1].text);

    let whole = journal(&t.h, 1);
    assert_eq!(
        kinds(&whole),
        [
            "outcome.created",
            "outcome.updated",
            "observation.started",
            "guardrail.violated",
            "incident.created",
            "evidence.collected",
            "incident.dismissed",
            "outcome.ready_for_evaluation",
            "outcome.confirmed",
        ]
    );
    assert_eq!(payload(&whole, "incident.dismissed")["note"], "a load test, not a regression");
}
