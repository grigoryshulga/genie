//! Observation plane: outcome cases watch a delivered change after delivery, check its
//! contract against evidence, open guardrail incidents and conclude. Specification:
//! `docs/platform/observation-plane.md`; implementation plan: `docs/development/observation-plane-plan.md`.
//!
//! The module is named `observation` because `crate::outcome` in the application layer already
//! means "how an agent run ended". Case keys are `O-<id>`, incident keys `I-<id>`, and journal
//! subjects `observation:<id>` (§2.2, §4.2).

pub mod contract;
pub mod evaluate;
pub mod sources;
pub mod store;

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSql, ToSqlOutput, ValueRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{GenieError, Result};
use crate::model::str_enum;

pub use contract::{Contract, Expectation, Guardrail, Hypothesis, ObservationWindows};

str_enum!(
    /// Which checks a case runs: `guardrails` watches limits, `verify` checks hypotheses (D3, D4).
    ObservationPolicy("observation policy") {
        Guardrails => "guardrails",
        Verify => "verify",
    }
);

str_enum!(
    /// Lifecycle of a case (§6).
    CaseStatus("case status") {
        Proposed => "proposed",
        Waiting => "waiting",
        Observing => "observing",
        Evaluating => "evaluating",
        Concluded => "concluded",
        Cancelled => "cancelled",
    }
);

str_enum!(
    /// Result of a concluded case. Set only with status `concluded` (D5).
    CaseResult("case result") {
        Confirmed => "confirmed",
        Negative => "negative",
        Inconclusive => "inconclusive",
    }
);

str_enum!(
    /// Output of one evaluation. `insufficient` keeps the case observing (D6).
    Verdict("verdict") {
        Confirmed => "confirmed",
        Negative => "negative",
        Inconclusive => "inconclusive",
        Insufficient => "insufficient",
    }
);

str_enum!(
    /// How much a conclusion can be trusted (§9.4).
    Confidence("confidence") {
        Low => "low",
        Medium => "medium",
        High => "high",
    }
);

str_enum!(
    /// Whether a criterion is a hypothesis (`verify`) or a guardrail.
    CriterionKind("criterion kind") {
        Hypothesis => "hypothesis",
        Guardrail => "guardrail",
    }
);

str_enum!(
    /// State of one criterion in an evaluation (§9.1).
    CriterionState("criterion state") {
        Met => "met",
        NotMet => "not_met",
        Ok => "ok",
        Violated => "violated",
        Insufficient => "insufficient",
    }
);

str_enum!(
    /// Whether a record measures the baseline window or the observed window (§7.2).
    EvidencePhase("evidence phase") {
        Baseline => "baseline",
        Observed => "observed",
    }
);

str_enum!(
    /// Provenance class of a record (D9): `measured` by an adapter, `reported` by a person,
    /// `agent` from an agent report. The evaluator ignores `agent` records.
    TrustLevel("trust level") {
        Measured => "measured",
        Reported => "reported",
        Agent => "agent",
    }
);

str_enum!(
    /// Why an evaluation runs (§11.4).
    EvalTrigger("evaluation trigger") {
        Window => "window",
        Manual => "manual",
        Final => "final",
    }
);

str_enum!(
    /// What starts observation (§5.1). `tasks_done` is the default.
    StartOn("observation start") {
        TasksDone => "tasks_done",
        Deployment => "deployment",
        Manual => "manual",
    }
);

str_enum!(
    /// Whether follow-up tasks are created by people or automatically (§10.2). `manual` is the default.
    FollowupMode("followup mode") {
        Manual => "manual",
        Auto => "auto",
    }
);

str_enum!(
    /// State of an incident (§10.1).
    IncidentStatus("incident status") {
        Open => "open",
        Resolved => "resolved",
        Dismissed => "dismissed",
    }
);

str_enum!(
    /// Kind of an observation source (§7.3).
    SourceKind("source kind") {
        Prometheus => "prometheus",
        Sentry => "sentry",
    }
);

str_enum!(
    /// How the server authenticates to a source (§7.3). `none` is the default.
    SourceAuth("source auth") {
        None => "none",
        Bearer => "bearer",
        Basic => "basic",
    }
);

// `str_enum!` cannot mark a `#[default]` variant, so the defaults are explicit.
#[allow(clippy::derivable_impls)]
impl Default for StartOn {
    fn default() -> Self {
        StartOn::TasksDone
    }
}

#[allow(clippy::derivable_impls)]
impl Default for FollowupMode {
    fn default() -> Self {
        FollowupMode::Manual
    }
}

#[allow(clippy::derivable_impls)]
impl Default for SourceAuth {
    fn default() -> Self {
        SourceAuth::None
    }
}

/// `O-<id>`: the display key of a case.
pub fn case_key(id: i64) -> String {
    format!("O-{id}")
}

/// `I-<id>`: the display key of an incident.
pub fn incident_key(id: i64) -> String {
    format!("I-{id}")
}

/// `observation:<id>`: the journal subject of a case's events.
pub fn case_subject(id: i64) -> String {
    format!("observation:{id}")
}

/// Accepts `O-7`, `o-7`, `7` and `observation:7`; anything else is invalid input.
pub fn parse_case_id(s: &str) -> Result<i64> {
    let rest = strip_prefix_ci(s, "o-").or_else(|| strip_prefix_ci(s, "observation:")).unwrap_or(s);
    parse_id(rest).ok_or_else(|| GenieError::invalid(format!("unknown observation id {s}")))
}

/// Accepts `I-3`, `i-3` and `3`; anything else is invalid input.
pub fn parse_incident_id(s: &str) -> Result<i64> {
    let rest = strip_prefix_ci(s, "i-").unwrap_or(s);
    parse_id(rest).ok_or_else(|| GenieError::invalid(format!("unknown incident id {s}")))
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix).then(|| &s[prefix.len()..])
}

/// Digits only: no sign, no whitespace, no empty string.
fn parse_id(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// RFC 3339 UTC with milliseconds and `Z`, exactly as `genie_core::db::now()` writes it, so
/// timestamps sort as strings.
pub fn ts(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// Reads an RFC 3339 timestamp (any offset) as UTC.
pub fn parse_ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| GenieError::invalid(format!("invalid timestamp {s}, expected RFC 3339 such as 2026-10-17T12:00:00.000Z")))
}

/// A case of the observation plane (§4.3).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase")]
pub struct ObservationCase {
    pub id: i64,
    pub key: String,
    pub title: String,
    pub policy: ObservationPolicy,
    pub status: CaseStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<CaseResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    /// Summary of the concluding evaluation, or the cancellation reason.
    pub conclusion: String,
    /// 0 while no contract exists.
    pub contract_version: i64,
    /// Linked task IDs, sorted.
    pub tasks: Vec<String>,
    pub author: String,
    pub created: String,
    pub updated: String,
    /// When the case entered `waiting`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armed_at: Option<String>,
    /// The anchor: when observation started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_started_at: Option<String>,
    /// When the case concluded or was cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_completed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_evaluation_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_guardrail_at: Option<String>,
    /// `manual` or `final` when an evaluation was requested (§6.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_requested: Option<EvalTrigger>,
    /// Consecutive scheduler passes with a collection error.
    pub failures: i64,
    /// Last collection error, for the UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

/// One version of a case's contract (§4.3).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase")]
pub struct ContractVersion {
    pub case_id: i64,
    /// 1-based.
    pub version: i64,
    pub contract: Contract,
    pub author: String,
    /// Empty unless the contract was revised after observation started.
    pub reason: String,
    pub created: String,
}

/// The history line of a contract version, without its body.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase")]
pub struct ContractVersionInfo {
    pub version: i64,
    pub author: String,
    pub reason: String,
    pub created: String,
}

/// An explicit deployment marker of a case (§6.5).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase")]
pub struct Deployment {
    pub id: i64,
    pub case_id: i64,
    /// A commit SHA, tag or release name (1–200 characters).
    #[serde(rename = "ref")]
    pub reference: String,
    /// May be empty.
    pub environment: String,
    pub at: String,
    pub recorded_by: String,
    pub created: String,
}

/// One measurement with provenance (§4.3, §7).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub id: i64,
    pub case_id: i64,
    pub contract_version: i64,
    /// Hypothesis or guardrail ID.
    pub criterion: String,
    pub kind: CriterionKind,
    pub phase: EvidencePhase,
    /// Source name or `manual`.
    pub source: String,
    /// The query sent, after placeholder substitution; empty for manual records.
    pub query: String,
    pub at: String,
    pub window_start: String,
    pub window_end: String,
    /// `None` when `error` is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Copied from the baseline record for change criteria.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<f64>,
    /// `value - baseline`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_abs: Option<f64>,
    /// `(value - baseline) / abs(baseline) * 100`; `None` when the baseline is 0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta_rel: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples: Option<i64>,
    #[ts(type = "unknown")]
    pub provenance: Value,
    pub trust: TrustLevel,
    /// Collection failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Free text for manual reports.
    pub note: String,
}

/// The result of one evaluation (§4.3, §9).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase")]
pub struct Evaluation {
    pub id: i64,
    pub case_id: i64,
    pub contract_version: i64,
    pub at: String,
    pub trigger: EvalTrigger,
    pub verdict: Verdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<Confidence>,
    pub summary: String,
    pub details: Vec<CriterionResult>,
    pub actor: String,
}

/// The state of one criterion in one evaluation.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase")]
pub struct CriterionResult {
    pub criterion: String,
    pub kind: CriterionKind,
    pub state: CriterionState,
    /// The evidence record used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<i64>,
    /// Human-readable English, such as `delta -23.1% meets target -20%`.
    pub reason: String,
}

/// An open guardrail violation of a case (§4.3, §10.1).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase")]
pub struct Incident {
    pub id: i64,
    pub key: String,
    pub case_id: i64,
    pub case_key: String,
    pub guardrail: String,
    pub status: IncidentStatus,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<i64>,
    pub created: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_by: Option<String>,
    pub note: String,
    /// Task IDs.
    pub followups: Vec<String>,
}

/// A follow-up task created through the normal task services (§4.3, §10.2).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase")]
pub struct Followup {
    pub id: i64,
    pub case_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incident: Option<i64>,
    pub task: String,
    pub actor: String,
    pub created: String,
}

/// A server-wide observation source of one project (§4.3, §7.3). Its secret is never part of it.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase")]
pub struct SourceConfig {
    pub name: String,
    pub kind: SourceKind,
    pub url: String,
    pub auth: SourceAuth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sentry_project: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dataset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    pub created: String,
    pub updated: String,
}

/// What the API shows of a stored source secret: a hint, never the value (§15).
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase")]
pub struct SecretHint {
    pub hint: String,
    pub updated: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::de::DeserializeOwned;
    use serde_json::json;

    /// Every value has its specified wire string, serializes as it, parses back, and an unknown
    /// string is refused with the label in the message.
    fn round_trips<T>(all: &[T], wire: &[&str], label: &str)
    where
        T: Copy + PartialEq + fmt::Debug + fmt::Display + FromStr<Err = GenieError> + Serialize + DeserializeOwned,
    {
        let got: Vec<String> = all.iter().map(|v| v.to_string()).collect();
        assert_eq!(got, wire, "{label}: wire strings");
        for (v, s) in all.iter().zip(wire) {
            assert_eq!(serde_json::to_value(v).unwrap(), json!(s), "{label}: serialized");
            assert_eq!(serde_json::from_value::<T>(json!(s)).unwrap(), *v, "{label}: deserialized");
            assert_eq!(s.parse::<T>().unwrap(), *v, "{label}: parsed");
        }
        match "bogus".parse::<T>() {
            Err(GenieError::Invalid(m)) => assert_eq!(m, format!("unknown {label} bogus")),
            other => panic!("{label}: expected invalid input, got {other:?}"),
        }
    }

    #[test]
    fn observation_policy_round_trips() {
        round_trips(ObservationPolicy::ALL, &["guardrails", "verify"], "observation policy");
    }

    #[test]
    fn case_status_round_trips() {
        round_trips(CaseStatus::ALL, &["proposed", "waiting", "observing", "evaluating", "concluded", "cancelled"], "case status");
    }

    #[test]
    fn case_result_round_trips() {
        round_trips(CaseResult::ALL, &["confirmed", "negative", "inconclusive"], "case result");
    }

    #[test]
    fn verdict_round_trips() {
        round_trips(Verdict::ALL, &["confirmed", "negative", "inconclusive", "insufficient"], "verdict");
    }

    #[test]
    fn confidence_round_trips() {
        round_trips(Confidence::ALL, &["low", "medium", "high"], "confidence");
    }

    #[test]
    fn criterion_kind_round_trips() {
        round_trips(CriterionKind::ALL, &["hypothesis", "guardrail"], "criterion kind");
    }

    #[test]
    fn criterion_state_round_trips() {
        round_trips(CriterionState::ALL, &["met", "not_met", "ok", "violated", "insufficient"], "criterion state");
    }

    #[test]
    fn evidence_phase_round_trips() {
        round_trips(EvidencePhase::ALL, &["baseline", "observed"], "evidence phase");
    }

    #[test]
    fn trust_level_round_trips() {
        round_trips(TrustLevel::ALL, &["measured", "reported", "agent"], "trust level");
    }

    #[test]
    fn eval_trigger_round_trips() {
        round_trips(EvalTrigger::ALL, &["window", "manual", "final"], "evaluation trigger");
    }

    #[test]
    fn start_on_round_trips() {
        round_trips(StartOn::ALL, &["tasks_done", "deployment", "manual"], "observation start");
    }

    #[test]
    fn followup_mode_round_trips() {
        round_trips(FollowupMode::ALL, &["manual", "auto"], "followup mode");
    }

    #[test]
    fn incident_status_round_trips() {
        round_trips(IncidentStatus::ALL, &["open", "resolved", "dismissed"], "incident status");
    }

    #[test]
    fn source_kind_round_trips() {
        round_trips(SourceKind::ALL, &["prometheus", "sentry"], "source kind");
    }

    #[test]
    fn source_auth_round_trips() {
        round_trips(SourceAuth::ALL, &["none", "bearer", "basic"], "source auth");
    }

    #[test]
    fn defaults_are_the_specified_ones() {
        assert_eq!(StartOn::default(), StartOn::TasksDone);
        assert_eq!(FollowupMode::default(), FollowupMode::Manual);
        assert_eq!(SourceAuth::default(), SourceAuth::None);
    }

    fn invalid_message<T: fmt::Debug>(r: Result<T>) -> String {
        match r {
            Err(GenieError::Invalid(m)) => m,
            other => panic!("expected invalid input, got {other:?}"),
        }
    }

    #[test]
    fn case_ids_accept_every_spelling() {
        assert_eq!(parse_case_id("O-7").unwrap(), 7);
        assert_eq!(parse_case_id("o-7").unwrap(), 7);
        assert_eq!(parse_case_id("7").unwrap(), 7);
        assert_eq!(parse_case_id("observation:7").unwrap(), 7);
        assert_eq!(parse_case_id(&case_key(42)).unwrap(), 42);
    }

    #[test]
    fn case_ids_reject_other_spellings_with_the_input_named() {
        for bad in ["G-7", "O-", "O-x", "-1", "+7", "", "I-7", "observation:", "O-7 x"] {
            assert_eq!(invalid_message(parse_case_id(bad)), format!("unknown observation id {bad}"), "input {bad:?}");
        }
    }

    #[test]
    fn incident_ids_accept_every_spelling() {
        assert_eq!(parse_incident_id("I-3").unwrap(), 3);
        assert_eq!(parse_incident_id("i-3").unwrap(), 3);
        assert_eq!(parse_incident_id("3").unwrap(), 3);
        assert_eq!(parse_incident_id(&incident_key(9)).unwrap(), 9);
    }

    #[test]
    fn incident_ids_reject_other_spellings_with_the_input_named() {
        for bad in ["O-3", "I-", "I-x", "-1", "observation:3", ""] {
            assert_eq!(invalid_message(parse_incident_id(bad)), format!("unknown incident id {bad}"), "input {bad:?}");
        }
    }

    #[test]
    fn keys_and_subjects_have_the_specified_form() {
        assert_eq!(case_key(7), "O-7");
        assert_eq!(incident_key(3), "I-3");
        assert_eq!(case_subject(7), "observation:7");
    }

    #[test]
    fn timestamps_round_trip_in_the_journal_format() {
        let s = "2026-10-17T12:00:00.000Z";
        assert_eq!(ts(parse_ts(s).unwrap()), s);
        // The format the rest of the database uses.
        let now = crate::db::now();
        assert_eq!(ts(parse_ts(&now).unwrap()), now);
    }

    #[test]
    fn timestamps_are_read_as_utc_and_rejected_when_malformed() {
        assert_eq!(ts(parse_ts("2026-10-17T14:00:00.250+02:00").unwrap()), "2026-10-17T12:00:00.250Z");
        match parse_ts("yesterday") {
            Err(GenieError::Invalid(m)) => assert!(m.starts_with("invalid timestamp yesterday"), "{m}"),
            other => panic!("expected invalid input, got {other:?}"),
        }
    }

    const SPEC_CONTRACT: &str = r#"{
      "hypotheses": [
        {
          "id": "checkout_latency_p95",
          "source": "prom",
          "query": "histogram_quantile(0.95, sum(rate(http_request_duration_seconds_bucket{route=\"/checkout\"}[$__range])) by (le))",
          "samplesQuery": "sum(increase(http_requests_total{route=\"/checkout\"}[$__range]))",
          "expect": { "change": "-20%" }
        }
      ],
      "guardrails": [
        { "id": "error_rate", "source": "prom", "query": "100 * sum(rate(http_requests_total{code=~\"5..\"}[$__range])) / sum(rate(http_requests_total[$__range]))", "maxChange": "+0.1pp" },
        { "id": "checkout_errors", "source": "sentry", "query": "transaction:/checkout level:error", "max": 50 }
      ],
      "observation": {
        "start": "tasks_done",
        "baselineWindow": "7d",
        "minimumWindow": "24h",
        "preferredWindow": "72h",
        "minimumSamples": 10000,
        "evaluationInterval": "6h",
        "guardrailInterval": "15m",
        "guardrailWindow": "1h"
      },
      "followup": "manual"
    }"#;

    #[test]
    fn the_specified_contract_example_deserializes() {
        let c: Contract = serde_json::from_str(SPEC_CONTRACT).unwrap();
        assert_eq!(c.hypotheses.len(), 1);
        assert_eq!(c.hypotheses[0].id, "checkout_latency_p95");
        assert_eq!(c.hypotheses[0].samples_query.as_deref(), Some("sum(increase(http_requests_total{route=\"/checkout\"}[$__range]))"));
        assert_eq!(c.hypotheses[0].expect.change.as_deref(), Some("-20%"));
        assert_eq!(c.guardrails.len(), 2);
        assert_eq!(c.guardrails[0].max_change.as_deref(), Some("+0.1pp"));
        assert_eq!(c.guardrails[1].source, "sentry");
        assert_eq!(c.guardrails[1].max, Some(50.0));
        assert_eq!(c.observation.start, StartOn::TasksDone);
        assert_eq!(c.observation.minimum_window, "24h");
        assert_eq!(c.observation.minimum_samples, Some(10000));
        assert_eq!(c.followup, FollowupMode::Manual);
        // Re-serializing keeps the wire names of §5.1.
        let back = serde_json::to_value(&c).unwrap();
        assert_eq!(back["observation"]["minimumWindow"], "24h");
        assert_eq!(back["guardrails"][0]["maxChange"], "+0.1pp");
    }

    #[test]
    fn a_contract_with_an_unknown_field_is_rejected() {
        let mut v: Value = serde_json::from_str(SPEC_CONTRACT).unwrap();
        v["surprise"] = json!(true);
        let err = serde_json::from_value::<Contract>(v).unwrap_err().to_string();
        assert!(err.contains("unknown field `surprise`"), "{err}");

        let mut nested: Value = serde_json::from_str(SPEC_CONTRACT).unwrap();
        nested["guardrails"][0]["maxchange"] = json!("+1");
        let err = serde_json::from_value::<Contract>(nested).unwrap_err().to_string();
        assert!(err.contains("unknown field `maxchange`"), "{err}");
    }

    #[test]
    fn a_contract_defaults_followup_and_start_and_needs_its_minimum_window() {
        let c: Contract = serde_json::from_value(json!({ "observation": { "minimumWindow": "1h" } })).unwrap();
        assert!(c.hypotheses.is_empty() && c.guardrails.is_empty());
        assert_eq!(c.followup, FollowupMode::Manual);
        assert_eq!(c.observation.start, StartOn::TasksDone);

        let err = serde_json::from_value::<Contract>(json!({ "observation": {} })).unwrap_err().to_string();
        assert!(err.contains("minimumWindow"), "{err}");
    }

    fn case_with_every_option_empty() -> ObservationCase {
        ObservationCase {
            id: 7,
            key: "O-7".into(),
            title: "Checkout latency".into(),
            policy: ObservationPolicy::Verify,
            status: CaseStatus::Proposed,
            result: None,
            confidence: None,
            conclusion: String::new(),
            contract_version: 0,
            tasks: vec!["GNE-1".into()],
            author: "project-manager".into(),
            created: "2026-10-17T12:00:00.000Z".into(),
            updated: "2026-10-17T12:00:00.000Z".into(),
            armed_at: None,
            observation_started_at: None,
            observation_completed_at: None,
            next_evaluation_at: None,
            next_guardrail_at: None,
            evaluation_requested: None,
            failures: 0,
            last_error: None,
        }
    }

    #[test]
    fn absent_optional_fields_are_left_out_of_the_wire_shape() {
        let v = serde_json::to_value(case_with_every_option_empty()).unwrap();
        let obj = v.as_object().unwrap();
        for key in [
            "result",
            "confidence",
            "armedAt",
            "observationStartedAt",
            "observationCompletedAt",
            "nextEvaluationAt",
            "nextGuardrailAt",
            "evaluationRequested",
            "lastError",
        ] {
            assert!(!obj.contains_key(key), "{key} must be absent when None, got {v}");
        }
        // Required fields are still written, in camelCase.
        assert_eq!(obj["key"], "O-7");
        assert_eq!(obj["contractVersion"], 0);
        assert_eq!(obj["tasks"], json!(["GNE-1"]));
        assert_eq!(obj["failures"], 0);
        // And the object reads back to the same case.
        let back: ObservationCase = serde_json::from_value(v).unwrap();
        assert_eq!(back.result, None);
        assert_eq!(back.last_error, None);
    }

    #[test]
    fn present_optional_fields_keep_their_wire_names() {
        let mut c = case_with_every_option_empty();
        c.result = Some(CaseResult::Negative);
        c.confidence = Some(Confidence::High);
        c.evaluation_requested = Some(EvalTrigger::Final);
        c.last_error = Some("prometheus: timeout".into());
        let v = serde_json::to_value(c).unwrap();
        assert_eq!(v["result"], "negative");
        assert_eq!(v["confidence"], "high");
        assert_eq!(v["evaluationRequested"], "final");
        assert_eq!(v["lastError"], "prometheus: timeout");
        assert!(v.get("armedAt").is_none());
    }

    #[test]
    fn a_deployment_serializes_its_reference_as_ref() {
        let d = Deployment {
            id: 1,
            case_id: 7,
            reference: "3f2a9c1".into(),
            environment: String::new(),
            at: "2026-10-17T12:00:00.000Z".into(),
            recorded_by: "ops".into(),
            created: "2026-10-17T12:00:00.000Z".into(),
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["ref"], "3f2a9c1");
        assert!(v.get("reference").is_none(), "{v}");
        assert_eq!(v["recordedBy"], "ops");
        let back: Deployment = serde_json::from_value(json!({
            "id": 1, "caseId": 7, "ref": "v1.2.0", "environment": "", "at": "x", "recordedBy": "ops", "created": "x"
        }))
        .unwrap();
        assert_eq!(back.reference, "v1.2.0");
    }

    #[test]
    fn evidence_leaves_out_the_values_of_a_failed_collection() {
        let e = Evidence {
            id: 3,
            case_id: 7,
            contract_version: 1,
            criterion: "error_rate".into(),
            kind: CriterionKind::Guardrail,
            phase: EvidencePhase::Observed,
            source: "prom".into(),
            query: "up".into(),
            at: "2026-10-17T12:00:00.000Z".into(),
            window_start: "2026-10-17T11:00:00.000Z".into(),
            window_end: "2026-10-17T12:00:00.000Z".into(),
            value: None,
            baseline: None,
            delta_abs: None,
            delta_rel: None,
            samples: None,
            provenance: json!({}),
            trust: TrustLevel::Measured,
            error: Some("connection refused".into()),
            note: String::new(),
        };
        let v = serde_json::to_value(e).unwrap();
        for key in ["value", "baseline", "deltaAbs", "deltaRel", "samples"] {
            assert!(v.get(key).is_none(), "{key} must be absent: {v}");
        }
        assert_eq!(v["error"], "connection refused");
        assert_eq!(v["windowStart"], "2026-10-17T11:00:00.000Z");
    }

    #[test]
    fn a_contract_stores_only_the_keys_it_sets() {
        let c: Contract = serde_json::from_value(json!({
            "hypotheses": [{ "id": "p95", "source": "prom", "expect": { "change": "-20%" } }],
            "observation": { "minimumWindow": "24h" }
        }))
        .unwrap();
        let v = serde_json::to_value(&c).unwrap();
        let mut keys: Vec<&str> = v["hypotheses"][0].as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["expect", "id", "query", "source"], "{v}");
        assert!(v["hypotheses"][0].get("samplesQuery").is_none(), "{v}");
        assert!(v["hypotheses"][0]["expect"].get("max").is_none(), "{v}");
        assert!(v["observation"].get("baselineWindow").is_none(), "{v}");
        assert_eq!(v["observation"]["minimumWindow"], "24h");
        assert_eq!(v["followup"], "manual");
    }
}
