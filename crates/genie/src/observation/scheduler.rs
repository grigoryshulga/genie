//! Scheduler of the observation plane (`docs/platform/observation-plane.md` §11). It starts waiting
//! cases when their tasks are done, collects evidence for the due cases, checks guardrails and
//! evaluates on schedule. Every step is stored before the next one runs, and the state lives in the
//! tracker, so a restart continues from what was stored (§6.1).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, MutexGuard};

use chrono::{DateTime, Duration, Utc};
use genie_core::events;
use genie_core::observation::contract::Contract;
use genie_core::observation::evaluate::{self, EvalInput};
use genie_core::observation::store::{CollectionInput, NewEvidence};
use genie_core::observation::{
    CaseStatus, CriterionKind, EvalTrigger, EvidencePhase, ObservationCase, SourceConfig, SourceKind, TrustLevel, case_key, parse_ts, ts,
};
use genie_core::{Actor, GenieError, Role};
use serde_json::json;

use crate::notify::{self, Message};
use crate::observation::adapters::{self, Measurement, Span};
use crate::observation::service;
use crate::state::{App, AppError, AppResult, SAFETY_NET, block_on, sleep_until};

/// The actor of the scheduler's writes (§11.2, §11.4).
const SCHEDULER: &str = "genie";
/// The journal cursor of intake (§11.2).
const CURSOR: &str = "observation";
/// Events read per intake batch (§11.2).
const BATCH: usize = 200;
/// Consecutive failed passes after which the project's admins are told (§11.3 step 6).
const FAILURE_ALERT: i64 = 3;
/// The notification kind of a case whose collection keeps failing. §18 lists only the incident and
/// conclusion kinds; this one is the scheduler's own.
pub const FAILURE_KIND: &str = "observation.failing";
/// The wait before a case whose pass failed, or a pass that could not finish, is tried again.
const RETRY_AFTER: Duration = Duration::seconds(10);
/// Passes in a row that ask for another at once; a bug that keeps asking falls back to the deadlines.
const MAX_BACK_TO_BACK: u32 = 20;
/// The source of values that people and the project manager report (§7.1). Never measured.
const MANUAL: &str = "manual";

/// What a pass learned about when it must run again without being woken (§11.5).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// The earliest `nextGuardrailAt`, `nextEvaluationAt` or retry time of a case that is still due.
    pub at: Option<DateTime<Utc>>,
    /// An evaluation was requested while the pass ran, for a case the pass had not reached: pass again
    /// at once. A failed case never asks for this; it waits for its retry time.
    pub again: bool,
}

impl Plan {
    fn due(&mut self, at: DateTime<Utc>) {
        self.at = Some(self.at.map_or(at, |t| t.min(at)));
    }
}

/// Starts the scheduler loop (§11.1). It runs a pass, then sleeps until the soonest deadline, the
/// safety net, or a wake-up from the journal or the service (`App::wake_observation`).
pub fn start(app: &Arc<App>) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut streak = 0;
        loop {
            let plan = match app.blocking(|app| pass_at(app, Utc::now())).await {
                Ok(plan) => plan,
                Err(e) => {
                    eprintln!("genie observation: {e}");
                    Plan { at: Some(retry_at(Utc::now())), again: false }
                }
            };
            streak = if plan.again { streak + 1 } else { 0 };
            if plan.again && streak < MAX_BACK_TO_BACK {
                continue;
            }
            tokio::select! {
                _ = app.wake_observation.notified() => {}
                _ = tokio::time::sleep(sleep_until(plan.at, SAFETY_NET)) => {}
            }
        }
    });
}

/// One pass over every project at time `now` (§11.1). A failure of one project is logged and the
/// other projects still run; the pass itself fails only when the projects cannot be listed.
pub fn pass_at(app: &App, now: DateTime<Utc>) -> AppResult<Plan> {
    let mut plan = Plan::default();
    for project in app.with_server(|db| db.projects())? {
        if let Err(e) = run_project(app, &project.slug, now, &mut plan) {
            eprintln!("genie observation: {}: {e}", project.slug);
            plan.due(retry_at(now));
        }
    }
    Ok(plan)
}

/// Intake, then every due case of one project, then the schedule for the next pass (§11.2, §11.5).
/// A failed intake does not stop the due cases; a failed case is skipped until its retry time.
fn run_project(app: &App, project: &str, now: DateTime<Utc>, plan: &mut Plan) -> AppResult<()> {
    if let Err(e) = intake(app, project, now) {
        eprintln!("genie observation: {project}: intake: {e}");
        plan.due(retry_at(now));
    }
    let due = app.with_tracker(project, |t| t.observation().due(now))?;
    let initial: BTreeSet<i64> = due.iter().map(|case| case.id).collect();
    for case in &due {
        if let Some(until) = retry_until(app, project, case.id)?
            && until > now
        {
            continue;
        }
        match process(app, project, case.id, now) {
            Ok(()) => clear_retry(app, project, case.id)?,
            Err(e) => {
                eprintln!("genie observation: {project}: {}: {e}", case_key(case.id));
                set_retry(app, project, case.id, retry_at(now))?;
            }
        }
    }
    let pending = app.with_tracker(project, |t| t.observation().due(now))?;
    // A request that arrived while this pass ran, for a case the pass did not reach, runs at once.
    plan.again |= pending.iter().any(|case| case.evaluation_requested.is_some() && !initial.contains(&case.id));
    for case in &pending {
        if let Some(until) = retry_until(app, project, case.id)?
            && until > now
        {
            plan.due(until);
        }
    }
    if let Some(at) = app.with_tracker(project, |t| t.observation().next_due())? {
        let at = parse_ts(&at)?;
        // A deadline that has passed belongs to a case that stayed due (a race): try again later
        // instead of spinning on it.
        plan.due(if at > now { at } else { retry_at(now) });
    }
    Ok(())
}

/// Starts the waiting cases whose tasks became done since the last pass (§6.3, §11.2). The journal
/// cursor moves after each batch, and only after a batch succeeded; starting a case again is a
/// no-op, so a re-read is harmless.
fn intake(app: &App, project: &str, now: DateTime<Utc>) -> AppResult<()> {
    let actor = scheduler_actor();
    let mut cursor = app.with_tracker(project, |t| t.event_cursor(CURSOR))?;
    loop {
        let events = app.with_tracker(project, |t| t.events_after(cursor, BATCH))?;
        let Some(last) = events.last().map(|e| e.id) else { return Ok(()) };
        for event in &events {
            if event.kind == events::TASK_STATUS_CHANGED
                && event.payload["to"].as_str() == Some("done")
                && let Some(task) = event.subject.as_deref()
            {
                app.with_tracker(project, |t| t.observation().on_task_done(&actor, task, now))?;
            }
        }
        app.with_tracker(project, |t| t.ack_events(CURSOR, last))?;
        if events.len() < BATCH {
            return Ok(());
        }
        cursor = last;
    }
}

/// One due case of a pass (§11.3, §11.4): snapshot, measure, write, the side effects after the
/// write, and the evaluation when it is due.
fn process(app: &App, project: &str, id: i64, now: DateTime<Utc>) -> AppResult<()> {
    let actor = scheduler_actor();
    let Some(Snapshot { case, contract, baselines }) = snapshot(app, project, id)? else { return Ok(()) };
    let anchor = anchor_of(&case)?;
    let windows = contract
        .try_resolved()
        .ok_or_else(|| AppError::Genie(GenieError::invalid(format!("the contract of {} has a duration that does not parse", case.key))))?;
    let guardrails_due = is_due(case.next_guardrail_at.as_deref(), now)?;
    let evaluation_due = case.evaluation_requested.is_some() || is_due(case.next_evaluation_at.as_deref(), now)?;
    if !guardrails_due && !evaluation_due {
        return Ok(());
    }
    let sources = sources_of(app, project, &contract)?;
    // The evaluation begins before anything is measured, so a crash leaves the case `evaluating` (§11.4).
    let evaluating = evaluation_due && app.with_tracker(project, |t| t.observation().begin_evaluation(id))?;
    let times = Times {
        key: case.key.clone(),
        anchor,
        end: now.max(anchor),
        guardrail_window: windows.guardrail_window,
        baseline_window: windows.baseline_window,
        guardrails_due,
        hypotheses_due: evaluating,
    };
    let wants = wanted(&contract, &sources, &baselines, &times)?;
    let measured = measure_all(wants, &sources);

    // Observed records of this pass use the baseline the pass has just measured, if it had none.
    let mut known = baselines;
    for (want, result) in &measured {
        if let (EvidencePhase::Baseline, Ok(m)) = (want.phase, result) {
            known.insert(want.criterion.clone(), m.value);
        }
    }
    let failed = measured.iter().find_map(|(_, result)| result.as_ref().err().cloned());
    let records: Vec<NewEvidence> = measured.into_iter().map(|(want, result)| record(want, result, &known)).collect();
    let next_guardrail_at = if guardrails_due { Some(add_to(now, windows.guardrail_interval, &case.key)?) } else { None };
    let input =
        CollectionInput { records, contract_version: case.contract_version, now, next_guardrail_at, failed: failed.clone(), pass: true };
    // A contract revised since the snapshot, or a case that stopped observing, writes nothing; the
    // next pass measures again under the current version.
    let Some(applied) = app.with_tracker(project, |t| t.observation().apply_collection(&actor, id, input))? else {
        return Ok(());
    };

    service::announce_incidents(app, project, id, &applied.new_incidents);
    if applied.failures == FAILURE_ALERT {
        // The first record of the alerting pass names the streak: a later streak stores new records.
        let streak = applied.evidence.first().map_or_else(|| ts(now), |record| record.id.to_string());
        notify_failures(app, project, &case, failed.as_deref().unwrap_or_default(), &streak);
    }
    if evaluating {
        let trigger = trigger_of(&case, anchor, now, windows.preferred_window)?;
        evaluate_case(app, project, &case, &contract, trigger, now)?;
    }
    Ok(())
}

/// What a pass reads before it measures (§11.3 step 1).
struct Snapshot {
    case: ObservationCase,
    contract: Contract,
    /// The latest usable baseline value of each criterion that needs one.
    baselines: BTreeMap<String, f64>,
}

/// `None` when the case is no longer observing or evaluating, so there is nothing to collect.
fn snapshot(app: &App, project: &str, id: i64) -> AppResult<Option<Snapshot>> {
    app.with_tracker(project, |t| {
        let obs = t.observation();
        let case = obs.get(id)?;
        if !matches!(case.status, CaseStatus::Observing | CaseStatus::Evaluating) {
            return Ok(None);
        }
        let version = obs.contract(id, None)?.ok_or_else(|| GenieError::invalid(format!("{} has no contract", case.key)))?;
        let mut baselines = BTreeMap::new();
        for criterion in criteria(&version.contract).iter().filter(|c| c.needs_baseline) {
            if let Some(value) = obs.latest_baseline(id, &criterion.id)?.and_then(|record| record.value) {
                baselines.insert(criterion.id.clone(), value);
            }
        }
        Ok(Some(Snapshot { case, contract: version.contract, baselines }))
    })
}

/// The configured source and its secret for each non-manual source the contract names (§11.3 step 2).
/// A source that is not configured is kept as `None`, so its measurements fail with a message.
type Sources = BTreeMap<String, (Option<SourceConfig>, Option<String>)>;

fn sources_of(app: &App, project: &str, contract: &Contract) -> AppResult<Sources> {
    app.with_server(|db| {
        let mut sources = BTreeMap::new();
        for name in contract.sources() {
            let source = db.observation_source(project, &name)?;
            // A secret that cannot be read back counts as none, as the arm rules do. The reason is
            // logged; the secret never is.
            let secret = match db.observation_secret(project, &name) {
                Ok(secret) => secret,
                Err(e) => {
                    eprintln!("genie observation: {project}: cannot read the secret of source {name}: {e}");
                    None
                }
            };
            sources.insert(name, (source, secret));
        }
        Ok(sources)
    })
}

/// The times and windows of one case in one pass.
struct Times {
    /// The case key, for the messages of out-of-range windows.
    key: String,
    anchor: DateTime<Utc>,
    /// The end of every observed window: the pass time, never before the anchor.
    end: DateTime<Utc>,
    guardrail_window: Duration,
    baseline_window: Duration,
    guardrails_due: bool,
    hypotheses_due: bool,
}

/// A criterion of the contract with what a pass needs to measure it (§7, §8, §9).
struct Criterion {
    id: String,
    kind: CriterionKind,
    source: String,
    query: String,
    samples_query: Option<String>,
    needs_baseline: bool,
}

fn criteria(contract: &Contract) -> Vec<Criterion> {
    let hypotheses = contract.hypotheses.iter().map(|h| Criterion {
        id: h.id.clone(),
        kind: CriterionKind::Hypothesis,
        source: h.source.clone(),
        query: h.query.clone(),
        samples_query: h.samples_query.clone(),
        needs_baseline: contract.needs_baseline(&h.id),
    });
    let guardrails = contract.guardrails.iter().map(|g| Criterion {
        id: g.id.clone(),
        kind: CriterionKind::Guardrail,
        source: g.source.clone(),
        query: g.query.clone(),
        samples_query: None,
        needs_baseline: contract.needs_baseline(&g.id),
    });
    hypotheses.chain(guardrails).collect()
}

/// One measurement of a pass: the criterion, the phase its window belongs to, and its source.
struct Want {
    criterion: String,
    kind: CriterionKind,
    phase: EvidencePhase,
    source: String,
    query: String,
    samples_query: Option<String>,
    span: Span,
    adapter: Option<SourceKind>,
}

/// The shortest observed guardrail window that is measured. The anchor is stored with millisecond
/// precision (§4.2) while a pass runs at its own time, so a window of a few milliseconds after the
/// anchor is an artifact of the storage, not observation: such a check is only scheduled.
const MIN_OBSERVED_WINDOW: Duration = Duration::seconds(1);

/// The measurements a pass takes (§11.3 steps 3, 4 and 7): a missing baseline, the guardrails when
/// their check is due, and the hypotheses when the case is evaluated. Manual criteria are never
/// measured (§7.1). Windows follow §7.2. An observed guardrail window shorter than
/// `MIN_OBSERVED_WINDOW`, as at the anchor, has nothing to observe yet: the check is only scheduled.
fn wanted(contract: &Contract, sources: &Sources, baselines: &BTreeMap<String, f64>, times: &Times) -> AppResult<Vec<Want>> {
    let mut wants = Vec::new();
    for c in criteria(contract).into_iter().filter(|c| c.source != MANUAL) {
        let adapter = sources.get(&c.source).and_then(|(source, _)| source.as_ref()).map(|source| source.kind);
        if c.needs_baseline && !baselines.contains_key(&c.id) {
            let span = Span { start: sub_from(times.anchor, times.baseline_window, &times.key)?, end: times.anchor };
            wants.push(want(&c, adapter, EvidencePhase::Baseline, span, None));
        }
        if c.kind == CriterionKind::Guardrail && times.guardrails_due {
            let start = times.anchor.max(sub_from(times.end, times.guardrail_window, &times.key)?);
            if times.end - start >= MIN_OBSERVED_WINDOW {
                wants.push(want(&c, adapter, EvidencePhase::Observed, Span { start, end: times.end }, None));
            }
        }
        if c.kind == CriterionKind::Hypothesis && times.hypotheses_due {
            let span = Span { start: times.anchor, end: times.end };
            let samples_query = c.samples_query.clone();
            wants.push(want(&c, adapter, EvidencePhase::Observed, span, samples_query));
        }
    }
    Ok(wants)
}

fn want(c: &Criterion, adapter: Option<SourceKind>, phase: EvidencePhase, span: Span, samples_query: Option<String>) -> Want {
    Want { criterion: c.id.clone(), kind: c.kind, phase, source: c.source.clone(), query: c.query.clone(), samples_query, span, adapter }
}

/// Takes the measurements one after another in one blocking call. No database lock is held while a
/// source answers (§11.3): the caller has read everything it needs before this runs.
fn measure_all(wants: Vec<Want>, sources: &Sources) -> Vec<(Want, Result<Measurement, String>)> {
    block_on(async {
        let mut measured = Vec::with_capacity(wants.len());
        for want in wants {
            let result = match sources.get(&want.source) {
                Some((Some(source), secret)) => {
                    adapters::measure(source, secret.as_deref(), &want.query, want.samples_query.as_deref(), want.span)
                        .await
                        .map_err(|e| e.to_string())
                }
                _ => Err(format!("unknown source {}", want.source)),
            };
            measured.push((want, result));
        }
        measured
    })
}

/// The record of one measurement (§4.3, §7.4). A failed measurement is stored as an error with no
/// value, so the evaluator and the guardrail check ignore it. The baseline is copied only into an
/// observed record that has a value.
fn record(want: Want, result: Result<Measurement, String>, baselines: &BTreeMap<String, f64>) -> NewEvidence {
    let Want { criterion, kind, phase, source, query, span, adapter, .. } = want;
    let baseline = match phase {
        EvidencePhase::Observed => baselines.get(&criterion).copied(),
        EvidencePhase::Baseline => None,
    };
    let (query, value, samples, provenance, error) = match result {
        Ok(m) => (m.query, Some(m.value), m.samples.and_then(|n| i64::try_from(n).ok()), m.provenance, None),
        Err(message) => {
            // The adapter is named only when the source is known; no null placeholder.
            let provenance = adapter.map_or_else(|| json!({}), |kind| json!({ "adapter": kind.as_str() }));
            (adapters::substitute_range(&query, span), None, None, provenance, Some(message))
        }
    };
    NewEvidence {
        criterion,
        kind,
        phase,
        source,
        query,
        window_start: span.start,
        window_end: span.end,
        baseline: baseline.filter(|_| value.is_some()),
        value,
        samples,
        provenance,
        trust: TrustLevel::Measured,
        error,
        note: String::new(),
    }
}

/// Why this evaluation runs (§11.4 step 2): a request wins, else the preferred window has passed.
fn trigger_of(case: &ObservationCase, anchor: DateTime<Utc>, now: DateTime<Utc>, preferred: Duration) -> AppResult<EvalTrigger> {
    Ok(match case.evaluation_requested {
        Some(EvalTrigger::Final) => EvalTrigger::Final,
        Some(EvalTrigger::Manual) => EvalTrigger::Manual,
        _ if now >= add_to(anchor, preferred, &case.key)? => EvalTrigger::Final,
        _ => EvalTrigger::Window,
    })
}

/// Evaluates the case from its stored records and stores the verdict (§11.4 steps 4 to 6). A stale
/// verdict (the case changed meanwhile) writes nothing; the request stays for a later pass.
fn evaluate_case(
    app: &App,
    project: &str,
    case: &ObservationCase,
    contract: &Contract,
    trigger: EvalTrigger,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let anchor = anchor_of(case)?;
    let (evidence, violated) = app.with_tracker(project, |t| {
        let obs = t.observation();
        Ok((obs.current_evidence(case.id)?, obs.violated_guardrails(case.id)?))
    })?;
    let outcome =
        evaluate::evaluate(&EvalInput { policy: case.policy, contract, anchor, now, trigger, evidence: &evidence, violated: &violated });
    let actor = scheduler_actor();
    // The request this pass read is the one it consumes; a request that arrived since stays (§6.4).
    let consumed = case.evaluation_requested;
    let concluded = app.with_tracker(project, |t| {
        t.observation().finish_evaluation(&actor, case.id, &outcome, trigger, case.contract_version, consumed, now)
    })?;
    if let Some(done) = concluded.filter(|c| c.status == CaseStatus::Concluded)
        && let Err(e) = service::notify_concluded(app, project, &done)
    {
        eprintln!("genie observation: cannot announce the conclusion of {}: {e}", done.key);
    }
    Ok(())
}

/// Tells the project's admins that a case's collection failed `FAILURE_ALERT` passes in a row
/// (§11.3 step 6). `streak` names the streak: the dedupe key holds one notice per streak and arming.
fn notify_failures(app: &App, project: &str, case: &ObservationCase, last_error: &str, streak: &str) {
    let result = (|| -> AppResult<()> {
        let users = notify::resolve(app, project, &["project.admins".to_string()], &json!({}))?;
        let msg = Message {
            kind: FAILURE_KIND.into(),
            title: format!("{}: сбор данных не удался {FAILURE_ALERT} раза подряд", case.key),
            body: last_error.to_string(),
            project: Some(project.into()),
            task: case.tasks.first().cloned(),
            link: Some(format!("/observation/{}", case.id)),
            ..Default::default()
        };
        let armed = case.armed_at.as_deref().unwrap_or_default();
        let dedupe = format!("observation-failures:{project}:{}:{armed}:{streak}", case.id);
        notify::send(app, &users, &msg, Some(&dedupe))?;
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("genie observation: cannot notify about the failures of {}: {e}", case.key);
    }
}

/// The locked retry map of `App::observation_retry`: per-case retry times of failed passes, keyed by
/// project and case ID. Kept in memory: a restart tries every due case at once.
type RetryGuard<'a> = MutexGuard<'a, HashMap<(String, i64), DateTime<Utc>>>;

fn retries(app: &App) -> AppResult<RetryGuard<'_>> {
    app.observation_retry.lock().map_err(|_| AppError::Internal("observation retry lock poisoned".into()))
}

fn retry_until(app: &App, project: &str, id: i64) -> AppResult<Option<DateTime<Utc>>> {
    Ok(retries(app)?.get(&(project.to_string(), id)).copied())
}

fn set_retry(app: &App, project: &str, id: i64, until: DateTime<Utc>) -> AppResult<()> {
    retries(app)?.insert((project.to_string(), id), until);
    Ok(())
}

fn clear_retry(app: &App, project: &str, id: i64) -> AppResult<()> {
    retries(app)?.remove(&(project.to_string(), id));
    Ok(())
}

/// When a failed pass is tried again: `RETRY_AFTER` after `now`, or at the end of time if the clock
/// cannot hold it.
fn retry_at(now: DateTime<Utc>) -> DateTime<Utc> {
    now.checked_add_signed(RETRY_AFTER).unwrap_or(DateTime::<Utc>::MAX_UTC)
}

/// `at` plus `by`; an out-of-range result fails the case, never the process (§11.3).
fn add_to(at: DateTime<Utc>, by: Duration, key: &str) -> AppResult<DateTime<Utc>> {
    at.checked_add_signed(by).ok_or_else(|| out_of_range(key))
}

/// `at` minus `by`; an out-of-range result fails the case, never the process (§11.3).
fn sub_from(at: DateTime<Utc>, by: Duration, key: &str) -> AppResult<DateTime<Utc>> {
    at.checked_sub_signed(by).ok_or_else(|| out_of_range(key))
}

fn out_of_range(key: &str) -> AppError {
    AppError::Genie(GenieError::invalid(format!("{key}: a window is out of range")))
}

fn is_due(at: Option<&str>, now: DateTime<Utc>) -> AppResult<bool> {
    match at {
        Some(text) => Ok(parse_ts(text)? <= now),
        None => Ok(false),
    }
}

fn anchor_of(case: &ObservationCase) -> AppResult<DateTime<Utc>> {
    let text = case
        .observation_started_at
        .as_deref()
        .ok_or_else(|| AppError::Genie(GenieError::invalid(format!("{} has no observation start", case.key))))?;
    Ok(parse_ts(text)?)
}

fn scheduler_actor() -> Actor {
    Actor::new(SCHEDULER, Role::Orchestrator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_that_leaves_the_calendar_is_an_error_of_its_case() {
        let at = DateTime::<Utc>::MAX_UTC;
        let error = add_to(at, Duration::seconds(1), "case-1").unwrap_err().to_string();
        assert_eq!(error, "case-1: a window is out of range");
        let error = sub_from(DateTime::<Utc>::MIN_UTC, Duration::seconds(1), "case-1").unwrap_err().to_string();
        assert_eq!(error, "case-1: a window is out of range");
        assert_eq!(add_to(at, Duration::zero(), "case-1").unwrap(), at);
    }

    #[test]
    fn a_failed_pass_is_retried_after_the_retry_interval_or_never_when_the_clock_cannot_hold_it() {
        let now = DateTime::<Utc>::MIN_UTC;
        assert_eq!(retry_at(now), now + RETRY_AFTER);
        assert_eq!(retry_at(DateTime::<Utc>::MAX_UTC), DateTime::<Utc>::MAX_UTC);
    }
}
