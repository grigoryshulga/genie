//! Outcome contract: the versioned expectations of a case, stored as canonical JSON
//! (`docs/platform/observation-plane.md` §5.1).

use serde::{Deserialize, Serialize};

use crate::observation::{FollowupMode, StartOn};

/// The expectations of a case: hypotheses to verify, guardrails to watch, and the windows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Contract {
    #[serde(default)]
    pub hypotheses: Vec<Hypothesis>,
    #[serde(default)]
    pub guardrails: Vec<Guardrail>,
    pub observation: ObservationWindows,
    #[serde(default)]
    pub followup: FollowupMode,
}

/// An expected change that `verify` checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Hypothesis {
    pub id: String,
    pub source: String,
    #[serde(default)]
    pub query: String,
    pub samples_query: Option<String>,
    pub expect: Expectation,
}

/// What a hypothesis expects: a change (`-20%`, `+0.1pp`), or a bound on the value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Expectation {
    pub change: Option<String>,
    pub max: Option<f64>,
    pub min: Option<f64>,
}

/// A limit that must not be crossed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Guardrail {
    pub id: String,
    pub source: String,
    #[serde(default)]
    pub query: String,
    pub max_change: Option<String>,
    pub min_change: Option<String>,
    pub max: Option<f64>,
    pub min: Option<f64>,
}

/// When observation starts and how long it watches. Durations use `30s`, `15m`, `24h`, `2d`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservationWindows {
    #[serde(default)]
    pub start: StartOn,
    /// Default `7d`.
    pub baseline_window: Option<String>,
    pub minimum_window: String,
    /// Default: `minimum_window`.
    pub preferred_window: Option<String>,
    pub minimum_samples: Option<u64>,
    /// Default `6h`.
    pub evaluation_interval: Option<String>,
    /// Default `15m`.
    pub guardrail_interval: Option<String>,
    /// Default `1h`.
    pub guardrail_window: Option<String>,
}
