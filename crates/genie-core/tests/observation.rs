//! Case lifecycle store, evidence, evaluations, incidents and follow-ups of the observation plane
//! (`docs/platform/observation-plane.md` §6 to §13) against a real tracker in a temporary directory.

use chrono::{DateTime, Duration, Utc};
use genie_core::db::SCHEMA_VERSION;
use genie_core::migrate;
use genie_core::observation::contract::Contract;
use genie_core::observation::evaluate::EvalOutcome;
use genie_core::observation::store::{
    Applied, CaseFilter, CasePatch, CollectionInput, EvidenceFilter, NewCase, NewDeployment, NewEvidence,
};
use genie_core::observation::{
    CaseResult, CaseStatus, Confidence, CriterionKind, EvalTrigger, EvidencePhase, IncidentStatus, ObservationCase, ObservationPolicy,
    StartOn, TrustLevel, Verdict, parse_ts, ts,
};
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
fn a_fresh_tracker_records_schema_8_with_every_observation_table() {
    let (_dir, t) = fresh();
    assert_eq!(SCHEMA_VERSION, 8);
    let recorded: String = t.conn().query_row("SELECT value FROM meta WHERE key = 'schema'", [], |r| r.get(0)).unwrap();
    assert_eq!(recorded, "8");
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
    expect_err(o.update(&me(), case.id, CasePatch { add_tasks: vec!["G-98".into()], ..Default::default() }, now()), "task G-98 not found");
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
            now(),
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
    let unchanged = o.update(&me(), case.id, CasePatch { title: Some("Latency of checkout".into()), ..Default::default() }, now()).unwrap();
    assert_eq!(unchanged.title, "Latency of checkout");
    assert_eq!(kinds(&t, case.id).len(), events_before, "an unchanged title writes no event");

    // A policy change re-validates the current contract (verify contract cannot become guardrails).
    let verify = o
        .create(&me(), NewCase { title: "V".into(), policy: ObservationPolicy::Verify, tasks: vec![], contract: Some(verify_contract()) })
        .unwrap();
    expect_err(
        o.update(&me(), verify.id, CasePatch { policy: Some(ObservationPolicy::Guardrails), ..Default::default() }, now()),
        "contract is invalid: hypotheses: the guardrails policy takes no hypotheses; use the verify policy; \
         guardrails: the guardrails policy needs at least one guardrail",
    );
    assert_eq!(o.get(verify.id).unwrap().policy, ObservationPolicy::Verify, "the rejected change stored nothing");

    // Waiting: the title and the task links may still change.
    let waiting = armed_case(&t, &a);
    assert_eq!(waiting.status, CaseStatus::Waiting);
    let linked = o
        .update(&me(), waiting.id, CasePatch { title: Some("Waiting".into()), add_tasks: vec![b.clone()], ..Default::default() }, now())
        .unwrap();
    assert_eq!((linked.title.as_str(), linked.tasks.len(), linked.status), ("Waiting", 2, CaseStatus::Waiting));

    // Observing: only the title.
    let observing = observing_case(&t, &b);
    assert_eq!(observing.status, CaseStatus::Observing);
    assert_eq!(o.update(&me(), observing.id, CasePatch { title: Some("Live".into()), ..Default::default() }, now()).unwrap().title, "Live");
    expect_err(
        o.update(&me(), observing.id, CasePatch { policy: Some(ObservationPolicy::Verify), ..Default::default() }, now()),
        &format!("the policy and the tasks can change only while the case is proposed or waiting; O-{} is observing", observing.id),
    );
    expect_err(
        o.update(&me(), observing.id, CasePatch { add_tasks: vec![a.clone()], ..Default::default() }, now()),
        &format!("the policy and the tasks can change only while the case is proposed or waiting; O-{} is observing", observing.id),
    );

    // Terminal: nothing.
    let cancelled = guard_case(&t, &[]);
    o.cancel(&me(), cancelled.id, "not needed", now()).unwrap();
    expect_err(
        o.update(&me(), cancelled.id, CasePatch { title: Some("Late".into()), ..Default::default() }, now()),
        &format!("the title can change only while the case is proposed, waiting or observing; O-{} is cancelled", cancelled.id),
    );
    expect_err(
        o.update(&me(), cancelled.id, CasePatch { title: Some("".into()), ..Default::default() }, now()),
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
    let v2 = o.set_contract(&pm(), case.id, &tighter, "", now()).unwrap();
    assert_eq!(v2.contract_version, 2);
    o.arm(&me(), case.id, now()).unwrap();
    o.start(&me(), case.id, now(), StartOn::Manual).unwrap();

    expect_denied(
        o.set_contract(&pm(), case.id, &verify_contract(), "agent reason", now()),
        "only a person may revise the contract after observation starts",
    );
    expect_err(
        o.set_contract(&me(), case.id, &verify_contract(), "   ", now()),
        "a contract revision after observation starts needs a reason",
    );
    expect_err(
        o.set_contract(&me(), case.id, &contract_of(json!({ "observation": { "minimumWindow": "1h" } })), "widen", now()),
        "contract is invalid: guardrails: the guardrails policy needs at least one guardrail",
    );

    let v3 = o.set_contract(&me(), case.id, &guard_contract(), "  the limit was too tight  ", now()).unwrap();
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
        o.set_contract(&me(), case.id, &guard_contract(), "later", now()),
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

    o.update(&me(), no_task.id, CasePatch { add_tasks: vec![task.clone()], ..Default::default() }, now()).unwrap();
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

    // An evaluating case is started already: starting it returns it unchanged.
    t.conn().execute("UPDATE observation_cases SET status = 'evaluating' WHERE id = ?1", [observing.id]).unwrap();
    let evaluating = o.start(&me(), observing.id, now() + Duration::hours(2), StartOn::Manual).unwrap();
    assert_eq!(evaluating.status, CaseStatus::Evaluating);
    assert_eq!(evaluating.observation_started_at.as_deref(), Some(NOW), "the anchor of an evaluating case is kept");
    assert_eq!(kinds(&t, observing.id).len(), before, "starting an evaluating case writes no event");
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
    assert_eq!(first.created, NOW, "the deployment is stamped with the caller's now");
    let started = o.get(case.id).unwrap();
    assert_eq!(started.status, CaseStatus::Observing);
    assert_eq!(started.updated, NOW, "the case is stamped with the caller's now too");
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
        &format!("an evaluation can be requested only while the case is observing or evaluating; O-{} is waiting", waiting.id),
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
    o.update(&me(), case.id, CasePatch { title: Some("Renamed".into()), ..Default::default() }, now()).unwrap();
    o.set_contract(&pm(), case.id, &guard_contract(), "", now()).unwrap();
    o.arm(&me(), case.id, now()).unwrap();
    o.start(&me(), case.id, now(), StartOn::Manual).unwrap();
    o.record_deployment(&pm(), case.id, NewDeployment { reference: "v1".into(), environment: "prod".into(), at: now() }, now()).unwrap();
    o.request_evaluation(&me(), case.id, false).unwrap();
    o.set_contract(&me(), case.id, &guard_contract(), "tighten", now()).unwrap();
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
        o.set_contract(&pm(), case.id, &guard_contract(), "agent reason", now()),
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
fn a_waiting_case_starts_when_removing_its_open_task_leaves_only_done_tasks() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let done = new_task(&t, "Done");
    let open = new_task(&t, "Open");
    finish_task(&t, &done);
    let case = guard_case(&t, &[&done, &open]);
    assert_eq!(o.arm(&me(), case.id, now()).unwrap().status, CaseStatus::Waiting);

    // The change is made 30 minutes after arming: the anchor is the caller's time of the change.
    let changed = now() + Duration::minutes(30);
    let started = o.update(&me(), case.id, CasePatch { remove_tasks: vec![open.clone()], ..Default::default() }, changed).unwrap();
    assert_eq!(started.status, CaseStatus::Observing);
    assert_eq!(started.tasks, vec![done.clone()]);
    assert_eq!(started.observation_started_at.as_deref(), Some(ts(changed).as_str()), "not the arming time");
    assert_eq!(started.next_guardrail_at, started.observation_started_at);
    assert_eq!(started.next_evaluation_at.as_deref(), Some(ts(changed + Duration::hours(24)).as_str()));
    assert_eq!(kinds(&t, case.id), ["outcome.created", "outcome.updated", "outcome.updated", "observation.started"]);
    let started_event = journal(&t, case.id).pop().unwrap();
    assert_eq!(started_event.payload["tasks"], json!([done]));
}

#[test]
fn a_waiting_case_keeps_at_least_one_task() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let only = new_task(&t, "Only");
    let case = armed_case(&t, &only);
    let before = kinds(&t, case.id);
    expect_err(
        o.update(&me(), case.id, CasePatch { remove_tasks: vec![only.clone()], ..Default::default() }, now()),
        "a waiting case needs at least one task",
    );
    let kept = o.get(case.id).unwrap();
    assert_eq!((kept.status, kept.tasks.clone()), (CaseStatus::Waiting, vec![only.clone()]));
    assert_eq!(kinds(&t, case.id), before, "a refused removal writes no event");

    // Swapping the only task in one patch keeps the case waiting and linked.
    let other = new_task(&t, "Other");
    let swapped = o
        .update(&me(), case.id, CasePatch { add_tasks: vec![other.clone()], remove_tasks: vec![only.clone()], ..Default::default() }, now())
        .unwrap();
    assert_eq!((swapped.status, swapped.tasks.clone()), (CaseStatus::Waiting, vec![other.clone()]));
}

#[test]
fn a_waiting_case_starts_when_its_new_contract_starts_on_done_tasks() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let task = new_task(&t, "Shipped");
    finish_task(&t, &task);
    let manual = contract_of(json!({
        "guardrails": [{ "id": "error_rate", "source": "prom", "query": "q", "max": 1 }],
        "observation": { "start": "manual", "minimumWindow": "24h" }
    }));
    let case = o
        .create(
            &me(),
            NewCase {
                title: "Manual start".into(),
                policy: ObservationPolicy::Guardrails,
                tasks: vec![task.clone()],
                contract: Some(manual),
            },
        )
        .unwrap();
    assert_eq!(o.arm(&me(), case.id, now()).unwrap().status, CaseStatus::Waiting);

    // Every linked task is done and the new contract starts on done tasks: the case starts at the caller's time.
    let revised_at = now() + Duration::minutes(30);
    let started = o.set_contract(&me(), case.id, &guard_contract(), "", revised_at).unwrap();
    assert_eq!(started.contract_version, 2);
    assert_eq!(started.status, CaseStatus::Observing);
    assert_eq!(started.observation_started_at.as_deref(), Some(ts(revised_at).as_str()));
    assert_eq!(kinds(&t, case.id).last().unwrap(), "observation.started");

    // With an open task the same contract leaves the case waiting.
    let open = new_task(&t, "Open");
    let waiting = armed_case(&t, &open);
    let still = o.set_contract(&me(), waiting.id, &guard_contract(), "", now()).unwrap();
    assert_eq!(still.contract_version, 2);
    assert_eq!(still.status, CaseStatus::Waiting);
}

#[test]
fn a_revision_after_the_start_moves_the_first_evaluation_to_the_revised_minimum_window() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let task = new_task(&t, "Ship");
    let case = observing_case(&t, &task);
    assert_eq!(case.next_evaluation_at.as_deref(), Some("2026-10-18T12:00:00.000Z"));

    let longer = contract_of(json!({
        "guardrails": [{ "id": "error_rate", "source": "prom", "query": "rate(errors[$__range])", "maxChange": "+0.1pp" }],
        "observation": { "minimumWindow": "48h" }
    }));
    let revised = o.set_contract(&me(), case.id, &longer, "longer window agreed", now()).unwrap();
    assert_eq!(revised.observation_started_at.as_deref(), Some(NOW), "the anchor is unchanged");
    assert_eq!(revised.next_evaluation_at.as_deref(), Some("2026-10-19T12:00:00.000Z"), "anchor plus the revised 48h minimum window");
    assert_eq!(revised.next_guardrail_at.as_deref(), Some(NOW), "the guardrail schedule is unchanged");
}

#[test]
fn a_schema_7_tracker_upgrades_to_schema_8_with_every_observation_table() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join(".genie");
    let t = Tracker::init(&root, None, None).unwrap();
    let task = new_task(&t, "Legacy");
    // Shape a schema-7 tracker: no observation tables or indexes, and the old version recorded.
    for table in [
        "observation_followups",
        "observation_incidents",
        "observation_evaluations",
        "observation_evidence",
        "observation_deployments",
        "observation_contracts",
        "observation_case_tasks",
        "observation_cases",
    ] {
        t.conn().execute_batch(&format!("DROP TABLE {table};")).unwrap();
    }
    t.conn().execute("UPDATE meta SET value = '7' WHERE key = 'schema'", []).unwrap();
    assert_eq!(names(&t, "table"), Vec::<String>::new());
    drop(t);

    let t = Tracker::open(&root).unwrap();
    assert_eq!(migrate::version(&root.join("genie.db")).unwrap(), Some(8), "the upgrade records schema 8");
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
    assert_eq!(t.get(&task).unwrap().title, "Legacy", "existing tasks survive the upgrade");
    assert!(t.observation().list(&CaseFilter::default()).unwrap().is_empty());
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

// Evidence, evaluations, incidents, follow-ups and the scheduler's queries (§7.5, §8, §9, §10, §11.3 to §11.5).

/// The scheduler's identity: an orchestrator, so the core accepts it for every write but a person's.
fn scheduler() -> Actor {
    Actor::new("genie", Role::Orchestrator)
}

/// Verify policy with one hypothesis and two guardrails, a change limit and a maximum. Minimum
/// window 24h, preferred 72h, evaluated every 6h, guardrails checked every 15m over the last hour.
fn mixed_contract() -> Contract {
    contract_of(json!({
        "hypotheses": [{ "id": "p95", "source": "prom", "query": "histogram_quantile(0.95, x)", "expect": { "change": "-20%" } }],
        "guardrails": [
            { "id": "error_rate", "source": "prom", "query": "rate(errors[$__range])", "maxChange": "+0.1pp" },
            { "id": "checkout_errors", "source": "sentry", "query": "level:error", "max": 50 }
        ],
        "observation": {
            "minimumWindow": "24h",
            "preferredWindow": "72h",
            "evaluationInterval": "6h",
            "guardrailInterval": "15m",
            "guardrailWindow": "1h"
        }
    }))
}

/// A verify case on `mixed_contract`, armed on `task` and started by hand at `now`.
fn observing_mixed(t: &Tracker, task: &str) -> ObservationCase {
    let o = t.observation();
    let case = o
        .create(
            &me(),
            NewCase {
                title: "Checkout".into(),
                policy: ObservationPolicy::Verify,
                tasks: vec![task.into()],
                contract: Some(mixed_contract()),
            },
        )
        .unwrap();
    o.arm(&me(), case.id, now()).unwrap();
    o.start(&me(), case.id, now(), StartOn::Manual).unwrap()
}

/// An observed record of `criterion`, measured over the hour before `now`.
fn observed(criterion: &str, kind: CriterionKind, value: f64, baseline: Option<f64>, trust: TrustLevel) -> NewEvidence {
    NewEvidence {
        criterion: criterion.into(),
        kind,
        phase: EvidencePhase::Observed,
        source: "prom".into(),
        query: "q".into(),
        window_start: now() - Duration::hours(1),
        window_end: now(),
        value: Some(value),
        baseline,
        samples: None,
        provenance: json!({ "adapter": "prometheus" }),
        trust,
        error: None,
        note: String::new(),
    }
}

/// A record whose collection failed: no value, the error stored.
fn failed_record(criterion: &str, kind: CriterionKind, error: &str) -> NewEvidence {
    NewEvidence { value: None, error: Some(error.into()), ..observed(criterion, kind, 0.0, None, TrustLevel::Measured) }
}

/// A scheduler pass at `now` on contract version 1 with these records and no schedule change.
fn pass(records: Vec<NewEvidence>) -> CollectionInput {
    CollectionInput { records, contract_version: 1, now: now(), next_guardrail_at: None, failed: None, pass: true }
}

/// A manual report (§7.5) at `now` on contract version 1: it has no failure and no schedule change.
fn report(records: Vec<NewEvidence>) -> CollectionInput {
    CollectionInput { pass: false, ..pass(records) }
}

fn apply(t: &Tracker, id: i64, input: CollectionInput) -> genie_core::Result<Option<Applied>> {
    t.observation().apply_collection(&scheduler(), id, input)
}

fn confirmed() -> EvalOutcome {
    EvalOutcome { verdict: Verdict::Confirmed, confidence: Some(Confidence::High), summary: "confirmed: test".into(), details: vec![] }
}

/// The journal events of one case with this kind, oldest first.
fn events_of(t: &Tracker, id: i64, kind: &str) -> Vec<Event> {
    journal(t, id).into_iter().filter(|e| e.kind == kind).collect()
}

#[track_caller]
fn assert_close(actual: Option<f64>, expected: Option<f64>) {
    match (actual, expected) {
        (Some(a), Some(e)) => assert!((a - e).abs() < 1e-9, "{a} is not {e}"),
        (a, e) => assert_eq!(a, e),
    }
}

#[test]
fn a_collection_on_a_case_that_is_not_observed_writes_nothing() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let o = t.observation();
    let proposed = o
        .create(
            &me(),
            NewCase {
                title: "Draft".into(),
                policy: ObservationPolicy::Verify,
                tasks: vec![task.clone()],
                contract: Some(mixed_contract()),
            },
        )
        .unwrap();
    let waiting = armed_case(&t, &task);
    let cancelled = observing_mixed(&t, &task);
    o.cancel(&me(), cancelled.id, "the release was rolled back", now()).unwrap();

    // Both a violating record and a failed pass would be written for an observed case.
    for id in [proposed.id, waiting.id, cancelled.id] {
        let violating = vec![observed("error_rate", CriterionKind::Guardrail, 1.0, Some(0.5), TrustLevel::Measured)];
        assert!(apply(&t, id, pass(violating.clone())).unwrap().is_none(), "O-{id} stores nothing");
        let failing = CollectionInput { failed: Some("timeout".into()), ..pass(violating) };
        assert!(apply(&t, id, failing).unwrap().is_none());
        assert!(o.evidence(id, &EvidenceFilter::default()).unwrap().is_empty());
        assert!(o.case_incidents(id).unwrap().is_empty());
        let case = o.get(id).unwrap();
        assert_eq!((case.failures, case.last_error), (0, None));
        assert!(!kinds(&t, id).iter().any(|k| k == "evidence.collected" || k == "guardrail.violated"));
    }
    assert!(o.incidents(None).unwrap().is_empty());
}

#[test]
fn records_must_name_a_criterion_of_the_current_contract_with_its_kind() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    expect_err(
        apply(&t, case.id, pass(vec![observed("nope", CriterionKind::Hypothesis, 1.0, None, TrustLevel::Measured)])),
        "criterion nope is not in the current contract of O-1",
    );
    expect_err(
        apply(&t, case.id, pass(vec![observed("p95", CriterionKind::Guardrail, 1.0, None, TrustLevel::Measured)])),
        "criterion p95 is a hypothesis, not a guardrail",
    );
    expect_err(
        apply(&t, case.id, pass(vec![observed("error_rate", CriterionKind::Hypothesis, 1.0, None, TrustLevel::Measured)])),
        "criterion error_rate is a guardrail, not a hypothesis",
    );
    // One bad record rejects the whole pass: the good record before it is not kept.
    expect_err(
        apply(
            &t,
            case.id,
            pass(vec![
                observed("p95", CriterionKind::Hypothesis, 80.0, Some(100.0), TrustLevel::Measured),
                observed("nope", CriterionKind::Hypothesis, 1.0, None, TrustLevel::Measured),
            ]),
        ),
        "criterion nope is not in the current contract of O-1",
    );
    assert!(o.evidence(case.id, &EvidenceFilter::default()).unwrap().is_empty());
    assert!(!kinds(&t, case.id).iter().any(|k| k == "evidence.collected"));
}

#[test]
fn malformed_records_are_rejected_with_the_reason_before_anything_is_stored() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let hypothesis = || observed("p95", CriterionKind::Hypothesis, 80.0, Some(100.0), TrustLevel::Measured);
    expect_err(apply(&t, case.id, pass(vec![NewEvidence { value: Some(f64::NAN), ..hypothesis() }])), "value must be a finite number");
    expect_err(
        apply(&t, case.id, pass(vec![NewEvidence { baseline: Some(f64::INFINITY), ..hypothesis() }])),
        "baseline must be a finite number",
    );
    expect_err(apply(&t, case.id, pass(vec![NewEvidence { value: None, ..hypothesis() }])), "a record needs a value or an error");
    expect_err(
        apply(&t, case.id, pass(vec![NewEvidence { error: Some("timeout".into()), ..hypothesis() }])),
        "a record has either a value or an error, not both",
    );
    expect_err(apply(&t, case.id, pass(vec![NewEvidence { samples: Some(-1), ..hypothesis() }])), "samples must be zero or more");
    let at = now();
    expect_err(
        apply(&t, case.id, pass(vec![NewEvidence { window_start: at + Duration::minutes(1), window_end: at, ..hypothesis() }])),
        "the window must not end before it starts",
    );
    assert!(o.evidence(case.id, &EvidenceFilter::default()).unwrap().is_empty());
    match o.evidence(9, &EvidenceFilter::default()) {
        Err(GenieError::NotFound(m)) => assert_eq!(m, "observation O-9 not found"),
        other => panic!("expected not found, got {other:?}"),
    }
}

#[test]
fn deltas_are_computed_from_the_baseline_and_left_out_without_one() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let baseline =
        NewEvidence { phase: EvidencePhase::Baseline, ..observed("p95", CriterionKind::Hypothesis, 100.0, None, TrustLevel::Measured) };
    apply(
        &t,
        case.id,
        pass(vec![
            baseline,
            observed("p95", CriterionKind::Hypothesis, 80.0, Some(100.0), TrustLevel::Measured),
            observed("p95", CriterionKind::Hypothesis, 5.0, Some(0.0), TrustLevel::Measured),
            observed("p95", CriterionKind::Hypothesis, 90.0, None, TrustLevel::Measured),
            failed_record("p95", CriterionKind::Hypothesis, "timeout"),
        ]),
    )
    .unwrap();
    let stored = o.current_evidence(case.id).unwrap();
    assert_eq!(stored.len(), 5);
    // The baseline record itself has no deltas.
    assert_eq!(stored[0].phase, EvidencePhase::Baseline);
    assert_close(stored[0].delta_abs, None);
    // 80 against a baseline of 100 is -20 absolute and -20 percent relative.
    assert_close(stored[1].delta_abs, Some(-20.0));
    assert_close(stored[1].delta_rel, Some(-20.0));
    assert_eq!(stored[1].baseline, Some(100.0));
    // A zero baseline has an absolute delta but no relative one.
    assert_close(stored[2].delta_abs, Some(5.0));
    assert_close(stored[2].delta_rel, None);
    // Without a baseline neither delta exists.
    assert_close(stored[3].delta_abs, None);
    assert_close(stored[3].delta_rel, None);
    // A failed record has no value, no deltas and its error.
    assert_eq!((stored[4].value, stored[4].delta_abs), (None, None));
    assert_eq!(stored[4].error.as_deref(), Some("timeout"));
    // The store fills the contract version, the pass time and the window.
    assert_eq!(stored[1].contract_version, 1);
    assert_eq!(stored[1].at, ts(now()));
    assert_eq!(stored[1].window_start, ts(now() - Duration::hours(1)));
    assert_eq!(stored[1].window_end, ts(now()));
}

#[test]
fn a_violating_guardrail_record_opens_one_incident_and_a_repeat_opens_none() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    // error_rate rose by 0.5pp against its baseline, and the limit is +0.1pp.
    let first = apply(&t, case.id, pass(vec![observed("error_rate", CriterionKind::Guardrail, 1.0, Some(0.5), TrustLevel::Measured)]))
        .unwrap()
        .unwrap();
    assert_eq!(first.new_incidents.len(), 1);
    let incident = &first.new_incidents[0];
    assert_eq!((incident.key.as_str(), incident.case_key.as_str(), incident.guardrail.as_str()), ("I-1", "O-1", "error_rate"));
    assert_eq!(incident.status, IncidentStatus::Open);
    assert_eq!(incident.summary, "change +0.5pp exceeds +0.1pp");
    assert_eq!(incident.evidence, Some(first.evidence[0].id));
    // A scheduler pass stamps the incident with the pass time, not the wall clock.
    assert_eq!(incident.created, ts(now()));
    let violated_first = events_of(&t, case.id, "guardrail.violated");
    assert_eq!(violated_first[0].payload["evidence"], first.evidence[0].id);

    // The same violation again: the open incident is kept and the record is still stored.
    let second = apply(&t, case.id, pass(vec![observed("error_rate", CriterionKind::Guardrail, 1.2, Some(0.5), TrustLevel::Measured)]))
        .unwrap()
        .unwrap();
    assert!(second.new_incidents.is_empty());
    assert_eq!(second.evidence.len(), 1);
    assert_eq!(o.case_incidents(case.id).unwrap().len(), 1);
    assert_eq!(events_of(&t, case.id, "guardrail.violated").len(), 1);
    assert_eq!(events_of(&t, case.id, "incident.created").len(), 1);

    // Another guardrail gets its own incident.
    let third = apply(&t, case.id, pass(vec![observed("checkout_errors", CriterionKind::Guardrail, 61.0, None, TrustLevel::Measured)]))
        .unwrap()
        .unwrap();
    assert_eq!(third.new_incidents.len(), 1);
    assert_eq!(third.new_incidents[0].key, "I-2");
    assert_eq!(third.new_incidents[0].summary, "value 61 exceeds max 50");
    let violated = events_of(&t, case.id, "guardrail.violated");
    assert_eq!(violated.len(), 2);
    assert_eq!(violated[0].payload["guardrail"], "error_rate");
    assert_eq!(violated[0].payload["message"], "change +0.5pp exceeds +0.1pp");
    assert_eq!(events_of(&t, case.id, "incident.created")[1].payload["incidentKey"], "I-2");
    assert_eq!(o.violated_guardrails(case.id).unwrap(), ["checkout_errors", "error_rate"]);
}

#[test]
fn an_agent_record_never_opens_an_incident_while_a_reported_one_does() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let agent = apply(
        &t,
        case.id,
        pass(vec![
            observed("error_rate", CriterionKind::Guardrail, 1.0, Some(0.5), TrustLevel::Agent),
            observed("checkout_errors", CriterionKind::Guardrail, 61.0, None, TrustLevel::Agent),
        ]),
    )
    .unwrap()
    .unwrap();
    assert!(agent.new_incidents.is_empty());
    assert_eq!(agent.evidence.len(), 2);
    assert!(agent.evidence.iter().all(|e| e.trust == TrustLevel::Agent));
    assert!(o.incidents(None).unwrap().is_empty());
    assert!(o.violated_guardrails(case.id).unwrap().is_empty());
    assert!(events_of(&t, case.id, "guardrail.violated").is_empty());

    // A person's report of a violation is usable and opens the incident at once (§7.5).
    let reported = apply(&t, case.id, pass(vec![observed("checkout_errors", CriterionKind::Guardrail, 61.0, None, TrustLevel::Reported)]))
        .unwrap()
        .unwrap();
    assert_eq!(reported.new_incidents.len(), 1);
    assert_eq!(reported.new_incidents[0].summary, "value 61 exceeds max 50");
}

#[test]
fn failed_passes_count_up_and_a_clean_pass_resets_them() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let failing = |message: &str| CollectionInput {
        records: vec![failed_record("error_rate", CriterionKind::Guardrail, message)],
        contract_version: 1,
        now: now(),
        next_guardrail_at: None,
        failed: Some(message.into()),
        pass: true,
    };
    let first = apply(&t, case.id, failing("prometheus: timeout")).unwrap().unwrap();
    assert_eq!(first.failures, 1);
    assert_eq!(first.evidence[0].error.as_deref(), Some("prometheus: timeout"));
    let stored = o.get(case.id).unwrap();
    assert_eq!((stored.failures, stored.last_error.as_deref()), (1, Some("prometheus: timeout")));

    let second = apply(&t, case.id, failing("sentry: unauthorized")).unwrap().unwrap();
    assert_eq!(second.failures, 2);
    assert_eq!(o.get(case.id).unwrap().last_error.as_deref(), Some("sentry: unauthorized"));

    // A pass in which every measurement succeeded resets the count and the error.
    let clean = apply(&t, case.id, pass(vec![observed("error_rate", CriterionKind::Guardrail, 0.4, Some(0.5), TrustLevel::Measured)]))
        .unwrap()
        .unwrap();
    assert_eq!(clean.failures, 0);
    let stored = o.get(case.id).unwrap();
    assert_eq!((stored.failures, stored.last_error), (0, None));
}

#[test]
fn each_pass_writes_one_evidence_collected_event_and_moves_the_schedule_only_when_given() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let next = now() + Duration::minutes(15);
    let applied = apply(
        &t,
        case.id,
        CollectionInput {
            records: vec![
                observed("p95", CriterionKind::Hypothesis, 80.0, Some(100.0), TrustLevel::Measured),
                failed_record("error_rate", CriterionKind::Guardrail, "timeout"),
                observed("checkout_errors", CriterionKind::Guardrail, 10.0, None, TrustLevel::Measured),
            ],
            contract_version: 1,
            now: now(),
            next_guardrail_at: Some(next),
            failed: Some("timeout".into()),
            pass: true,
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(applied.evidence.len(), 3);
    assert_eq!(applied.failures, 1);
    let collected = events_of(&t, case.id, "evidence.collected");
    assert_eq!(collected.len(), 1);
    assert_eq!(collected[0].payload["records"], 3);
    assert_eq!(collected[0].payload["errors"], 1);
    assert_eq!(collected[0].payload["key"], "O-1");
    assert_eq!(collected[0].subject.as_deref(), Some("observation:1"));
    assert_eq!(collected[0].actor, "genie");
    assert_eq!(o.get(case.id).unwrap().next_guardrail_at.as_deref(), Some(ts(next).as_str()));

    // A pass with no records writes no event and, with no schedule given, keeps the old one.
    let quiet = apply(&t, case.id, pass(vec![])).unwrap().unwrap();
    assert!(quiet.evidence.is_empty());
    assert_eq!(events_of(&t, case.id, "evidence.collected").len(), 1);
    assert_eq!(o.get(case.id).unwrap().next_guardrail_at.as_deref(), Some(ts(next).as_str()));
}

#[test]
fn an_insufficient_evaluation_reschedules_to_the_earlier_of_one_interval_and_the_preferred_window() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let insufficient = EvalOutcome {
        verdict: Verdict::Insufficient,
        confidence: None,
        summary: "insufficient: no usable evidence".into(),
        details: vec![],
    };

    // A request the pass read is cleared by the insufficient verdict as well.
    let requested = o.request_evaluation(&me(), case.id, false).unwrap();
    assert!(o.begin_evaluation(case.id).unwrap());
    let at = now() + Duration::hours(25);
    let after = o
        .finish_evaluation(&scheduler(), case.id, &insufficient, EvalTrigger::Window, 1, requested.evaluation_requested, at)
        .unwrap()
        .expect("the case was evaluating");
    assert_eq!(after.status, CaseStatus::Observing);
    assert_eq!(after.evaluation_requested, None);
    assert_eq!(after.next_evaluation_at.as_deref(), Some(ts(at + Duration::hours(6)).as_str()), "one interval after the pass");

    // Close to the preferred window (72h after the anchor), the window is the earlier one.
    assert!(o.begin_evaluation(case.id).unwrap());
    let late = now() + Duration::hours(70);
    let after = o.finish_evaluation(&scheduler(), case.id, &insufficient, EvalTrigger::Manual, 1, None, late).unwrap().unwrap();
    assert_eq!(after.next_evaluation_at.as_deref(), Some(ts(now() + Duration::hours(72)).as_str()));

    let evaluations = o.evaluations(case.id, None).unwrap();
    assert_eq!(evaluations.len(), 2, "newest first");
    assert_eq!((evaluations[0].trigger, evaluations[0].verdict), (EvalTrigger::Manual, Verdict::Insufficient));
    assert_eq!(evaluations[0].at, ts(late));
    assert_eq!(evaluations[0].actor, "genie");
    assert_eq!(evaluations[0].contract_version, 1);
    assert_eq!(evaluations[1].trigger, EvalTrigger::Window);
    let events = events_of(&t, case.id, "outcome.evaluated");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payload["verdict"], "insufficient");
    assert_eq!(events[0].payload["nextEvaluationAt"], ts(at + Duration::hours(6)).as_str());
    assert_eq!(events[0].payload["evaluation"], evaluations[1].id);
}

#[test]
fn a_final_request_that_arrives_during_an_evaluation_survives_an_insufficient_verdict() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let insufficient = EvalOutcome {
        verdict: Verdict::Insufficient,
        confidence: None,
        summary: "insufficient: minimum window not reached".into(),
        details: vec![],
    };

    // The pass reads no request and takes the case into `evaluating`.
    assert!(o.begin_evaluation(case.id).unwrap());
    // A person's final request arrives while the pass runs: evaluating accepts requests as observing does.
    let during = o.request_evaluation(&me(), case.id, true).unwrap();
    assert_eq!((during.status, during.evaluation_requested), (CaseStatus::Evaluating, Some(EvalTrigger::Final)));

    // The window is still insufficient. The pass consumed no request, so the final one is kept.
    let at = now() + Duration::hours(1);
    let after = o
        .finish_evaluation(&scheduler(), case.id, &insufficient, EvalTrigger::Window, 1, None, at)
        .unwrap()
        .expect("the case was evaluating");
    assert_eq!(after.status, CaseStatus::Observing);
    assert_eq!(after.evaluation_requested, Some(EvalTrigger::Final));

    // The next pass takes the request and concludes the case.
    assert!(o.begin_evaluation(case.id).unwrap());
    let concluded = o
        .finish_evaluation(&scheduler(), case.id, &confirmed(), EvalTrigger::Final, 1, Some(EvalTrigger::Final), at + Duration::hours(1))
        .unwrap()
        .unwrap();
    assert_eq!(concluded.status, CaseStatus::Concluded);
    assert_eq!(concluded.evaluation_requested, None);
}

#[test]
fn an_upgrade_to_final_during_an_evaluation_survives_the_insufficient_verdict_that_consumed_a_manual_request() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let insufficient = EvalOutcome {
        verdict: Verdict::Insufficient,
        confidence: None,
        summary: "insufficient: minimum window not reached".into(),
        details: vec![],
    };

    // The pass reads a manual request, then a person asks for the final evaluation before it finishes.
    let requested = o.request_evaluation(&me(), case.id, false).unwrap();
    assert!(o.begin_evaluation(case.id).unwrap());
    o.request_evaluation(&me(), case.id, true).unwrap();

    let at = now() + Duration::hours(1);
    let after = o
        .finish_evaluation(&scheduler(), case.id, &insufficient, EvalTrigger::Manual, 1, requested.evaluation_requested, at)
        .unwrap()
        .expect("the case was evaluating");
    assert_eq!(after.evaluation_requested, Some(EvalTrigger::Final), "the request changed, so the pass does not clear it");
}

#[test]
fn a_confirmed_evaluation_concludes_the_case_clears_its_schedule_and_writes_its_event() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let requested = o.request_evaluation(&me(), case.id, false).unwrap();
    assert_eq!(requested.evaluation_requested, Some(EvalTrigger::Manual));

    assert!(o.begin_evaluation(case.id).unwrap());
    let outcome = EvalOutcome {
        verdict: Verdict::Confirmed,
        confidence: Some(Confidence::High),
        summary: "confirmed: 1/1 hypotheses met; guardrails: 2/2 ok, 0 violated; attribution: not assessed".into(),
        details: vec![],
    };
    let at = now() + Duration::hours(72);
    let concluded = o.finish_evaluation(&scheduler(), case.id, &outcome, EvalTrigger::Final, 1, None, at).unwrap().unwrap();
    assert_eq!(concluded.status, CaseStatus::Concluded);
    assert_eq!(concluded.result, Some(CaseResult::Confirmed));
    assert_eq!(concluded.confidence, Some(Confidence::High));
    assert_eq!(concluded.conclusion, outcome.summary);
    assert_eq!(concluded.observation_completed_at.as_deref(), Some(ts(at).as_str()));
    assert_eq!((concluded.next_evaluation_at, concluded.next_guardrail_at, concluded.evaluation_requested), (None, None, None));

    let evaluations = o.evaluations(case.id, Some(10)).unwrap();
    assert_eq!(evaluations.len(), 1);
    assert_eq!(
        (evaluations[0].verdict, evaluations[0].trigger, evaluations[0].contract_version),
        (Verdict::Confirmed, EvalTrigger::Final, 1)
    );
    let event = events_of(&t, case.id, "outcome.confirmed");
    assert_eq!(event.len(), 1);
    assert_eq!(event[0].payload["evaluation"], evaluations[0].id);
    assert_eq!(event[0].payload["confidence"], "high");
    assert_eq!(event[0].payload["summary"], outcome.summary.as_str());
    assert_eq!(event[0].payload["tasks"], json!([task]));

    // A concluded case is neither evaluated nor due.
    assert!(!o.begin_evaluation(case.id).unwrap());
    assert!(o.finish_evaluation(&scheduler(), case.id, &outcome, EvalTrigger::Final, 1, None, at).unwrap().is_none());
    assert!(o.due(at + Duration::days(30)).unwrap().is_empty());
}

#[test]
fn each_conclusion_maps_to_its_result_confidence_and_event() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let cases = [
        (Verdict::Negative, CaseResult::Negative, Confidence::High, "outcome.negative"),
        (Verdict::Inconclusive, CaseResult::Inconclusive, Confidence::Low, "outcome.inconclusive"),
    ];
    for (verdict, result, confidence, kind) in cases {
        let task = new_task(&t, "Ship");
        let case = observing_mixed(&t, &task);
        assert!(o.begin_evaluation(case.id).unwrap());
        let outcome = EvalOutcome { verdict, confidence: Some(confidence), summary: format!("{verdict}: test"), details: vec![] };
        let done = o
            .finish_evaluation(&scheduler(), case.id, &outcome, EvalTrigger::Final, 1, None, now() + Duration::hours(72))
            .unwrap()
            .unwrap();
        assert_eq!((done.status, done.result, done.confidence), (CaseStatus::Concluded, Some(result), Some(confidence)));
        assert_eq!(events_of(&t, case.id, kind).len(), 1, "{kind} is written once");
    }
}

#[test]
fn a_finished_evaluation_after_a_cancel_or_without_a_begin_writes_nothing() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let o = t.observation();
    let cancelled = observing_mixed(&t, &task);
    assert!(o.begin_evaluation(cancelled.id).unwrap());
    o.cancel(&me(), cancelled.id, "the change was reverted", now()).unwrap();
    assert!(o.finish_evaluation(&scheduler(), cancelled.id, &confirmed(), EvalTrigger::Final, 1, None, now()).unwrap().is_none());
    assert!(o.evaluations(cancelled.id, None).unwrap().is_empty());
    assert!(events_of(&t, cancelled.id, "outcome.confirmed").is_empty());

    // Without a begin the case is still observing, so there is nothing to finish.
    let observing = observing_mixed(&t, &task);
    assert!(o.finish_evaluation(&scheduler(), observing.id, &confirmed(), EvalTrigger::Final, 1, None, now()).unwrap().is_none());
    assert!(o.evaluations(observing.id, None).unwrap().is_empty());
    assert_eq!(o.get(observing.id).unwrap().status, CaseStatus::Observing);
}

#[test]
fn begin_evaluation_is_true_only_for_observing_and_evaluating_cases() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let o = t.observation();
    let proposed = o
        .create(
            &me(),
            NewCase { title: "Draft".into(), policy: ObservationPolicy::Verify, tasks: vec![], contract: Some(mixed_contract()) },
        )
        .unwrap();
    let waiting = armed_case(&t, &task);
    let observing = observing_mixed(&t, &new_task(&t, "Other"));
    let cancelled = observing_mixed(&t, &task);
    o.cancel(&me(), cancelled.id, "dropped", now()).unwrap();
    let concluded = observing_mixed(&t, &new_task(&t, "Done"));
    assert!(o.begin_evaluation(concluded.id).unwrap());
    o.finish_evaluation(&scheduler(), concluded.id, &confirmed(), EvalTrigger::Final, 1, None, now() + Duration::hours(72)).unwrap();

    assert!(!o.begin_evaluation(proposed.id).unwrap());
    assert!(!o.begin_evaluation(waiting.id).unwrap());
    assert!(!o.begin_evaluation(cancelled.id).unwrap());
    assert!(!o.begin_evaluation(concluded.id).unwrap());
    assert!(o.begin_evaluation(observing.id).unwrap());
    assert_eq!(o.get(observing.id).unwrap().status, CaseStatus::Evaluating);
    assert!(o.begin_evaluation(observing.id).unwrap(), "an evaluating case is taken again after a crash");
    // An evaluating case still collects evidence.
    assert!(
        apply(&t, observing.id, pass(vec![observed("p95", CriterionKind::Hypothesis, 80.0, Some(100.0), TrustLevel::Measured)]))
            .unwrap()
            .is_some()
    );
    match o.begin_evaluation(99) {
        Err(GenieError::NotFound(m)) => assert_eq!(m, "observation O-99 not found"),
        other => panic!("expected not found, got {other:?}"),
    }
}

#[test]
fn an_incident_resolves_once_and_is_dismissed_only_by_a_person_with_a_note() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let (opened, created) = o.open_incident(&scheduler(), case.id, "error_rate", None, "change +0.5pp exceeds +0.1pp").unwrap();
    assert!(created);
    assert_eq!(opened.id, 1);

    let resolved = o.resolve_incident(&scheduler(), 1, "rollout paused").unwrap();
    assert_eq!(resolved.status, IncidentStatus::Resolved);
    assert_eq!(resolved.closed_by.as_deref(), Some("genie"));
    assert!(resolved.closed.is_some());
    assert_eq!(resolved.note, "rollout paused");
    expect_err(o.resolve_incident(&scheduler(), 1, ""), "I-1 is resolved; only an open incident can be resolved");

    // Dismissal is for people only, and needs a reason.
    expect_denied(
        o.dismiss_incident(&scheduler(), 1, "false alarm"),
        "only a person may dismiss an incident; resolve it instead, or ask a person to dismiss it",
    );
    expect_err(o.dismiss_incident(&me(), 1, "   "), "dismissing an incident needs a note that says why the violation is a false positive");
    let dismissed = o.dismiss_incident(&me(), 1, "  scrape gap, not a regression ").unwrap();
    assert_eq!(
        (dismissed.status, dismissed.closed_by.as_deref(), dismissed.note.as_str()),
        (IncidentStatus::Dismissed, Some("me"), "scrape gap, not a regression")
    );
    expect_err(o.dismiss_incident(&me(), 1, "again"), "I-1 is dismissed; only an open or resolved incident can be dismissed");
    expect_err(o.resolve_incident(&scheduler(), 1, ""), "I-1 is dismissed; only an open incident can be resolved");

    assert_eq!(events_of(&t, case.id, "incident.resolved").len(), 1);
    let dismissals = events_of(&t, case.id, "incident.dismissed");
    assert_eq!(dismissals.len(), 1);
    assert_eq!(dismissals[0].payload["incident"], 1);
    assert_eq!(dismissals[0].payload["note"], "scrape gap, not a regression");
    match o.incident(9) {
        Err(GenieError::NotFound(m)) => assert_eq!(m, "incident I-9 not found"),
        other => panic!("expected not found, got {other:?}"),
    }
}

#[test]
fn violated_guardrails_count_open_and_resolved_incidents_but_not_dismissed_ones() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let (first, _) = o.open_incident(&scheduler(), case.id, "error_rate", None, "error rate rose").unwrap();
    let (second, _) = o.open_incident(&scheduler(), case.id, "checkout_errors", None, "61 errors").unwrap();

    // Opening the same violation again returns the open incident and writes nothing.
    let (again, created) = o.open_incident(&scheduler(), case.id, "error_rate", None, "rose again").unwrap();
    assert!(!created);
    assert_eq!((again.id, again.summary.as_str()), (first.id, "error rate rose"));
    assert_eq!(events_of(&t, case.id, "incident.created").len(), 2);
    assert_eq!(o.violated_guardrails(case.id).unwrap(), ["checkout_errors", "error_rate"]);

    o.resolve_incident(&scheduler(), first.id, "").unwrap();
    assert_eq!(o.violated_guardrails(case.id).unwrap(), ["checkout_errors", "error_rate"], "a resolved violation still counts");
    o.dismiss_incident(&me(), first.id, "false positive from a scrape gap").unwrap();
    assert_eq!(o.violated_guardrails(case.id).unwrap(), ["checkout_errors"]);
    o.dismiss_incident(&me(), second.id, "false positive from a scrape gap").unwrap();
    assert!(o.violated_guardrails(case.id).unwrap().is_empty());

    // After a dismissal the same guardrail can be violated again and opens a new incident.
    let (third, created) = o.open_incident(&scheduler(), case.id, "error_rate", None, "rose once more").unwrap();
    assert!(created);
    assert_eq!(third.key, "I-3");
    assert_eq!(o.violated_guardrails(case.id).unwrap(), ["error_rate"]);
    assert_eq!(o.incidents(Some(IncidentStatus::Open)).unwrap().iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["I-3"]);
    assert_eq!(o.case_incidents(case.id).unwrap().iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["I-3", "I-2", "I-1"]);
}

#[test]
fn a_follow_up_task_is_recorded_once_and_listed_on_its_incident() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let fix = new_task(&t, "Fix the error rate");
    let docs = new_task(&t, "Docs");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let (incident, _) = o.open_incident(&scheduler(), case.id, "error_rate", None, "error rate rose").unwrap();

    let followup = o.add_followup(&me(), case.id, Some(incident.id), &fix.to_lowercase()).unwrap();
    assert_eq!(
        (followup.id, followup.case_id, followup.incident, followup.task.as_str(), followup.actor.as_str()),
        (1, case.id, Some(incident.id), fix.as_str(), "me")
    );
    assert_eq!(o.incident(incident.id).unwrap().followups, [fix.as_str()]);

    // Adding the same task again returns the same follow-up and writes no second event.
    let again = o.add_followup(&me(), case.id, Some(incident.id), &fix).unwrap();
    assert_eq!(again.id, followup.id);
    assert_eq!(events_of(&t, case.id, "followup_task.created").len(), 1);

    let plain = o.add_followup(&me(), case.id, None, &docs).unwrap();
    assert_eq!(plain.incident, None);
    assert_eq!(o.followups(case.id).unwrap().iter().map(|f| f.task.as_str()).collect::<Vec<_>>(), [fix.as_str(), docs.as_str()]);
    let created = events_of(&t, case.id, "followup_task.created");
    assert_eq!(created[0].payload["task"], fix.as_str());
    assert_eq!(created[0].payload["incident"], 1);
    assert!(created[1].payload["incident"].is_null());

    // Unknown tasks, unknown incidents and incidents of another case are refused by name.
    expect_err(o.add_followup(&me(), case.id, None, "G-99"), "task G-99 not found");
    expect_err(o.add_followup(&me(), case.id, Some(9), &docs), "incident I-9 not found");
    let other = observing_mixed(&t, &new_task(&t, "Other"));
    expect_err(o.add_followup(&me(), other.id, Some(incident.id), &docs), "I-1 belongs to O-1, not O-2");
    assert!(o.followups(other.id).unwrap().is_empty());
}

#[test]
fn due_lists_the_cases_with_a_check_an_evaluation_or_a_request_due_and_next_due_is_the_earliest() {
    let (_dir, t) = fresh();
    let o = t.observation();
    let anchor = now();
    // X's first guardrail check is due at the anchor, when observation started.
    let x = observing_mixed(&t, &new_task(&t, "X"));
    // Y and Z push their next check half a day later, and Z also has an evaluation requested.
    let y = observing_mixed(&t, &new_task(&t, "Y"));
    let z = observing_mixed(&t, &new_task(&t, "Z"));
    for case in [&y, &z] {
        apply(
            &t,
            case.id,
            CollectionInput {
                records: vec![],
                contract_version: 1,
                now: now(),
                next_guardrail_at: Some(anchor + Duration::hours(30)),
                failed: None,
                pass: true,
            },
        )
        .unwrap();
    }
    o.request_evaluation(&me(), z.id, false).unwrap();
    // W is waiting and C is cancelled: neither is ever due.
    let waiting = armed_case(&t, &new_task(&t, "W"));
    let cancelled = observing_mixed(&t, &new_task(&t, "C"));
    o.cancel(&me(), cancelled.id, "dropped", anchor).unwrap();

    let due = |at: DateTime<Utc>| -> Vec<i64> { o.due(at).unwrap().iter().map(|c| c.id).collect() };
    // Before the anchor only the requested case is due.
    assert_eq!(due(anchor - Duration::minutes(1)), [z.id]);
    // At the anchor X's check is due, with Z's request.
    assert_eq!(due(anchor), [x.id, z.id]);
    assert_eq!(o.next_due().unwrap().as_deref(), Some(ts(anchor).as_str()));

    // Once X's next check is 15 minutes later, the earliest scheduled time moves to it.
    apply(
        &t,
        x.id,
        CollectionInput {
            records: vec![],
            contract_version: 1,
            now: anchor,
            next_guardrail_at: Some(anchor + Duration::minutes(15)),
            failed: None,
            pass: true,
        },
    )
    .unwrap();
    assert_eq!(o.next_due().unwrap().as_deref(), Some(ts(anchor + Duration::minutes(15)).as_str()));
    assert_eq!(due(anchor + Duration::minutes(15)), [x.id, z.id]);
    // At 24 hours every observing case has its evaluation due, and the waiting and cancelled ones never do.
    assert_eq!(due(anchor + Duration::hours(24)), [x.id, y.id, z.id]);
    assert!(!due(anchor + Duration::hours(24)).contains(&waiting.id));
    assert!(!due(anchor + Duration::hours(24)).contains(&cancelled.id));

    // Once nothing is observing, nothing is scheduled.
    for case in [&x, &y, &z] {
        o.cancel(&me(), case.id, "end of the test", anchor).unwrap();
    }
    assert!(due(anchor + Duration::days(30)).is_empty());
    assert_eq!(o.next_due().unwrap(), None);
}

#[test]
fn evidence_is_filtered_and_paged_and_the_current_queries_see_only_the_current_contract_version() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let applied = apply(
        &t,
        case.id,
        pass(vec![
            NewEvidence { phase: EvidencePhase::Baseline, ..observed("p95", CriterionKind::Hypothesis, 100.0, None, TrustLevel::Measured) },
            NewEvidence {
                phase: EvidencePhase::Baseline,
                ..observed("error_rate", CriterionKind::Guardrail, 0.5, None, TrustLevel::Measured)
            },
            NewEvidence { phase: EvidencePhase::Baseline, ..failed_record("error_rate", CriterionKind::Guardrail, "timeout") },
            observed("p95", CriterionKind::Hypothesis, 80.0, Some(100.0), TrustLevel::Measured),
            observed("checkout_errors", CriterionKind::Guardrail, 10.0, None, TrustLevel::Measured),
        ]),
    )
    .unwrap()
    .unwrap();
    assert_eq!(applied.evidence.iter().map(|e| e.id).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);

    let listed = |filter: EvidenceFilter| -> Vec<i64> { o.evidence(case.id, &filter).unwrap().iter().map(|e| e.id).collect() };
    assert_eq!(listed(EvidenceFilter::default()), [1, 2, 3, 4, 5]);
    assert_eq!(listed(EvidenceFilter { criterion: Some("p95".into()), ..Default::default() }), [1, 4]);
    assert_eq!(listed(EvidenceFilter { after: Some(2), limit: Some(2), ..Default::default() }), [3, 4]);
    assert_eq!(listed(EvidenceFilter { newest_first: true, limit: Some(2), ..Default::default() }), [5, 4]);
    assert_eq!(listed(EvidenceFilter { newest_first: true, after: Some(4), ..Default::default() }), [3, 2, 1]);
    // Evidence pages go up to 500 (§15); the other listings stay at 200.
    expect_err(o.evidence(case.id, &EvidenceFilter { limit: Some(0), ..Default::default() }), "limit must be between 1 and 500");
    expect_err(o.evidence(case.id, &EvidenceFilter { limit: Some(501), ..Default::default() }), "limit must be between 1 and 500");
    assert_eq!(listed(EvidenceFilter { limit: Some(500), ..Default::default() }), [1, 2, 3, 4, 5]);
    expect_err(o.evaluations(case.id, Some(201)), "limit must be between 1 and 200");

    // The newest usable baseline: the failed record (3) is skipped.
    assert_eq!(o.latest_baseline(case.id, "error_rate").unwrap().map(|e| e.id), Some(2));
    assert_eq!(o.latest_baseline(case.id, "p95").unwrap().map(|e| e.id), Some(1));
    apply(
        &t,
        case.id,
        pass(vec![NewEvidence {
            phase: EvidencePhase::Baseline,
            ..observed("p95", CriterionKind::Hypothesis, 90.0, None, TrustLevel::Reported)
        }]),
    )
    .unwrap();
    assert_eq!(o.latest_baseline(case.id, "p95").unwrap().map(|e| e.id), Some(6), "a reported baseline is usable");
    apply(
        &t,
        case.id,
        pass(vec![NewEvidence {
            phase: EvidencePhase::Baseline,
            ..observed("p95", CriterionKind::Hypothesis, 1.0, None, TrustLevel::Agent)
        }]),
    )
    .unwrap();
    assert_eq!(o.latest_baseline(case.id, "p95").unwrap().map(|e| e.id), Some(6), "an agent baseline never counts");
    assert!(o.latest_baseline(case.id, "checkout_errors").unwrap().is_none());

    // A person revises the contract after the start: records of version 1 stop being current.
    let mut revised = mixed_contract();
    revised.guardrails[0].max_change = Some("+0.2pp".into());
    o.set_contract(&me(), case.id, &revised, "loosen error_rate after the release notes", now()).unwrap();
    assert!(o.current_evidence(case.id).unwrap().is_empty());
    assert!(o.latest_baseline(case.id, "p95").unwrap().is_none(), "baselines of version 1 do not count");
    assert_eq!(listed(EvidenceFilter::default()), [1, 2, 3, 4, 5, 6, 7], "history keeps every version");
    assert_eq!(o.evidence(case.id, &EvidenceFilter::default()).unwrap()[3].contract_version, 1);

    let under_v2 = CollectionInput {
        contract_version: 2,
        ..pass(vec![observed("p95", CriterionKind::Hypothesis, 85.0, Some(100.0), TrustLevel::Measured)])
    };
    apply(&t, case.id, under_v2).unwrap();
    let current = o.current_evidence(case.id).unwrap();
    assert_eq!(current.iter().map(|e| e.id).collect::<Vec<_>>(), [8]);
    assert_eq!(current[0].contract_version, 2);
}

#[test]
fn a_zero_length_window_is_valid_for_the_first_pass() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    // The first pass after the start measures [anchor, now] with anchor == now.
    let first = NewEvidence {
        window_start: now(),
        window_end: now(),
        ..observed("p95", CriterionKind::Hypothesis, 100.0, None, TrustLevel::Measured)
    };
    let applied = apply(&t, case.id, pass(vec![first])).unwrap().unwrap();
    assert_eq!(applied.evidence.len(), 1);
    assert_eq!(applied.evidence[0].window_start, applied.evidence[0].window_end);
    assert_eq!(o.current_evidence(case.id).unwrap().len(), 1);
}

#[test]
fn a_pass_or_an_evaluation_under_a_superseded_contract_writes_nothing() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    assert!(o.begin_evaluation(case.id).unwrap());
    // A person revises the contract while the scheduler is measuring under version 1 (D8).
    let mut revised = mixed_contract();
    revised.guardrails[0].max_change = Some("+0.2pp".into());
    o.set_contract(&me(), case.id, &revised, "loosen error_rate", now()).unwrap();
    let before = serde_json::to_value(o.get(case.id).unwrap()).unwrap();
    let journal_before = journal(&t, case.id).len();

    // A pass snapshotted under version 1 writes no record, no failure, no schedule and no event.
    let stale = CollectionInput {
        records: vec![observed("error_rate", CriterionKind::Guardrail, 1.0, Some(0.5), TrustLevel::Measured)],
        contract_version: 1,
        now: now(),
        next_guardrail_at: Some(now() + Duration::minutes(15)),
        failed: Some("timeout".into()),
        pass: true,
    };
    assert!(apply(&t, case.id, stale).unwrap().is_none());
    // The verdict computed under version 1 is refused the same way.
    let insufficient =
        EvalOutcome { verdict: Verdict::Insufficient, confidence: None, summary: "insufficient: test".into(), details: vec![] };
    assert!(
        o.finish_evaluation(&scheduler(), case.id, &insufficient, EvalTrigger::Window, 1, None, now() + Duration::hours(25))
            .unwrap()
            .is_none()
    );

    assert!(o.evidence(case.id, &EvidenceFilter::default()).unwrap().is_empty());
    assert!(o.evaluations(case.id, None).unwrap().is_empty());
    assert!(o.incidents(None).unwrap().is_empty());
    assert_eq!(serde_json::to_value(o.get(case.id).unwrap()).unwrap(), before, "the case is unchanged");
    assert_eq!(journal(&t, case.id).len(), journal_before, "no event was written");
    assert_eq!(o.get(case.id).unwrap().status, CaseStatus::Evaluating);

    // The current version is accepted, and the evaluation still finishes under it.
    let current = CollectionInput {
        contract_version: 2,
        ..pass(vec![observed("p95", CriterionKind::Hypothesis, 80.0, Some(100.0), TrustLevel::Measured)])
    };
    assert_eq!(apply(&t, case.id, current).unwrap().unwrap().evidence.len(), 1);
    let at = now() + Duration::hours(25);
    let finished = o.finish_evaluation(&scheduler(), case.id, &insufficient, EvalTrigger::Window, 2, None, at).unwrap().unwrap();
    assert_eq!(finished.status, CaseStatus::Observing);
}

#[test]
fn a_manual_report_leaves_the_failure_streak_alone() {
    let (_dir, t) = fresh();
    let task = new_task(&t, "Ship");
    let case = observing_mixed(&t, &task);
    let o = t.observation();
    let failing = |message: &str| CollectionInput {
        records: vec![failed_record("error_rate", CriterionKind::Guardrail, message)],
        contract_version: 1,
        now: now(),
        next_guardrail_at: None,
        failed: Some(message.into()),
        pass: true,
    };
    apply(&t, case.id, failing("prometheus: timeout")).unwrap();
    apply(&t, case.id, failing("prometheus: timeout")).unwrap();
    assert_eq!(o.get(case.id).unwrap().failures, 2);

    let manual = apply(&t, case.id, report(vec![observed("checkout_errors", CriterionKind::Guardrail, 10.0, None, TrustLevel::Reported)]))
        .unwrap()
        .unwrap();
    assert_eq!(manual.failures, 2);
    assert_eq!(manual.evidence.len(), 1);
    assert_eq!(manual.evidence[0].trust, TrustLevel::Reported);
    let stored = o.get(case.id).unwrap();
    assert_eq!((stored.failures, stored.last_error.as_deref()), (2, Some("prometheus: timeout")));

    // A manual report cannot carry a collection failure.
    expect_err(
        apply(&t, case.id, CollectionInput { failed: Some("boom".into()), ..report(vec![]) }),
        "a manual report cannot carry a collection failure",
    );
    // The next scheduler pass continues the streak.
    assert_eq!(apply(&t, case.id, failing("prometheus: timeout")).unwrap().unwrap().failures, 3);
}

#[test]
fn a_public_incident_must_name_a_guardrail_and_evidence_of_its_case() {
    let (_dir, t) = fresh();
    let case = observing_mixed(&t, &new_task(&t, "Ship"));
    let other = observing_mixed(&t, &new_task(&t, "Other"));
    let o = t.observation();
    let foreign = apply(&t, other.id, pass(vec![observed("checkout_errors", CriterionKind::Guardrail, 10.0, None, TrustLevel::Measured)]))
        .unwrap()
        .unwrap()
        .evidence[0]
        .id;

    expect_err(o.open_incident(&pm(), case.id, "p95", None, "x"), "criterion p95 is not a guardrail of the current contract of O-1");
    expect_err(o.open_incident(&pm(), case.id, "nope", None, "x"), "criterion nope is not a guardrail of the current contract of O-1");
    expect_err(
        o.open_incident(&pm(), case.id, "checkout_errors", Some(foreign), "x"),
        &format!("evidence {foreign} does not belong to O-1"),
    );
    expect_err(o.open_incident(&pm(), case.id, "checkout_errors", Some(99), "x"), "evidence 99 does not belong to O-1");
    assert!(o.incidents(None).unwrap().is_empty());

    // Evidence of the case itself is accepted.
    let own = apply(&t, case.id, pass(vec![observed("checkout_errors", CriterionKind::Guardrail, 10.0, None, TrustLevel::Measured)]))
        .unwrap()
        .unwrap()
        .evidence[0]
        .id;
    let (incident, created) = o.open_incident(&pm(), case.id, "checkout_errors", Some(own), "61 errors").unwrap();
    assert!(created);
    assert_eq!(incident.evidence, Some(own));
}
