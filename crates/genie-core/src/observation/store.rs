//! Persistence of cases, contracts, evidence, evaluations and incidents in the project tracker (§13).
//!
//! Persistence of cases, contracts, evidence, evaluations and incidents in the project tracker (§13).
//!
//! The case lifecycle up to the start of observation (§6) is first: creation, updates, contract
//! versions, arming, starting, deployments, evaluation requests and cancellation. After the start
//! come the collection of evidence (§7, §11.3), the guardrail check and incidents (§8, §10.1),
//! evaluations (§9, §11.4), follow-up tasks (§10.2) and the scheduler's queries (§11.3, §11.5).
//! Each change and its journal event (§12) commit in one transaction (`Tracker::tx`).
//!
//! The operations live on [`Observation`], reached through [`Tracker::observation`]. Writes that
//! take no `now` parameter (create, update, set_contract, start, request_evaluation, open_incident,
//! resolve and dismiss, add_followup) stamp their times with the current time; every other
//! time-dependent rule takes `now` from the caller.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use rusqlite::{OptionalExtension, Row, params};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::error::{GenieError, Result};
use crate::events;
use crate::model::{Actor, Role};
use crate::observation::contract::{Contract, CriterionRef, Resolved};
use crate::observation::evaluate::{self, EvalOutcome};
use crate::observation::{
    CaseResult, CaseStatus, ContractVersion, ContractVersionInfo, CriterionKind, CriterionResult, Deployment, EvalTrigger, Evaluation,
    Evidence, EvidencePhase, Followup, Incident, IncidentStatus, ObservationCase, ObservationPolicy, StartOn, TrustLevel, Verdict,
    case_key, case_subject, incident_key, parse_ts, ts,
};
use crate::tracker::Tracker;

/// Longest title of a case (§4.3).
const MAX_TITLE_CHARS: usize = 200;
/// Longest reference of a deployment (§4.3).
const MAX_REF_CHARS: usize = 200;
/// Deployments older than this are refused (§6.5).
const MAX_DEPLOYMENT_AGE_DAYS: i64 = 30;
/// Default and maximum page of [`Observation::list`].
const DEFAULT_LIMIT: i64 = 100;
const MAX_LIMIT: i64 = 200;

/// Input of [`Observation::create`] (§6.2).
#[derive(Debug, Clone)]
pub struct NewCase {
    pub title: String,
    pub policy: ObservationPolicy,
    /// Task IDs, each of which must exist. Normalized, deduplicated and stored sorted.
    pub tasks: Vec<String>,
    /// Validated against `policy`; stored as version 1 when present.
    pub contract: Option<Contract>,
}

/// Input of [`Observation::update`]. A `None` field is left unchanged (§6.2).
#[derive(Debug, Clone, Default)]
pub struct CasePatch {
    pub title: Option<String>,
    pub policy: Option<ObservationPolicy>,
    pub add_tasks: Vec<String>,
    pub remove_tasks: Vec<String>,
}

/// Filter of [`Observation::list`]: newest first, `limit` defaults to 100 and at most 200.
#[derive(Debug, Clone, Default)]
pub struct CaseFilter {
    pub status: Option<CaseStatus>,
    pub task: Option<String>,
    pub limit: Option<i64>,
}

/// Input of [`Observation::record_deployment`] (§6.5). The entrance fills `at` when none is given.
#[derive(Debug, Clone)]
pub struct NewDeployment {
    /// A commit SHA, tag or release name (1 to 200 characters).
    pub reference: String,
    /// May be empty.
    pub environment: String,
    pub at: DateTime<Utc>,
}

/// One record to store with [`Observation::apply_collection`] (§7.5, §11.3 step 5). The store fills
/// `contract_version` and `at`, and computes both deltas from `value` and `baseline` (§4.3).
#[derive(Debug, Clone)]
pub struct NewEvidence {
    /// A hypothesis or guardrail ID of the current contract.
    pub criterion: String,
    pub kind: CriterionKind,
    pub phase: EvidencePhase,
    /// A source name or `manual`.
    pub source: String,
    /// The query sent, after placeholder substitution; empty for manual records.
    pub query: String,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    /// Finite. `None` only when `error` is set.
    pub value: Option<f64>,
    /// The value of the latest usable baseline record, copied by the caller (§7.2). Finite.
    pub baseline: Option<f64>,
    /// Sample count when known; zero or more.
    pub samples: Option<i64>,
    pub provenance: Value,
    pub trust: TrustLevel,
    /// Collection failure. A record with an error has no value.
    pub error: Option<String>,
    pub note: String,
}

/// Input of [`Observation::apply_collection`]: the records of one scheduler pass (§11.3 step 5).
#[derive(Debug, Clone)]
pub struct CollectionInput {
    pub records: Vec<NewEvidence>,
    /// The pass time: the `at` of the records and the `updated` stamp of the case.
    pub now: DateTime<Utc>,
    /// The next guardrail check, when the pass schedules one. `None` leaves the schedule unchanged.
    pub next_guardrail_at: Option<DateTime<Utc>>,
    /// The first collection error of the pass, when a measurement failed. `None` means every
    /// measurement succeeded and resets the failure count.
    pub failed: Option<String>,
}

/// What [`Observation::apply_collection`] stored.
#[derive(Debug, Clone)]
pub struct Applied {
    /// The stored records, in input order.
    pub evidence: Vec<Evidence>,
    /// The incidents this call opened. A violation of an already open incident is not one of them.
    pub new_incidents: Vec<Incident>,
    /// Consecutive passes with a collection error, after this call.
    pub failures: i64,
}

/// Filter of [`Observation::evidence`]. Records are listed by ID, oldest first unless
/// `newest_first`. `after` is an exclusive cursor: the page holds the records that come after that
/// ID in the listing order. `limit` defaults to 100 and is at most 200.
#[derive(Debug, Clone, Default)]
pub struct EvidenceFilter {
    pub after: Option<i64>,
    pub limit: Option<i64>,
    pub criterion: Option<String>,
    pub newest_first: bool,
}

/// The observation operations of one tracker (see the module documentation).
pub struct Observation<'a>(&'a Tracker);

impl Tracker {
    pub fn observation(&self) -> Observation<'_> {
        Observation(self)
    }
}

impl Observation<'_> {
    /// Creates a `proposed` case linked to `input.tasks`, with the contract as version 1 when given.
    pub fn create(&self, actor: &Actor, input: NewCase) -> Result<ObservationCase> {
        let title = clean_title(&input.title)?;
        if let Some(contract) = &input.contract {
            contract.validate(input.policy)?;
        }
        self.0.tx(|| {
            let tasks = self.resolve_tasks(&input.tasks)?;
            let stamp = clock();
            let version = i64::from(input.contract.is_some());
            self.0.conn().execute(
                "INSERT INTO observation_cases(title, policy, contract_version, author, created, updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![title, input.policy, version, actor.name, stamp],
            )?;
            let id = self.0.conn().last_insert_rowid();
            for task in &tasks {
                self.link(id, task)?;
            }
            if let Some(contract) = &input.contract {
                self.insert_contract(id, 1, contract, &actor.name, "", &stamp)?;
            }
            self.emit(
                actor,
                id,
                events::OUTCOME_CREATED,
                &[("title", json!(title)), ("policy", json!(input.policy)), ("tasks", json!(tasks))],
            )?;
            self.get(id)
        })
    }

    /// One case with its linked tasks. An unknown ID is `observation O-<id> not found`.
    pub fn get(&self, id: i64) -> Result<ObservationCase> {
        let case = self.0.conn().query_row("SELECT * FROM observation_cases WHERE id = ?1", [id], case_row).optional()?;
        let mut case = case.ok_or_else(|| not_found(id))?;
        case.tasks = self.task_links(id)?;
        Ok(case)
    }

    /// Cases matching `filter`, newest first.
    pub fn list(&self, filter: &CaseFilter) -> Result<Vec<ObservationCase>> {
        let limit = page_limit(filter.limit)?;
        let task = filter.task.as_deref().map(|t| self.0.normalize_id(t)).transpose()?;
        let status = filter.status.map(CaseStatus::as_str);
        let mut stmt = self.0.conn().prepare_cached(
            "SELECT * FROM observation_cases c
             WHERE (?1 IS NULL OR c.status = ?1)
               AND (?2 IS NULL OR EXISTS (SELECT 1 FROM observation_case_tasks l WHERE l.case_id = c.id AND l.task = ?2))
             ORDER BY c.id DESC LIMIT ?3",
        )?;
        let cases = stmt.query_map(params![status, task, limit], case_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        self.with_tasks(cases)
    }

    /// Every case linked to `task`, newest first. An unknown task has no cases.
    pub fn for_task(&self, task: &str) -> Result<Vec<ObservationCase>> {
        let task = self.0.normalize_id(task)?;
        let mut stmt = self.0.conn().prepare_cached(
            "SELECT c.* FROM observation_cases c
             JOIN observation_case_tasks l ON l.case_id = c.id
             WHERE l.task = ?1 ORDER BY c.id DESC",
        )?;
        let cases = stmt.query_map([task], case_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        self.with_tasks(cases)
    }

    /// Changes the title in `proposed`, `waiting` or `observing`; the policy and the tasks only in
    /// `proposed` or `waiting` (§6.2). A policy change re-validates the current contract.
    pub fn update(&self, actor: &Actor, id: i64, patch: CasePatch) -> Result<ObservationCase> {
        self.0.tx(|| {
            let case = self.get(id)?;
            let open = matches!(case.status, CaseStatus::Proposed | CaseStatus::Waiting);
            if patch.title.is_some() && !(open || case.status == CaseStatus::Observing) {
                return Err(GenieError::invalid(format!(
                    "the title can change only while the case is proposed, waiting or observing; {} is {}",
                    case.key, case.status
                )));
            }
            if (patch.policy.is_some() || !patch.add_tasks.is_empty() || !patch.remove_tasks.is_empty()) && !open {
                return Err(GenieError::invalid(format!(
                    "the policy and the tasks can change only while the case is proposed or waiting; {} is {}",
                    case.key, case.status
                )));
            }
            let conn = self.0.conn();
            let mut changes: Vec<(&str, Value)> = Vec::new();
            if let Some(raw) = &patch.title {
                let title = clean_title(raw)?;
                if title != case.title {
                    conn.execute("UPDATE observation_cases SET title = ?1 WHERE id = ?2", params![title, id])?;
                    changes.push(("title", json!(title)));
                }
            }
            if let Some(policy) = patch.policy
                && policy != case.policy
            {
                if let Some(contract) = self.current_contract(&case)? {
                    contract.validate(policy)?;
                }
                conn.execute("UPDATE observation_cases SET policy = ?1 WHERE id = ?2", params![policy, id])?;
                changes.push(("policy", json!(policy)));
            }
            let add = self.resolve_tasks(&patch.add_tasks)?;
            let remove = self.resolve_tasks(&patch.remove_tasks)?;
            for task in &add {
                self.link(id, task)?;
            }
            for task in &remove {
                conn.execute("DELETE FROM observation_case_tasks WHERE case_id = ?1 AND task = ?2", params![id, task])?;
            }
            let tasks = self.task_links(id)?;
            // A waiting case keeps at least one task: arming requires one (§6.2).
            if case.status == CaseStatus::Waiting && tasks.is_empty() {
                return Err(GenieError::invalid("a waiting case needs at least one task"));
            }
            let tasks_changed = tasks != case.tasks;
            if tasks_changed {
                changes.push(("tasks", json!(tasks)));
            }
            if changes.is_empty() {
                return Ok(case);
            }
            let stamp = clock();
            conn.execute("UPDATE observation_cases SET updated = ?1 WHERE id = ?2", params![stamp, id])?;
            self.emit(actor, id, events::OUTCOME_UPDATED, &changes)?;
            // Removing the open task of a waiting case can leave every linked task done (§6.2, §6.3).
            if tasks_changed && case.status == CaseStatus::Waiting {
                self.start_if_tasks_done(actor, id, &stamp)?;
            }
            self.get(id)
        })
    }

    /// Sets a new contract version. In `proposed` or `waiting` any manager may do it. After the
    /// observation started (`observing`, `evaluating`) only a person may, with a reason, and the
    /// older versions are kept (§6.2, D8). A concluded or cancelled case has no contract changes.
    pub fn set_contract(&self, actor: &Actor, id: i64, contract: &Contract, reason: &str) -> Result<ObservationCase> {
        self.0.tx(|| {
            let case = self.get(id)?;
            let reason = reason.trim();
            let revised = match case.status {
                CaseStatus::Proposed | CaseStatus::Waiting => false,
                CaseStatus::Observing | CaseStatus::Evaluating => true,
                CaseStatus::Concluded | CaseStatus::Cancelled => {
                    return Err(GenieError::invalid(format!("{} is {}; its contract cannot change", case.key, case.status)));
                }
            };
            if revised {
                if actor.role != Role::Human {
                    return Err(GenieError::Denied("only a person may revise the contract after observation starts".into()));
                }
                if reason.is_empty() {
                    return Err(GenieError::invalid("a contract revision after observation starts needs a reason"));
                }
            }
            contract.validate(case.policy)?;
            let stamp = clock();
            let version = case.contract_version + 1;
            // Only a revision after the start carries a reason; earlier versions have none.
            let stored_reason = if revised { reason } else { "" };
            self.insert_contract(id, version, contract, &actor.name, stored_reason, &stamp)?;
            self.0
                .conn()
                .execute("UPDATE observation_cases SET contract_version = ?1, updated = ?2 WHERE id = ?3", params![version, stamp, id])?;
            // After a revision the first evaluation falls a minimum window of the revised contract after the anchor.
            if revised && let Some(started) = case.observation_started_at.as_deref() {
                let anchor = parse_ts(started)?;
                self.0.conn().execute(
                    "UPDATE observation_cases SET next_evaluation_at = ?1 WHERE id = ?2",
                    params![ts(anchor + contract.resolved().minimum_window), id],
                )?;
            }
            self.emit(actor, id, events::OUTCOME_UPDATED, &[("contractVersion", json!(version)), ("reason", json!(stored_reason))])?;
            // A waiting case whose new contract starts on done tasks starts at once (§6.2).
            if case.status == CaseStatus::Waiting {
                self.start_if_tasks_done(actor, id, &stamp)?;
            }
            self.get(id)
        })
    }

    /// The contract of `version` (the current one when `None`). `None` when the case has no such
    /// version, for instance no contract yet.
    pub fn contract(&self, id: i64, version: Option<i64>) -> Result<Option<ContractVersion>> {
        let current = self.get(id)?.contract_version;
        let version = version.unwrap_or(current);
        let row = self
            .0
            .conn()
            .query_row(
                "SELECT content, author, reason, created FROM observation_contracts WHERE case_id = ?1 AND version = ?2",
                params![id, version],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)),
            )
            .optional()?;
        let Some((content, author, reason, created)) = row else { return Ok(None) };
        Ok(Some(ContractVersion { case_id: id, version, contract: serde_json::from_str(&content)?, author, reason, created }))
    }

    /// The history of contract versions, oldest first, without their bodies.
    pub fn contracts(&self, id: i64) -> Result<Vec<ContractVersionInfo>> {
        self.ensure_case(id)?;
        let mut stmt = self
            .0
            .conn()
            .prepare_cached("SELECT version, author, reason, created FROM observation_contracts WHERE case_id = ?1 ORDER BY version")?;
        let versions = stmt
            .query_map([id], |r| Ok(ContractVersionInfo { version: r.get(0)?, author: r.get(1)?, reason: r.get(2)?, created: r.get(3)? }))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(versions)
    }

    /// `proposed` → `waiting`. Needs a contract valid for the policy and at least one task. When
    /// the contract starts on `tasks_done` and every linked task is done, observation starts at
    /// once with anchor `now` (§6.2).
    pub fn arm(&self, actor: &Actor, id: i64, now: DateTime<Utc>) -> Result<ObservationCase> {
        self.0.tx(|| {
            let case = self.get(id)?;
            if case.status != CaseStatus::Proposed {
                return Err(GenieError::invalid(format!("only a proposed case can be armed; {} is {}", case.key, case.status)));
            }
            let contract = self.current_contract(&case)?.ok_or_else(|| GenieError::invalid("no contract yet"))?;
            contract.validate(case.policy)?;
            if case.tasks.is_empty() {
                return Err(GenieError::invalid("link at least one task"));
            }
            self.0.conn().execute(
                "UPDATE observation_cases SET status = 'waiting', armed_at = ?1, updated = ?1 WHERE id = ?2",
                params![ts(now), id],
            )?;
            self.emit(actor, id, events::OUTCOME_UPDATED, &[("status", json!(CaseStatus::Waiting))])?;
            if contract.observation.start == StartOn::TasksDone && self.all_tasks_done(id)? {
                self.begin(actor, &case, &contract, now, StartOn::TasksDone, &ts(now))?;
            }
            self.get(id)
        })
    }

    /// `waiting` → `observing`, with the anchor `anchor` (§6.2). A case that is already observing
    /// or evaluating is returned unchanged: starting it again is a no-op.
    pub fn start(&self, actor: &Actor, id: i64, anchor: DateTime<Utc>, how: StartOn) -> Result<ObservationCase> {
        self.0.tx(|| {
            let case = self.get(id)?;
            match case.status {
                CaseStatus::Observing | CaseStatus::Evaluating => return Ok(case),
                CaseStatus::Waiting => {}
                _ => return Err(GenieError::invalid(format!("only a waiting case can start; {} is {}", case.key, case.status))),
            }
            let contract = self.current_contract(&case)?.ok_or_else(|| GenieError::invalid("no contract yet"))?;
            self.begin(actor, &case, &contract, anchor, how, &clock())?;
            self.get(id)
        })
    }

    /// Called for a task that became `done`: starts every `waiting` case with `start = tasks_done`
    /// that links it and whose linked tasks are all done (§6.3). Cancelled tasks are not done.
    /// Returns the IDs of the cases started.
    pub fn on_task_done(&self, actor: &Actor, task: &str, now: DateTime<Utc>) -> Result<Vec<i64>> {
        let task = self.0.normalize_id(task)?;
        self.0.tx(|| {
            let waiting: Vec<i64> = {
                let mut stmt = self.0.conn().prepare_cached(
                    "SELECT c.id FROM observation_cases c
                     JOIN observation_case_tasks l ON l.case_id = c.id
                     WHERE l.task = ?1 AND c.status = 'waiting' ORDER BY c.id",
                )?;
                stmt.query_map([&task], |r| r.get::<_, i64>(0))?.collect::<rusqlite::Result<_>>()?
            };
            let mut started = Vec::new();
            for id in waiting {
                let case = self.get(id)?;
                let Some(contract) = self.current_contract(&case)? else { continue };
                if contract.observation.start != StartOn::TasksDone || !self.all_tasks_done(id)? {
                    continue;
                }
                self.begin(actor, &case, &contract, now, StartOn::TasksDone, &ts(now))?;
                started.push(id);
            }
            Ok(started)
        })
    }

    /// Records a deployment marker (§6.5). In `waiting` with `start = deployment` it starts the
    /// case with anchor `at`; later deployments do not move the anchor.
    pub fn record_deployment(&self, actor: &Actor, id: i64, input: NewDeployment, now: DateTime<Utc>) -> Result<Deployment> {
        let reference = input.reference.trim().to_string();
        if reference.is_empty() || reference.chars().count() > MAX_REF_CHARS {
            return Err(GenieError::invalid(format!(
                "a deployment needs a reference of 1 to {MAX_REF_CHARS} characters, such as a commit SHA or a tag"
            )));
        }
        if input.at > now {
            return Err(GenieError::invalid("a deployment cannot be in the future; record the time it completed"));
        }
        if input.at < now - Duration::days(MAX_DEPLOYMENT_AGE_DAYS) {
            return Err(GenieError::invalid(format!("a deployment must be at most {MAX_DEPLOYMENT_AGE_DAYS} days old")));
        }
        let environment = input.environment.trim().to_string();
        self.0.tx(|| {
            let case = self.get(id)?;
            if !matches!(case.status, CaseStatus::Waiting | CaseStatus::Observing | CaseStatus::Evaluating) {
                return Err(GenieError::invalid(format!(
                    "deployments are recorded while the case is waiting, observing or evaluating; {} is {}",
                    case.key, case.status
                )));
            }
            // The deployment is stamped with the caller's `now`, like every other write of this operation.
            let stamp = ts(now);
            let at = ts(input.at);
            self.0.conn().execute(
                "INSERT INTO observation_deployments(case_id, ref, environment, at, recorded_by, created)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, reference, environment, at, actor.name, stamp],
            )?;
            let deployment_id = self.0.conn().last_insert_rowid();
            self.0.conn().execute("UPDATE observation_cases SET updated = ?1 WHERE id = ?2", params![stamp, id])?;
            self.emit(
                actor,
                id,
                events::DEPLOYMENT_COMPLETED,
                &[("deployment", json!(deployment_id)), ("ref", json!(reference)), ("environment", json!(environment)), ("at", json!(at))],
            )?;
            if case.status == CaseStatus::Waiting {
                let contract = self.current_contract(&case)?.ok_or_else(|| GenieError::invalid("no contract yet"))?;
                if contract.observation.start == StartOn::Deployment {
                    self.begin(actor, &case, &contract, input.at, StartOn::Deployment, &stamp)?;
                }
            }
            Ok(Deployment { id: deployment_id, case_id: id, reference, environment, at, recorded_by: actor.name.clone(), created: stamp })
        })
    }

    /// The deployments of a case, oldest first.
    pub fn deployments(&self, id: i64) -> Result<Vec<Deployment>> {
        self.ensure_case(id)?;
        let mut stmt = self.0.conn().prepare_cached("SELECT * FROM observation_deployments WHERE case_id = ?1 ORDER BY id")?;
        let deployments = stmt.query_map([id], deployment_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(deployments)
    }

    /// Asks the scheduler to evaluate an observing case at its next pass. A `final` request is
    /// a person's and beats a `manual` one (§6.4).
    pub fn request_evaluation(&self, actor: &Actor, id: i64, final_: bool) -> Result<ObservationCase> {
        if final_ && actor.role != Role::Human {
            return Err(GenieError::Denied("only a person may request a final evaluation".into()));
        }
        self.0.tx(|| {
            let case = self.get(id)?;
            if case.status != CaseStatus::Observing {
                return Err(GenieError::invalid(format!(
                    "an evaluation can be requested only while the case is observing; {} is {}",
                    case.key, case.status
                )));
            }
            let trigger =
                if final_ || case.evaluation_requested == Some(EvalTrigger::Final) { EvalTrigger::Final } else { EvalTrigger::Manual };
            self.0.conn().execute(
                "UPDATE observation_cases SET evaluation_requested = ?1, updated = ?2 WHERE id = ?3",
                params![trigger, clock(), id],
            )?;
            self.emit(actor, id, events::OUTCOME_READY, &[("trigger", json!(trigger))])?;
            self.get(id)
        })
    }

    /// Cancels an open case with a reason (§6.2). A case that had started keeps the moment it
    /// completed; its schedule is cleared.
    pub fn cancel(&self, actor: &Actor, id: i64, reason: &str, now: DateTime<Utc>) -> Result<ObservationCase> {
        let reason = reason.trim().to_string();
        if reason.is_empty() {
            return Err(GenieError::invalid("a cancellation needs a reason"));
        }
        self.0.tx(|| {
            let case = self.get(id)?;
            if matches!(case.status, CaseStatus::Concluded | CaseStatus::Cancelled) {
                return Err(GenieError::invalid(format!("{} is {}; only an open case can be cancelled", case.key, case.status)));
            }
            let completed = case.observation_started_at.is_some().then(|| ts(now));
            self.0.conn().execute(
                "UPDATE observation_cases
                 SET status = 'cancelled', conclusion = ?1, observation_completed_at = ?2,
                     next_evaluation_at = NULL, next_guardrail_at = NULL, evaluation_requested = NULL, updated = ?3
                 WHERE id = ?4",
                params![reason, completed, ts(now), id],
            )?;
            self.emit(actor, id, events::OUTCOME_CANCELLED, &[("reason", json!(reason))])?;
            self.get(id)
        })
    }

    /// Writes the starting transition: `observing`, anchor, first guardrail check now, first
    /// evaluation after the minimum window (§6.2). The caller holds the transaction and has
    /// checked that the case is `waiting`.
    fn begin(
        &self,
        actor: &Actor,
        case: &ObservationCase,
        contract: &Contract,
        anchor: DateTime<Utc>,
        how: StartOn,
        stamp: &str,
    ) -> Result<()> {
        let Resolved { minimum_window, .. } = contract.resolved();
        let anchor_ts = ts(anchor);
        let first_evaluation = ts(anchor + minimum_window);
        self.0.conn().execute(
            "UPDATE observation_cases
             SET status = 'observing', observation_started_at = ?1, next_guardrail_at = ?1,
                 next_evaluation_at = ?2, updated = ?3
             WHERE id = ?4",
            params![anchor_ts, first_evaluation, stamp, case.id],
        )?;
        self.emit(
            actor,
            case.id,
            events::OBSERVATION_STARTED,
            &[("anchor", json!(anchor_ts)), ("start", json!(how)), ("tasks", json!(case.tasks))],
        )
    }

    fn insert_contract(&self, case: i64, version: i64, contract: &Contract, author: &str, reason: &str, created: &str) -> Result<()> {
        self.0.conn().execute(
            "INSERT INTO observation_contracts(case_id, version, content, author, reason, created)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![case, version, serde_json::to_string(contract)?, author, reason, created],
        )?;
        Ok(())
    }

    /// The contract of the case's current version; `None` before one was set.
    fn current_contract(&self, case: &ObservationCase) -> Result<Option<Contract>> {
        if case.contract_version == 0 {
            return Ok(None);
        }
        let content: String = self.0.conn().query_row(
            "SELECT content FROM observation_contracts WHERE case_id = ?1 AND version = ?2",
            params![case.id, case.contract_version],
            |r| r.get(0),
        )?;
        Ok(Some(serde_json::from_str(&content)?))
    }

    /// Starts a waiting case whose contract starts on `tasks_done` once every linked task is done,
    /// anchored at the current time. This is the arming rule (§6.2), re-checked after a task or
    /// contract change. Writes nothing when the case stays waiting.
    fn start_if_tasks_done(&self, actor: &Actor, id: i64, stamp: &str) -> Result<()> {
        let case = self.get(id)?;
        let Some(contract) = self.current_contract(&case)? else { return Ok(()) };
        if contract.observation.start == StartOn::TasksDone && self.all_tasks_done(id)? {
            self.begin(actor, &case, &contract, wall_clock(), StartOn::TasksDone, stamp)?;
        }
        Ok(())
    }

    /// Whether the case links at least one task and every linked task is `done`.
    fn all_tasks_done(&self, id: i64) -> Result<bool> {
        let (linked, open): (i64, i64) = self.0.conn().query_row(
            "SELECT COUNT(*), COALESCE(SUM(t.status <> 'done'), 0)
             FROM observation_case_tasks l JOIN tasks t ON t.id = l.task
             WHERE l.case_id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(linked > 0 && open == 0)
    }

    fn task_links(&self, case: i64) -> Result<Vec<String>> {
        let mut stmt = self.0.conn().prepare_cached("SELECT task FROM observation_case_tasks WHERE case_id = ?1 ORDER BY task")?;
        let tasks = stmt.query_map([case], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(tasks)
    }

    fn with_tasks(&self, mut cases: Vec<ObservationCase>) -> Result<Vec<ObservationCase>> {
        for case in &mut cases {
            case.tasks = self.task_links(case.id)?;
        }
        Ok(cases)
    }

    fn ensure_case(&self, id: i64) -> Result<()> {
        let found = self.0.conn().query_row("SELECT 1 FROM observation_cases WHERE id = ?1", [id], |_| Ok(())).optional()?;
        found.ok_or_else(|| not_found(id))
    }

    /// Normalizes and checks task IDs; the first unknown one is named in the error.
    fn resolve_tasks(&self, tasks: &[String]) -> Result<BTreeSet<String>> {
        let mut out = BTreeSet::new();
        for task in tasks {
            if task.trim().is_empty() {
                return Err(GenieError::invalid("a task ID is required"));
            }
            let id = self.0.normalize_id(task)?;
            if !self.0.exists(&id)? {
                return Err(GenieError::not_found(format!("task {id} not found")));
            }
            out.insert(id);
        }
        Ok(out)
    }

    fn link(&self, case: i64, task: &str) -> Result<()> {
        self.0.conn().execute("INSERT OR IGNORE INTO observation_case_tasks(case_id, task) VALUES (?1, ?2)", params![case, task])?;
        Ok(())
    }

    /// Appends one journal event for the case. Every payload carries `case` and `key` (§12).
    fn emit(&self, actor: &Actor, id: i64, kind: &str, fields: &[(&str, Value)]) -> Result<()> {
        let mut payload = serde_json::Map::new();
        payload.insert("case".into(), json!(id));
        payload.insert("key".into(), json!(case_key(id)));
        for (name, value) in fields {
            payload.insert((*name).into(), value.clone());
        }
        let subject = case_subject(id);
        self.0.append_event(kind, Some(subject.as_str()), &actor.name, actor.role.as_str(), Value::Object(payload))?;
        Ok(())
    }
}

/// Evidence, evaluations, incidents, follow-ups and the scheduler's queries (§7.5, §8, §9, §10,
/// §11.3 to §11.5). Every write checks the state it needs inside its transaction.
impl Observation<'_> {
    /// Stores one scheduler pass in one transaction (§11.3 step 5). Returns `None` without writing
    /// anything unless the case is `observing` or `evaluating`. Every record is checked before any
    /// is stored: its criterion must belong to the current contract, with the same kind. An observed
    /// usable guardrail record that crosses its limit opens an incident (§8). `failed` counts
    /// consecutive failed passes; a clean pass resets the count and the last error.
    pub fn apply_collection(&self, actor: &Actor, id: i64, input: CollectionInput) -> Result<Option<Applied>> {
        self.0.tx(|| {
            let case = self.get(id)?;
            if !is_collecting(case.status) {
                return Ok(None);
            }
            let contract = self.current_contract(&case)?.ok_or_else(|| GenieError::invalid("no contract yet"))?;
            for record in &input.records {
                check_record(&contract, id, record)?;
            }
            let mut evidence = Vec::with_capacity(input.records.len());
            let mut new_incidents = Vec::new();
            for record in input.records {
                let stored = self.insert_evidence(id, case.contract_version, record, input.now)?;
                if let Some(incident) = self.check_new_record(actor, &contract, &stored)? {
                    new_incidents.push(incident);
                }
                evidence.push(stored);
            }
            if let Some(next) = input.next_guardrail_at {
                self.0.conn().execute("UPDATE observation_cases SET next_guardrail_at = ?1 WHERE id = ?2", params![ts(next), id])?;
            }
            let failures = if input.failed.is_some() { case.failures + 1 } else { 0 };
            self.0.conn().execute(
                "UPDATE observation_cases SET failures = ?1, last_error = ?2, updated = ?3 WHERE id = ?4",
                params![failures, input.failed, ts(input.now), id],
            )?;
            if !evidence.is_empty() {
                let errors = evidence.iter().filter(|e| e.error.is_some()).count();
                self.emit(actor, id, events::EVIDENCE_COLLECTED, &[("records", json!(evidence.len())), ("errors", json!(errors))])?;
            }
            Ok(Some(Applied { evidence, new_incidents, failures }))
        })
    }

    /// The guardrail check of one stored record (§8). A usable observed guardrail record that
    /// crosses a limit opens its incident; the incident is returned only when this call opened it.
    fn check_new_record(&self, actor: &Actor, contract: &Contract, record: &Evidence) -> Result<Option<Incident>> {
        if record.kind != CriterionKind::Guardrail || record.phase != EvidencePhase::Observed || !evaluate::is_usable(record) {
            return Ok(None);
        }
        let Some(CriterionRef::Guardrail(guardrail)) = contract.criterion(&record.criterion) else {
            return Ok(None);
        };
        let Some(message) = evaluate::check_guardrail(guardrail, record) else {
            return Ok(None);
        };
        let (incident, created) = self.open_incident(actor, record.case_id, &guardrail.id, Some(record.id), &message)?;
        Ok(created.then_some(incident))
    }

    /// The records of a case, filtered and paged (§7). Records of every contract version are listed.
    pub fn evidence(&self, id: i64, filter: &EvidenceFilter) -> Result<Vec<Evidence>> {
        self.ensure_case(id)?;
        let limit = page_limit(filter.limit)?;
        // The keywords are fixed here; every value still goes through a bound parameter.
        let (order, cmp) = if filter.newest_first { ("DESC", "<") } else { ("ASC", ">") };
        let sql = format!(
            "SELECT * FROM observation_evidence
             WHERE case_id = ?1 AND (?2 IS NULL OR criterion = ?2) AND (?3 IS NULL OR id {cmp} ?3)
             ORDER BY id {order} LIMIT ?4"
        );
        let mut stmt = self.0.conn().prepare_cached(&sql)?;
        let criterion = filter.criterion.as_deref();
        let rows = stmt.query_map(params![id, criterion, filter.after, limit], evidence_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Every record of the current contract version, oldest first (§9 evaluates these).
    pub fn current_evidence(&self, id: i64) -> Result<Vec<Evidence>> {
        let version = self.get(id)?.contract_version;
        let mut stmt =
            self.0.conn().prepare_cached("SELECT * FROM observation_evidence WHERE case_id = ?1 AND contract_version = ?2 ORDER BY id")?;
        let rows = stmt.query_map(params![id, version], evidence_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The newest usable baseline record of a criterion in the current contract version (§7.2).
    /// Usable is the rule of [`evaluate::is_usable`], written in SQL here.
    pub fn latest_baseline(&self, id: i64, criterion: &str) -> Result<Option<Evidence>> {
        let version = self.get(id)?.contract_version;
        let row = self
            .0
            .conn()
            .query_row(
                "SELECT * FROM observation_evidence
                 WHERE case_id = ?1 AND contract_version = ?2 AND criterion = ?3 AND phase = 'baseline'
                   AND error IS NULL AND value IS NOT NULL AND trust IN ('measured', 'reported')
                 ORDER BY id DESC LIMIT 1",
                params![id, version, criterion],
                evidence_row,
            )
            .optional()?;
        Ok(row)
    }

    /// Moves a case to `evaluating` for one evaluation pass (§11.4 step 1). Returns `false` for any
    /// other status, so the pass skips the case. No journal event: the status is transient (§6.1).
    pub fn begin_evaluation(&self, id: i64) -> Result<bool> {
        self.0.tx(|| {
            let case = self.get(id)?;
            match case.status {
                CaseStatus::Observing => {
                    self.0.conn().execute("UPDATE observation_cases SET status = 'evaluating' WHERE id = ?1", [id])?;
                    Ok(true)
                }
                CaseStatus::Evaluating => Ok(true),
                _ => Ok(false),
            }
        })
    }

    /// Records an evaluation and applies its verdict (§11.4 step 5). Returns `None`, and writes
    /// nothing, unless the case is still `evaluating` (for instance cancelled meanwhile). The
    /// evaluation is stored and the request cleared. `insufficient` keeps the case observing and
    /// schedules the next evaluation at the earlier of one interval later and the preferred window;
    /// any other verdict concludes the case with its result, confidence and summary.
    pub fn finish_evaluation(
        &self,
        actor: &Actor,
        id: i64,
        outcome: &EvalOutcome,
        trigger: EvalTrigger,
        now: DateTime<Utc>,
    ) -> Result<Option<ObservationCase>> {
        self.0.tx(|| {
            let case = self.get(id)?;
            if case.status != CaseStatus::Evaluating {
                return Ok(None);
            }
            let contract = self.current_contract(&case)?.ok_or_else(|| GenieError::invalid("no contract yet"))?;
            let stamp = ts(now);
            let details = serde_json::to_string(&outcome.details)?;
            self.0.conn().execute(
                "INSERT INTO observation_evaluations(case_id, contract_version, at, trigger, verdict, confidence, summary, details, actor)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    case.contract_version,
                    stamp,
                    trigger,
                    outcome.verdict,
                    outcome.confidence,
                    outcome.summary,
                    details,
                    actor.name
                ],
            )?;
            let evaluation = self.0.conn().last_insert_rowid();
            match conclusion_of(outcome.verdict) {
                None => {
                    let next = next_evaluation(&case, &contract, now)?;
                    self.0.conn().execute(
                        "UPDATE observation_cases
                         SET status = 'observing', next_evaluation_at = ?1, evaluation_requested = NULL, updated = ?2
                         WHERE id = ?3",
                        params![ts(next), stamp, id],
                    )?;
                    self.emit(
                        actor,
                        id,
                        events::OUTCOME_EVALUATED,
                        &[("evaluation", json!(evaluation)), ("verdict", json!(outcome.verdict)), ("nextEvaluationAt", json!(ts(next)))],
                    )?;
                }
                Some((result, event)) => {
                    self.0.conn().execute(
                        "UPDATE observation_cases
                         SET status = 'concluded', result = ?1, confidence = ?2, conclusion = ?3, observation_completed_at = ?4,
                             next_evaluation_at = NULL, next_guardrail_at = NULL, evaluation_requested = NULL, updated = ?4
                         WHERE id = ?5",
                        params![result, outcome.confidence, outcome.summary, stamp, id],
                    )?;
                    self.emit(
                        actor,
                        id,
                        event,
                        &[
                            ("evaluation", json!(evaluation)),
                            ("confidence", json!(outcome.confidence)),
                            ("summary", json!(outcome.summary)),
                            ("tasks", json!(case.tasks)),
                        ],
                    )?;
                }
            }
            Ok(Some(self.get(id)?))
        })
    }

    /// The evaluations of a case, newest first, at most `limit` (default 100, at most 200).
    pub fn evaluations(&self, id: i64, limit: Option<i64>) -> Result<Vec<Evaluation>> {
        self.ensure_case(id)?;
        let limit = page_limit(limit)?;
        let mut stmt =
            self.0.conn().prepare_cached("SELECT * FROM observation_evaluations WHERE case_id = ?1 ORDER BY id DESC LIMIT ?2")?;
        let rows = stmt.query_map(params![id, limit], evaluation_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Opens an incident for a guardrail violation (§10.1). At most one incident is open per case
    /// and guardrail: when one is open, it is returned with `false` and nothing is written. Otherwise
    /// the incident is created with `guardrail.violated` and `incident.created`, and returned with `true`.
    pub fn open_incident(&self, actor: &Actor, id: i64, guardrail: &str, evidence: Option<i64>, summary: &str) -> Result<(Incident, bool)> {
        self.0.tx(|| {
            self.ensure_case(id)?;
            let open: Option<i64> = self
                .0
                .conn()
                .query_row(
                    "SELECT id FROM observation_incidents WHERE case_id = ?1 AND guardrail = ?2 AND status = 'open'",
                    params![id, guardrail],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(existing) = open {
                return Ok((self.incident(existing)?, false));
            }
            self.0.conn().execute(
                "INSERT INTO observation_incidents(case_id, guardrail, status, summary, evidence, created)
                 VALUES (?1, ?2, 'open', ?3, ?4, ?5)",
                params![id, guardrail, summary, evidence, clock()],
            )?;
            let incident = self.0.conn().last_insert_rowid();
            self.emit(
                actor,
                id,
                events::GUARDRAIL_VIOLATED,
                &[("guardrail", json!(guardrail)), ("evidence", json!(evidence)), ("message", json!(summary))],
            )?;
            self.emit(
                actor,
                id,
                events::INCIDENT_CREATED,
                &[("incident", json!(incident)), ("incidentKey", json!(incident_key(incident))), ("guardrail", json!(guardrail))],
            )?;
            Ok((self.incident(incident)?, true))
        })
    }

    /// One incident with its follow-up task IDs. An unknown ID is `incident I-<id> not found`.
    pub fn incident(&self, id: i64) -> Result<Incident> {
        let found = self.0.conn().query_row("SELECT * FROM observation_incidents WHERE id = ?1", [id], incident_row).optional()?;
        let mut incident = found.ok_or_else(|| GenieError::not_found(format!("incident {} not found", incident_key(id))))?;
        incident.followups = self.incident_task_links(id)?;
        Ok(incident)
    }

    /// Incidents of every case, newest first, optionally of one status.
    pub fn incidents(&self, status: Option<IncidentStatus>) -> Result<Vec<Incident>> {
        let status = status.map(IncidentStatus::as_str);
        let mut stmt =
            self.0.conn().prepare_cached("SELECT * FROM observation_incidents WHERE (?1 IS NULL OR status = ?1) ORDER BY id DESC")?;
        let incidents = stmt.query_map(params![status], incident_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        self.with_followups(incidents)
    }

    /// The incidents of one case, newest first.
    pub fn case_incidents(&self, id: i64) -> Result<Vec<Incident>> {
        self.ensure_case(id)?;
        let mut stmt = self.0.conn().prepare_cached("SELECT * FROM observation_incidents WHERE case_id = ?1 ORDER BY id DESC")?;
        let incidents = stmt.query_map([id], incident_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        self.with_followups(incidents)
    }

    /// `open` → `resolved` (§10.1). The violation still counts for evaluation. The note is optional.
    pub fn resolve_incident(&self, actor: &Actor, id: i64, note: &str) -> Result<Incident> {
        let note = note.trim().to_string();
        self.0.tx(|| {
            let incident = self.incident(id)?;
            if incident.status != IncidentStatus::Open {
                return Err(GenieError::invalid(format!("{} is {}; only an open incident can be resolved", incident.key, incident.status)));
            }
            self.0.conn().execute(
                "UPDATE observation_incidents SET status = 'resolved', closed = ?1, closed_by = ?2, note = ?3 WHERE id = ?4",
                params![clock(), actor.name, note, id],
            )?;
            self.emit(actor, incident.case_id, events::INCIDENT_RESOLVED, &[("incident", json!(id)), ("note", json!(note))])?;
            self.incident(id)
        })
    }

    /// `open` or `resolved` → `dismissed` (§10.1): the violation no longer counts, as a false
    /// positive. Only a person may dismiss, and the note says why it is a false positive.
    pub fn dismiss_incident(&self, actor: &Actor, id: i64, note: &str) -> Result<Incident> {
        if actor.role != Role::Human {
            return Err(GenieError::Denied(
                "only a person may dismiss an incident; resolve it instead, or ask a person to dismiss it".into(),
            ));
        }
        let note = note.trim().to_string();
        if note.is_empty() {
            return Err(GenieError::invalid("dismissing an incident needs a note that says why the violation is a false positive"));
        }
        self.0.tx(|| {
            let incident = self.incident(id)?;
            if incident.status == IncidentStatus::Dismissed {
                return Err(GenieError::invalid(format!(
                    "{} is dismissed; only an open or resolved incident can be dismissed",
                    incident.key
                )));
            }
            self.0.conn().execute(
                "UPDATE observation_incidents SET status = 'dismissed', closed = ?1, closed_by = ?2, note = ?3 WHERE id = ?4",
                params![clock(), actor.name, note, id],
            )?;
            self.emit(actor, incident.case_id, events::INCIDENT_DISMISSED, &[("incident", json!(id)), ("note", json!(note))])?;
            self.incident(id)
        })
    }

    /// The guardrails with an open or resolved incident of a case, sorted and without duplicates.
    /// Dismissed incidents are not counted (§9.1).
    pub fn violated_guardrails(&self, id: i64) -> Result<Vec<String>> {
        self.ensure_case(id)?;
        let mut stmt = self.0.conn().prepare_cached(
            "SELECT DISTINCT guardrail FROM observation_incidents
             WHERE case_id = ?1 AND status IN ('open', 'resolved') ORDER BY guardrail",
        )?;
        let guardrails = stmt.query_map([id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(guardrails)
    }

    /// Links a follow-up task to a case, optionally to one of its incidents (§10.2). The task must
    /// exist. Adding the same task again returns the existing follow-up and writes nothing.
    pub fn add_followup(&self, actor: &Actor, id: i64, incident: Option<i64>, task: &str) -> Result<Followup> {
        self.0.tx(|| {
            self.ensure_case(id)?;
            if let Some(incident_id) = incident {
                let found = self.incident(incident_id)?;
                if found.case_id != id {
                    return Err(GenieError::invalid(format!("{} belongs to {}, not {}", found.key, found.case_key, case_key(id))));
                }
            }
            let task = self.0.normalize_id(task)?;
            if !self.0.exists(&task)? {
                return Err(GenieError::not_found(format!("task {task} not found")));
            }
            let existing = self
                .0
                .conn()
                .query_row("SELECT * FROM observation_followups WHERE case_id = ?1 AND task = ?2", params![id, task], followup_row)
                .optional()?;
            if let Some(followup) = existing {
                return Ok(followup);
            }
            let stamp = clock();
            self.0.conn().execute(
                "INSERT INTO observation_followups(case_id, incident, task, actor, created) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, incident, task, actor.name, stamp],
            )?;
            let followup = self.0.conn().last_insert_rowid();
            self.emit(actor, id, events::FOLLOWUP_TASK_CREATED, &[("task", json!(task)), ("incident", json!(incident))])?;
            Ok(Followup { id: followup, case_id: id, incident, task, actor: actor.name.clone(), created: stamp })
        })
    }

    /// The follow-up tasks of a case, oldest first.
    pub fn followups(&self, id: i64) -> Result<Vec<Followup>> {
        self.ensure_case(id)?;
        let mut stmt = self.0.conn().prepare_cached("SELECT * FROM observation_followups WHERE case_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([id], followup_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Cases the scheduler must work on now (§11.3): observing or evaluating, with a guardrail check
    /// or an evaluation due at `now`, or with an evaluation requested. Oldest first.
    pub fn due(&self, now: DateTime<Utc>) -> Result<Vec<ObservationCase>> {
        let cases = {
            let mut stmt = self.0.conn().prepare_cached(
                "SELECT * FROM observation_cases
                 WHERE status IN ('observing', 'evaluating')
                   AND (next_guardrail_at <= ?1 OR next_evaluation_at <= ?1 OR evaluation_requested IS NOT NULL)
                 ORDER BY id",
            )?;
            stmt.query_map([ts(now)], case_row)?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        self.with_tasks(cases)
    }

    /// The earliest scheduled check or evaluation among observing and evaluating cases (§11.5), or
    /// `None` when no such case is scheduled. Requests are not counted: they are due at once.
    pub fn next_due(&self) -> Result<Option<String>> {
        let next: Option<String> = self.0.conn().query_row(
            "SELECT MIN(at) FROM (
                 SELECT next_guardrail_at AS at FROM observation_cases WHERE status IN ('observing', 'evaluating')
                 UNION ALL
                 SELECT next_evaluation_at FROM observation_cases WHERE status IN ('observing', 'evaluating')
             )",
            [],
            |r| r.get(0),
        )?;
        Ok(next)
    }

    /// Writes one evidence record of the case. The caller has checked the record (`check_record`).
    fn insert_evidence(&self, case: i64, version: i64, record: NewEvidence, now: DateTime<Utc>) -> Result<Evidence> {
        let (delta_abs, delta_rel) = match record.value {
            Some(value) => evaluate::deltas(value, record.baseline),
            None => (None, None),
        };
        let at = ts(now);
        let window_start = ts(record.window_start);
        let window_end = ts(record.window_end);
        let provenance = serde_json::to_string(&record.provenance)?;
        self.0.conn().execute(
            "INSERT INTO observation_evidence(case_id, contract_version, criterion, kind, phase, source, query, at,
                 window_start, window_end, value, baseline, delta_abs, delta_rel, samples, provenance, trust, error, note)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                case,
                version,
                record.criterion,
                record.kind,
                record.phase,
                record.source,
                record.query,
                at,
                window_start,
                window_end,
                record.value,
                record.baseline,
                delta_abs,
                delta_rel,
                record.samples,
                provenance,
                record.trust,
                record.error,
                record.note,
            ],
        )?;
        Ok(Evidence {
            id: self.0.conn().last_insert_rowid(),
            case_id: case,
            contract_version: version,
            criterion: record.criterion,
            kind: record.kind,
            phase: record.phase,
            source: record.source,
            query: record.query,
            at,
            window_start,
            window_end,
            value: record.value,
            baseline: record.baseline,
            delta_abs,
            delta_rel,
            samples: record.samples,
            provenance: record.provenance,
            trust: record.trust,
            error: record.error,
            note: record.note,
        })
    }

    /// Fills the follow-up task IDs of each incident (§4.3).
    fn with_followups(&self, mut incidents: Vec<Incident>) -> Result<Vec<Incident>> {
        for incident in &mut incidents {
            incident.followups = self.incident_task_links(incident.id)?;
        }
        Ok(incidents)
    }

    fn incident_task_links(&self, incident: i64) -> Result<Vec<String>> {
        let mut stmt = self.0.conn().prepare_cached("SELECT task FROM observation_followups WHERE incident = ?1 ORDER BY id")?;
        let tasks = stmt.query_map([incident], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(tasks)
    }
}

/// Whether the scheduler collects evidence in this status (§6.1). `evaluating` counts, so a case
/// left there by a crash is collected again.
fn is_collecting(status: CaseStatus) -> bool {
    matches!(status, CaseStatus::Observing | CaseStatus::Evaluating)
}

/// The checks of one record before anything is stored (§5.3, §7.5): its criterion belongs to the
/// current contract with the same kind, its numbers are finite, its value and error do not
/// contradict each other, and its window is ordered.
fn check_record(contract: &Contract, case: i64, record: &NewEvidence) -> Result<()> {
    let expected = match contract.criterion(&record.criterion) {
        Some(CriterionRef::Hypothesis(_)) => CriterionKind::Hypothesis,
        Some(CriterionRef::Guardrail(_)) => CriterionKind::Guardrail,
        None => {
            return Err(GenieError::invalid(format!(
                "criterion {} is not in the current contract of {}",
                record.criterion,
                case_key(case)
            )));
        }
    };
    if record.kind != expected {
        return Err(GenieError::invalid(format!("criterion {} is a {expected}, not a {}", record.criterion, record.kind)));
    }
    for (name, number) in [("value", record.value), ("baseline", record.baseline)] {
        if number.is_some_and(|v| !v.is_finite()) {
            return Err(GenieError::invalid(format!("{name} must be a finite number")));
        }
    }
    match (record.value.is_some(), record.error.is_some()) {
        (false, false) => return Err(GenieError::invalid("a record needs a value or an error")),
        (true, true) => return Err(GenieError::invalid("a record has either a value or an error, not both")),
        _ => {}
    }
    if record.samples.is_some_and(|n| n < 0) {
        return Err(GenieError::invalid("samples must be zero or more"));
    }
    if record.window_start >= record.window_end {
        return Err(GenieError::invalid("the window must start before it ends"));
    }
    Ok(())
}

/// The case result and journal event of a verdict that concludes the case (§11.4 step 5). `None`
/// for `insufficient`, which keeps the case observing.
fn conclusion_of(verdict: Verdict) -> Option<(CaseResult, &'static str)> {
    match verdict {
        Verdict::Confirmed => Some((CaseResult::Confirmed, events::OUTCOME_CONFIRMED)),
        Verdict::Negative => Some((CaseResult::Negative, events::OUTCOME_NEGATIVE)),
        Verdict::Inconclusive => Some((CaseResult::Inconclusive, events::OUTCOME_INCONCLUSIVE)),
        Verdict::Insufficient => None,
    }
}

/// The next evaluation after an `insufficient` verdict (§11.4 step 5): one evaluation interval
/// after `now`, but no later than the preferred window after the anchor.
fn next_evaluation(case: &ObservationCase, contract: &Contract, now: DateTime<Utc>) -> Result<DateTime<Utc>> {
    let resolved = contract
        .try_resolved()
        .ok_or_else(|| GenieError::invalid(format!("the contract of {} has a duration that does not parse", case.key)))?;
    let anchor = case
        .observation_started_at
        .as_deref()
        .ok_or_else(|| GenieError::invalid(format!("{} has no observation start to schedule from", case.key)))?;
    let preferred_end = parse_ts(anchor)? + resolved.preferred_window;
    Ok((now + resolved.evaluation_interval).min(preferred_end))
}

/// The page size of a listing: the default when none is given, otherwise 1 to 200.
fn page_limit(limit: Option<i64>) -> Result<i64> {
    let limit = limit.unwrap_or(DEFAULT_LIMIT);
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(GenieError::invalid(format!("limit must be between 1 and {MAX_LIMIT}")));
    }
    Ok(limit)
}

fn not_found(id: i64) -> GenieError {
    GenieError::not_found(format!("observation {} not found", case_key(id)))
}

/// The title as stored: trimmed, non-empty, at most 200 characters (§4.3).
fn clean_title(title: &str) -> Result<String> {
    let title = title.trim();
    if title.is_empty() {
        return Err(GenieError::invalid("an observation needs a title"));
    }
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(GenieError::invalid(format!("the observation title must be at most {MAX_TITLE_CHARS} characters")));
    }
    Ok(title.to_string())
}

/// The current time, for the operations with no `now` parameter. Only `wall_clock` and `clock`
/// read the clock inside the store; time-dependent rules take `now` as a parameter.
fn wall_clock() -> DateTime<Utc> {
    Utc::now()
}

/// The write stamp of an operation with no `now` parameter.
fn clock() -> String {
    ts(wall_clock())
}

/// A case row without its tasks (loaded by the caller).
fn case_row(r: &Row<'_>) -> rusqlite::Result<ObservationCase> {
    let id: i64 = r.get("id")?;
    Ok(ObservationCase {
        id,
        key: case_key(id),
        title: r.get("title")?,
        policy: r.get("policy")?,
        status: r.get("status")?,
        result: r.get("result")?,
        confidence: r.get("confidence")?,
        conclusion: r.get("conclusion")?,
        contract_version: r.get("contract_version")?,
        tasks: Vec::new(),
        author: r.get("author")?,
        created: r.get("created")?,
        updated: r.get("updated")?,
        armed_at: r.get("armed_at")?,
        observation_started_at: r.get("observation_started_at")?,
        observation_completed_at: r.get("observation_completed_at")?,
        next_evaluation_at: r.get("next_evaluation_at")?,
        next_guardrail_at: r.get("next_guardrail_at")?,
        evaluation_requested: r.get("evaluation_requested")?,
        failures: r.get("failures")?,
        last_error: r.get("last_error")?,
    })
}

fn deployment_row(r: &Row<'_>) -> rusqlite::Result<Deployment> {
    Ok(Deployment {
        id: r.get("id")?,
        case_id: r.get("case_id")?,
        reference: r.get("ref")?,
        environment: r.get("environment")?,
        at: r.get("at")?,
        recorded_by: r.get("recorded_by")?,
        created: r.get("created")?,
    })
}

fn evidence_row(r: &Row<'_>) -> rusqlite::Result<Evidence> {
    Ok(Evidence {
        id: r.get("id")?,
        case_id: r.get("case_id")?,
        contract_version: r.get("contract_version")?,
        criterion: r.get("criterion")?,
        kind: r.get("kind")?,
        phase: r.get("phase")?,
        source: r.get("source")?,
        query: r.get("query")?,
        at: r.get("at")?,
        window_start: r.get("window_start")?,
        window_end: r.get("window_end")?,
        value: r.get("value")?,
        baseline: r.get("baseline")?,
        delta_abs: r.get("delta_abs")?,
        delta_rel: r.get("delta_rel")?,
        samples: r.get("samples")?,
        provenance: json_column(r, "provenance")?,
        trust: r.get("trust")?,
        error: r.get("error")?,
        note: r.get("note")?,
    })
}

fn evaluation_row(r: &Row<'_>) -> rusqlite::Result<Evaluation> {
    Ok(Evaluation {
        id: r.get("id")?,
        case_id: r.get("case_id")?,
        contract_version: r.get("contract_version")?,
        at: r.get("at")?,
        trigger: r.get("trigger")?,
        verdict: r.get("verdict")?,
        confidence: r.get("confidence")?,
        summary: r.get("summary")?,
        details: json_column::<Vec<CriterionResult>>(r, "details")?,
        actor: r.get("actor")?,
    })
}

/// An incident row; its follow-up task IDs are filled by the caller.
fn incident_row(r: &Row<'_>) -> rusqlite::Result<Incident> {
    let id: i64 = r.get("id")?;
    let case_id: i64 = r.get("case_id")?;
    Ok(Incident {
        id,
        key: incident_key(id),
        case_id,
        case_key: case_key(case_id),
        guardrail: r.get("guardrail")?,
        status: r.get("status")?,
        summary: r.get("summary")?,
        evidence: r.get("evidence")?,
        created: r.get("created")?,
        closed: r.get("closed")?,
        closed_by: r.get("closed_by")?,
        note: r.get("note")?,
        followups: Vec::new(),
    })
}

fn followup_row(r: &Row<'_>) -> rusqlite::Result<Followup> {
    Ok(Followup {
        id: r.get("id")?,
        case_id: r.get("case_id")?,
        incident: r.get("incident")?,
        task: r.get("task")?,
        actor: r.get("actor")?,
        created: r.get("created")?,
    })
}

/// Reads a column holding JSON that this module wrote. Text that does not parse is reported as a
/// conversion failure of that column, never a panic.
fn json_column<T: DeserializeOwned>(r: &Row<'_>, name: &str) -> rusqlite::Result<T> {
    let text: String = r.get(name)?;
    let index = r.as_ref().column_index(name)?;
    serde_json::from_str(&text).map_err(|e| rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(e)))
}
