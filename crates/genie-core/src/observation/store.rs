//! Persistence of cases, contracts, evidence, evaluations and incidents in the project tracker (§13).
//!
//! This file holds the case lifecycle up to the start of observation (§6): creation, updates,
//! contract versions, arming, starting, deployments, evaluation requests and cancellation. Each
//! change and its journal event (§12) commit in one transaction (`Tracker::tx`).
//!
//! The operations live on [`Observation`], reached through [`Tracker::observation`]. Writes that
//! take no `now` parameter (create, update, set_contract, start, request_evaluation) stamp
//! `created` and `updated` with the current time; every other time-dependent rule takes `now`
//! from the caller.

use std::collections::BTreeSet;

use chrono::{DateTime, Duration, Utc};
use rusqlite::{OptionalExtension, Row, params};
use serde_json::{Value, json};

use crate::error::{GenieError, Result};
use crate::events;
use crate::model::{Actor, Role};
use crate::observation::contract::{Contract, Resolved};
use crate::observation::{
    CaseStatus, ContractVersion, ContractVersionInfo, Deployment, EvalTrigger, ObservationCase, ObservationPolicy, StartOn, case_key,
    case_subject, ts,
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
        let limit = filter.limit.unwrap_or(DEFAULT_LIMIT);
        if !(1..=MAX_LIMIT).contains(&limit) {
            return Err(GenieError::invalid(format!("limit must be between 1 and {MAX_LIMIT}")));
        }
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
            if tasks != case.tasks {
                changes.push(("tasks", json!(tasks)));
            }
            if changes.is_empty() {
                return Ok(case);
            }
            conn.execute("UPDATE observation_cases SET updated = ?1 WHERE id = ?2", params![clock(), id])?;
            self.emit(actor, id, events::OUTCOME_UPDATED, &changes)?;
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
            self.emit(actor, id, events::OUTCOME_UPDATED, &[("contractVersion", json!(version)), ("reason", json!(stored_reason))])?;
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

    /// `waiting` → `observing`, with the anchor `anchor` (§6.2). Starting an observing case is a
    /// no-op that returns it.
    pub fn start(&self, actor: &Actor, id: i64, anchor: DateTime<Utc>, how: StartOn) -> Result<ObservationCase> {
        self.0.tx(|| {
            let case = self.get(id)?;
            match case.status {
                CaseStatus::Observing => return Ok(case),
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
            let stamp = clock();
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

/// The write stamp of an operation with no `now` parameter. Only this function reads the clock
/// inside the store; time-dependent rules take `now` as a parameter.
fn clock() -> String {
    ts(Utc::now())
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
