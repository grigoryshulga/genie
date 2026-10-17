//! Case lifecycle store of the observation plane (`docs/platform/observation-plane.md` §6, §12, §13.1)
//! against a real tracker in a temporary directory.

use chrono::{DateTime, Duration, Utc};
use genie_core::db::SCHEMA_VERSION;
use genie_core::observation::contract::Contract;
use genie_core::observation::store::{CaseFilter, CasePatch, NewCase, NewDeployment};
use genie_core::observation::{CaseStatus, EvalTrigger, ObservationCase, ObservationPolicy, StartOn, parse_ts};
use genie_core::*;
use serde_json::{Value, json};

const NOW: &str = "2026-10-17T12:00:00.000Z";

fn now() -> DateTime<Utc> {
    parse_ts(NOW).unwrap()
}

fn me() -> Actor {
    Actor::new("me", Role::Human)
}

/// An agent acting as the project manager: not a person, so it may not revise after the start.
fn pm() -> Actor {
    Actor::new("project-manager", Role::Orchestrator)
}

fn fresh() -> (tempfile::TempDir, Tracker) {
    let dir = tempfile::tempdir().unwrap();
    let t = Tracker::init(dir.path().join(".genie"), None, None).unwrap();
    (dir, t)
}

fn new_task(t: &Tracker, title: &str) -> String {
    t.create(&me(), CreateInput { title: title.into(), ..Default::default() }).unwrap().id
}

fn finish_task(t: &Tracker, id: &str) {
    t.set_status(&me(), id, Status::Done, StatusOptions { note: None, force: true, ..Default::default() }).unwrap();
}

fn cancel_task(t: &Tracker, id: &str) {
    t.set_status(&me(), id, Status::Cancelled, StatusOptions { note: None, force: true, ..Default::default() }).unwrap();
}

fn contract_of(value: Value) -> Contract {
    serde_json::from_value(value).unwrap()
}

/// Guardrails policy: one agent-collected guardrail, minimum window 24h, starts on tasks done.
fn guard_contract() -> Contract {
    contract_of(json!({
        "guardrails": [{ "id": "error_rate", "source": "prom", "query": "rate(errors[$__range])", "maxChange": "+0.1pp" }],
        "observation": { "minimumWindow": "24h" }
    }))
}

/// Verify policy: one hypothesis, minimum window 24h.
fn verify_contract() -> Contract {
    contract_of(json!({
        "hypotheses": [{ "id": "p95", "source": "prom", "query": "histogram_quantile(0.95, x)", "expect": { "change": "-20%" } }],
        "observation": { "minimumWindow": "24h" }
    }))
}

fn guard_case(t: &Tracker, tasks: &[&str]) -> ObservationCase {
    t.observation()
        .create(
            &me(),
            NewCase {
                title: "Checkout latency".into(),
                policy: ObservationPolicy::Guardrails,
                tasks: tasks.iter().map(|s| s.to_string()).collect(),
                contract: Some(guard_contract()),
            },
        )
        .unwrap()
}

/// A case armed with one open task: `waiting` until the task is done.
fn armed_case(t: &Tracker, task: &str) -> ObservationCase {
    let case = guard_case(t, &[task]);
    t.observation().arm(&me(), case.id, now()).unwrap()
}

/// A case in `observing` started by hand at `now`.
fn observing_case(t: &Tracker, task: &str) -> ObservationCase {
    let case = armed_case(t, task);
    t.observation().start(&me(), case.id, now(), StartOn::Manual).unwrap()
}

#[track_caller]
fn expect_err<T: std::fmt::Debug>(r: genie_core::Result<T>, message: &str) {
    match r {
        Err(e) => assert_eq!(e.to_string(), message),
        Ok(v) => panic!("expected the error {message:?}, got {v:?}"),
    }
}

#[track_caller]
fn expect_denied<T: std::fmt::Debug>(r: genie_core::Result<T>, message: &str) {
    match r {
        Err(GenieError::Denied(m)) => assert_eq!(m, message),
        other => panic!("expected a denial {message:?}, got {other:?}"),
    }
}

fn names(t: &Tracker, kind: &str) -> Vec<String> {
    let mut stmt = t.conn().prepare("SELECT name FROM sqlite_master WHERE type = ?1 AND name LIKE 'observation%' ORDER BY name").unwrap();
    stmt.query_map([kind], |r| r.get(0)).unwrap().collect::<rusqlite::Result<_>>().unwrap()
}

/// The journal events of one case, oldest first.
fn journal(t: &Tracker, id: i64) -> Vec<Event> {
    let subject = format!("observation:{id}");
    t.events_after(0, 10_000).unwrap().into_iter().filter(|e| e.subject.as_deref() == Some(subject.as_str())).collect()
}

fn kinds(t: &Tracker, id: i64) -> Vec<String> {
    journal(t, id).into_iter().map(|e| e.kind).collect()
}

#[test]
fn a_fresh_tracker_records_schema_7_with_every_observation_table() {
    let (_dir, t) = fresh();
    assert_eq!(SCHEMA_VERSION, 7);
    let recorded: String = t.conn().query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| r.get(0)).unwrap();
    assert_eq!(recorded, "7");
    assert_eq!(
        names(&t, "table"),
        [
            "observation_case_tasks",
            "observation_cases",
            "observation_contracts",
            "observation_deployments",
            "observation_evaluations",
            "observation_evidence",
            "observation_followups",
            "observation_incidents",
        ]
    );
    assert_eq!(
        names(&t, "index"),
        [
            "observation_case_tasks_task",
            "observation_cases_status",
            "observation_evaluations_case",
            "observation_evidence_case",
            "observation_incidents_open",
        ]
    );
}

#[test]
fn a_case_is_created_with_or_without_a_contract() {
    let (_dir, t) = fresh();
    let g1 = new_task(&t, "Ship it");
    let g2 = new_task(&t, "Docs");
    let o = t.observation();

    let bare = o
        .create(
            &me(),
            NewCase {
                title: "  Checkout latency  ".into(),
                policy: ObservationPolicy::Guardrails,
                tasks: vec![g2.to_lowercase(), g1.clone(), g1.clone()],
                contract: None,
            },
        )
        .unwrap();
    assert_eq!(bare.id, 1);
    assert_eq!(bare.key, "O-1");
    assert_eq!(bare.title, "Checkout latency", "the title is trimmed");
    assert_eq!(bare.status, CaseStatus::Proposed);
    assert_eq!(bare.contract_version, 0);
    assert_eq!(bare.tasks, [g1.clone(), g2.clone()], "tasks are normalized, deduplicated and sorted");
    assert_eq!(bare.author, "me");
    assert_eq!(bare.failures, 0);
    assert!(bare.armed_at.is_none() && bare.result.is_none());
    assert!(o.contract(bare.id, None).unwrap().is_none());
    assert!(o.contracts(bare.id).unwrap().is_empty());

    let with = o
        .create(
            &me(),
            NewCase { title: "Latency".into(), policy: ObservationPolicy::Verify, tasks: vec![], contract: Some(verify_contract()) },
        )
        .unwrap();
    assert_eq!(with.id, 2);
    assert_eq!(with.contract_version, 1);
    assert!(with.tasks.is_empty(), "a case may start without tasks");
    let versions = o.contracts(with.id).unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!((versions[0].version, versions[0].author.as_str(), versions[0].reason.as_str()), (1, "me", ""));
    let stored = o.contract(with.id, None).unwrap().expect("version 1 is stored");
    assert_eq!(stored.version, 1);
    assert_eq!(stored.contract, verify_contract());
}

#[test]
fn an_invalid_contract_or_title_is_rejected_and_nothing_is_stored() {
    let (_dir, t) = fresh();
    let o = t.observation();
    expect_err(
        o.create(&me(), NewCase { title: "x".into(), policy: ObservationPolicy::Verify, tasks: vec![], contract: Some(guard_contract()) }),
        "contract is invalid: hypotheses: the verify policy needs at least one hypothesis",
    );
    expect_err(
        o.create(&me(), NewCase { title: "   ".into(), policy: ObservationPolicy::Guardrails, tasks: vec![], contract: None }),
        "an observation needs a title",
    );
    let long = "t".repeat(201);
    expect_err(
        o.create(&me(), NewCase { title: long, policy: ObservationPolicy::Guardrails, tasks: vec![], contract: None }),
        "the observation title must be at most 200 characters",
    );
    assert!(o.list(&CaseFilter::default()).unwrap().is_empty());
    assert_eq!(t.events_after(0, 100).unwrap().iter().filter(|e| e.kind == "outcome.created").count(), 0);
}

#[test]
fn an_unknown_task_is_rejected_by_name() {
    let (_dir, t) = fresh();
    let o = t.observation();
    expect_err(
        o.create(&me(), NewCase { title: "x".into(), policy: ObservationPolicy::Guardrails, tasks: vec!["G-99".into()], contract: None }),
        "task G-99 not found",
    );
    let case = guard_case(&t, &[]);
    expect_err(o.update(&me(), case.id, CasePatch { add_tasks: vec!["G-98".into()], ..Default::default() }), "task G-98 not found");
}

#[test]
fn an_unknown_case_is_not_found_with_its_key() {
    let (_dir, t) = fresh();
    let o = t.observation();
    match o.get(9) {
        Err(GenieError::NotFound(m)) => assert_eq!(m, "observation O-9 not found"),
        other => panic!("expected not found, got {other:?}"),
    }
    assert!(matches!(o.contracts(9), Err(GenieError::NotFound(_))));
}

#[test]
fn update_changes_each_field_in_the_statuses_that_allow_it() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let a = new_task(&t, "A");
    let b = new_task(&t, "B");

    // Proposed: title, policy and tasks may all change (no contract, so the policy is free).
    let case = o
        .create(&me(), NewCase { title: "Bare".into(), policy: ObservationPolicy::Guardrails, tasks: vec![a.clone()], contract: None })
        .unwrap();
    let updated = o
        .update(
            &me(),
            case.id,
            CasePatch {
                title: Some("Latency of checkout".into()),
                policy: Some(ObservationPolicy::Verify),
                add_tasks: vec![b.clone()],
                remove_tasks: vec![a.clone()],
            },
        )
        .unwrap();
    assert_eq!(updated.title, "Latency of checkout");
    assert_eq!(updated.policy, ObservationPolicy::Verify);
    assert_eq!(updated.tasks, vec![b.clone()]);
    let last = journal(&t, case.id).pop().unwrap();
    assert_eq!(last.kind, "outcome.updated");
    assert_eq!(last.payload["title"], "Latency of checkout");
    assert_eq!(last.payload["policy"], "verify");
    assert_eq!(last.payload["tasks"], json!([b]));
    // A patch that changes nothing writes no event.
    let events_before = kinds(&t, case.id).len();
    let unchanged = o.update(&me(), case.id, CasePatch { title: Some("Latency of checkout".into()), ..Default::default() }).unwrap();
    assert_eq!(unchanged.title, "Latency of checkout");
    assert_eq!(kinds(&t, case.id).len(), events_before, "an unchanged title writes no event");

    // A policy change re-validates the current contract (verify contract cannot become guardrails).
    let verify = o
        .create(&me(), NewCase { title: "V".into(), policy: ObservationPolicy::Verify, tasks: vec![], contract: Some(verify_contract()) })
        .unwrap();
    expect_err(
        o.update(&me(), verify.id, CasePatch { policy: Some(ObservationPolicy::Guardrails), ..Default::default() }),
        "contract is invalid: hypotheses: the guardrails policy takes no hypotheses; use the verify policy; \
         guardrails: the guardrails policy needs at least one guardrail",
    );
    assert_eq!(o.get(verify.id).unwrap().policy, ObservationPolicy::Verify, "the rejected change stored nothing");

    // Waiting: the title and the task links may still change.
    let waiting = armed_case(&t, &a);
    assert_eq!(waiting.status, CaseStatus::Waiting);
    let linked =
        o.update(&me(), waiting.id, CasePatch { title: Some("Waiting".into()), add_tasks: vec![b.clone()], ..Default::default() }).unwrap();
    assert_eq!((linked.title.as_str(), linked.tasks.len(), linked.status), ("Waiting", 2, CaseStatus::Waiting));

    // Observing: only the title.
    let observing = observing_case(&t, &b);
    assert_eq!(observing.status, CaseStatus::Observing);
    assert_eq!(o.update(&me(), observing.id, CasePatch { title: Some("Live".into()), ..Default::default() }).unwrap().title, "Live");
    expect_err(
        o.update(&me(), observing.id, CasePatch { policy: Some(ObservationPolicy::Verify), ..Default::default() }),
        &format!("the policy and the tasks can change only while the case is proposed or waiting; O-{} is observing", observing.id),
    );
    expect_err(
        o.update(&me(), observing.id, CasePatch { add_tasks: vec![a.clone()], ..Default::default() }),
        &format!("the policy and the tasks can change only while the case is proposed or waiting; O-{} is observing", observing.id),
    );

    // Terminal: nothing.
    let cancelled = guard_case(&t, &[]);
    o.cancel(&me(), cancelled.id, "not needed", now()).unwrap();
    expect_err(
        o.update(&me(), cancelled.id, CasePatch { title: Some("Late".into()), ..Default::default() }),
        &format!("the title can change only while the case is proposed, waiting or observing; O-{} is cancelled", cancelled.id),
    );
    expect_err(
        o.update(&me(), cancelled.id, CasePatch { title: Some("".into()), ..Default::default() }),
        &format!("the title can change only while the case is proposed, waiting or observing; O-{} is cancelled", cancelled.id),
    );
}

#[test]
fn contract_versions_increment_and_an_agent_cannot_revise_after_the_start() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let task = new_task(&t, "Ship");
    let case = guard_case(&t, &[&task]);
    assert_eq!(case.contract_version, 1);

    // Before observation, an agent may set the contract; the version increments.
    let tighter = contract_of(json!({
        "guardrails": [{ "id": "error_rate", "source": "prom", "query": "rate(errors[$__range])", "maxChange": "+0.05pp" }],
        "observation": { "minimumWindow": "24h" }
    }));
    let v2 = o.set_contract(&pm(), case.id, &tighter, "").unwrap();
    assert_eq!(v2.contract_version, 2);
    o.arm(&me(), case.id, now()).unwrap();
    o.start(&me(), case.id, now(), StartOn::Manual).unwrap();

    expect_denied(
        o.set_contract(&pm(), case.id, &verify_contract(), "agent reason"),
        "only a person may revise the contract after observation starts",
    );
    expect_err(o.set_contract(&me(), case.id, &verify_contract(), "   "), "a contract revision after observation starts needs a reason");
    expect_err(
        o.set_contract(&me(), case.id, &contract_of(json!({ "observation": { "minimumWindow": "1h" } })), "widen"),
        "contract is invalid: guardrails: the guardrails policy needs at least one guardrail",
    );

    let v3 = o.set_contract(&me(), case.id, &guard_contract(), "  the limit was too tight  ").unwrap();
    assert_eq!(v3.contract_version, 3);
    let history = o.contracts(case.id).unwrap();
    assert_eq!(history.iter().map(|v| v.version).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!(history[0].reason, "", "the first version has no reason");
    assert_eq!(history[1].reason, "", "a revision before the start has no reason");
    assert_eq!(history[2].reason, "the limit was too tight");
    assert_eq!(history[2].author, "me");
    // Older versions are kept as they were written.
    assert_eq!(o.contract(case.id, Some(2)).unwrap().unwrap().contract, tighter);
    assert_eq!(o.contract(case.id, None).unwrap().unwrap().version, 3);
    assert!(o.contract(case.id, Some(4)).unwrap().is_none());

    let last = journal(&t, case.id).pop().unwrap();
    assert_eq!(last.kind, "outcome.updated");
    assert_eq!(last.payload["contractVersion"], 3);
    assert_eq!(last.payload["reason"], "the limit was too tight");

    // A concluded or cancelled case has no contract changes.
    o.cancel(&me(), case.id, "stop", now()).unwrap();
    expect_err(
        o.set_contract(&me(), case.id, &guard_contract(), "later"),
        &format!("O-{} is cancelled; its contract cannot change", case.id),
    );
}

#[test]
fn arming_needs_a_contract_and_a_task_and_leaves_no_trace_when_refused() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let task = new_task(&t, "Ship");

    let no_contract = o
        .create(&me(), NewCase { title: "Bare".into(), policy: ObservationPolicy::Guardrails, tasks: vec![task.clone()], contract: None })
        .unwrap();
    expect_err(o.arm(&me(), no_contract.id, now()), "no contract yet");

    let no_task = guard_case(&t, &[]);
    let before = kinds(&t, no_task.id).len();
    expect_err(o.arm(&me(), no_task.id, now()), "link at least one task");
    assert_eq!(o.get(no_task.id).unwrap().status, CaseStatus::Proposed);
    assert_eq!(kinds(&t, no_task.id).len(), before, "a refused arm writes no event");

    o.update(&me(), no_task.id, CasePatch { add_tasks: vec![task.clone()], ..Default::default() }).unwrap();
    let armed = o.arm(&me(), no_task.id, now()).unwrap();
    assert_eq!(armed.status, CaseStatus::Waiting);
    assert_eq!(armed.armed_at.as_deref(), Some(NOW));
    assert!(armed.observation_started_at.is_none(), "one open task does not start observation");
    assert_eq!(kinds(&t, no_task.id).last().unwrap(), "outcome.updated");
    expect_err(o.arm(&me(), no_task.id, now()), "only a proposed case can be armed; O-2 is waiting");
}

#[test]
fn arming_with_every_task_done_starts_at_once_with_the_arm_time_as_anchor() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let task = new_task(&t, "Shipped");
    finish_task(&t, &task);

    let case = guard_case(&t, &[&task]);
    let armed = o.arm(&me(), case.id, now()).unwrap();
    assert_eq!(armed.status, CaseStatus::Observing);
    assert_eq!(armed.observation_started_at.as_deref(), Some(NOW));
    assert_eq!(armed.next_guardrail_at.as_deref(), Some(NOW));
    assert_eq!(armed.next_evaluation_at.as_deref(), Some("2026-10-18T12:00:00.000Z"), "anchor plus the 24h minimum window");
    assert_eq!(kinds(&t, case.id), ["outcome.created", "outcome.updated", "observation.started"]);
    let started = journal(&t, case.id).pop().unwrap();
    assert_eq!(started.payload["anchor"], NOW);
    assert_eq!(started.payload["start"], "tasks_done");
    assert_eq!(started.payload["tasks"], json!([task]));

    // A contract that starts by hand does not start on arming, even when every task is done.
    let manual = o
        .create(
            &me(),
            NewCase {
                title: "Manual".into(),
                policy: ObservationPolicy::Guardrails,
                tasks: vec![task.clone()],
                contract: Some(contract_of(json!({
                    "guardrails": [{ "id": "error_rate", "source": "prom", "query": "q", "max": 1 }],
                    "observation": { "start": "manual", "minimumWindow": "24h" }
                }))),
            },
        )
        .unwrap();
    assert_eq!(o.arm(&me(), manual.id, now()).unwrap().status, CaseStatus::Waiting);
}

#[test]
fn on_task_done_starts_only_when_every_linked_task_is_done() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let a = new_task(&t, "A");
    let b = new_task(&t, "B");
    let c = new_task(&t, "C");
    let d = new_task(&t, "D");
    let case = guard_case(&t, &[&a, &b]);
    o.arm(&me(), case.id, now()).unwrap();

    finish_task(&t, &a);
    assert_eq!(o.on_task_done(&pm(), &a, now()).unwrap(), Vec::<i64>::new(), "B is still open");
    assert_eq!(o.get(case.id).unwrap().status, CaseStatus::Waiting);

    finish_task(&t, &b);
    let later = now() + Duration::hours(3);
    assert_eq!(o.on_task_done(&pm(), &b, later).unwrap(), [case.id]);
    let started = o.get(case.id).unwrap();
    assert_eq!(started.status, CaseStatus::Observing);
    assert_eq!(started.observation_started_at.as_deref(), Some("2026-10-17T15:00:00.000Z"), "the anchor is the time of the call");
    assert_eq!(o.on_task_done(&pm(), &b, later).unwrap(), Vec::<i64>::new(), "an observing case is not started again");

    // A cancelled task is not done: the case waits for it, and the manual start still works.
    let with_cancel = guard_case(&t, &[&c, &d]);
    o.arm(&me(), with_cancel.id, now()).unwrap();
    cancel_task(&t, &c);
    finish_task(&t, &d);
    assert_eq!(o.on_task_done(&pm(), &d, now()).unwrap(), Vec::<i64>::new(), "a cancelled task does not count as done");
    assert_eq!(o.get(with_cancel.id).unwrap().status, CaseStatus::Waiting);
    assert_eq!(o.start(&me(), with_cancel.id, now(), StartOn::Manual).unwrap().status, CaseStatus::Observing);

    // A proposed case is not started by the intake, though its task is done.
    let proposed = guard_case(&t, &[&a]);
    assert_eq!(o.on_task_done(&pm(), &a, now()).unwrap(), Vec::<i64>::new());
    assert_eq!(o.get(proposed.id).unwrap().status, CaseStatus::Proposed);
}

#[test]
fn start_is_idempotent_on_an_observing_case_and_refused_from_other_statuses() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let task = new_task(&t, "A");
    let proposed = guard_case(&t, &[&task]);
    expect_err(
        o.start(&me(), proposed.id, now(), StartOn::Manual),
        &format!("only a waiting case can start; O-{} is proposed", proposed.id),
    );

    let observing = observing_case(&t, &task);
    let before = kinds(&t, observing.id).len();
    let again = o.start(&me(), observing.id, now() + Duration::hours(1), StartOn::Deployment).unwrap();
    assert_eq!(again.observation_started_at.as_deref(), Some(NOW), "the first anchor is kept");
    assert_eq!(kinds(&t, observing.id).len(), before, "a repeated start writes no event");
}

#[test]
fn a_deployment_starts_a_waiting_case_at_its_time_and_later_ones_keep_the_anchor() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let task = new_task(&t, "Ship");
    let case = o
        .create(
            &me(),
            NewCase {
                title: "Deployed".into(),
                policy: ObservationPolicy::Guardrails,
                tasks: vec![task.clone()],
                contract: Some(contract_of(json!({
                    "guardrails": [{ "id": "error_rate", "source": "prom", "query": "q", "max": 1 }],
                    "observation": { "start": "deployment", "minimumWindow": "24h" }
                }))),
            },
        )
        .unwrap();
    finish_task(&t, &task);
    // Every task is done, but the contract waits for a deployment.
    assert_eq!(o.arm(&me(), case.id, now()).unwrap().status, CaseStatus::Waiting);

    let first = o
        .record_deployment(
            &pm(),
            case.id,
            NewDeployment { reference: "v1.4.0".into(), environment: "prod".into(), at: now() - Duration::hours(2) },
            now(),
        )
        .unwrap();
    assert_eq!((first.id, first.reference.as_str(), first.environment.as_str()), (1, "v1.4.0", "prod"));
    assert_eq!(first.at, "2026-10-17T10:00:00.000Z");
    assert_eq!(first.recorded_by, "project-manager");
    let started = o.get(case.id).unwrap();
    assert_eq!(started.status, CaseStatus::Observing);
    assert_eq!(started.observation_started_at.as_deref(), Some("2026-10-17T10:00:00.000Z"), "the anchor is the deployment time");
    assert_eq!(started.next_guardrail_at.as_deref(), Some("2026-10-17T10:00:00.000Z"));
    assert_eq!(started.next_evaluation_at.as_deref(), Some("2026-10-18T10:00:00.000Z"));

    let second = o
        .record_deployment(
            &me(),
            case.id,
            NewDeployment { reference: "v1.4.1".into(), environment: String::new(), at: now() - Duration::hours(1) },
            now(),
        )
        .unwrap();
    assert_eq!(second.id, 2);
    let after = o.get(case.id).unwrap();
    assert_eq!(after.observation_started_at.as_deref(), Some("2026-10-17T10:00:00.000Z"), "a later deployment does not move the anchor");
    assert_eq!(after.next_evaluation_at.as_deref(), Some("2026-10-18T10:00:00.000Z"));
    assert_eq!(o.deployments(case.id).unwrap().iter().map(|d| d.reference.as_str()).collect::<Vec<_>>(), ["v1.4.0", "v1.4.1"]);

    let k = kinds(&t, case.id);
    assert_eq!(k.iter().filter(|k| k.as_str() == "deployment.completed").count(), 2);
    assert_eq!(k.iter().filter(|k| k.as_str() == "observation.started").count(), 1);
    let deployed = journal(&t, case.id).into_iter().find(|e| e.kind == "deployment.completed").unwrap();
    assert_eq!(deployed.payload["ref"], "v1.4.0");
    assert_eq!(deployed.payload["environment"], "prod");
    assert_eq!(deployed.payload["deployment"], 1);
}

#[test]
fn deployments_outside_the_window_or_without_a_reference_are_rejected() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let task = new_task(&t, "Ship");
    let proposed = guard_case(&t, &[&task]);
    let deploy = |at: DateTime<Utc>, reference: &str| NewDeployment { reference: reference.into(), environment: String::new(), at };

    expect_err(
        o.record_deployment(&me(), proposed.id, deploy(now(), "v1"), now()),
        &format!("deployments are recorded while the case is waiting, observing or evaluating; O-{} is proposed", proposed.id),
    );

    let case = armed_case(&t, &task);
    expect_err(
        o.record_deployment(&me(), case.id, deploy(now() + Duration::minutes(1), "v1"), now()),
        "a deployment cannot be in the future; record the time it completed",
    );
    expect_err(
        o.record_deployment(&me(), case.id, deploy(now() - Duration::days(31), "v1"), now()),
        "a deployment must be at most 30 days old",
    );
    expect_err(
        o.record_deployment(&me(), case.id, deploy(now(), "   "), now()),
        "a deployment needs a reference of 1 to 200 characters, such as a commit SHA or a tag",
    );
    expect_err(
        o.record_deployment(&me(), case.id, deploy(now(), &"r".repeat(201)), now()),
        "a deployment needs a reference of 1 to 200 characters, such as a commit SHA or a tag",
    );
    assert!(o.deployments(case.id).unwrap().is_empty(), "rejected deployments store nothing");

    // Exactly 30 days old is accepted; a waiting case with start tasks_done does not start.
    let edge = o.record_deployment(&me(), case.id, deploy(now() - Duration::days(30), "v0"), now()).unwrap();
    assert_eq!(edge.at, "2026-09-17T12:00:00.000Z");
    assert_eq!(o.get(case.id).unwrap().status, CaseStatus::Waiting);
}

#[test]
fn request_evaluation_keeps_final_over_manual_and_only_a_person_asks_for_final() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let task = new_task(&t, "Ship");
    let case = observing_case(&t, &task);

    let manual = o.request_evaluation(&pm(), case.id, false).unwrap();
    assert_eq!(manual.evaluation_requested, Some(EvalTrigger::Manual));
    let ready = journal(&t, case.id).pop().unwrap();
    assert_eq!(ready.kind, "outcome.ready_for_evaluation");
    assert_eq!(ready.payload["trigger"], "manual");

    expect_denied(o.request_evaluation(&pm(), case.id, true), "only a person may request a final evaluation");
    assert_eq!(o.get(case.id).unwrap().evaluation_requested, Some(EvalTrigger::Manual), "the denied request stored nothing");

    let final_ = o.request_evaluation(&me(), case.id, true).unwrap();
    assert_eq!(final_.evaluation_requested, Some(EvalTrigger::Final));
    let kept = o.request_evaluation(&me(), case.id, false).unwrap();
    assert_eq!(kept.evaluation_requested, Some(EvalTrigger::Final), "a manual request does not replace a final one");
    let agent_again = o.request_evaluation(&pm(), case.id, false).unwrap();
    assert_eq!(agent_again.evaluation_requested, Some(EvalTrigger::Final));

    let waiting = armed_case(&t, &new_task(&t, "Later"));
    expect_err(
        o.request_evaluation(&me(), waiting.id, false),
        &format!("an evaluation can be requested only while the case is observing; O-{} is waiting", waiting.id),
    );
}

#[test]
fn cancel_works_from_each_open_status_and_is_refused_from_terminal_ones() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let task = new_task(&t, "Ship");

    let proposed = guard_case(&t, &[&task]);
    let cancelled = o.cancel(&me(), proposed.id, "  duplicate of O-9  ", now()).unwrap();
    assert_eq!(cancelled.status, CaseStatus::Cancelled);
    assert_eq!(cancelled.conclusion, "duplicate of O-9");
    assert!(cancelled.observation_completed_at.is_none(), "a case that never started has no completion");

    let waiting = armed_case(&t, &task);
    assert_eq!(o.cancel(&me(), waiting.id, "not now", now()).unwrap().status, CaseStatus::Cancelled);

    let observing = {
        let other = new_task(&t, "Other");
        observing_case(&t, &other)
    };
    let stopped = o.cancel(&pm(), observing.id, "rollback", now() + Duration::hours(1)).unwrap();
    assert_eq!(stopped.status, CaseStatus::Cancelled);
    assert_eq!(stopped.observation_completed_at.as_deref(), Some("2026-10-17T13:00:00.000Z"));
    assert!(stopped.next_evaluation_at.is_none() && stopped.next_guardrail_at.is_none(), "the schedule is cleared");

    // Evaluating is set by the scheduler (OBS-05); the store accepts a cancel from it too.
    let evaluating = armed_case(&t, &new_task(&t, "Eval"));
    t.conn().execute("UPDATE observation_cases SET status = 'evaluating' WHERE id = ?1", [evaluating.id]).unwrap();
    assert_eq!(o.cancel(&me(), evaluating.id, "stop", now()).unwrap().status, CaseStatus::Cancelled);

    // Concluded is terminal: a cancel is refused.
    let concluded = armed_case(&t, &new_task(&t, "Done"));
    t.conn().execute("UPDATE observation_cases SET status = 'concluded', result = 'confirmed' WHERE id = ?1", [concluded.id]).unwrap();
    expect_err(
        o.cancel(&me(), concluded.id, "late", now()),
        &format!("O-{} is concluded; only an open case can be cancelled", concluded.id),
    );
    expect_err(
        o.cancel(&me(), proposed.id, "again", now()),
        &format!("O-{} is cancelled; only an open case can be cancelled", proposed.id),
    );
    expect_err(o.cancel(&me(), waiting.id, "   ", now()), "a cancellation needs a reason");
    assert_eq!(o.get(waiting.id).unwrap().conclusion, "not now", "the refused cancel stored nothing");
}

#[test]
fn every_operation_appends_its_journal_event_with_the_case_subject() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let task = new_task(&t, "Ship");
    let other = new_task(&t, "Other");

    let case = guard_case(&t, &[&task]);
    o.update(&me(), case.id, CasePatch { title: Some("Renamed".into()), ..Default::default() }).unwrap();
    o.set_contract(&pm(), case.id, &guard_contract(), "").unwrap();
    o.arm(&me(), case.id, now()).unwrap();
    o.start(&me(), case.id, now(), StartOn::Manual).unwrap();
    o.record_deployment(&pm(), case.id, NewDeployment { reference: "v1".into(), environment: "prod".into(), at: now() }, now()).unwrap();
    o.request_evaluation(&me(), case.id, false).unwrap();
    o.set_contract(&me(), case.id, &guard_contract(), "tighten").unwrap();
    o.cancel(&me(), case.id, "done", now()).unwrap();
    // Another case's events do not show up in this one's journal.
    let second = guard_case(&t, &[&other]);
    o.cancel(&me(), second.id, "unused", now()).unwrap();

    assert_eq!(
        kinds(&t, case.id),
        [
            "outcome.created",
            "outcome.updated",
            "outcome.updated",
            "outcome.updated",
            "observation.started",
            "deployment.completed",
            "outcome.ready_for_evaluation",
            "outcome.updated",
            "outcome.cancelled",
        ]
    );
    let events = journal(&t, case.id);
    assert!(events.iter().all(|e| e.subject.as_deref() == Some("observation:1")));
    assert!(events.iter().all(|e| e.actor == "me" || e.actor == "project-manager"));
    assert_eq!(events[0].actor, "me");
    assert_eq!(events[0].actor_role, "human");
    assert_eq!(events[0].payload["key"], "O-1");
    assert_eq!(events[0].payload["case"], 1);
    assert_eq!(events[0].payload["tasks"], json!([task]));
    assert_eq!(events[2].payload["contractVersion"], 2);
    assert_eq!(events[3].payload["status"], "waiting");
    assert_eq!(events[4].payload["start"], "manual");
    assert_eq!(events[5].payload["ref"], "v1");
    assert_eq!(events[7].payload["reason"], "tighten");
    assert_eq!(events[8].payload["reason"], "done");
    assert_eq!(events[8].actor_role, "human");
    assert_eq!(journal(&t, second.id).len(), 2, "the second case has its own created and cancelled events");
}

#[test]
fn a_rejected_operation_writes_neither_state_nor_event() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let task = new_task(&t, "Ship");
    let case = observing_case(&t, &task);
    let before = kinds(&t, case.id);

    expect_denied(
        o.set_contract(&pm(), case.id, &guard_contract(), "agent reason"),
        "only a person may revise the contract after observation starts",
    );
    expect_denied(o.request_evaluation(&pm(), case.id, true), "only a person may request a final evaluation");
    assert_eq!(kinds(&t, case.id), before, "a denied operation writes no event");
    assert_eq!(o.contracts(case.id).unwrap().len(), 1, "and stores no contract version");
    assert_eq!(o.get(case.id).unwrap().evaluation_requested, None);
}

#[test]
fn deleting_a_linked_task_removes_the_link_and_keeps_the_case() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let keep = new_task(&t, "Keep");
    let gone = new_task(&t, "Gone");
    let case = guard_case(&t, &[&keep, &gone]);
    t.delete_tasks(&me(), &gone, false).unwrap();

    let after = o.get(case.id).unwrap();
    assert_eq!(after.tasks, vec![keep.clone()]);
    assert_eq!(after.status, CaseStatus::Proposed);
    assert_eq!(o.for_task(&gone).unwrap().len(), 0);
    assert_eq!(o.for_task(&keep).unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), [case.id]);
}

#[test]
fn list_is_newest_first_filtered_and_bounded() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let a = new_task(&t, "A");
    let b = new_task(&t, "B");
    let first = guard_case(&t, &[&a]);
    let second = guard_case(&t, &[&a, &b]);
    let third = guard_case(&t, &[&b]);
    o.cancel(&me(), second.id, "no", now()).unwrap();

    let ids = |cases: Vec<ObservationCase>| cases.iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(ids(o.list(&CaseFilter::default()).unwrap()), [third.id, second.id, first.id]);
    assert_eq!(ids(o.list(&CaseFilter { status: Some(CaseStatus::Cancelled), ..Default::default() }).unwrap()), [second.id]);
    assert_eq!(ids(o.list(&CaseFilter { task: Some("g-1".into()), ..Default::default() }).unwrap()), [second.id, first.id]);
    assert_eq!(ids(o.list(&CaseFilter { limit: Some(1), ..Default::default() }).unwrap()), [third.id]);
    assert!(o.list(&CaseFilter { task: Some("G-42".into()), ..Default::default() }).unwrap().is_empty(), "an unknown task has no cases");
    assert_eq!(ids(o.for_task(&b).unwrap()), [third.id, second.id]);
    expect_err(o.list(&CaseFilter { limit: Some(0), ..Default::default() }), "limit must be between 1 and 200");
    expect_err(o.list(&CaseFilter { limit: Some(201), ..Default::default() }), "limit must be between 1 and 200");
    let bounded = o.list(&CaseFilter { limit: Some(200), ..Default::default() }).unwrap();
    assert_eq!(bounded.len(), 3);
    assert_eq!(bounded[0].tasks, vec![b.clone()]);
    assert_eq!(bounded[2].tasks, vec![a.clone()]);
}

#[test]
fn a_case_key_resolves_through_the_accessor_and_the_status_strings_round_trip() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = armed_case(&t, &task);
    let by_key = genie_core::observation::parse_case_id("o-1").unwrap();
    assert_eq!(t.observation().get(by_key).unwrap().status, CaseStatus::Waiting);
    assert_eq!(case.key, "O-1");
    assert_eq!(serde_json::to_value(&case).unwrap()["status"], "waiting");
    assert_eq!(serde_json::to_value(&case).unwrap()["armedAt"], NOW);
}
