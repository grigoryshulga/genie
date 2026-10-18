//! Observation application service (§14, §15): every function's permission and its refusal, the arm
//! rules, manual evidence and its incidents, follow-up tasks, the case view and the sources. Actors
//! are passed to the service directly; the HTTP routes are tested by `observation_api`.

use std::fmt;

use chrono::{DateTime, Duration, Utc};
use genie::observation::service::{
    self, CaseView, ContractBody, CreateCaseBody, DeploymentBody, EvaluateBody, EvidenceBody, FollowupBody, NoteBody, ReasonBody,
    SecretBody, SourceBody, SourceTestBody, UpdateCaseBody,
};
use genie::state::{AppError, AppResult};
use genie_core::inbox::Notification;
use genie_core::management::manager_name;
use genie_core::observation::contract::Contract;
use genie_core::observation::evaluate::EvalOutcome;
use genie_core::observation::store::{CaseFilter, EvidenceFilter};
use genie_core::observation::{
    CaseStatus, Confidence, EvalTrigger, EvidencePhase, IncidentStatus, ObservationPolicy, SourceAuth, SourceKind, TrustLevel, Verdict,
    parse_ts, ts,
};
use genie_core::team::ORCHESTRATOR;
use genie_core::{Actor, CreateInput, GenieError, Role, TaskType};
use serde_json::{Value, json};

use crate::common::Harness;
use crate::common::fakemetrics;

const PROJECT: &str = "shop";
const NOW: &str = "2026-10-17T12:00:00.000Z";
const MANAGE: &str = "only people and the project manager manage observation";
const SOURCES: &str = "only people manage observation sources and their secrets";

fn now() -> DateTime<Utc> {
    parse_ts(NOW).unwrap()
}

/// An hour after the anchor: the windows of reports end here.
fn later() -> DateTime<Utc> {
    now() + Duration::hours(1)
}

fn person() -> Actor {
    Actor::new("ann", Role::Human)
}

fn project_manager() -> Actor {
    Actor::new("project-manager", Role::Orchestrator)
}

fn member() -> Actor {
    Actor::new("dana", Role::Analyst)
}

/// The agent that manages one task: an orchestrator by class, but not the project manager.
fn task_manager() -> Actor {
    Actor::new(manager_name("GNE-1"), Role::Orchestrator)
}

fn contract(value: Value) -> Contract {
    serde_json::from_value(value).unwrap()
}

/// One manual guardrail: `checkout_errors` must stay at or below 50, observed for at least 24h.
fn guard_contract() -> Contract {
    contract(json!({
        "guardrails": [{ "id": "checkout_errors", "source": "manual", "max": 50 }],
        "observation": { "minimumWindow": "24h" }
    }))
}

/// A project with one open task, created by a person.
fn project() -> (Harness, String) {
    let h = Harness::new();
    h.project(PROJECT);
    let task = h
        .app
        .with_tracker(PROJECT, |t| t.create(&person(), CreateInput { title: "Faster checkout".into(), ..Default::default() }))
        .unwrap()
        .id;
    (h, task)
}

fn create_body(task: &str, contract: Option<Contract>) -> CreateCaseBody {
    CreateCaseBody { title: "Checkout latency".into(), policy: ObservationPolicy::Guardrails, tasks: vec![task.into()], contract }
}

/// A case armed and started by a person at `now`, while its task is still open.
fn observing(h: &Harness, task: &str, contract: Contract) -> CaseView {
    let case = service::create(&h.app, PROJECT, &person(), create_body(task, Some(contract))).unwrap();
    service::arm(&h.app, PROJECT, &person(), case.case.id, now()).unwrap();
    service::start(&h.app, PROJECT, &person(), case.case.id, now()).unwrap()
}

fn manual(criterion: &str, value: f64) -> EvidenceBody {
    EvidenceBody {
        criterion: criterion.into(),
        phase: None,
        value,
        baseline: None,
        samples: None,
        window_start: None,
        window_end: None,
        note: None,
    }
}

/// A manual report by `actor`, with the default window (the anchor to `later`).
fn report(h: &Harness, actor: &Actor, id: i64, criterion: &str, value: f64) -> AppResult<CaseView> {
    service::report_evidence(&h.app, PROJECT, actor, id, manual(criterion, value), later())
}

fn prometheus_body(name: &str, url: &str) -> SourceBody {
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

fn test_body(query: &str, window: Option<&str>) -> SourceTestBody {
    SourceTestBody { query: query.into(), window: window.map(str::to_string) }
}

/// The error of a call that must fail, whatever the type of its value.
#[track_caller]
fn failure<T: fmt::Debug>(r: AppResult<T>) -> AppError {
    match r {
        Ok(value) => panic!("expected the call to fail, got {value:?}"),
        Err(error) => error,
    }
}

#[track_caller]
fn forbidden<T: fmt::Debug>(r: AppResult<T>, message: &str) {
    match failure(r) {
        AppError::Forbidden(m) => assert_eq!(m, message),
        other => panic!("expected a refusal {message:?}, got {other:?}"),
    }
}

#[track_caller]
fn conflict<T: fmt::Debug>(r: AppResult<T>, message: &str) {
    match failure(r) {
        AppError::Conflict(m) => assert_eq!(m, message),
        other => panic!("expected a conflict {message:?}, got {other:?}"),
    }
}

#[track_caller]
fn bad<T: fmt::Debug>(r: AppResult<T>, message: &str) {
    match failure(r) {
        AppError::Bad(m) => assert_eq!(m, message),
        other => panic!("expected a bad request {message:?}, got {other:?}"),
    }
}

#[track_caller]
fn invalid<T: fmt::Debug>(r: AppResult<T>, message: &str) {
    match failure(r) {
        AppError::Genie(GenieError::Invalid(m)) => assert_eq!(m, message),
        other => panic!("expected invalid input {message:?}, got {other:?}"),
    }
}

#[test]
fn a_person_and_the_project_manager_create_arm_and_start_a_case() {
    let (h, task) = project();
    for actor in [person(), project_manager()] {
        let created = service::create(&h.app, PROJECT, &actor, create_body(&task, Some(guard_contract()))).unwrap();
        assert_eq!(created.case.status, CaseStatus::Proposed);
        assert_eq!(created.case.author, actor.name);
        assert_eq!(created.case.contract_version, 1);
        assert!(created.arm_problems.is_empty(), "{:?}", created.arm_problems);
        let armed = service::arm(&h.app, PROJECT, &actor, created.case.id, now()).unwrap();
        assert_eq!(armed.case.status, CaseStatus::Waiting);
        let started = service::start(&h.app, PROJECT, &actor, created.case.id, now()).unwrap();
        assert_eq!(started.case.status, CaseStatus::Observing);
        assert_eq!(started.case.observation_started_at.as_deref(), Some(NOW));
    }
}

#[test]
fn team_members_and_task_managers_are_refused_every_mutation() {
    let (h, task) = project();
    let case = observing(&h, &task, guard_contract());
    let id = case.case.id;
    let incident = report(&h, &person(), id, "checkout_errors", 61.0).unwrap().incidents[0].id;
    for actor in [member(), task_manager()] {
        forbidden(service::create(&h.app, PROJECT, &actor, create_body(&task, None)), MANAGE);
        forbidden(
            service::update(
                &h.app,
                PROJECT,
                &actor,
                id,
                UpdateCaseBody { title: Some("Renamed".into()), policy: None, add_tasks: None, remove_tasks: None },
            ),
            MANAGE,
        );
        forbidden(
            service::set_contract(&h.app, PROJECT, &actor, id, ContractBody { contract: guard_contract(), reason: Some("tighter".into()) }),
            MANAGE,
        );
        forbidden(service::arm(&h.app, PROJECT, &actor, id, now()), MANAGE);
        forbidden(service::start(&h.app, PROJECT, &actor, id, now()), MANAGE);
        forbidden(service::evaluate(&h.app, PROJECT, &actor, id, EvaluateBody { final_: None }), MANAGE);
        forbidden(service::cancel(&h.app, PROJECT, &actor, id, ReasonBody { reason: "stop".into() }, now()), MANAGE);
        forbidden(
            service::deploy(&h.app, PROJECT, &actor, id, DeploymentBody { reference: "v1".into(), environment: None, at: None }, now()),
            MANAGE,
        );
        forbidden(report(&h, &actor, id, "checkout_errors", 10.0), MANAGE);
        forbidden(service::resolve_incident(&h.app, PROJECT, &actor, incident, NoteBody { note: None }), MANAGE);
        forbidden(service::dismiss_incident(&h.app, PROJECT, &actor, incident, NoteBody { note: Some("noise".into()) }), MANAGE);
        forbidden(
            service::followup(&h.app, PROJECT, &actor, id, FollowupBody { incident: Some(incident), title: None, description: None }),
            MANAGE,
        );
        forbidden(service::sources(&h.app, PROJECT, &actor), SOURCES);
        forbidden(service::add_source(&h.app, PROJECT, &actor, prometheus_body("metrics", "https://prom.example.com")), SOURCES);
        forbidden(
            service::update_source(&h.app, PROJECT, &actor, "metrics", prometheus_body("metrics", "https://prom.example.com")),
            SOURCES,
        );
        forbidden(service::remove_source(&h.app, PROJECT, &actor, "metrics"), SOURCES);
        forbidden(
            service::set_source_secret(&h.app, PROJECT, &actor, "metrics", SecretBody { value: "sk-test-0123456789".into() }),
            SOURCES,
        );
        forbidden(service::clear_source_secret(&h.app, PROJECT, &actor, "metrics"), SOURCES);
        forbidden(
            service::test_source(&h.app, PROJECT, &actor, "metrics", SourceTestBody { query: "up".into(), window: None }, now()),
            SOURCES,
        );
    }
    // Every refusal left the case as it was.
    let after = service::show(&h.app, PROJECT, id).unwrap();
    assert_eq!(after.case.status, CaseStatus::Observing);
    assert_eq!(after.case.title, "Checkout latency");
    assert_eq!(after.incidents[0].status, IncidentStatus::Open);
    assert!(after.followups.is_empty());
    assert!(after.deployments.is_empty());
    assert_eq!(after.evidence.len(), 1);
    assert_eq!(after.contracts.len(), 1);
}

#[test]
fn the_project_manager_may_not_revise_request_final_dismiss_or_manage_sources() {
    let (h, task) = project();
    let case = observing(&h, &task, guard_contract());
    let id = case.case.id;
    let incident = report(&h, &person(), id, "checkout_errors", 61.0).unwrap().incidents[0].id;
    let pm = project_manager();
    forbidden(
        service::set_contract(&h.app, PROJECT, &pm, id, ContractBody { contract: guard_contract(), reason: Some("tighter".into()) }),
        "only a person may revise the contract after observation starts",
    );
    forbidden(
        service::evaluate(&h.app, PROJECT, &pm, id, EvaluateBody { final_: Some(true) }),
        "only a person may request a final evaluation",
    );
    forbidden(
        service::dismiss_incident(&h.app, PROJECT, &pm, incident, NoteBody { note: Some("noise".into()) }),
        "only a person may dismiss an incident; resolve it instead, or ask a person to dismiss it",
    );
    forbidden(service::add_source(&h.app, PROJECT, &pm, prometheus_body("metrics", "https://prom.example.com")), SOURCES);

    // What the project manager may do: request a plain evaluation and resolve the incident.
    let requested = service::evaluate(&h.app, PROJECT, &pm, id, EvaluateBody { final_: None }).unwrap();
    assert_eq!(requested.case.evaluation_requested, Some(EvalTrigger::Manual));
    let resolved = service::resolve_incident(&h.app, PROJECT, &pm, incident, NoteBody { note: None }).unwrap();
    assert_eq!(resolved.status, IncidentStatus::Resolved);

    // A person may do all four: a final evaluation, and dismissing with a note.
    let final_request = service::evaluate(&h.app, PROJECT, &person(), id, EvaluateBody { final_: Some(true) }).unwrap();
    assert_eq!(final_request.case.evaluation_requested, Some(EvalTrigger::Final));
    let dismissed =
        service::dismiss_incident(&h.app, PROJECT, &person(), incident, NoteBody { note: Some("a false positive".into()) }).unwrap();
    assert_eq!(dismissed.status, IncidentStatus::Dismissed);
    assert_eq!(dismissed.note, "a false positive");
}

#[test]
fn arm_problems_name_unknown_sources_and_missing_secrets_until_they_are_fixed() {
    let (h, task) = project();
    let metrics = contract(json!({
        "guardrails": [{ "id": "errors", "source": "metrics", "query": "sum(rate(errors[$__range]))", "max": 50 }],
        "observation": { "minimumWindow": "24h" }
    }));
    let case = service::create(&h.app, PROJECT, &person(), create_body(&task, Some(metrics))).unwrap();
    let id = case.case.id;

    conflict(service::arm(&h.app, PROJECT, &person(), id, now()), "unknown source metrics");
    assert_eq!(service::show(&h.app, PROJECT, id).unwrap().arm_problems, ["unknown source metrics"]);

    let mut bearer = prometheus_body("metrics", "https://prom.example.com");
    bearer.auth = Some(SourceAuth::Bearer);
    service::add_source(&h.app, PROJECT, &person(), bearer).unwrap();
    conflict(service::arm(&h.app, PROJECT, &person(), id, now()), "source metrics needs a secret");
    assert_eq!(service::show(&h.app, PROJECT, id).unwrap().arm_problems, ["source metrics needs a secret"]);

    service::set_source_secret(&h.app, PROJECT, &person(), "metrics", SecretBody { value: "sk-test-0123456789".into() }).unwrap();
    assert!(service::show(&h.app, PROJECT, id).unwrap().arm_problems.is_empty());
    let armed = service::arm(&h.app, PROJECT, &person(), id, now()).unwrap();
    assert_eq!(armed.case.status, CaseStatus::Waiting);
}

#[test]
fn manual_evidence_is_reported_for_a_person_and_agent_for_the_project_manager() {
    let (h, task) = project();
    let case = observing(&h, &task, guard_contract());
    let id = case.case.id;
    report(&h, &person(), id, "checkout_errors", 10.0).unwrap();
    let view = report(&h, &project_manager(), id, "checkout_errors", 11.0).unwrap();

    // Evidence is listed newest first.
    let newest = &view.evidence[0];
    assert_eq!(newest.trust, TrustLevel::Agent);
    assert_eq!(newest.provenance["reportedBy"], "project-manager");
    assert_eq!(newest.value, Some(11.0));
    let oldest = &view.evidence[1];
    assert_eq!(oldest.trust, TrustLevel::Reported);
    assert_eq!(oldest.provenance["reportedBy"], "ann");
    assert_eq!(oldest.kind, genie_core::observation::CriterionKind::Guardrail);
    assert_eq!(oldest.phase, EvidencePhase::Observed);
    assert_eq!(oldest.source, "manual");
    assert_eq!(oldest.window_start, NOW);
    assert_eq!(oldest.window_end, ts(later()));

    // The evaluator ignores agent records, so an agent's violation opens no incident.
    let violated = report(&h, &project_manager(), id, "checkout_errors", 61.0).unwrap();
    assert!(violated.incidents.is_empty());
    assert_eq!(violated.evidence[0].trust, TrustLevel::Agent);
}

#[test]
fn a_reported_violation_opens_one_incident_and_repeats_add_none() {
    let (h, task) = project();
    let case = observing(&h, &task, guard_contract());
    let id = case.case.id;
    let first = report(&h, &person(), id, "checkout_errors", 61.0).unwrap();
    assert_eq!(first.incidents.len(), 1);
    let incident = &first.incidents[0];
    assert_eq!(incident.key, "I-1");
    assert_eq!(incident.guardrail, "checkout_errors");
    assert_eq!(incident.status, IncidentStatus::Open);
    assert_eq!(incident.summary, "value 61 exceeds max 50");
    assert!(incident.followups.is_empty());

    let repeat = report(&h, &person(), id, "checkout_errors", 62.0).unwrap();
    assert_eq!(repeat.incidents.len(), 1);
    assert_eq!(repeat.incidents[0].summary, "value 61 exceeds max 50");
    // The new record still exists.
    assert_eq!(repeat.evidence.len(), 2);
}

#[test]
fn a_change_guardrail_compares_with_the_latest_baseline_record() {
    let (h, task) = project();
    let contract = contract(json!({
        "guardrails": [{ "id": "p95", "source": "manual", "maxChange": "+10%" }],
        "observation": { "minimumWindow": "24h" }
    }));
    let case = observing(&h, &task, contract);
    let id = case.case.id;
    let baseline = EvidenceBody { phase: Some(EvidencePhase::Baseline), ..manual("p95", 100.0) };
    service::report_evidence(&h.app, PROJECT, &person(), id, baseline, later()).unwrap();
    let view = report(&h, &person(), id, "p95", 130.0).unwrap();
    assert_eq!(view.evidence[0].baseline, Some(100.0));
    let rel = view.evidence[0].delta_rel.unwrap();
    assert!((rel - 30.0).abs() < 1e-9, "{rel}");
    assert_eq!(view.incidents.len(), 1);
    assert_eq!(view.incidents[0].summary, "change +30% exceeds +10%");
}

#[test]
fn a_follow_up_for_an_incident_is_a_bug_labelled_observation_and_linked_from_it() {
    let (h, task) = project();
    let case = observing(&h, &task, guard_contract());
    let id = case.case.id;
    let incident = report(&h, &person(), id, "checkout_errors", 61.0).unwrap().incidents[0].id;

    let view = service::followup(
        &h.app,
        PROJECT,
        &project_manager(),
        id,
        FollowupBody { incident: Some(incident), title: None, description: None },
    )
    .unwrap();
    assert_eq!(view.followups.len(), 1);
    let followup = &view.followups[0];
    assert_eq!(followup.incident, Some(incident));
    assert_eq!(followup.actor, "project-manager");

    let created = h.app.with_tracker(PROJECT, |t| t.get(&followup.task)).unwrap();
    assert_eq!(created.task_type, TaskType::Bug);
    assert_eq!(created.labels, ["observation"]);
    assert_eq!(created.title, "Regression: checkout_errors (O-1)");
    assert!(created.description.contains("/observation/1"), "{}", created.description);
    assert!(created.description.contains("I-1"), "{}", created.description);
    assert_eq!(view.incidents[0].followups, std::slice::from_ref(&followup.task));
}

#[test]
fn a_follow_up_without_an_incident_is_a_plain_task_and_an_incident_of_another_case_is_refused() {
    let (h, task) = project();
    let first = observing(&h, &task, guard_contract());
    let incident = report(&h, &person(), first.case.id, "checkout_errors", 61.0).unwrap().incidents[0].id;
    let second = observing(&h, &task, guard_contract());
    invalid(
        service::followup(
            &h.app,
            PROJECT,
            &person(),
            second.case.id,
            FollowupBody { incident: Some(incident), title: None, description: None },
        ),
        "I-1 belongs to O-1, not O-2",
    );

    let view =
        service::followup(&h.app, PROJECT, &person(), second.case.id, FollowupBody { incident: None, title: None, description: None })
            .unwrap();
    let followup = &view.followups[0];
    assert_eq!(followup.incident, None);
    let created = h.app.with_tracker(PROJECT, |t| t.get(&followup.task)).unwrap();
    assert_eq!(created.task_type, TaskType::Task);
    assert_eq!(created.labels, ["observation"]);
    assert_eq!(created.title, "Outcome not achieved: Checkout latency");
    assert!(view.incidents.is_empty());
}

#[test]
fn automatic_follow_ups_file_one_bug_per_new_incident() {
    let (h, task) = project();
    let auto = contract(json!({
        "guardrails": [{ "id": "checkout_errors", "source": "manual", "max": 50 }],
        "observation": { "minimumWindow": "24h" },
        "followup": "auto"
    }));
    let case = observing(&h, &task, auto);
    let id = case.case.id;
    let first = report(&h, &person(), id, "checkout_errors", 61.0).unwrap();
    assert_eq!(first.followups.len(), 1);
    assert_eq!(first.followups[0].actor, "observation");
    let filed = h.app.with_tracker(PROJECT, |t| t.get(&first.followups[0].task)).unwrap();
    assert_eq!(filed.task_type, TaskType::Bug);

    // A repeated violation of the open incident files nothing more.
    let repeat = report(&h, &person(), id, "checkout_errors", 62.0).unwrap();
    assert_eq!(repeat.incidents.len(), 1);
    assert_eq!(repeat.followups.len(), 1);
}

#[test]
fn the_case_view_carries_contract_history_arm_problems_incidents_and_follow_ups() {
    let (h, task) = project();
    let case = observing(&h, &task, guard_contract());
    let id = case.case.id;
    let revised = service::set_contract(
        &h.app,
        PROJECT,
        &person(),
        id,
        ContractBody {
            contract: contract(json!({
                "guardrails": [{ "id": "checkout_errors", "source": "manual", "max": 40 }],
                "observation": { "minimumWindow": "24h" }
            })),
            reason: Some("tighter limit after the incident review".into()),
        },
    )
    .unwrap();
    assert_eq!(revised.case.contract_version, 2);
    assert_eq!(revised.contracts.len(), 2);
    assert_eq!(revised.contracts[0].reason, "");
    assert_eq!(revised.contracts[1].reason, "tighter limit after the incident review");
    let current = revised.contract.as_ref().unwrap();
    assert_eq!(current.version, 2);
    assert_eq!(current.contract.guardrails[0].max, Some(40.0));
    assert!(revised.arm_problems.is_empty());

    let incidents = report(&h, &person(), id, "checkout_errors", 45.0).unwrap().incidents;
    assert_eq!(incidents.len(), 1, "45 exceeds the revised limit of 40");
    let with_follow_up = service::followup(
        &h.app,
        PROJECT,
        &person(),
        id,
        FollowupBody { incident: Some(incidents[0].id), title: Some("Roll back the checkout change".into()), description: None },
    )
    .unwrap();
    assert_eq!(with_follow_up.incidents[0].followups.len(), 1);
    assert_eq!(with_follow_up.followups.len(), 1);
    assert_eq!(with_follow_up.followups[0].incident, Some(incidents[0].id));
    let filed = h.app.with_tracker(PROJECT, |t| t.get(&with_follow_up.followups[0].task)).unwrap();
    assert_eq!(filed.title, "Roll back the checkout change");
    assert!(with_follow_up.evaluations.is_empty());
    assert!(with_follow_up.deployments.is_empty());
    assert_eq!(with_follow_up.evidence.len(), 1);
}

#[test]
fn reports_need_an_ordered_window_a_known_criterion_and_an_observing_case() {
    let (h, task) = project();
    let waiting = service::create(&h.app, PROJECT, &person(), create_body(&task, Some(guard_contract()))).unwrap();
    service::arm(&h.app, PROJECT, &person(), waiting.case.id, now()).unwrap();
    conflict(
        report(&h, &person(), waiting.case.id, "checkout_errors", 10.0),
        "O-1 is waiting; evidence is reported only while it is observing or evaluating",
    );

    let case = observing(&h, &task, guard_contract());
    let id = case.case.id;
    let mut same_bounds = manual("checkout_errors", 10.0);
    same_bounds.window_start = Some(NOW.into());
    same_bounds.window_end = Some(NOW.into());
    invalid(
        service::report_evidence(&h.app, PROJECT, &person(), id, same_bounds, later()),
        "windowStart must be before windowEnd; got 2026-10-17T12:00:00.000Z and 2026-10-17T12:00:00.000Z",
    );
    let mut reversed = manual("checkout_errors", 10.0);
    reversed.window_start = Some(ts(later()));
    reversed.window_end = Some(NOW.into());
    invalid(
        service::report_evidence(&h.app, PROJECT, &person(), id, reversed, later()),
        &format!("windowStart must be before windowEnd; got {} and {}", ts(later()), NOW),
    );
    invalid(report(&h, &person(), id, "nope", 1.0), "criterion nope is not in the current contract of O-2");
}

#[test]
fn listings_show_every_case_of_the_task_newest_first() {
    let (h, task) = project();
    let first = observing(&h, &task, guard_contract()).case.id;
    let second = observing(&h, &task, guard_contract()).case.id;
    let all = service::list(&h.app, PROJECT, &CaseFilter::default()).unwrap();
    assert_eq!(all.iter().map(|c| c.id).collect::<Vec<_>>(), [second, first]);
    assert_eq!(service::for_task(&h.app, PROJECT, &task).unwrap().len(), 2);
    assert!(service::incidents(&h.app, PROJECT, None).unwrap().is_empty());
}

#[test]
fn a_deployment_starts_a_case_whose_contract_starts_on_deployment() {
    let (h, task) = project();
    let deploy_first = contract(json!({
        "guardrails": [{ "id": "checkout_errors", "source": "manual", "max": 50 }],
        "observation": { "minimumWindow": "24h", "start": "deployment" }
    }));
    let case = service::create(&h.app, PROJECT, &person(), create_body(&task, Some(deploy_first))).unwrap();
    let id = case.case.id;
    let armed = service::arm(&h.app, PROJECT, &person(), id, now()).unwrap();
    assert_eq!(armed.case.status, CaseStatus::Waiting);

    let deployed_at = now() - Duration::minutes(30);
    let view = service::deploy(
        &h.app,
        PROJECT,
        &person(),
        id,
        DeploymentBody { reference: "3f2a9c1".into(), environment: Some("production".into()), at: Some(ts(deployed_at)) },
        now(),
    )
    .unwrap();
    assert_eq!(view.case.status, CaseStatus::Observing);
    assert_eq!(view.case.observation_started_at, Some(ts(deployed_at)));
    assert_eq!(view.deployments.len(), 1);
    assert_eq!(view.deployments[0].reference, "3f2a9c1");

    invalid(
        service::deploy(
            &h.app,
            PROJECT,
            &person(),
            id,
            DeploymentBody { reference: "v2".into(), environment: None, at: Some(ts(now() + Duration::hours(1))) },
            now(),
        ),
        "a deployment cannot be in the future; record the time it completed",
    );
}

#[test]
fn a_cancellation_needs_a_reason_and_a_cancelled_case_keeps_its_start() {
    let (h, task) = project();
    let case = observing(&h, &task, guard_contract());
    let id = case.case.id;
    invalid(service::cancel(&h.app, PROJECT, &person(), id, ReasonBody { reason: "   ".into() }, now()), "a cancellation needs a reason");
    let cancelled = service::cancel(&h.app, PROJECT, &project_manager(), id, ReasonBody { reason: "superseded".into() }, now()).unwrap();
    assert_eq!(cancelled.case.status, CaseStatus::Cancelled);
    assert_eq!(cancelled.case.conclusion, "superseded");
    assert_eq!(cancelled.case.observation_completed_at.as_deref(), Some(NOW));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_source_test_returns_the_value_the_source_gave() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("api_calls", vector("0.25"));
    let (h, _task) = project();
    service::add_source(&h.app, PROJECT, &person(), prometheus_body("metrics", &fake.url)).unwrap();
    let tested = h
        .app
        .blocking(|app| {
            service::test_source(
                app,
                PROJECT,
                &person(),
                "metrics",
                SourceTestBody { query: "sum(rate(api_calls[5m]))".into(), window: Some("30m".into()) },
                now(),
            )
        })
        .await
        .unwrap();
    assert_eq!(tested.value, 0.25);
    assert_eq!(tested.query, "sum(rate(api_calls[5m]))");
    assert_eq!(tested.provenance["adapter"], "prometheus");
    let calls = fake.state.lock().calls.clone();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].url.contains("api_calls"), "{}", calls[0].url);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failing_source_test_is_refused_with_the_adapter_message_before_any_secret_is_needed() {
    let fake = fakemetrics::spawn(None).await;
    let (h, _task) = project();
    service::add_source(&h.app, PROJECT, &person(), prometheus_body("metrics", &fake.url)).unwrap();

    let missing =
        h.app.blocking(|app| service::test_source(app, PROJECT, &person(), "metrics", test_body("missing_metric", None), now())).await;
    invalid(missing, "the source answered HTTP 404: no scripted answer");

    let mut bearer = prometheus_body("metrics", &fake.url);
    bearer.auth = Some(SourceAuth::Bearer);
    service::update_source(&h.app, PROJECT, &person(), "metrics", bearer).unwrap();
    let no_secret =
        h.app.blocking(|app| service::test_source(app, PROJECT, &person(), "metrics", test_body("api_calls", None), now())).await;
    invalid(no_secret, "source metrics needs a secret");
    let bad_window =
        h.app.blocking(|app| service::test_source(app, PROJECT, &person(), "metrics", test_body("up", Some("soon")), now())).await;
    bad(bad_window, "window soon must be a positive duration of at most 90 days, such as 30m, 1h or 2d");
    // Only the first test reached the source: the other two were refused before any request.
    assert_eq!(fake.state.lock().calls.len(), 1);
}

#[test]
fn sources_are_managed_by_people_and_their_secrets_stay_hidden() {
    let (h, _task) = project();
    let added = service::add_source(&h.app, PROJECT, &person(), prometheus_body("metrics", "https://prom.example.com")).unwrap();
    assert!(added.secret.is_none());
    assert_eq!(added.source.kind, SourceKind::Prometheus);

    let with_secret =
        service::set_source_secret(&h.app, PROJECT, &person(), "metrics", SecretBody { value: "sk-test-0123456789".into() }).unwrap();
    assert_eq!(with_secret.secret.as_ref().unwrap().hint, "…6789");
    let wire = serde_json::to_string(&with_secret).unwrap();
    assert!(!wire.contains("sk-test"), "{wire}");
    assert_eq!(service::sources(&h.app, PROJECT, &person()).unwrap().len(), 1);

    let cleared = service::clear_source_secret(&h.app, PROJECT, &person(), "metrics").unwrap();
    assert!(cleared.secret.is_none());

    invalid(
        service::add_source(&h.app, PROJECT, &person(), prometheus_body("other", "prom.example.com")),
        "url must start with http:// or https://",
    );
    service::remove_source(&h.app, PROJECT, &person(), "metrics").unwrap();
    assert!(service::sources(&h.app, PROJECT, &person()).unwrap().is_empty());
}

/// Two people: `ann`, who authors the cases of these tests, and `root`, a server administrator
/// (a project administrator for notifications: every server admin is one).
fn people(h: &Harness) {
    h.app
        .with_server(|db| {
            db.create_user("ann", "Ann", None, None, false)?;
            db.create_user("root", "Root", None, None, true)?;
            Ok(())
        })
        .unwrap();
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

/// The mail waiting in the orchestrator's mailbox, where the project manager reads it.
fn orchestrator_mail(h: &Harness) -> Vec<genie_core::team::Mail> {
    h.app.with_tracker(PROJECT, |t| t.bus().pending(None, ORCHESTRATOR)).unwrap()
}

#[test]
fn a_reported_violation_notifies_the_admins_and_the_human_author_once() {
    let (h, task) = project();
    people(&h);
    let case = observing(&h, &task, guard_contract());
    let id = case.case.id;
    report(&h, &person(), id, "checkout_errors", 61.0).unwrap();

    for login in ["ann", "root"] {
        let mine = notifications(&h, login, "observation.incident");
        assert_eq!(mine.len(), 1, "{login}");
        assert_eq!(mine[0].link.as_deref(), Some("/observation/1"));
        assert_eq!(mine[0].body, "value 61 exceeds max 50");
    }
    // The same incident again: the dedupe key keeps every notification to one.
    let incident = service::incidents(&h.app, PROJECT, None).unwrap().remove(0);
    let current = service::show(&h.app, PROJECT, id).unwrap().case;
    service::notify_incident(&h.app, PROJECT, &incident, &current).unwrap();
    for login in ["ann", "root"] {
        assert_eq!(notifications(&h, login, "observation.incident").len(), 1, "{login}");
    }

    // The project manager is mailed in English, with the command that answers the incident.
    let mail = orchestrator_mail(&h);
    let incident_mail = mail.iter().find(|m| m.text.contains("is violated")).expect("the incident mail");
    assert_eq!(incident_mail.from, "observation");
    assert_eq!(incident_mail.task.as_deref(), Some(task.as_str()));
    assert!(incident_mail.text.contains("genie observe resolve I-1"), "{}", incident_mail.text);
}

#[test]
fn a_case_of_the_project_manager_notifies_the_admins_and_no_author() {
    let (h, task) = project();
    people(&h);
    let case = service::create(&h.app, PROJECT, &project_manager(), create_body(&task, Some(guard_contract()))).unwrap();
    let id = case.case.id;
    service::arm(&h.app, PROJECT, &project_manager(), id, now()).unwrap();
    service::start(&h.app, PROJECT, &project_manager(), id, now()).unwrap();
    report(&h, &person(), id, "checkout_errors", 61.0).unwrap();

    assert_eq!(notifications(&h, "root", "observation.incident").len(), 1);
    // The author is an agent, not a person: nobody is notified as the author.
    assert!(notifications(&h, "ann", "observation.incident").is_empty());
}

#[test]
fn a_concluded_case_notifies_once_and_mails_the_manager() {
    let (h, task) = project();
    people(&h);
    let case = observing(&h, &task, guard_contract());
    let id = case.case.id;
    let contract_version = case.case.contract_version;
    h.app
        .with_tracker(PROJECT, |t| {
            t.observation().begin_evaluation(id)?;
            let outcome = EvalOutcome {
                verdict: Verdict::Negative,
                confidence: Some(Confidence::High),
                summary: "negative: 0/1 ok, 1 violated; attribution: not assessed".into(),
                details: Vec::new(),
            };
            t.observation().finish_evaluation(&person(), id, &outcome, EvalTrigger::Manual, contract_version, later())
        })
        .unwrap();
    let concluded = service::show(&h.app, PROJECT, id).unwrap();
    assert_eq!(concluded.case.status, CaseStatus::Concluded);

    service::notify_concluded(&h.app, PROJECT, &concluded.case).unwrap();
    service::notify_concluded(&h.app, PROJECT, &concluded.case).unwrap();
    for login in ["ann", "root"] {
        let mine = notifications(&h, login, "observation.concluded");
        assert_eq!(mine.len(), 1, "{login}");
        assert_eq!(mine[0].link.as_deref(), Some("/observation/1"));
    }
    let mail = orchestrator_mail(&h);
    let done = mail.iter().find(|m| m.text.contains("concluded as negative")).expect("the conclusion mail");
    assert_eq!(done.from, "observation");
    assert!(done.text.contains("genie observe followup O-1"), "{}", done.text);
}

#[test]
fn an_unreadable_secret_counts_as_missing_for_arming() {
    let (h, task) = project();
    let mut bearer = prometheus_body("metrics", "https://prom.example.com");
    bearer.auth = Some(SourceAuth::Bearer);
    service::add_source(&h.app, PROJECT, &person(), bearer).unwrap();
    service::set_source_secret(&h.app, PROJECT, &person(), "metrics", SecretBody { value: "sk-test-0123456789".into() }).unwrap();
    let metrics = contract(json!({
        "guardrails": [{ "id": "errors", "source": "metrics", "query": "sum(rate(errors[$__range]))", "max": 50 }],
        "observation": { "minimumWindow": "24h" }
    }));
    let case = service::create(&h.app, PROJECT, &person(), create_body(&task, Some(metrics))).unwrap();
    let id = case.case.id;
    assert!(service::show(&h.app, PROJECT, id).unwrap().arm_problems.is_empty());

    // The server's key file is gone: the stored secret can no longer be read back.
    std::fs::remove_file(h.dir.path().join("secrets.key")).unwrap();
    assert_eq!(service::show(&h.app, PROJECT, id).unwrap().arm_problems, ["source metrics needs a secret"]);
    conflict(service::arm(&h.app, PROJECT, &person(), id, now()), "source metrics needs a secret");
}

#[test]
fn arming_a_case_that_is_not_proposed_gets_the_cores_refusal() {
    let (h, task) = project();
    let case = observing(&h, &task, guard_contract());
    invalid(service::arm(&h.app, PROJECT, &person(), case.case.id, now()), "only a proposed case can be armed; O-1 is observing");
}

#[test]
fn the_evidence_listing_pages_and_filters_in_store_order() {
    let (h, task) = project();
    let two = contract(json!({
        "guardrails": [
            { "id": "checkout_errors", "source": "manual", "max": 50 },
            { "id": "error_rate", "source": "manual", "max": 5 }
        ],
        "observation": { "minimumWindow": "24h" }
    }));
    let id = observing(&h, &task, two).case.id;
    report(&h, &person(), id, "checkout_errors", 10.0).unwrap();
    report(&h, &person(), id, "error_rate", 1.0).unwrap();
    report(&h, &person(), id, "checkout_errors", 12.0).unwrap();

    let all = service::evidence(&h.app, PROJECT, id, &EvidenceFilter::default()).unwrap();
    assert_eq!(all.iter().map(|e| e.value).collect::<Vec<_>>(), [Some(10.0), Some(1.0), Some(12.0)]);
    let only = service::evidence(&h.app, PROJECT, id, &EvidenceFilter { criterion: Some("checkout_errors".into()), ..Default::default() })
        .unwrap();
    assert_eq!(only.len(), 2);
    let after =
        service::evidence(&h.app, PROJECT, id, &EvidenceFilter { after: Some(all[0].id), limit: Some(1), ..Default::default() }).unwrap();
    assert_eq!(after.iter().map(|e| e.id).collect::<Vec<_>>(), [all[1].id]);
    let newest =
        service::evidence(&h.app, PROJECT, id, &EvidenceFilter { newest_first: true, limit: Some(1), ..Default::default() }).unwrap();
    assert_eq!(newest[0].value, Some(12.0));
    match failure(service::evidence(&h.app, PROJECT, 99, &EvidenceFilter::default())) {
        AppError::Genie(GenieError::NotFound(m)) => assert_eq!(m, "observation O-99 not found"),
        other => panic!("expected a missing case, got {other:?}"),
    }
}

#[test]
fn a_secret_body_never_prints_its_value() {
    let body = SecretBody { value: "sk-test-0123456789".into() };
    assert_eq!(format!("{body:?}"), r#"SecretBody { value: "<redacted>" }"#);
}
