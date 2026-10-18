//! Application service of the observation plane: authorization, validation and the shared rules of
//! every entrance (§14). The HTTP routes, the command line and MCP call these functions, and the
//! scheduler uses the follow-up and notification helpers. Each function checks its permission
//! first, then runs the core inside `App::with_tracker` or `App::with_server`. No database lock is
//! held while a source is measured (§11.3).

use std::fmt;

use chrono::{DateTime, Duration, Utc};
use genie_core::automation::parse_duration;
use genie_core::observation::contract::{Contract, CriterionRef};
use genie_core::observation::sources::NewSource;
use genie_core::observation::store::{CaseFilter, CasePatch, CollectionInput, EvidenceFilter, NewCase, NewDeployment, NewEvidence};
use genie_core::observation::{
    CaseResult, CaseStatus, ContractVersion, ContractVersionInfo, CriterionKind, Deployment, Evaluation, Evidence, EvidencePhase, Followup,
    FollowupMode, Incident, IncidentStatus, ObservationCase, ObservationPolicy, SecretHint, SourceAuth, SourceConfig, SourceKind, StartOn,
    TrustLevel, case_key, parse_ts, ts,
};
use genie_core::team::ORCHESTRATOR;
use genie_core::{Actor, GenieError, Role, TaskType, Tracker};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::management::project_manager;
use crate::notify;
use crate::observation::adapters::{self, Span};
use crate::state::{App, AppError, AppResult, block_on};
use crate::tasks::{self, Caller, CreateBody, Kind};

/// The refusal of anyone who is neither a person nor the project manager (§14).
const MANAGE_DENIED: &str = "only people and the project manager manage observation";
/// The refusal of anyone who is not a person, for sources and their secrets (§14).
const SOURCES_DENIED: &str = "only people manage observation sources and their secrets";
/// The actor of the observation plane itself: automatic follow-ups and mail to the project manager.
const OBSERVATION_ACTOR: &str = "observation";
/// The label every follow-up task carries (§10.2).
const FOLLOWUP_LABEL: &str = "observation";
/// Longest default title of a follow-up task (§10.2).
const FOLLOWUP_TITLE_CHARS: usize = 120;
/// The source of records that a person or the project manager reports (§7.1).
const MANUAL_SOURCE: &str = "manual";
/// Evaluations and evidence records shown by the case view (§15).
const VIEW_EVALUATIONS: i64 = 50;
const VIEW_EVIDENCE: i64 = 100;
/// The window of a source test when none is given, and the longest one allowed. A test reads at
/// most 90 days, the longest window a contract may use (§5.3).
const TEST_WINDOW: &str = "1h";
const TEST_WINDOW_MAX_DAYS: i64 = 90;
/// Longer numbers are refused before `parse_duration`, because chrono overflows on them.
const MAX_WINDOW_DIGITS: usize = 9;

// --- requests and views (§15) -----------------------------------------------------------------

/// `POST /observation/cases`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct CreateCaseBody {
    pub title: String,
    pub policy: ObservationPolicy,
    pub tasks: Vec<String>,
    pub contract: Option<Contract>,
}

/// `PATCH /observation/cases/{id}`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct UpdateCaseBody {
    pub title: Option<String>,
    pub policy: Option<ObservationPolicy>,
    pub add_tasks: Option<Vec<String>>,
    pub remove_tasks: Option<Vec<String>>,
}

/// `PUT /observation/cases/{id}/contract`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct ContractBody {
    pub contract: Contract,
    /// Required for a revision after observation started.
    pub reason: Option<String>,
}

/// `POST /observation/cases/{id}/evaluate`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct EvaluateBody {
    /// A final evaluation, which only a person may request (§6.4).
    #[serde(rename = "final")]
    pub final_: Option<bool>,
}

/// `POST /observation/cases/{id}/cancel`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct ReasonBody {
    pub reason: String,
}

/// `POST /observation/cases/{id}/deployments`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct DeploymentBody {
    /// A commit SHA, tag or release name (1 to 200 characters).
    #[serde(rename = "ref")]
    pub reference: String,
    pub environment: Option<String>,
    /// RFC 3339; defaults to now and may be at most 30 days old (§6.5).
    pub at: Option<String>,
}

/// `POST /observation/cases/{id}/evidence` (§7.5).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct EvidenceBody {
    pub criterion: String,
    /// `observed` when absent.
    pub phase: Option<EvidencePhase>,
    pub value: f64,
    /// Overrides the baseline of the latest usable baseline record of the criterion.
    pub baseline: Option<f64>,
    pub samples: Option<i64>,
    /// RFC 3339; defaults to the anchor of observation.
    pub window_start: Option<String>,
    /// RFC 3339; defaults to now.
    pub window_end: Option<String>,
    pub note: Option<String>,
}

/// `POST /observation/cases/{id}/followup`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct FollowupBody {
    /// The incident of this case the follow-up answers.
    pub incident: Option<i64>,
    pub title: Option<String>,
    pub description: Option<String>,
}

/// `POST /observation/incidents/{id}/resolve` and `…/dismiss`. A dismissal needs its note.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct NoteBody {
    pub note: Option<String>,
}

/// `POST /observation/sources` and `PUT /observation/sources/{name}` (§13.2).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct SourceBody {
    pub name: String,
    pub kind: SourceKind,
    pub url: String,
    pub auth: Option<SourceAuth>,
    pub organization: Option<String>,
    pub sentry_project: Option<String>,
    pub environment: Option<String>,
    pub field: Option<String>,
    pub dataset: Option<String>,
    pub timeout_secs: Option<u64>,
}

impl From<SourceBody> for NewSource {
    fn from(body: SourceBody) -> NewSource {
        NewSource {
            name: body.name,
            kind: body.kind,
            url: body.url,
            auth: body.auth.unwrap_or_default(),
            organization: body.organization,
            sentry_project: body.sentry_project,
            environment: body.environment,
            field: body.field,
            dataset: body.dataset,
            timeout_secs: body.timeout_secs,
        }
    }
}

/// `PUT /observation/sources/{name}/secret`. A blank value removes the secret.
#[derive(Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct SecretBody {
    pub value: String,
}

/// The value is never printed: a secret must not reach a log or an error message.
impl fmt::Debug for SecretBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretBody").field("value", &"<redacted>").finish()
    }
}

/// `POST /observation/sources/{name}/test`.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct SourceTestBody {
    pub query: String,
    /// A duration such as `30m`, `1h` or `2d`; `1h` when absent.
    pub window: Option<String>,
}

/// A case with everything its page shows (§15). Long lists are newest first.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct CaseView {
    pub case: ObservationCase,
    /// The current contract version, when one exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<ContractVersion>,
    /// Every version, oldest first, without bodies.
    pub contracts: Vec<ContractVersionInfo>,
    /// Why the case cannot be armed; empty when it can (§6.2).
    pub arm_problems: Vec<String>,
    pub deployments: Vec<Deployment>,
    /// The newest 50, newest first.
    pub evaluations: Vec<Evaluation>,
    pub incidents: Vec<Incident>,
    pub followups: Vec<Followup>,
    /// The newest 100, newest first.
    pub evidence: Vec<Evidence>,
}

/// A source with its secret as a hint only (§15).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct SourceView {
    #[serde(flatten)]
    pub source: SourceConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<SecretHint>,
}

/// The answer of a source test: the value the source gave for the query (§15).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export, optional_fields)]
pub struct SourceTestView {
    pub value: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples: Option<u64>,
    /// The query sent, after placeholder substitution.
    pub query: String,
    #[ts(type = "unknown")]
    pub provenance: Value,
}

// --- permissions and error mapping ------------------------------------------------------------

/// People and the project manager manage observation (§14). Team members, task managers and jobs
/// may not.
pub fn authorize_manage(actor: &Actor) -> AppResult<()> {
    project_manager(actor).map_err(|_| AppError::Forbidden(MANAGE_DENIED.into()))
}

/// Sources and their secrets belong to people (§14). The HTTP layer adds the project's admin check.
fn authorize_sources(actor: &Actor) -> AppResult<()> {
    if actor.role == Role::Human { Ok(()) } else { Err(AppError::Forbidden(SOURCES_DENIED.into())) }
}

/// Runs `f` on the project's tracker. A refusal of the domain (`Denied`) is a 403 for the caller,
/// not the 422 that other domain errors map to (§14, §15).
fn tracker<T>(app: &App, project: &str, f: impl FnOnce(&Tracker) -> Result<T, GenieError>) -> AppResult<T> {
    app.with_tracker(project, f).map_err(|e| match e {
        AppError::Genie(GenieError::Denied(message)) => AppError::Forbidden(message),
        other => other,
    })
}

// --- cases ------------------------------------------------------------------------------------

/// The cases of the project matching `filter`, newest first.
pub fn list(app: &App, project: &str, filter: &CaseFilter) -> AppResult<Vec<ObservationCase>> {
    tracker(app, project, |t| t.observation().list(filter))
}

/// The cases linked to a task, newest first.
pub fn for_task(app: &App, project: &str, task: &str) -> AppResult<Vec<ObservationCase>> {
    tracker(app, project, |t| t.observation().for_task(task))
}

/// One case with its page: contract history, arm problems, deployments, evaluations, incidents,
/// follow-ups and evidence (§15).
pub fn show(app: &App, project: &str, id: i64) -> AppResult<CaseView> {
    view(app, project, id)
}

/// The evidence records of a case, filtered and paged in store order: oldest first unless
/// `newest_first` is set (§7, §15).
pub fn evidence(app: &App, project: &str, id: i64, filter: &EvidenceFilter) -> AppResult<Vec<Evidence>> {
    tracker(app, project, |t| t.observation().evidence(id, filter))
}

fn view(app: &App, project: &str, id: i64) -> AppResult<CaseView> {
    let (case, contract, contracts, deployments, evaluations, incidents, followups, evidence) = tracker(app, project, |t| {
        let obs = t.observation();
        Ok((
            obs.get(id)?,
            obs.contract(id, None)?,
            obs.contracts(id)?,
            obs.deployments(id)?,
            obs.evaluations(id, Some(VIEW_EVALUATIONS))?,
            obs.case_incidents(id)?,
            obs.followups(id)?,
            obs.evidence(id, &EvidenceFilter { limit: Some(VIEW_EVIDENCE), newest_first: true, ..Default::default() })?,
        ))
    })?;
    let arm_problems = arm_problems(app, project, &case)?;
    Ok(CaseView { case, contract, contracts, arm_problems, deployments, evaluations, incidents, followups, evidence })
}

/// The reasons a case cannot be armed, each an English sentence; empty when it can (§6.2). The
/// same list the case view returns as `armProblems`.
pub fn arm_problems(app: &App, project: &str, case: &ObservationCase) -> AppResult<Vec<String>> {
    let contract = tracker(app, project, |t| t.observation().contract(case.id, None))?;
    let mut problems = Vec::new();
    match &contract {
        Some(version) => {
            let invalid = version.contract.problems(case.policy);
            if !invalid.is_empty() {
                problems.push(format!("contract is invalid: {}", invalid.join("; ")));
            }
        }
        None => problems.push("no contract yet".into()),
    }
    if case.tasks.is_empty() {
        problems.push("link at least one task".into());
    }
    if let Some(version) = &contract {
        for name in version.contract.sources() {
            match app.with_server(|db| db.observation_source(project, &name))? {
                None => problems.push(format!("unknown source {name}")),
                Some(source) if matches!(source.auth, SourceAuth::Bearer | SourceAuth::Basic) => {
                    // A stored secret that cannot be read back (the server's key changed) is no secret.
                    if app.with_server(|db| db.observation_secret(project, &name))?.is_none() {
                        problems.push(format!("source {name} needs a secret"));
                    }
                }
                Some(_) => {}
            }
        }
    }
    Ok(problems)
}

/// Creates a proposed case, linked to its tasks, with the contract as version 1 when given.
pub fn create(app: &App, project: &str, actor: &Actor, body: CreateCaseBody) -> AppResult<CaseView> {
    authorize_manage(actor)?;
    let input = NewCase { title: body.title, policy: body.policy, tasks: body.tasks, contract: body.contract };
    let case = tracker(app, project, |t| t.observation().create(actor, input))?;
    view(app, project, case.id)
}

/// Changes the title, the policy or the linked tasks, as the case's status allows (§6.2).
pub fn update(app: &App, project: &str, actor: &Actor, id: i64, body: UpdateCaseBody) -> AppResult<CaseView> {
    authorize_manage(actor)?;
    let patch = CasePatch {
        title: body.title,
        policy: body.policy,
        add_tasks: body.add_tasks.unwrap_or_default(),
        remove_tasks: body.remove_tasks.unwrap_or_default(),
    };
    tracker(app, project, |t| t.observation().update(actor, id, patch))?;
    view(app, project, id)
}

/// Sets a new contract version. After observation started only a person may, with a reason (§6.2, D8).
pub fn set_contract(app: &App, project: &str, actor: &Actor, id: i64, body: ContractBody) -> AppResult<CaseView> {
    authorize_manage(actor)?;
    let reason = body.reason.unwrap_or_default();
    tracker(app, project, |t| t.observation().set_contract(actor, id, &body.contract, &reason))?;
    view(app, project, id)
}

/// `proposed` → `waiting`, when no arm problem is left (§6.2).
pub fn arm(app: &App, project: &str, actor: &Actor, id: i64, now: DateTime<Utc>) -> AppResult<CaseView> {
    authorize_manage(actor)?;
    let case = tracker(app, project, |t| t.observation().get(id))?;
    // Only a proposed case can be armed: any other status gets the core's own refusal below.
    if case.status == CaseStatus::Proposed {
        let problems = arm_problems(app, project, &case)?;
        if !problems.is_empty() {
            return Err(AppError::Conflict(problems.join("; ")));
        }
    }
    tracker(app, project, |t| t.observation().arm(actor, id, now))?;
    // OBS-09: wake the observation scheduler
    view(app, project, id)
}

/// `waiting` → `observing`, anchored at `now` (§6.2).
pub fn start(app: &App, project: &str, actor: &Actor, id: i64, now: DateTime<Utc>) -> AppResult<CaseView> {
    authorize_manage(actor)?;
    tracker(app, project, |t| t.observation().start(actor, id, now, StartOn::Manual))?;
    // OBS-09: wake the observation scheduler
    view(app, project, id)
}

/// Asks the scheduler to evaluate an observing case; `final` only for a person (§6.4).
pub fn evaluate(app: &App, project: &str, actor: &Actor, id: i64, body: EvaluateBody) -> AppResult<CaseView> {
    authorize_manage(actor)?;
    let final_ = body.final_.unwrap_or(false);
    tracker(app, project, |t| t.observation().request_evaluation(actor, id, final_))?;
    // OBS-09: wake the observation scheduler
    view(app, project, id)
}

/// Cancels an open case with a reason (§6.2).
pub fn cancel(app: &App, project: &str, actor: &Actor, id: i64, body: ReasonBody, now: DateTime<Utc>) -> AppResult<CaseView> {
    authorize_manage(actor)?;
    tracker(app, project, |t| t.observation().cancel(actor, id, &body.reason, now))?;
    view(app, project, id)
}

/// Records a deployment marker; in `waiting` with `start = deployment` it starts the case (§6.5).
pub fn deploy(app: &App, project: &str, actor: &Actor, id: i64, body: DeploymentBody, now: DateTime<Utc>) -> AppResult<CaseView> {
    authorize_manage(actor)?;
    let at = match body.at.as_deref() {
        Some(text) => parse_ts(text)?,
        None => now,
    };
    let input = NewDeployment { reference: body.reference, environment: body.environment.unwrap_or_default(), at };
    tracker(app, project, |t| t.observation().record_deployment(actor, id, input, now))?;
    // OBS-09: wake the observation scheduler
    view(app, project, id)
}

/// Stores a manual evidence record (§7.5). A person's record is `reported`, the project manager's
/// `agent`. A reported violation of a guardrail opens its incident at once (§8, §10.1).
pub fn report_evidence(app: &App, project: &str, actor: &Actor, id: i64, body: EvidenceBody, now: DateTime<Utc>) -> AppResult<CaseView> {
    authorize_manage(actor)?;
    let case = tracker(app, project, |t| t.observation().get(id))?;
    if !matches!(case.status, CaseStatus::Observing | CaseStatus::Evaluating) {
        return Err(AppError::Conflict(format!(
            "{} is {}; evidence is reported only while it is observing or evaluating",
            case.key, case.status
        )));
    }
    let EvidenceBody { criterion, phase, value, baseline: given_baseline, samples, window_start, window_end, note } = body;
    let anchor = match case.observation_started_at.as_deref() {
        Some(text) => parse_ts(text)?,
        None => now,
    };
    let window_start = match window_start.as_deref() {
        Some(text) => parse_ts(text)?,
        None => anchor,
    };
    let window_end = match window_end.as_deref() {
        Some(text) => parse_ts(text)?,
        None => now,
    };
    if window_start >= window_end {
        return Err(AppError::Genie(GenieError::invalid(format!(
            "windowStart must be before windowEnd; got {} and {}",
            ts(window_start),
            ts(window_end)
        ))));
    }
    let phase = phase.unwrap_or(EvidencePhase::Observed);
    let trust = if actor.role == Role::Human { TrustLevel::Reported } else { TrustLevel::Agent };
    let (kind, baseline) = tracker(app, project, |t| {
        let obs = t.observation();
        let contract = obs.contract(id, None)?.map(|version| version.contract);
        let kind = match contract.as_ref().and_then(|c| c.criterion(&criterion)) {
            Some(CriterionRef::Hypothesis(_)) => CriterionKind::Hypothesis,
            Some(CriterionRef::Guardrail(_)) => CriterionKind::Guardrail,
            None => {
                return Err(GenieError::invalid(format!("criterion {criterion} is not in the current contract of {}", case.key)));
            }
        };
        let baseline = match (given_baseline, phase) {
            (Some(value), _) => Some(value),
            (None, EvidencePhase::Observed) => obs.latest_baseline(id, &criterion)?.and_then(|record| record.value),
            (None, EvidencePhase::Baseline) => None,
        };
        Ok((kind, baseline))
    })?;
    let record = NewEvidence {
        criterion,
        kind,
        phase,
        source: MANUAL_SOURCE.into(),
        query: String::new(),
        window_start,
        window_end,
        value: Some(value),
        baseline,
        samples,
        provenance: json!({ "adapter": "manual", "reportedBy": actor.name }),
        trust,
        error: None,
        note: note.unwrap_or_default(),
    };
    let input = CollectionInput {
        contract_version: case.contract_version,
        pass: false,
        failed: None,
        next_guardrail_at: None,
        now,
        records: vec![record],
    };
    let applied = tracker(app, project, |t| t.observation().apply_collection(actor, id, input))?;
    let Some(applied) = applied else {
        return Err(AppError::Conflict(format!("{} changed while the evidence was stored; report it again", case.key)));
    };
    // OBS-09: wake the observation scheduler
    announce_incidents(app, project, id, &applied.new_incidents);
    view(app, project, id)
}

/// The side effects of incidents opened by a committed write: notifications and automatic
/// follow-ups (§10.1, §10.2). The write has happened, so a failure here is logged and the caller
/// still succeeds.
fn announce_incidents(app: &App, project: &str, id: i64, incidents: &[Incident]) {
    if incidents.is_empty() {
        return;
    }
    let case = match tracker(app, project, |t| t.observation().get(id)) {
        Ok(case) => case,
        Err(e) => {
            eprintln!("genie observation: cannot read {} to announce its incidents: {e}", case_key(id));
            return;
        }
    };
    for incident in incidents {
        if let Err(e) = notify_incident(app, project, incident, &case) {
            eprintln!("genie observation: cannot notify about {} of {}: {e}", incident.key, case.key);
        }
    }
    if let Err(e) = open_followups_for(app, project, &case, incidents) {
        eprintln!("genie observation: cannot file automatic follow-ups for {}: {e}", case.key);
    }
}

// --- incidents and follow-ups -----------------------------------------------------------------

/// The incidents of every case of the project, newest first, optionally of one status (§15).
pub fn incidents(app: &App, project: &str, status: Option<IncidentStatus>) -> AppResult<Vec<Incident>> {
    tracker(app, project, |t| t.observation().incidents(status))
}

/// `open` → `resolved` (§10.1).
pub fn resolve_incident(app: &App, project: &str, actor: &Actor, id: i64, body: NoteBody) -> AppResult<Incident> {
    authorize_manage(actor)?;
    let note = body.note.unwrap_or_default();
    tracker(app, project, |t| t.observation().resolve_incident(actor, id, &note))
}

/// `open` or `resolved` → `dismissed`, as a false positive. Only a person may (§10.1).
pub fn dismiss_incident(app: &App, project: &str, actor: &Actor, id: i64, body: NoteBody) -> AppResult<Incident> {
    authorize_manage(actor)?;
    let note = body.note.unwrap_or_default();
    tracker(app, project, |t| t.observation().dismiss_incident(actor, id, &note))
}

/// Files a follow-up task for a case, or for one of its incidents (§10.2). The task is created
/// through the task services; the case records it afterwards.
pub fn followup(app: &App, project: &str, actor: &Actor, id: i64, body: FollowupBody) -> AppResult<CaseView> {
    authorize_manage(actor)?;
    let case = tracker(app, project, |t| t.observation().get(id))?;
    let incident = match body.incident {
        Some(incident_id) => {
            let found = tracker(app, project, |t| t.observation().incident(incident_id))?;
            if found.case_id != id {
                return Err(AppError::Genie(GenieError::invalid(format!("{} belongs to {}, not {}", found.key, found.case_key, case.key))));
            }
            Some(found)
        }
        None => None,
    };
    let caller = caller_for(project, actor);
    file_followup(app, &caller, &case, incident.as_ref(), body.title.as_deref(), body.description.as_deref())?;
    view(app, project, id)
}

/// The follow-up tasks of the incidents an evaluation or a report just opened, when the contract
/// says `followup: auto` (§10.2). An incident that already has a follow-up gets none.
pub fn open_followups_for(app: &App, project: &str, case: &ObservationCase, incidents: &[Incident]) -> AppResult<Vec<Followup>> {
    let auto = tracker(app, project, |t| t.observation().contract(case.id, None))?
        .is_some_and(|version| version.contract.followup == FollowupMode::Auto);
    if !auto {
        return Ok(Vec::new());
    }
    let caller = Caller::automation(project, Actor::new(OBSERVATION_ACTOR, Role::Orchestrator));
    let mut filed = Vec::new();
    for incident in incidents.iter().filter(|i| i.followups.is_empty()) {
        filed.push(file_followup(app, &caller, case, Some(incident), None, None)?);
    }
    Ok(filed)
}

/// Creates the follow-up task and records it on the case: `bug` for an incident, `task` otherwise.
fn file_followup(
    app: &App,
    caller: &Caller,
    case: &ObservationCase,
    incident: Option<&Incident>,
    title: Option<&str>,
    description: Option<&str>,
) -> AppResult<Followup> {
    let body = CreateBody {
        title: non_blank(title).unwrap_or_else(|| default_followup_title(case, incident)),
        task_type: Some(if incident.is_some() { TaskType::Bug } else { TaskType::Task }),
        description: Some(non_blank(description).unwrap_or_else(|| default_followup_description(case, incident))),
        labels: Some(vec![FOLLOWUP_LABEL.into()]),
        ..Default::default()
    };
    let task = tasks::create(app, caller, body)?;
    tasks::changed(app);
    tracker(app, &caller.project, |t| t.observation().add_followup(&caller.actor, case.id, incident.map(|i| i.id), &task.id))
}

/// The caller of the task services for a person, or for an agent acting for the project.
fn caller_for(project: &str, actor: &Actor) -> Caller {
    let kind = if actor.role == Role::Human { Kind::Person } else { Kind::Agent { team: None, job: None } };
    Caller { project: project.into(), actor: actor.clone(), kind }
}

fn non_blank(text: Option<&str>) -> Option<String> {
    text.map(str::trim).filter(|t| !t.is_empty()).map(str::to_string)
}

fn default_followup_title(case: &ObservationCase, incident: Option<&Incident>) -> String {
    let title = match incident {
        Some(incident) => format!("Regression: {} ({})", incident.guardrail, case.key),
        None => format!("Outcome not achieved: {}", case.title),
    };
    title.chars().take(FOLLOWUP_TITLE_CHARS).collect()
}

fn default_followup_description(case: &ObservationCase, incident: Option<&Incident>) -> String {
    let mut lines = vec![format!("Observation {}: {}", case.key, case.title), format!("Case: {}", case_link(case))];
    if !case.tasks.is_empty() {
        lines.push(format!("Delivered tasks: {}", case.tasks.join(", ")));
    }
    if let Some(incident) = incident {
        lines.push(format!("Incident {} (guardrail {}): {}", incident.key, incident.guardrail, incident.summary));
        if let Some(record) = incident.evidence {
            lines.push(format!("Evidence record: {record}"));
        }
    }
    lines.join("\n")
}

// --- sources and secrets (people only, §14) ----------------------------------------------------

/// The sources of the project, by name, each with its secret's hint.
pub fn sources(app: &App, project: &str, actor: &Actor) -> AppResult<Vec<SourceView>> {
    authorize_sources(actor)?;
    let configs = app.with_server(|db| db.observation_sources(project))?;
    configs.into_iter().map(|source| source_view(app, project, source)).collect()
}

/// Adds a source to the project (§13.2).
pub fn add_source(app: &App, project: &str, actor: &Actor, body: SourceBody) -> AppResult<SourceView> {
    authorize_sources(actor)?;
    let source = app.with_server(|db| db.add_observation_source(project, body.into()))?;
    source_view(app, project, source)
}

/// Replaces a source's configuration; its name cannot change (§13.2).
pub fn update_source(app: &App, project: &str, actor: &Actor, name: &str, body: SourceBody) -> AppResult<SourceView> {
    authorize_sources(actor)?;
    let source = app.with_server(|db| db.update_observation_source(project, name, body.into()))?;
    source_view(app, project, source)
}

/// Removes a source and its secret. Cases that use it then fail to collect (§13.2).
pub fn remove_source(app: &App, project: &str, actor: &Actor, name: &str) -> AppResult<()> {
    authorize_sources(actor)?;
    app.with_server(|db| db.remove_observation_source(project, name))
}

/// Sets a source's secret: its bearer token, or `user:password`. The value is never returned.
pub fn set_source_secret(app: &App, project: &str, actor: &Actor, name: &str, body: SecretBody) -> AppResult<SourceView> {
    authorize_sources(actor)?;
    app.with_server(|db| db.set_observation_secret(project, name, &body.value))?;
    source_view(app, project, source_config(app, project, name)?)
}

/// Removes a source's secret.
pub fn clear_source_secret(app: &App, project: &str, actor: &Actor, name: &str) -> AppResult<SourceView> {
    authorize_sources(actor)?;
    app.with_server(|db| db.delete_observation_secret(project, name))?;
    source_view(app, project, source_config(app, project, name)?)
}

/// Runs one query against a source over a window ending at `now`, and returns the value it gives
/// (§15). Adapter errors are refused with their message, which never contains the secret.
pub fn test_source(
    app: &App,
    project: &str,
    actor: &Actor,
    name: &str,
    body: SourceTestBody,
    now: DateTime<Utc>,
) -> AppResult<SourceTestView> {
    authorize_sources(actor)?;
    let query = body.query.trim();
    if query.is_empty() {
        return Err(AppError::Bad("a test needs a query, such as up or sum(rate(http_requests_total[$__range]))".into()));
    }
    let window = parse_window(body.window.as_deref().unwrap_or(TEST_WINDOW))?;
    let (source, secret) = app.with_server(|db| {
        let source = db.observation_source(project, name)?.ok_or_else(|| GenieError::not_found(format!("source {name} not found")))?;
        let secret = db.observation_secret(project, name)?;
        Ok((source, secret))
    })?;
    // The source may answer slowly or not at all: no database lock is held while it is asked (§11.3).
    let span = Span { start: now - window, end: now };
    // A source that refuses the query or cannot answer it is invalid input for the test (§15).
    let measured = block_on(adapters::measure(&source, secret.as_deref(), query, None, span))
        .map_err(|e| AppError::Genie(GenieError::invalid(e.to_string())))?;
    Ok(SourceTestView { value: measured.value, samples: measured.samples, query: measured.query, provenance: measured.provenance })
}

fn source_config(app: &App, project: &str, name: &str) -> AppResult<SourceConfig> {
    app.with_server(|db| db.observation_source(project, name))?
        .ok_or_else(|| AppError::Genie(GenieError::not_found(format!("source {name} not found"))))
}

fn source_view(app: &App, project: &str, source: SourceConfig) -> AppResult<SourceView> {
    let secret = app
        .with_server(|db| db.observation_secret_info(project, &source.name))?
        .map(|info| SecretHint { hint: info.hint, updated: info.updated });
    Ok(SourceView { source, secret })
}

/// A test window: a positive duration of at most 90 days.
fn parse_window(text: &str) -> AppResult<Duration> {
    let digits = text.trim().bytes().take_while(u8::is_ascii_digit).count();
    let window = if digits > MAX_WINDOW_DIGITS { None } else { parse_duration(text) };
    match window {
        Some(window) if window > Duration::zero() && window <= Duration::days(TEST_WINDOW_MAX_DAYS) => Ok(window),
        _ => Err(AppError::Bad(format!(
            "window {text} must be a positive duration of at most {TEST_WINDOW_MAX_DAYS} days, such as 30m, 1h or 2d"
        ))),
    }
}

// --- notifications (§18) ----------------------------------------------------------------------

/// Tells the people about a new incident (`observation.incident`): the project's admins and the
/// case's author when it is a person. The project manager gets mail (§10.1).
pub fn notify_incident(app: &App, project: &str, incident: &Incident, case: &ObservationCase) -> AppResult<()> {
    let users = notify::resolve(app, project, &notify_specs(case), &json!({}))?;
    let msg = notify::Message {
        kind: "observation.incident".into(),
        title: format!("{}: инцидент {} ({})", case.key, incident.key, incident.guardrail),
        body: incident.summary.clone(),
        project: Some(project.into()),
        task: case.tasks.first().cloned(),
        link: Some(case_link(case)),
        ..Default::default()
    };
    notify::send(app, &users, &msg, Some(&format!("observation-incident:{project}:{}", incident.id)))?;
    let text = format!(
        "Guardrail {} of {} is violated: incident {} says {}. Resolve it with `genie observe resolve {}`, or file a follow-up with `genie observe followup {} --incident {}`.",
        incident.guardrail, case.key, incident.key, incident.summary, incident.key, case.key, incident.key
    );
    mail_manager(app, project, case, &text)
}

/// Tells the people that a case concluded (`observation.concluded`) and mails the project manager
/// the summary and the follow-up command (§11.4, §18).
pub fn notify_concluded(app: &App, project: &str, case: &ObservationCase) -> AppResult<()> {
    let users = notify::resolve(app, project, &notify_specs(case), &json!({}))?;
    let msg = notify::Message {
        kind: "observation.concluded".into(),
        title: format!("{}: наблюдение завершено, {}", case.key, case.result.map_or("без результата", result_label)),
        body: case.conclusion.clone(),
        project: Some(project.into()),
        task: case.tasks.first().cloned(),
        link: Some(case_link(case)),
        ..Default::default()
    };
    notify::send(app, &users, &msg, Some(&format!("observation-concluded:{project}:{}", case.id)))?;
    let text = format!(
        "Observation {} concluded as {}: {}. Decide on follow-up tasks with `genie observe followup {}`.",
        case.key,
        case.result.map_or("without a result", CaseResult::as_str),
        case.conclusion,
        case.key
    );
    mail_manager(app, project, case, &text)
}

/// The recipients of a case's notifications: the project's admins, and the author by login. The
/// resolver skips an author who is not a person.
fn notify_specs(case: &ObservationCase) -> Vec<String> {
    vec!["project.admins".into(), case.author.clone()]
}

/// Mails the project manager through the orchestrator's mailbox (§10.1). The text is English, like
/// the manager's other mail.
fn mail_manager(app: &App, project: &str, case: &ObservationCase, text: &str) -> AppResult<()> {
    let task = case.tasks.first().map(String::as_str);
    tracker(app, project, |t| t.management().notify(ORCHESTRATOR, OBSERVATION_ACTOR, text, task))?;
    tasks::changed(app);
    Ok(())
}

fn case_link(case: &ObservationCase) -> String {
    format!("/observation/{}", case.id)
}

/// The Russian label of a result (glossary: `result`).
fn result_label(result: CaseResult) -> &'static str {
    match result {
        CaseResult::Confirmed => "Подтверждено",
        CaseResult::Negative => "Не подтверждено",
        CaseResult::Inconclusive => "Нет вывода",
    }
}
