//! Immutable task intelligence. The tracker is authoritative; provider call IDs are
//! evidence pointers, never a second task journal or a distributed transaction.
use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{Actor, GenieError, Result, Tracker, db::now};

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS execution_attempts (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
 ordinal INTEGER NOT NULL, started_at TEXT NOT NULL, start_event_id INTEGER NOT NULL REFERENCES events(id),
 UNIQUE(task, ordinal)
);
CREATE TABLE IF NOT EXISTS task_signal_snapshots (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
 revision INTEGER NOT NULL, created_at TEXT NOT NULL,
 trigger_event_id INTEGER REFERENCES events(id), data TEXT NOT NULL,
 UNIQUE(task, revision)
);
CREATE TABLE IF NOT EXISTS routing_decisions (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
 attempt_id INTEGER REFERENCES execution_attempts(id) ON DELETE CASCADE,
 snapshot_id INTEGER NOT NULL REFERENCES task_signal_snapshots(id) ON DELETE CASCADE,
 created_at TEXT NOT NULL, request_key TEXT NOT NULL, data TEXT NOT NULL,
 UNIQUE(task, request_key)
);
CREATE INDEX IF NOT EXISTS routing_attempt ON routing_decisions(attempt_id, id);
CREATE INDEX IF NOT EXISTS routing_snapshot ON routing_decisions(snapshot_id, id);
CREATE INDEX IF NOT EXISTS routing_created ON routing_decisions(created_at, task);
CREATE TABLE IF NOT EXISTS execution_assessments (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
 attempt_id INTEGER NOT NULL REFERENCES execution_attempts(id) ON DELETE CASCADE,
 revision INTEGER NOT NULL, created_at TEXT NOT NULL, result TEXT NOT NULL,
 event_id INTEGER NOT NULL REFERENCES events(id), UNIQUE(attempt_id, revision)
);
CREATE INDEX IF NOT EXISTS assessment_task ON execution_assessments(task, created_at);
CREATE TABLE IF NOT EXISTS usage_attributions (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
 attempt_id INTEGER REFERENCES execution_attempts(id) ON DELETE CASCADE,
 at TEXT NOT NULL, day TEXT NOT NULL, agent TEXT NOT NULL, model TEXT NOT NULL,
 stage TEXT NOT NULL, role TEXT,
 input INTEGER NOT NULL, output INTEGER NOT NULL, cache_read INTEGER NOT NULL, cache_write INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS usage_attribution_task ON usage_attributions(task, at, attempt_id);
CREATE INDEX IF NOT EXISTS usage_attribution_attempt ON usage_attributions(attempt_id, stage);
CREATE TABLE IF NOT EXISTS policy_evaluations (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
 snapshot_id INTEGER NOT NULL REFERENCES task_signal_snapshots(id) ON DELETE CASCADE,
 created_at TEXT NOT NULL, data TEXT NOT NULL
);
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SignalName {
    Ambiguity,
    Scope,
    Complexity,
    BlastRadius,
    Reversibility,
    Sensitivity,
    Verification,
    Novelty,
}
impl SignalName {
    pub const ALL: [Self; 8] = [
        Self::Ambiguity,
        Self::Scope,
        Self::Complexity,
        Self::BlastRadius,
        Self::Reversibility,
        Self::Sensitivity,
        Self::Verification,
        Self::Novelty,
    ];
    pub fn values(self) -> &'static [&'static str] {
        match self {
            Self::Ambiguity => &["clear", "partially_ambiguous", "needs_owner_decision"],
            Self::Scope => &["local", "component", "cross_component"],
            Self::Complexity => &["routine", "moderate", "substantial", "extensive"],
            Self::BlastRadius => &["low", "medium", "high"],
            Self::Reversibility => &["high", "medium", "low"],
            Self::Sensitivity => &["ordinary", "security", "privacy", "financial", "protected"],
            Self::Verification => &["strong", "partial", "missing"],
            Self::Novelty => &["familiar", "novel"],
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum SignalSource {
    Deterministic,
    Policy,
    Jev,
    Runtime,
    Human,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct Signal {
    pub name: SignalName,
    pub value: String,
    pub source: SignalSource,
    pub evidence_refs: Vec<String>,
    pub assessed_at: String,
    pub assessor_version: String,
    pub confidence: Option<f64>,
}
impl Signal {
    pub fn unknown(name: SignalName, version: &str) -> Self {
        Self {
            name,
            value: "unknown".into(),
            source: SignalSource::Deterministic,
            evidence_refs: vec![],
            assessed_at: now(),
            assessor_version: version.into(),
            confidence: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct SignalProfile {
    pub schema_version: u32,
    /// `0` means no versioned manager brief exists; fingerprint still identifies input.
    pub brief_version: i64,
    pub brief_fingerprint: String,
    pub project_profile_version: String,
    pub extractor_version: String,
    pub signals: Vec<Signal>,
    pub affected_paths: Vec<String>,
}
impl SignalProfile {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.brief_version < 0
            || self.signals.len() != 8
            || self.signals.iter().map(|s| s.name).collect::<std::collections::BTreeSet<_>>().len() != 8
        {
            return Err(GenieError::invalid("signal schema v1 requires all eight unique signals and a nonnegative brief version"));
        }
        for text in [&self.brief_fingerprint, &self.project_profile_version, &self.extractor_version] {
            bounded(text, 200)?;
        }
        refs(&self.affected_paths, 128)?;
        for s in &self.signals {
            if !matches!(s.value.as_str(), "unknown" | "not_applicable") && !s.name.values().contains(&s.value.as_str()) {
                return Err(GenieError::invalid(format!("invalid {:?} signal value {}", s.name, s.value)));
            }
            bounded(&s.assessor_version, 200)?;
            if chrono::DateTime::parse_from_rfc3339(&s.assessed_at).is_err()
                || s.confidence.is_some_and(|c| !c.is_finite() || !(0.0..=1.0).contains(&c))
            {
                return Err(GenieError::invalid("signal requires an RFC3339 timestamp and finite confidence in [0, 1]"));
            }
            refs(&s.evidence_refs, 32)?;
            if s.value != "unknown" && s.evidence_refs.is_empty() {
                return Err(GenieError::invalid("a known signal requires evidence references"));
            }
        }
        Ok(())
    }
    pub fn value(&self, name: SignalName) -> &str {
        self.signals.iter().find(|s| s.name == name).map_or("unknown", |s| s.value.as_str())
    }
}
fn bounded(s: &str, max: usize) -> Result<()> {
    if s.trim().is_empty() || s.len() > max || s.chars().any(char::is_control) {
        return Err(GenieError::invalid("intelligence text is empty, oversized, or contains control characters"));
    }
    Ok(())
}
fn refs(items: &[String], max: usize) -> Result<()> {
    if items.len() > max {
        return Err(GenieError::invalid("too many intelligence evidence references"));
    }
    for s in items {
        bounded(s, 512)?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Snapshot {
    pub id: i64,
    pub task: String,
    pub revision: i64,
    pub created_at: String,
    pub trigger_event_id: Option<i64>,
    pub profile: SignalProfile,
}
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Attempt {
    pub id: i64,
    pub task: String,
    pub ordinal: i64,
    pub started_at: String,
    pub start_event_id: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DecisionKind {
    Initial,
    Reassessment,
    RoleModelChange,
}
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum RoutingStatus {
    Selected,
    Fallback,
    Abstained,
    Overridden,
}
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ModelSelection {
    pub role: String,
    pub model: Option<String>,
    pub thinking: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct RoutingPlan {
    pub decision_kind: DecisionKind,
    pub router_provider: String,
    pub router_version: String,
    pub policy_version: String,
    pub eligible_templates: Vec<String>,
    pub selected_template: Option<String>,
    pub selected_models: Vec<ModelSelection>,
    pub required_checks: Vec<String>,
    pub decision_status: RoutingStatus,
    pub override_reason: Option<String>,
    pub reason_codes: Vec<String>,
    pub provider_decision_ids: Vec<i64>,
}
impl RoutingPlan {
    pub fn validate(&self) -> Result<()> {
        bounded(&self.router_provider, 200)?;
        bounded(&self.router_version, 200)?;
        bounded(&self.policy_version, 200)?;
        refs(&self.eligible_templates, 64)?;
        refs(&self.required_checks, 64)?;
        refs(&self.reason_codes, 32)?;
        if let Some(id) = &self.selected_template {
            bounded(id, 120)?;
            if !self.eligible_templates.contains(id) {
                return Err(GenieError::invalid("selected template is outside policy eligibility"));
            }
        }
        if self.selected_models.len() > 64 || self.provider_decision_ids.len() > 64 {
            return Err(GenieError::invalid("too many selected models or provider references"));
        }
        for m in &self.selected_models {
            bounded(&m.role, 120)?;
            if let Some(model) = &m.model {
                bounded(model, 512)?;
            }
            if let Some(thinking) = &m.thinking {
                bounded(thinking, 40)?;
            }
        }
        if let Some(reason) = &self.override_reason {
            bounded(reason, 2000)?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RoutingDecision {
    pub id: i64,
    pub task: String,
    pub attempt_id: Option<i64>,
    pub snapshot_id: i64,
    pub created_at: String,
    pub plan: RoutingPlan,
}
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ExecutionAssessment {
    pub id: i64,
    pub task: String,
    pub attempt_id: i64,
    pub revision: i64,
    pub created_at: String,
    pub result: String,
    pub event_id: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PolicyEvaluation {
    pub id: i64,
    pub task: String,
    pub snapshot_id: i64,
    pub created_at: String,
    #[ts(type = "unknown")]
    pub evidence: Value,
}
#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct IntelligenceHistory {
    pub snapshots: Vec<Snapshot>,
    pub routing: Vec<RoutingDecision>,
    pub attempts: Vec<Attempt>,
    pub assessments: Vec<ExecutionAssessment>,
    pub evaluations: Vec<PolicyEvaluation>,
}
fn decode<T: serde::de::DeserializeOwned>(r: &rusqlite::Row<'_>, column: &str) -> rusqlite::Result<T> {
    let s: String = r.get(column)?;
    serde_json::from_str(&s).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}
impl Tracker {
    fn intelligence_event_ref(&self, task: &str, id: i64) -> Result<()> {
        let valid: bool =
            self.conn().query_row("SELECT EXISTS(SELECT 1 FROM events WHERE id=?1 AND subject=?2)", params![id, task], |r| r.get(0))?;
        if !valid {
            return Err(GenieError::invalid("trigger event does not belong to the task"));
        }
        Ok(())
    }
    pub fn signal_snapshot(&self, task: &str, id: i64) -> Result<Snapshot> {
        let task = self.get(task)?.id;
        self.conn()
            .query_row("SELECT * FROM task_signal_snapshots WHERE task=?1 AND id=?2", params![task, id], |r| {
                Ok(Snapshot {
                    id: r.get("id")?,
                    task: r.get("task")?,
                    revision: r.get("revision")?,
                    created_at: r.get("created_at")?,
                    trigger_event_id: r.get("trigger_event_id")?,
                    profile: decode(r, "data")?,
                })
            })
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("snapshot {id} for {task}")))
    }
    pub fn append_signal_snapshot(&self, actor: &Actor, task: &str, profile: &SignalProfile, trigger: Option<i64>) -> Result<Snapshot> {
        profile.validate()?;
        let task = self.get(task)?.id;
        self.tx(|| {
            if let Some(id) = trigger {
                self.intelligence_event_ref(&task, id)?;
            }
            let revision: i64 =
                self.conn()
                    .query_row("SELECT COALESCE(MAX(revision),0)+1 FROM task_signal_snapshots WHERE task=?1", [&task], |r| r.get(0))?;
            self.conn().execute(
                "INSERT INTO task_signal_snapshots(task,revision,created_at,trigger_event_id,data) VALUES (?1,?2,?3,?4,?5)",
                params![task, revision, now(), trigger, serde_json::to_string(profile)?],
            )?;
            let id = self.conn().last_insert_rowid();
            self.append_event(
                "task.signals_assessed",
                Some(&task),
                &actor.name,
                actor.role.as_str(),
                json!({"snapshotId":id,"revision":revision}),
            )?;
            self.signal_snapshot(&task, id)
        })
    }
    pub fn execution_attempts(&self, task: &str) -> Result<Vec<Attempt>> {
        let mut q = self.conn().prepare("SELECT * FROM execution_attempts WHERE task=?1 ORDER BY ordinal")?;
        Ok(q.query_map([task], |r| {
            Ok(Attempt {
                id: r.get("id")?,
                task: r.get("task")?,
                ordinal: r.get("ordinal")?,
                started_at: r.get("started_at")?,
                start_event_id: r.get("start_event_id")?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }
    pub fn open_attempt(&self, task: &str) -> Result<Option<Attempt>> {
        let attempts = self.execution_attempts(task)?;
        for a in attempts.into_iter().rev() {
            let finalized: bool =
                self.conn().query_row("SELECT EXISTS(SELECT 1 FROM execution_assessments WHERE attempt_id=?1)", [a.id], |r| r.get(0))?;
            if !finalized {
                return Ok(Some(a));
            }
        }
        Ok(None)
    }
    pub fn begin_attempt(&self, actor: &Actor, task: &str) -> Result<Attempt> {
        let task = self.get(task)?.id;
        self.tx(|| {
            if let Some(a) = self.open_attempt(&task)? {
                return Ok(a);
            }
            let ordinal: i64 =
                self.conn().query_row("SELECT COALESCE(MAX(ordinal),0)+1 FROM execution_attempts WHERE task=?1", [&task], |r| r.get(0))?;
            let event =
                self.append_event("task.execution_started", Some(&task), &actor.name, actor.role.as_str(), json!({"ordinal":ordinal}))?;
            self.conn().execute(
                "INSERT INTO execution_attempts(task,ordinal,started_at,start_event_id) VALUES (?1,?2,?3,?4)",
                params![task, ordinal, now(), event],
            )?;
            self.open_attempt(&task)?.ok_or_else(|| GenieError::invalid("attempt was not created"))
        })
    }
    pub fn finalize_attempt(&self, actor: &Actor, task: &str, result: &str, event: i64) -> Result<()> {
        if !["accepted", "rejected", "cancelled", "failed", "unknown"].contains(&result) {
            return Err(GenieError::invalid("invalid execution result"));
        }
        self.tx(|| {
            self.intelligence_event_ref(task, event)?;
            if let Some(a) = self.open_attempt(task)? {
                self.conn().execute(
                    "INSERT INTO execution_assessments(task,attempt_id,revision,created_at,result,event_id) VALUES (?1,?2,1,?3,?4,?5)",
                    params![task, a.id, now(), result, event],
                )?;
                self.append_event(
                    "task.execution_assessed",
                    Some(task),
                    &actor.name,
                    actor.role.as_str(),
                    json!({"attemptId":a.id,"result":result,"eventId":event}),
                )?;
            }
            Ok(())
        })
    }
    pub fn append_routing(
        &self,
        actor: &Actor,
        task: &str,
        attempt: Option<i64>,
        snapshot: i64,
        key: &str,
        plan: &RoutingPlan,
    ) -> Result<RoutingDecision> {
        bounded(key, 200)?;
        plan.validate()?;
        let task = self.get(task)?.id;
        self.tx(|| {
            self.signal_snapshot(&task, snapshot)?;
            if let Some(id) = attempt {
                if !self.execution_attempts(&task)?.iter().any(|a| a.id == id) {
                    return Err(GenieError::invalid("attempt belongs to another task"));
                }
            }
            let existing: Option<i64> = self
                .conn()
                .query_row("SELECT id FROM routing_decisions WHERE task=?1 AND request_key=?2", params![task, key], |r| r.get(0))
                .optional()?;
            if let Some(id) = existing {
                return self.routing_record(&task, id);
            }
            self.conn().execute(
                "INSERT INTO routing_decisions(task,attempt_id,snapshot_id,created_at,request_key,data) VALUES (?1,?2,?3,?4,?5,?6)",
                params![task, attempt, snapshot, now(), key, serde_json::to_string(plan)?],
            )?;
            let id = self.conn().last_insert_rowid();
            self.append_event(
                "task.routing_decided",
                Some(&task),
                &actor.name,
                actor.role.as_str(),
                json!({"routingId":id,"snapshotId":snapshot,"attemptId":attempt}),
            )?;
            self.routing_record(&task, id)
        })
    }
    fn routing_record(&self, task: &str, id: i64) -> Result<RoutingDecision> {
        Ok(self.conn().query_row("SELECT * FROM routing_decisions WHERE task=?1 AND id=?2", params![task, id], |r| {
            Ok(RoutingDecision {
                id: r.get("id")?,
                task: r.get("task")?,
                attempt_id: r.get("attempt_id")?,
                snapshot_id: r.get("snapshot_id")?,
                created_at: r.get("created_at")?,
                plan: decode(r, "data")?,
            })
        })?)
    }
    pub fn append_policy_evaluation(&self, actor: &Actor, task: &str, snapshot: i64, evidence: &Value) -> Result<PolicyEvaluation> {
        if evidence.to_string().len() > 64_000 {
            return Err(GenieError::invalid("policy evaluation exceeds 64000 bytes"));
        }
        let task = self.get(task)?.id;
        self.tx(|| {
            self.signal_snapshot(&task, snapshot)?;
            let at = now();
            self.conn().execute(
                "INSERT INTO policy_evaluations(task,snapshot_id,created_at,data) VALUES (?1,?2,?3,?4)",
                params![task, snapshot, at, evidence.to_string()],
            )?;
            let id = self.conn().last_insert_rowid();
            self.append_event(
                "task.policy_evaluated",
                Some(&task),
                &actor.name,
                actor.role.as_str(),
                json!({"evaluationId":id,"snapshotId":snapshot}),
            )?;
            Ok(PolicyEvaluation { id, task, snapshot_id: snapshot, created_at: at, evidence: evidence.clone() })
        })
    }
    pub fn intelligence_history(&self, task: &str) -> Result<IntelligenceHistory> {
        let task = self.get(task)?.id;
        let ids = |table: &str| -> Result<Vec<i64>> {
            let mut q = self.conn().prepare(&format!("SELECT id FROM {table} WHERE task=?1 ORDER BY id"))?;
            Ok(q.query_map([&task], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
        };
        let snapshots = ids("task_signal_snapshots")?.iter().map(|id| self.signal_snapshot(&task, *id)).collect::<Result<_>>()?;
        let routing = ids("routing_decisions")?.iter().map(|id| self.routing_record(&task, *id)).collect::<Result<_>>()?;
        let mut q = self.conn().prepare("SELECT * FROM execution_assessments WHERE task=?1 ORDER BY id")?;
        let assessments = q
            .query_map([&task], |r| {
                Ok(ExecutionAssessment {
                    id: r.get("id")?,
                    task: r.get("task")?,
                    attempt_id: r.get("attempt_id")?,
                    revision: r.get("revision")?,
                    created_at: r.get("created_at")?,
                    result: r.get("result")?,
                    event_id: r.get("event_id")?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut q = self.conn().prepare("SELECT * FROM policy_evaluations WHERE task=?1 ORDER BY id")?;
        let evaluations = q
            .query_map([&task], |r| {
                Ok(PolicyEvaluation {
                    id: r.get("id")?,
                    task: r.get("task")?,
                    snapshot_id: r.get("snapshot_id")?,
                    created_at: r.get("created_at")?,
                    evidence: decode(r, "data")?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(IntelligenceHistory { snapshots, routing, attempts: self.execution_attempts(&task)?, assessments, evaluations })
    }
}

/// A policy-owned project profile. No agent-supplied or model-supplied policy provenance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
#[ts(export)]
pub struct IntelligencePolicy {
    pub enabled: bool,
    pub version: String,
    pub semantic_assessment: bool,
    pub fallback_template: Option<String>,
    pub components: Vec<ProtectedComponent>,
    /// Check id → role id. Every matching role is independent of executor roles.
    pub checks: BTreeMap<String, String>,
    pub required_checks: Vec<String>,
    pub cooldown_secs: u32,
}
impl Default for IntelligencePolicy {
    fn default() -> Self {
        Self {
            enabled: false,
            version: "1".into(),
            semantic_assessment: false,
            fallback_template: None,
            components: vec![],
            checks: BTreeMap::new(),
            required_checks: vec![],
            cooldown_secs: 60,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ProtectedComponent {
    pub id: String,
    /// Repository-relative directory prefixes; no traversal, globs or absolute paths.
    pub paths: Vec<String>,
    pub sensitivity: String,
    pub required_checks: Vec<String>,
}
impl IntelligencePolicy {
    pub fn validate(&self) -> Result<()> {
        bounded(&self.version, 120)?;
        if self.components.len() > 64 || self.checks.len() > 32 || self.cooldown_secs > 3600 {
            return Err(GenieError::invalid("intelligence policy exceeds configured bounds"));
        }
        refs(&self.required_checks, 32)?;
        for (id, role) in &self.checks {
            bounded(id, 120)?;
            bounded(role, 120)?;
        }
        for c in &self.components {
            bounded(&c.id, 120)?;
            refs(&c.paths, 64)?;
            refs(&c.required_checks, 32)?;
            if !SignalName::Sensitivity.values().contains(&c.sensitivity.as_str()) {
                return Err(GenieError::invalid("invalid protected component sensitivity"));
            }
            if c.sensitivity != "ordinary" && c.required_checks.is_empty() {
                return Err(GenieError::invalid("protected components require at least one independent check"));
            }
            if c.paths.is_empty() || c.paths.iter().any(|p| !valid_path(p)) {
                return Err(GenieError::invalid("component paths must be relative file or directory prefixes"));
            }
        }
        for check in self.required_checks.iter().chain(self.components.iter().flat_map(|c| &c.required_checks)) {
            if !self.checks.contains_key(check) {
                return Err(GenieError::invalid(format!("unknown mandatory check {check}")));
            }
        }
        Ok(())
    }
    pub fn required(&self, paths: &[String]) -> Vec<String> {
        let mut checks = self.required_checks.clone();
        // Unknown scope conservatively includes every protected component. A low
        // semantic label cannot clear the explicit project policy.
        for c in &self.components {
            if paths.is_empty() || paths.iter().any(|p| c.paths.iter().any(|prefix| path_matches(p, prefix))) {
                checks.extend(c.required_checks.clone());
            }
        }
        checks.sort();
        checks.dedup();
        checks
    }
}
pub fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 512
        && !path.starts_with('/')
        && !path.contains(['\\', '*', '?'])
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| part != ".." && part != ".")
}
pub fn path_matches(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches('/');
    path == prefix || path.strip_prefix(prefix).is_some_and(|s| s.starts_with('/'))
}
