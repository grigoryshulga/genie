//! Observation scheduler: due cases, collection passes, guardrail checks, evaluations on schedule and
//! the state a restart finds. Passes run through `scheduler::pass_at` with explicit times; the sources
//! are the fake Prometheus server, which answers per fragment of the query.

use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use genie::observation::scheduler::{self, FAILURE_KIND, Plan};
use genie::observation::service::{self, CaseView, CreateCaseBody, EvaluateBody, SourceBody};
use genie_core::inbox::Notification;
use genie_core::observation::contract::Contract;
use genie_core::observation::evaluate::EvalOutcome;
use genie_core::observation::store::EvidenceFilter;
use genie_core::observation::{
    CaseResult, CaseStatus, Confidence, EvalTrigger, Evidence, EvidencePhase, IncidentStatus, ObservationCase, ObservationPolicy,
    SourceKind, Verdict, parse_ts, ts,
};
use genie_core::{Actor, CreateInput, GenieError, Role, Status, StatusOptions};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::common::Harness;
use crate::common::fakemetrics::{self, FakeMetrics};

const PROJECT: &str = "shop";
const NOW: &str = "2026-10-17T12:00:00.000Z";

/// The anchor of most cases: observation starts here.
fn now() -> DateTime<Utc> {
    parse_ts(NOW).unwrap()
}

/// `minutes` after the anchor.
fn minutes(minutes: i64) -> DateTime<Utc> {
    now() + Duration::minutes(minutes)
}

fn person() -> Actor {
    Actor::new("ann", Role::Human)
}

fn project_manager() -> Actor {
    Actor::new("project-manager", Role::Orchestrator)
}

fn contract(value: Value) -> Contract {
    serde_json::from_value(value).unwrap()
}

/// A manual guardrail: no source is asked, so a pass only schedules the next check.
fn manual_guardrail() -> Contract {
    contract(json!({
        "guardrails": [{ "id": "checkout_errors", "source": "manual", "max": 50 }],
        "observation": { "minimumWindow": "24h" }
    }))
}

/// A hypothesis that must fall by 20% against its 7-day baseline (`verify`).
fn latency(preferred: Option<&str>) -> Contract {
    let mut observation = json!({ "minimumWindow": "24h" });
    if let Some(window) = preferred {
        observation["preferredWindow"] = json!(window);
    }
    contract(json!({
        "hypotheses": [{ "id": "latency", "source": "metrics", "query": "sum(rate(latency[$__range]))", "expect": { "change": "-20%" } }],
        "observation": observation
    }))
}

/// The tracker of a project with one person and one administrator (who receives the notices).
fn project() -> (Harness, String) {
    let h = Harness::new();
    h.project(PROJECT);
    people(&h);
    let task = task(&h, "Faster checkout");
    (h, task)
}

fn people(h: &Harness) {
    h.app
        .with_server(|db| {
            db.create_user("ann", "Ann", None, None, false)?;
            db.create_user("root", "Root", None, None, true)?;
            Ok(())
        })
        .unwrap();
}

/// The wait before a failed pass is tried again (`RETRY_AFTER` in the scheduler).
fn retry_after() -> Duration {
    Duration::seconds(10)
}

/// The tracker database of a project, opened beside the app to inject faults that the app then meets.
fn tracker_db(h: &Harness, slug: &str) -> Connection {
    let project = h.app.with_server(|db| db.project(slug)).unwrap();
    Connection::open(Path::new(&project.tracker_dir).join("genie.db")).unwrap()
}

/// Makes every insert into `table` fail (a full disk, say) until `lift` removes the fault.
fn fault(h: &Harness, table: &str) {
    tracker_db(h, PROJECT)
        .execute_batch(&format!(
            "CREATE TRIGGER injected_{table} BEFORE INSERT ON {table} BEGIN SELECT RAISE(ABORT, 'injected write fault'); END;"
        ))
        .unwrap();
}

fn lift(h: &Harness, table: &str) {
    tracker_db(h, PROJECT).execute_batch(&format!("DROP TRIGGER injected_{table};")).unwrap();
}

/// Makes a waiting case fail to start: its status change to `observing` is refused.
fn fault_start(h: &Harness) {
    tracker_db(h, PROJECT)
        .execute_batch(
            "CREATE TRIGGER injected_start BEFORE UPDATE OF status ON observation_cases
             WHEN OLD.status = 'waiting' AND NEW.status = 'observing'
             BEGIN SELECT RAISE(ABORT, 'injected start fault'); END;",
        )
        .unwrap();
}

fn lift_start(h: &Harness) {
    tracker_db(h, PROJECT).execute_batch("DROP TRIGGER injected_start;").unwrap();
}

fn task(h: &Harness, title: &str) -> String {
    h.app.with_tracker(PROJECT, |t| t.create(&person(), CreateInput { title: title.into(), ..Default::default() })).unwrap().id
}

fn task_in(h: &Harness, slug: &str, title: &str) -> String {
    h.app.with_tracker(slug, |t| t.create(&person(), CreateInput { title: title.into(), ..Default::default() })).unwrap().id
}

fn finish(h: &Harness, task: &str) {
    h.app
        .with_tracker(PROJECT, |t| t.set_status(&person(), task, Status::Done, StatusOptions { force: true, ..Default::default() }))
        .unwrap();
}

fn source(name: &str, url: &str) -> SourceBody {
    SourceBody {
        name: name.into(),
        kind: SourceKind::Prometheus,
        url: url.into(),
        auth: None,
        organization: None,
        sentry_project: None,
        environment: None,
        field: None,
        dataset: None,
        timeout_secs: None,
    }
}

/// A vector answer with one series, as Prometheus gives it.
fn vector(sample: &str) -> Value {
    json!({ "status": "success", "data": { "resultType": "vector", "result": [{ "metric": { "job": "api" }, "value": [1760702400, sample] }] } })
}

/// A case created and armed by a person; it stays `waiting` while its tasks are open.
fn armed(h: &Harness, tasks: Vec<String>, policy: ObservationPolicy, contract: Contract) -> i64 {
    let created = service::create(
        &h.app,
        PROJECT,
        &person(),
        CreateCaseBody { title: "Checkout latency".into(), policy, tasks, contract: Some(contract) },
    )
    .unwrap();
    service::arm(&h.app, PROJECT, &person(), created.case.id, now()).unwrap();
    created.case.id
}

/// A case armed and started by a person at the anchor.
fn observing(h: &Harness, task: &str, policy: ObservationPolicy, contract: Contract) -> i64 {
    let id = armed(h, vec![task.into()], policy, contract);
    service::start(&h.app, PROJECT, &person(), id, now()).unwrap();
    id
}

fn case(h: &Harness, id: i64) -> ObservationCase {
    h.app.with_tracker(PROJECT, |t| t.observation().get(id)).unwrap()
}

fn view(h: &Harness, id: i64) -> CaseView {
    service::show(&h.app, PROJECT, id).unwrap()
}

fn evidence(h: &Harness, id: i64) -> Vec<Evidence> {
    service::evidence(&h.app, PROJECT, id, &EvidenceFilter::default()).unwrap()
}

/// One scheduler pass at `at`, on the blocking pool as the server runs it.
async fn pass(h: &Harness, at: DateTime<Utc>) -> Plan {
    h.app.blocking(move |app| scheduler::pass_at(app, at)).await.unwrap()
}

/// The request URLs the fake received, in order.
fn calls(fake: &FakeMetrics) -> Vec<String> {
    fake.state.lock().calls.iter().map(|c| c.url.clone()).collect()
}

/// Whether a request asked for `range` (written `[range]` in the query) and ended at `end`. The
/// query is percent-encoded in the URL, so the closing bracket appears as `%5D`.
fn asked(url: &str, range: &str, end: DateTime<Utc>) -> bool {
    url.contains(&format!("{range}%5D")) && url.contains(&format!("time={}.000", end.timestamp()))
}

/// The web notifications of one kind for a login.
fn notifications(h: &Harness, login: &str, kind: &str) -> Vec<Notification> {
    h.app
        .with_server(|db| {
            let user = db.user_by_login(login)?.ok_or_else(|| GenieError::not_found(format!("user {login} not found")))?;
            db.notifications(user.id, false, 100)
        })
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == kind)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_waiting_case_starts_when_its_last_linked_task_is_done() {
    let (h, first) = project();
    let second = task(&h, "Cache the basket");
    let id = armed(&h, vec![first.clone(), second.clone()], ObservationPolicy::Guardrails, manual_guardrail());
    assert_eq!(case(&h, id).status, CaseStatus::Waiting);

    finish(&h, &first);
    pass(&h, minutes(5)).await;
    assert_eq!(case(&h, id).status, CaseStatus::Waiting, "a task is still open");

    finish(&h, &second);
    let plan = pass(&h, minutes(10)).await;
    let started = case(&h, id);
    assert_eq!(started.status, CaseStatus::Observing);
    assert_eq!(started.observation_started_at, Some(ts(minutes(10))));
    // The manual guardrail is not measured; the pass only schedules the next check, 15 minutes on.
    assert_eq!(started.next_guardrail_at, Some(ts(minutes(25))));
    assert_eq!(plan.at, Some(minutes(25)));

    // A cancelled task is not done (§6.3): its case keeps waiting.
    let dropped = task(&h, "Dropped");
    let other = armed(&h, vec![dropped.clone()], ObservationPolicy::Guardrails, manual_guardrail());
    h.app
        .with_tracker(PROJECT, |t| {
            t.set_status(&person(), &dropped, Status::Cancelled, StatusOptions { force: true, ..Default::default() })
        })
        .unwrap();
    pass(&h, minutes(15)).await;
    assert_eq!(case(&h, other).status, CaseStatus::Waiting);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pass_with_nothing_scheduled_has_no_deadline() {
    let (h, _task) = project();
    assert_eq!(pass(&h, now()).await, Plan::default());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_first_pass_collects_baselines_then_guardrails_in_the_windows_of_section_7_2() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("errors", vector("10"));
    let (h, task) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let id = observing(
        &h,
        &task,
        ObservationPolicy::Guardrails,
        contract(json!({
            "guardrails": [{ "id": "errors", "source": "metrics", "query": "sum(rate(errors[$__range]))", "max": 50, "maxChange": "+5" }],
            "observation": { "minimumWindow": "24h", "preferredWindow": "72h" }
        })),
    );

    // At the anchor only the baseline window (7 days before the anchor) is asked: the observed guardrail
    // window is empty there, so nothing is observed yet and the check is only scheduled.
    let plan = pass(&h, now()).await;
    let urls = calls(&fake);
    assert_eq!(urls.len(), 1, "{urls:?}");
    assert!(asked(&urls[0], "604800s", now()), "baseline: {}", urls[0]);
    assert_eq!(case(&h, id).next_guardrail_at, Some(ts(minutes(15))));
    assert_eq!(plan.at, Some(minutes(15)), "the next check is the earliest deadline");

    // Two hours on: the guardrail window is the last hour, and the baseline is not collected again.
    pass(&h, minutes(120)).await;
    let urls = calls(&fake);
    assert_eq!(urls.len(), 2, "{urls:?}");
    assert!(asked(&urls[1], "3600s", minutes(120)), "{}", urls[1]);
    assert_eq!(case(&h, id).next_guardrail_at, Some(ts(minutes(135))), "the interval advances from the pass time");

    // Before the next check is due nothing is asked, and the plan says when it is.
    let plan = pass(&h, minutes(130)).await;
    assert_eq!(calls(&fake).len(), 2);
    assert_eq!(plan.at, Some(minutes(135)));
    pass(&h, minutes(135)).await;
    assert_eq!(calls(&fake).len(), 3);

    // The records: the baseline covers the 7 days before the anchor; observed records copy it.
    let records = evidence(&h, id);
    assert_eq!(records.len(), 3);
    assert_eq!(records[0].phase, EvidencePhase::Baseline);
    assert_eq!(records[0].window_start, ts(now() - Duration::days(7)));
    assert_eq!(records[0].window_end, ts(now()));
    assert_eq!(records[1].phase, EvidencePhase::Observed);
    assert_eq!(records[1].baseline, Some(10.0));
    assert_eq!(records[1].delta_abs, Some(0.0));
    assert_eq!(records[1].window_start, ts(minutes(60)));
    assert_eq!(records[1].window_end, ts(minutes(120)));
    assert_eq!(records[2].window_start, ts(minutes(75)));
    assert_eq!(records[2].window_end, ts(minutes(135)));
    assert!(view(&h, id).incidents.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_violating_guardrail_opens_one_incident_across_passes_and_files_one_follow_up() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("latency_ms", vector("61"));
    let (h, task) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let id = observing(
        &h,
        &task,
        ObservationPolicy::Guardrails,
        contract(json!({
            "guardrails": [{ "id": "latency_ms", "source": "metrics", "query": "max(latency_ms[$__range])", "max": 50 }],
            "observation": { "minimumWindow": "24h" },
            "followup": "auto"
        })),
    );

    // The check due at the anchor has an empty window; the first observed window ends 15 minutes on.
    pass(&h, now()).await;
    assert!(view(&h, id).incidents.is_empty());
    pass(&h, minutes(15)).await;
    let first = view(&h, id);
    assert_eq!(first.incidents.len(), 1);
    assert_eq!(first.incidents[0].key, "I-1");
    assert_eq!(first.incidents[0].guardrail, "latency_ms");
    assert_eq!(first.incidents[0].status, IncidentStatus::Open);
    assert_eq!(first.followups.len(), 1);
    assert_eq!(first.incidents[0].followups.len(), 1);
    assert_eq!(notifications(&h, "root", "observation.incident").len(), 1);
    assert_eq!(notifications(&h, "ann", "observation.incident").len(), 1, "the author is told too");

    // The violation holds over later passes: the same incident, no new follow-up, no new notice.
    for later in [30, 45, 60] {
        pass(&h, minutes(later)).await;
    }
    let later = view(&h, id);
    assert_eq!(later.incidents.len(), 1);
    assert_eq!(later.incidents[0].id, first.incidents[0].id);
    assert_eq!(later.followups.len(), 1);
    assert_eq!(notifications(&h, "root", "observation.incident").len(), 1);
    assert_eq!(notifications(&h, "ann", "observation.incident").len(), 1);
    let violations = evidence(&h, id).iter().filter(|r| r.value == Some(61.0)).count();
    assert_eq!(violations, 4, "every pass keeps its record");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_met_hypothesis_concludes_confirmed_at_the_minimum_window() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("latency", vector("100"));
    let (h, task) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let id = observing(&h, &task, ObservationPolicy::Verify, latency(None));
    // The first pass collects the baseline (100); the hypothesis is measured later.
    pass(&h, now()).await;
    fake.state.lock().respond("latency", vector("70"));

    // Before the minimum window nothing is evaluated.
    pass(&h, minutes(23 * 60)).await;
    assert_eq!(case(&h, id).status, CaseStatus::Observing);
    assert!(view(&h, id).evaluations.is_empty());

    pass(&h, minutes(24 * 60)).await;
    let done = view(&h, id);
    assert_eq!(done.case.status, CaseStatus::Concluded);
    assert_eq!(done.case.result, Some(CaseResult::Confirmed));
    assert_eq!(done.case.confidence, Some(Confidence::High), "measured records only");
    assert_eq!(done.evaluations.len(), 1);
    assert_eq!(done.evaluations[0].verdict, Verdict::Confirmed);
    assert_eq!(done.evaluations[0].trigger, EvalTrigger::Final, "the preferred window is the minimum here");
    assert_eq!(done.evaluations[0].details[0].reason, "delta -30% meets target -20%");
    assert_eq!(done.case.next_evaluation_at, None);
    assert_eq!(done.case.next_guardrail_at, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unmet_hypothesis_keeps_observing_and_reschedules_until_the_preferred_window_then_concludes_negative() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("latency", vector("100"));
    let (h, task) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let id = observing(&h, &task, ObservationPolicy::Verify, latency(Some("72h")));
    pass(&h, now()).await;
    fake.state.lock().respond("latency", vector("90"));

    // At the minimum window the hypothesis is not met yet: insufficient, observing again, next in 6 hours.
    pass(&h, minutes(24 * 60)).await;
    let waiting = view(&h, id);
    assert_eq!(waiting.case.status, CaseStatus::Observing);
    assert_eq!(waiting.evaluations[0].verdict, Verdict::Insufficient);
    assert_eq!(waiting.case.next_evaluation_at, Some(ts(minutes(30 * 60))));

    pass(&h, minutes(30 * 60)).await;
    assert_eq!(case(&h, id).next_evaluation_at, Some(ts(minutes(36 * 60))));

    // At the preferred window the answer is final: a hypothesis that is not met is negative.
    pass(&h, minutes(72 * 60)).await;
    let concluded = view(&h, id);
    assert_eq!(concluded.case.status, CaseStatus::Concluded);
    assert_eq!(concluded.case.result, Some(CaseResult::Negative));
    assert_eq!(concluded.evaluations[0].verdict, Verdict::Negative);
    assert_eq!(concluded.evaluations[0].trigger, EvalTrigger::Final);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_guardrail_that_is_never_measured_is_inconclusive_at_the_preferred_window() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("errors", vector("40"));
    fake.state.lock().status = Some(500);
    let (h, task) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let id = observing(
        &h,
        &task,
        ObservationPolicy::Guardrails,
        contract(json!({
            "guardrails": [{ "id": "errors", "source": "metrics", "query": "sum(rate(errors[$__range]))", "max": 50 }],
            "observation": { "minimumWindow": "24h", "preferredWindow": "48h" }
        })),
    );
    pass(&h, now()).await;
    pass(&h, minutes(24 * 60)).await;
    assert_eq!(case(&h, id).status, CaseStatus::Observing, "guardrails policy waits for the final evaluation");
    pass(&h, minutes(48 * 60)).await;
    let concluded = case(&h, id);
    assert_eq!(concluded.status, CaseStatus::Concluded);
    assert_eq!(concluded.result, Some(CaseResult::Inconclusive));
    assert_eq!(concluded.confidence, Some(Confidence::Low));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_manual_request_evaluates_now_and_only_a_person_s_final_request_concludes_before_the_minimum_window() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("errors", vector("40"));
    let (h, task) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let id = observing(
        &h,
        &task,
        ObservationPolicy::Guardrails,
        contract(json!({
            "guardrails": [{ "id": "errors", "source": "metrics", "query": "sum(rate(errors[$__range]))", "max": 50 }],
            "observation": { "minimumWindow": "24h" }
        })),
    );
    pass(&h, now()).await;

    // The project manager's request is evaluated at the next pass, but the minimum window has not passed.
    service::evaluate(&h.app, PROJECT, &project_manager(), id, EvaluateBody { final_: None }).unwrap();
    pass(&h, minutes(60)).await;
    let asked_once = view(&h, id);
    assert_eq!(asked_once.case.status, CaseStatus::Observing);
    assert_eq!(asked_once.case.evaluation_requested, None, "the request is cleared by the evaluation");
    assert_eq!(asked_once.evaluations[0].trigger, EvalTrigger::Manual);
    assert_eq!(asked_once.evaluations[0].verdict, Verdict::Insufficient);

    // A person's final request concludes at the next pass: the guardrail holds, so the case is confirmed.
    service::evaluate(&h.app, PROJECT, &person(), id, EvaluateBody { final_: Some(true) }).unwrap();
    pass(&h, minutes(75)).await;
    let concluded = view(&h, id);
    assert_eq!(concluded.case.status, CaseStatus::Concluded);
    assert_eq!(concluded.case.result, Some(CaseResult::Confirmed));
    assert_eq!(concluded.evaluations[0].trigger, EvalTrigger::Final);
}

#[test]
fn a_case_cancelled_during_an_evaluation_is_never_written_by_it() {
    let (h, task) = project();
    let id = observing(&h, &task, ObservationPolicy::Guardrails, manual_guardrail());
    // The pass takes the case into `evaluating`; a cancellation lands before its verdict is stored.
    assert!(h.app.with_tracker(PROJECT, |t| t.observation().begin_evaluation(id)).unwrap());
    h.app.with_tracker(PROJECT, |t| t.observation().cancel(&person(), id, "no longer needed", now())).unwrap();

    let outcome =
        EvalOutcome { verdict: Verdict::Confirmed, confidence: Some(Confidence::High), summary: "confirmed".into(), details: vec![] };
    let scheduler_actor = Actor::new("genie", Role::Orchestrator);
    let stored = h
        .app
        .with_tracker(PROJECT, |t| t.observation().finish_evaluation(&scheduler_actor, id, &outcome, EvalTrigger::Final, 1, now()))
        .unwrap();
    assert!(stored.is_none());
    let cancelled = view(&h, id);
    assert_eq!(cancelled.case.status, CaseStatus::Cancelled);
    assert_eq!(cancelled.case.result, None);
    assert!(cancelled.evaluations.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_case_left_evaluating_by_a_crash_is_evaluated_like_an_observing_one() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("latency", vector("100"));
    let (h, task) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let id = observing(&h, &task, ObservationPolicy::Verify, latency(None));
    pass(&h, now()).await;
    fake.state.lock().respond("latency", vector("70"));

    // The pass stopped after it took the case into `evaluating`.
    assert!(h.app.with_tracker(PROJECT, |t| t.observation().begin_evaluation(id)).unwrap());
    assert_eq!(case(&h, id).status, CaseStatus::Evaluating);

    pass(&h, minutes(24 * 60)).await;
    let done = case(&h, id);
    assert_eq!(done.status, CaseStatus::Concluded);
    assert_eq!(done.result, Some(CaseResult::Confirmed));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn source_errors_are_stored_as_evidence_errors_and_announced_once_per_failing_streak() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("errors", vector("40"));
    fake.state.lock().status = Some(500);
    let (h, task) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let id = observing(
        &h,
        &task,
        ObservationPolicy::Guardrails,
        contract(json!({
            "guardrails": [{ "id": "errors", "source": "metrics", "query": "sum(rate(errors[$__range]))", "max": 50 }],
            "observation": { "minimumWindow": "24h" }
        })),
    );

    // The check due at the anchor has an empty window: nothing is asked, so nothing fails.
    pass(&h, now()).await;
    assert!(calls(&fake).is_empty());
    assert_eq!(case(&h, id).failures, 0);

    pass(&h, minutes(15)).await;
    let first = case(&h, id);
    assert_eq!(first.failures, 1);
    assert!(first.last_error.as_deref().is_some_and(|e| e.contains("HTTP 500")), "{:?}", first.last_error);
    let records = evidence(&h, id);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].value, None);
    assert!(records[0].error.as_deref().is_some_and(|e| e.contains("HTTP 500")));

    pass(&h, minutes(30)).await;
    assert_eq!(case(&h, id).failures, 2);
    assert!(notifications(&h, "root", FAILURE_KIND).is_empty(), "two failed passes are not yet a streak");

    pass(&h, minutes(45)).await;
    assert_eq!(case(&h, id).failures, 3);
    assert_eq!(notifications(&h, "root", FAILURE_KIND).len(), 1);
    assert!(notifications(&h, "ann", FAILURE_KIND).is_empty(), "only the administrators are told");

    // The same streak: a fourth failure is deduplicated.
    pass(&h, minutes(60)).await;
    assert_eq!(case(&h, id).failures, 4);
    assert_eq!(notifications(&h, "root", FAILURE_KIND).len(), 1);

    // A clean pass ends the streak.
    fake.state.lock().status = None;
    pass(&h, minutes(75)).await;
    let recovered = case(&h, id);
    assert_eq!(recovered.failures, 0);
    assert_eq!(recovered.last_error, None);

    // Three more failures are a new streak with its own notice; the first one is not repeated.
    fake.state.lock().status = Some(500);
    pass(&h, minutes(90)).await;
    pass(&h, minutes(105)).await;
    assert_eq!(case(&h, id).failures, 2);
    assert_eq!(notifications(&h, "root", FAILURE_KIND).len(), 1, "two failures are not yet a streak");
    pass(&h, minutes(120)).await;
    assert_eq!(case(&h, id).failures, 3);
    assert_eq!(notifications(&h, "root", FAILURE_KIND).len(), 2, "one notice per streak");
    assert!(notifications(&h, "ann", FAILURE_KIND).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_source_removed_after_a_case_started_fails_its_measurements_as_unknown_without_an_adapter() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("errors", vector("40"));
    let (h, task) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let id = observing(
        &h,
        &task,
        ObservationPolicy::Guardrails,
        contract(json!({
            "guardrails": [{ "id": "errors", "source": "metrics", "query": "sum(rate(errors[$__range]))", "max": 50 }],
            "observation": { "minimumWindow": "24h" }
        })),
    );
    service::remove_source(&h.app, PROJECT, &person(), "metrics").unwrap();

    pass(&h, minutes(15)).await;
    assert!(calls(&fake).is_empty(), "no source is asked");
    let records = evidence(&h, id);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].value, None);
    assert_eq!(records[0].error.as_deref(), Some("unknown source metrics"));
    assert_eq!(records[0].provenance, json!({}), "no adapter is named for an unknown source");
    assert_eq!(case(&h, id).failures, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_pass_waits_for_its_retry_time_and_a_wake_at_the_same_time_asks_no_source() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("latency", vector("100"));
    let (h, task) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let id = observing(&h, &task, ObservationPolicy::Verify, latency(None));
    pass(&h, now()).await;
    assert_eq!(calls(&fake).len(), 1, "the baseline");
    fake.state.lock().respond("latency", vector("70"));

    // The evaluation at the minimum window measures the hypothesis, then its records cannot be stored.
    fault(&h, "observation_evidence");
    let failed_at = minutes(24 * 60);
    let failed = pass(&h, failed_at).await;
    assert_eq!(calls(&fake).len(), 2);
    assert!(!failed.again, "a failed case never asks for a pass at once");
    assert_eq!(failed.at, Some(failed_at + retry_after()));
    assert_eq!(case(&h, id).status, CaseStatus::Evaluating);

    // A wake at the same time: the case is held until its retry time, so nothing is asked again.
    let woken = pass(&h, failed_at).await;
    assert_eq!(calls(&fake).len(), 2);
    assert_eq!(woken.at, failed.at);
    pass(&h, failed_at + Duration::seconds(5)).await;
    assert_eq!(calls(&fake).len(), 2);

    // At the retry time, with the store working again, the evaluation runs once more and concludes.
    lift(&h, "observation_evidence");
    pass(&h, failed_at + retry_after()).await;
    assert_eq!(calls(&fake).len(), 3);
    let done = view(&h, id);
    assert_eq!(done.case.status, CaseStatus::Concluded);
    assert_eq!(done.case.result, Some(CaseResult::Confirmed));
    assert_eq!(done.evaluations.len(), 1, "the failed evaluation stored nothing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_intake_does_not_stop_the_due_cases_and_its_re_read_starts_each_waiting_case_once() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("errors", vector("10"));
    let (h, first) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let watched_task = task(&h, "Already watched");
    let watching = observing(
        &h,
        &watched_task,
        ObservationPolicy::Guardrails,
        contract(json!({
            "guardrails": [{ "id": "errors", "source": "metrics", "query": "sum(rate(errors[$__range]))", "max": 50 }],
            "observation": { "minimumWindow": "24h" }
        })),
    );
    let waiting = armed(&h, vec![first.clone()], ObservationPolicy::Guardrails, manual_guardrail());
    pass(&h, now()).await;

    // Both tasks finish; the start of the waiting case fails, so intake fails with its batch unacknowledged.
    finish(&h, &watched_task);
    finish(&h, &first);
    fault_start(&h);
    let failed = pass(&h, minutes(15)).await;
    assert_eq!(case(&h, waiting).status, CaseStatus::Waiting);
    // The due guardrail check of the observing case ran anyway.
    assert_eq!(calls(&fake).len(), 1);
    assert_eq!(evidence(&h, watching).iter().filter(|r| r.phase == EvidencePhase::Observed).count(), 1);
    assert_eq!(case(&h, watching).next_guardrail_at, Some(ts(minutes(30))));
    assert_eq!(failed.at, Some(minutes(15) + retry_after()), "the intake is tried again after the retry wait");

    // Once the fault is gone, the re-read starts the waiting case; the observing case is untouched.
    lift_start(&h);
    pass(&h, minutes(15) + retry_after()).await;
    let started = case(&h, waiting);
    assert_eq!(started.status, CaseStatus::Observing);
    assert_eq!(started.observation_started_at, Some(ts(minutes(15) + retry_after())));
    assert_eq!(case(&h, watching).observation_started_at, Some(ts(now())), "already observing: not started again");

    // A later pass reads nothing new: the batch was acknowledged, so the start is not repeated.
    pass(&h, minutes(15) + Duration::seconds(20)).await;
    assert_eq!(case(&h, waiting).observation_started_at, Some(ts(minutes(15) + retry_after())));
    assert_eq!(case(&h, watching).observation_started_at, Some(ts(now())));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_project_whose_pass_fails_does_not_stop_the_pass_of_another_project() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("errors", vector("10"));
    let h = Harness::new();
    // The failing project is listed first, so the healthy one runs after it.
    h.project("alpha");
    h.project(PROJECT);
    people(&h);
    let (broken_task, healthy_task) = (task_in(&h, "alpha", "Broken"), task(&h, "Healthy"));
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let broken = service::create(
        &h.app,
        "alpha",
        &person(),
        CreateCaseBody {
            title: "Broken".into(),
            policy: ObservationPolicy::Guardrails,
            tasks: vec![broken_task],
            contract: Some(manual_guardrail()),
        },
    )
    .unwrap()
    .case
    .id;
    service::arm(&h.app, "alpha", &person(), broken, now()).unwrap();
    service::start(&h.app, "alpha", &person(), broken, now()).unwrap();
    // A stored check time that cannot be read: the project's schedule cannot be worked out.
    tracker_db(&h, "alpha")
        .execute("UPDATE observation_cases SET next_guardrail_at = 'garbage', next_evaluation_at = NULL WHERE id = ?1", [broken])
        .unwrap();
    let healthy = observing(
        &h,
        &healthy_task,
        ObservationPolicy::Guardrails,
        contract(json!({
            "guardrails": [{ "id": "errors", "source": "metrics", "query": "sum(rate(errors[$__range]))", "max": 50 }],
            "observation": { "minimumWindow": "24h" }
        })),
    );

    let plan = pass(&h, minutes(15)).await;
    assert_eq!(evidence(&h, healthy).len(), 1, "the healthy project's due check ran");
    assert_eq!(calls(&fake).len(), 1);
    assert_eq!(plan.at, Some(minutes(15) + retry_after()), "the failing project is tried again soon");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hypothesis_is_measured_over_the_anchor_to_the_pass_time_with_its_samples_query() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("latency", vector("100"));
    fake.state.lock().respond("samples", vector("42"));
    let (h, task) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let id = observing(
        &h,
        &task,
        ObservationPolicy::Verify,
        contract(json!({
            "hypotheses": [{
                "id": "latency",
                "source": "metrics",
                "query": "sum(rate(latency[$__range]))",
                "samplesQuery": "count(samples[$__range])",
                "expect": { "change": "-20%" }
            }],
            "observation": { "minimumWindow": "24h" }
        })),
    );
    pass(&h, now()).await;
    fake.state.lock().respond("latency", vector("70"));
    pass(&h, minutes(24 * 60)).await;

    // The baseline first; then the hypothesis over [anchor, now], its samples query over the same span.
    let urls = calls(&fake);
    assert_eq!(urls.len(), 3, "{urls:?}");
    assert!(asked(&urls[0], "604800s", now()), "baseline: {}", urls[0]);
    assert!(urls[1].contains("latency") && asked(&urls[1], "86400s", minutes(24 * 60)), "{}", urls[1]);
    assert!(urls[2].contains("samples") && asked(&urls[2], "86400s", minutes(24 * 60)), "{}", urls[2]);
    assert_eq!(view(&h, id).case.status, CaseStatus::Concluded);
    let observed = evidence(&h, id);
    let hypothesis = observed.iter().find(|r| r.phase == EvidencePhase::Observed).unwrap();
    assert_eq!(hypothesis.window_start, ts(now()));
    assert_eq!(hypothesis.window_end, ts(minutes(24 * 60)));
    assert_eq!(hypothesis.samples, Some(42));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plan_at_is_the_earliest_deadline_when_it_is_an_evaluation_of_one_of_two_cases() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("latency", vector("100"));
    let (h, first) = project();
    service::add_source(&h.app, PROJECT, &person(), source("metrics", &fake.url)).unwrap();
    let second = task(&h, "Second");
    // Both check their guardrails every two days; the first is evaluated at 24h and stays observing.
    let evaluated = observing(
        &h,
        &first,
        ObservationPolicy::Verify,
        contract(json!({
            "hypotheses": [{ "id": "latency", "source": "metrics", "query": "sum(rate(latency[$__range]))", "expect": { "change": "-20%" } }],
            "observation": { "minimumWindow": "24h", "preferredWindow": "72h", "guardrailInterval": "2d" }
        })),
    );
    let waiting_for_longer = observing(
        &h,
        &second,
        ObservationPolicy::Guardrails,
        contract(json!({
            "guardrails": [{ "id": "checkout_errors", "source": "manual", "max": 50 }],
            "observation": { "minimumWindow": "48h", "preferredWindow": "72h", "guardrailInterval": "2d" }
        })),
    );
    pass(&h, now()).await;
    fake.state.lock().respond("latency", vector("90"));

    let plan = pass(&h, minutes(24 * 60)).await;
    assert_eq!(case(&h, evaluated).next_evaluation_at, Some(ts(minutes(30 * 60))));
    assert_eq!(case(&h, waiting_for_longer).next_evaluation_at, Some(ts(minutes(48 * 60))));
    assert_eq!(case(&h, waiting_for_longer).next_guardrail_at, Some(ts(minutes(48 * 60))));
    assert_eq!(plan.at, Some(minutes(30 * 60)), "the evaluation at 30h is the earliest deadline");
    assert!(!plan.again);
}
