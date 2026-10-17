//! Outcome contract: the versioned expectations of a case, stored as canonical JSON
//! (`docs/platform/observation-plane.md` §5.1). This module also holds the change grammar (§5.2),
//! the validation rules every stored contract passes (§5.3) and the views the lifecycle needs:
//! resolved windows, criterion lookup and the sources a contract reads.

use std::collections::BTreeSet;

use chrono::Duration;
use serde::{Deserialize, Serialize};

use crate::automation::parse_duration;
use crate::error::{GenieError, Result};
use crate::observation::{FollowupMode, ObservationPolicy, StartOn};

/// The source name that means "reported by a person" (§7.1).
const MANUAL_SOURCE: &str = "manual";
/// Limits of §5.3.
const MAX_CRITERIA: usize = 20;
const MAX_ID_CHARS: usize = 64;
const MAX_SOURCE_CHARS: usize = 32;
const MAX_QUERY_CHARS: usize = 4000;
const MAX_WINDOW_DAYS: i64 = 90;
const INVALID_RESOLVE: &str = "Contract::resolved needs a valid contract; call validate first";

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub samples_query: Option<String>,
    pub expect: Expectation,
}

/// What a hypothesis expects: a change (`-20%`, `+0.1pp`), or a bound on the value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, optional_fields)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Expectation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_change: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_change: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_window: Option<String>,
    pub minimum_window: String,
    /// Default: `minimum_window`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_samples: Option<u64>,
    /// Default `6h`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_interval: Option<String>,
    /// Default `15m`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_interval: Option<String>,
    /// Default `1h`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrail_window: Option<String>,
}

/// A parsed change threshold (§5.2). Not a wire type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Change {
    /// Keeps its sign: `-20%` is `-20.0`.
    pub value: f64,
    pub unit: ChangeUnit,
}

/// The unit of a change threshold (§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeUnit {
    /// `%`: the delta in percent of the baseline.
    Relative,
    /// `pp`, or no unit: the delta in the metric's own units.
    Absolute,
}

/// The windows of a valid contract with the §5.1 defaults applied. Not a wire type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolved {
    /// Default `7d`.
    pub baseline_window: Duration,
    pub minimum_window: Duration,
    /// Default: `minimum_window`.
    pub preferred_window: Duration,
    pub minimum_samples: Option<u64>,
    /// Default `6h`.
    pub evaluation_interval: Duration,
    /// Default `15m`.
    pub guardrail_interval: Duration,
    /// Default `1h`.
    pub guardrail_window: Duration,
}

/// A criterion of a contract, found by its ID.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CriterionRef<'a> {
    Hypothesis(&'a Hypothesis),
    Guardrail(&'a Guardrail),
}

/// Parses a change threshold such as `-20%`, `+0.1pp` or `+5` (§5.2).
///
/// Surrounding whitespace is ignored. The sign is required; the number is `\d+(\.\d+)?`; the unit
/// is `%` (relative), `pp` or nothing (absolute). Zero parses here: a guardrail may say `+0`
/// ("must not grow"), so the zero rule for hypotheses is a validation rule (§5.3).
pub fn parse_change(s: &str) -> Result<Change, String> {
    let text = s.trim();
    let sign = match text.as_bytes().first() {
        Some(b'+') => 1.0,
        Some(b'-') => -1.0,
        _ => return Err(format!("\"{s}\" must start with + or -, such as -20% or +0.1pp")),
    };
    let body = &text[1..];
    let int_digits = count_digits(body.as_bytes());
    if int_digits == 0 {
        return Err(format!("\"{s}\" needs a number after the sign"));
    }
    let mut end = int_digits;
    if body.as_bytes().get(end) == Some(&b'.') {
        let frac_digits = count_digits(&body.as_bytes()[end + 1..]);
        if frac_digits == 0 {
            return Err(format!("\"{s}\" needs digits after the decimal point"));
        }
        end += 1 + frac_digits;
    }
    let unit = match &body[end..] {
        "" | "pp" => ChangeUnit::Absolute,
        "%" => ChangeUnit::Relative,
        _ => return Err(format!("\"{s}\" has an unknown unit; use % for a relative change, pp for an absolute one, or no unit")),
    };
    let magnitude: f64 = body[..end].parse().map_err(|_| format!("\"{s}\" is not a number"))?;
    if !magnitude.is_finite() {
        return Err(format!("\"{s}\" is out of range"));
    }
    Ok(Change { value: sign * magnitude, unit })
}

fn count_digits(bytes: &[u8]) -> usize {
    bytes.iter().take_while(|b| b.is_ascii_digit()).count()
}

/// Reads `30s`, `15m`, `24h` or `2d`. Numbers longer than nine digits are refused before
/// `parse_duration`, because `chrono::Duration` panics on overflow.
fn duration(s: &str) -> Option<Duration> {
    if count_digits(s.trim_start().as_bytes()) > 9 {
        return None;
    }
    parse_duration(s)
}

impl Contract {
    /// Every problem of this contract for `policy`, in the order of the rules of §5.3. Empty when
    /// the contract is valid.
    pub fn problems(&self, policy: ObservationPolicy) -> Vec<String> {
        let mut out = Vec::new();
        self.check_policy(policy, &mut out);
        self.check_criteria(&mut out);
        self.check_windows(&mut out);
        out
    }

    /// Fails with `contract is invalid: …`, naming every problem (§5.3).
    pub fn validate(&self, policy: ObservationPolicy) -> Result<()> {
        let problems = self.problems(policy);
        if problems.is_empty() {
            return Ok(());
        }
        Err(GenieError::invalid(format!("contract is invalid: {}", problems.join("; "))))
    }

    /// The windows with the §5.1 defaults applied. Call only on a valid contract.
    pub fn resolved(&self) -> Resolved {
        let w = &self.observation;
        let minimum_window = duration(&w.minimum_window).expect(INVALID_RESOLVE);
        let or_default =
            |value: &Option<String>, default: Duration| value.as_deref().map_or(default, |s| duration(s).expect(INVALID_RESOLVE));
        let preferred_window = w.preferred_window.as_deref().map_or(minimum_window, |s| duration(s).expect(INVALID_RESOLVE));
        Resolved {
            baseline_window: or_default(&w.baseline_window, Duration::days(7)),
            minimum_window,
            preferred_window,
            minimum_samples: w.minimum_samples,
            evaluation_interval: or_default(&w.evaluation_interval, Duration::hours(6)),
            guardrail_interval: or_default(&w.guardrail_interval, Duration::minutes(15)),
            // The default 1h is clamped, so a short contract (minimumWindow 30m) watches 30m and
            // the guardrail window never exceeds the preferred window.
            guardrail_window: w
                .guardrail_window
                .as_deref()
                .map_or(Duration::hours(1).min(preferred_window), |s| duration(s).expect(INVALID_RESOLVE)),
        }
    }

    /// Whether `criterion` needs a baseline record (§5.3): a hypothesis with `expect.change`, or a
    /// guardrail with `maxChange` or `minChange`. Unknown criteria need none.
    pub fn needs_baseline(&self, criterion: &str) -> bool {
        match self.criterion(criterion) {
            Some(CriterionRef::Hypothesis(h)) => h.expect.change.is_some(),
            Some(CriterionRef::Guardrail(g)) => g.max_change.is_some() || g.min_change.is_some(),
            None => false,
        }
    }

    /// The hypothesis or guardrail with this ID. Hypotheses are searched first.
    pub fn criterion(&self, id: &str) -> Option<CriterionRef<'_>> {
        self.hypotheses
            .iter()
            .find(|h| h.id == id)
            .map(CriterionRef::Hypothesis)
            .or_else(|| self.guardrails.iter().find(|g| g.id == id).map(CriterionRef::Guardrail))
    }

    /// The non-manual source names this contract reads (§7.1).
    pub fn sources(&self) -> BTreeSet<String> {
        self.hypotheses
            .iter()
            .map(|h| h.source.as_str())
            .chain(self.guardrails.iter().map(|g| g.source.as_str()))
            .filter(|source| *source != MANUAL_SOURCE)
            .map(str::to_string)
            .collect()
    }

    fn check_policy(&self, policy: ObservationPolicy, out: &mut Vec<String>) {
        match policy {
            ObservationPolicy::Verify => {
                if self.hypotheses.is_empty() {
                    out.push("hypotheses: the verify policy needs at least one hypothesis".into());
                }
            }
            ObservationPolicy::Guardrails => {
                if !self.hypotheses.is_empty() {
                    out.push("hypotheses: the guardrails policy takes no hypotheses; use the verify policy".into());
                }
                if self.guardrails.is_empty() {
                    out.push("guardrails: the guardrails policy needs at least one guardrail".into());
                }
            }
        }
    }

    /// Rules 2 to 6 and 9 of §5.3: the criteria themselves.
    fn check_criteria(&self, out: &mut Vec<String>) {
        if self.hypotheses.len() > MAX_CRITERIA {
            out.push(format!("hypotheses: at most {MAX_CRITERIA} hypotheses, got {}", self.hypotheses.len()));
        }
        if self.guardrails.len() > MAX_CRITERIA {
            out.push(format!("guardrails: at most {MAX_CRITERIA} guardrails, got {}", self.guardrails.len()));
        }
        let mut ids = BTreeSet::new();
        for (i, h) in self.hypotheses.iter().enumerate() {
            let path = format!("hypotheses[{i}]");
            check_id(&path, h.id.as_str(), &mut ids, out);
            check_source(&path, &h.source, out);
            check_query(&path, &h.query, &h.source, out);
            if let Some(samples) = &h.samples_query {
                if samples.trim().is_empty() {
                    out.push(format!("{path}.samplesQuery: must not be empty when present"));
                } else if samples.chars().count() > MAX_QUERY_CHARS {
                    out.push(format!("{path}.samplesQuery: at most {MAX_QUERY_CHARS} characters"));
                }
            }
            check_expect(&path, &h.expect, out);
        }
        for (i, g) in self.guardrails.iter().enumerate() {
            let path = format!("guardrails[{i}]");
            check_id(&path, g.id.as_str(), &mut ids, out);
            check_source(&path, &g.source, out);
            check_query(&path, &g.query, &g.source, out);
            check_limits(&path, g, out);
        }
    }

    /// Rule 7 and 8 of §5.3: durations and sample counts.
    fn check_windows(&self, out: &mut Vec<String>) {
        let w = &self.observation;
        let max = Duration::days(MAX_WINDOW_DAYS);
        let minimum = parse_window(out, "minimumWindow", &w.minimum_window);
        if let Some(baseline) = optional_window(out, "baselineWindow", &w.baseline_window)
            && baseline > max
        {
            out.push(format!("observation.baselineWindow: must be at most {MAX_WINDOW_DAYS}d"));
        }
        // Without a preferredWindow, the minimum window is the preferred one (§5.1).
        let (preferred_field, preferred) = match &w.preferred_window {
            Some(s) => ("preferredWindow", parse_window(out, "preferredWindow", s)),
            None => ("minimumWindow", minimum),
        };
        if let (Some(minimum), Some(preferred)) = (minimum, preferred)
            && preferred < minimum
        {
            out.push("observation.preferredWindow: must be at least minimumWindow".into());
        }
        if let Some(preferred) = preferred
            && preferred > max
        {
            out.push(format!("observation.{preferred_field}: must be at most {MAX_WINDOW_DAYS}d"));
        }
        // Only an explicit guardrailWindow is compared: the default 1h is not checked against a
        // short preferredWindow, so a short contract stays valid.
        if let (Some(guardrail), Some(preferred)) = (optional_window(out, "guardrailWindow", &w.guardrail_window), preferred)
            && guardrail > preferred
        {
            out.push(format!("observation.guardrailWindow: must not exceed {preferred_field}"));
        }
        // Explicit intervals are bounded by the preferred window: an unbounded interval would make
        // `now + interval` overflow when the scheduler computes the next due time. The defaults
        // (6h, 15m) are not compared, so a short contract stays valid.
        let evaluation = optional_window(out, "evaluationInterval", &w.evaluation_interval);
        let guardrail_interval = optional_window(out, "guardrailInterval", &w.guardrail_interval);
        for (field, interval) in [("evaluationInterval", evaluation), ("guardrailInterval", guardrail_interval)] {
            if let (Some(interval), Some(preferred)) = (interval, preferred)
                && interval > preferred
            {
                out.push(format!("observation.{field}: must not exceed {preferred_field}"));
            }
        }
        if let Some(interval) = guardrail_interval
            && interval < Duration::minutes(1)
        {
            out.push("observation.guardrailInterval: must be at least 1m".into());
        }
        if w.minimum_samples == Some(0) {
            out.push("observation.minimumSamples: must be at least 1".into());
        }
    }
}

/// Rule 2 of §5.3: an ID of `[a-z0-9_]` with 1 to 64 characters, unique across all criteria.
fn check_id<'a>(path: &str, id: &'a str, seen: &mut BTreeSet<&'a str>, out: &mut Vec<String>) {
    if !is_criterion_id(id) {
        out.push(format!("{path}.id: \"{id}\" must be 1 to {MAX_ID_CHARS} characters of a-z, 0-9 and _"));
    } else if !seen.insert(id) {
        out.push(format!("{path}.id: \"{id}\" is already used by another criterion"));
    }
}

fn is_criterion_id(id: &str) -> bool {
    (1..=MAX_ID_CHARS).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Rule 3 of §5.3: `manual`, or `^[a-z0-9][a-z0-9_-]{0,31}$`.
fn check_source(path: &str, source: &str, out: &mut Vec<String>) {
    if !is_source_name(source) {
        out.push(format!(
            "{path}.source: \"{source}\" must be \"manual\" or 1 to {MAX_SOURCE_CHARS} characters of a-z, 0-9, _ and -, starting with a-z or 0-9"
        ));
    }
}

fn is_source_name(source: &str) -> bool {
    source == MANUAL_SOURCE
        || ((1..=MAX_SOURCE_CHARS).contains(&source.len())
            && source.bytes().enumerate().all(|(i, b)| b.is_ascii_lowercase() || b.is_ascii_digit() || (i > 0 && (b == b'_' || b == b'-'))))
}

/// Rule 4 of §5.3: a query is required unless the source is `manual`; at most 4000 characters.
fn check_query(path: &str, query: &str, source: &str, out: &mut Vec<String>) {
    if source != MANUAL_SOURCE && query.trim().is_empty() {
        out.push(format!("{path}.query: required unless the source is manual"));
    }
    if query.chars().count() > MAX_QUERY_CHARS {
        out.push(format!("{path}.query: at most {MAX_QUERY_CHARS} characters"));
    }
}

/// Rule 5 of §5.3: exactly one of `change`, `max`, `min`; a change parses and is not zero.
fn check_expect(path: &str, expect: &Expectation, out: &mut Vec<String>) {
    let set = [expect.change.is_some(), expect.max.is_some(), expect.min.is_some()].into_iter().filter(|is_set| *is_set).count();
    if set != 1 {
        out.push(format!("{path}.expect: exactly one of change, max, min"));
    }
    if let Some(change) = &expect.change {
        match parse_change(change) {
            Err(e) => out.push(format!("{path}.expect.change: {e}")),
            Ok(parsed) if parsed.value == 0.0 => out.push(format!("{path}.expect.change must not be zero")),
            Ok(_) => {}
        }
    }
}

/// Rule 6 of §5.3: a guardrail names at least one limit; each present change parses. Zero is
/// allowed here (`+0` means "must not grow").
fn check_limits(path: &str, g: &Guardrail, out: &mut Vec<String>) {
    if g.max_change.is_none() && g.min_change.is_none() && g.max.is_none() && g.min.is_none() {
        out.push(format!("{path}: needs at least one of maxChange, minChange, max, min"));
    }
    for (field, change) in [("maxChange", &g.max_change), ("minChange", &g.min_change)] {
        if let Some(change) = change
            && let Err(e) = parse_change(change)
        {
            out.push(format!("{path}.{field}: {e}"));
        }
    }
}

/// Parses a required or present window; a problem is recorded and yields `None`.
fn parse_window(out: &mut Vec<String>, field: &str, value: &str) -> Option<Duration> {
    let Some(parsed) = duration(value) else {
        out.push(format!(
            "observation.{field}: \"{value}\" is not a duration; use a whole number with s, m, h or d, such as 30s, 15m, 24h or 2d"
        ));
        return None;
    };
    if parsed <= Duration::zero() {
        out.push(format!("observation.{field}: must be positive"));
        return None;
    }
    Some(parsed)
}

fn optional_window(out: &mut Vec<String>, field: &str, value: &Option<String>) -> Option<Duration> {
    value.as_deref().and_then(|s| parse_window(out, field, s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    use crate::observation::ObservationPolicy::{Guardrails, Verify};

    /// The §5.1 example, verbatim.
    const SPEC_EXAMPLE: &str = r#"{
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

    fn contract(v: Value) -> Contract {
        serde_json::from_value(v).expect("test contract deserializes")
    }

    fn hypothesis() -> Value {
        json!({ "id": "p95", "source": "prom", "query": "q", "expect": { "change": "-20%" } })
    }

    fn guardrail() -> Value {
        json!({ "id": "err", "source": "prom", "query": "q", "max": 5 })
    }

    /// Passes every rule for `verify`: one hypothesis, no guardrails, a 24h minimum window.
    fn valid_verify() -> Value {
        json!({ "hypotheses": [hypothesis()], "observation": { "minimumWindow": "24h" } })
    }

    fn with_hypothesis(h: Value) -> Value {
        json!({ "hypotheses": [h], "observation": { "minimumWindow": "24h" } })
    }

    fn with_guardrail(g: Value) -> Value {
        json!({ "hypotheses": [hypothesis()], "guardrails": [g], "observation": { "minimumWindow": "24h" } })
    }

    fn with_windows(observation: Value) -> Value {
        json!({ "hypotheses": [hypothesis()], "observation": observation })
    }

    /// Passes every rule for `guardrails`.
    fn guardrails_only(g: Value) -> Value {
        json!({ "guardrails": [g], "observation": { "minimumWindow": "24h" } })
    }

    fn verify_problems(v: Value) -> Vec<String> {
        contract(v).problems(Verify)
    }

    #[test]
    fn parse_change_follows_the_grammar() {
        let cases = [
            ("-20%", -20.0, ChangeUnit::Relative),
            ("+0.1pp", 0.1, ChangeUnit::Absolute),
            ("+5", 5.0, ChangeUnit::Absolute),
            ("-0.25%", -0.25, ChangeUnit::Relative),
            (" +3 ", 3.0, ChangeUnit::Absolute),
            ("+0", 0.0, ChangeUnit::Absolute),
            ("+0%", 0.0, ChangeUnit::Relative),
            ("-7pp", -7.0, ChangeUnit::Absolute),
        ];
        for (input, value, unit) in cases {
            let parsed = parse_change(input).unwrap_or_else(|e| panic!("{input:?} must parse: {e}"));
            assert_eq!(parsed, Change { value, unit }, "input {input:?}");
        }
    }

    #[test]
    fn parse_change_rejects_what_the_grammar_does_not_allow() {
        for bad in ["20%", "+%", "+1.2.3", "+1e3", "±1", "", "+", "-", "+.5", "+5.", "+5 %", "+5 pp", "+5x", "+1.5pp%"] {
            assert!(parse_change(bad).is_err(), "{bad:?} must be rejected");
        }
        assert_eq!(parse_change("20%").unwrap_err(), "\"20%\" must start with + or -, such as -20% or +0.1pp");
        assert_eq!(parse_change("+%").unwrap_err(), "\"+%\" needs a number after the sign");
        assert_eq!(
            parse_change("+5x").unwrap_err(),
            "\"+5x\" has an unknown unit; use % for a relative change, pp for an absolute one, or no unit"
        );
        assert_eq!(parse_change("+5.").unwrap_err(), "\"+5.\" needs digits after the decimal point");
        assert_eq!(parse_change(&format!("+{}", "9".repeat(400))).unwrap_err(), format!("\"+{}\" is out of range", "9".repeat(400)));
    }

    #[test]
    fn the_spec_example_is_valid_for_verify_and_invalid_for_guardrails() {
        let c: Contract = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        assert_eq!(c.problems(Verify), Vec::<String>::new());
        assert!(c.validate(Verify).is_ok());
        assert_eq!(c.problems(Guardrails), ["hypotheses: the guardrails policy takes no hypotheses; use the verify policy"]);
    }

    #[test]
    fn verify_needs_a_hypothesis_and_guardrails_need_no_hypothesis() {
        assert_eq!(
            verify_problems(json!({ "guardrails": [guardrail()], "observation": { "minimumWindow": "24h" } })),
            ["hypotheses: the verify policy needs at least one hypothesis"]
        );
        assert_eq!(contract(guardrails_only(guardrail())).problems(Guardrails), Vec::<String>::new());
        assert_eq!(
            contract(json!({ "observation": { "minimumWindow": "24h" } })).problems(Guardrails),
            ["guardrails: the guardrails policy needs at least one guardrail"]
        );
    }

    #[test]
    fn guardrails_policy_takes_no_hypotheses() {
        let mixed = json!({
            "hypotheses": [hypothesis()],
            "guardrails": [guardrail()],
            "observation": { "minimumWindow": "24h" }
        });
        assert_eq!(contract(mixed).problems(Guardrails), ["hypotheses: the guardrails policy takes no hypotheses; use the verify policy"]);
    }

    #[test]
    fn criterion_ids_must_match_the_pattern() {
        assert_eq!(
            verify_problems(with_hypothesis(
                json!({ "id": "Checkout-P95", "source": "prom", "query": "q", "expect": { "change": "-20%" } })
            )),
            ["hypotheses[0].id: \"Checkout-P95\" must be 1 to 64 characters of a-z, 0-9 and _"]
        );
        let long = "a".repeat(65);
        assert_eq!(
            verify_problems(with_hypothesis(json!({ "id": long.clone(), "source": "prom", "query": "q", "expect": { "max": 1 } }))),
            [format!("hypotheses[0].id: \"{long}\" must be 1 to 64 characters of a-z, 0-9 and _")]
        );
        assert_eq!(
            verify_problems(with_hypothesis(json!({ "id": "", "source": "prom", "query": "q", "expect": { "max": 1 } }))),
            ["hypotheses[0].id: \"\" must be 1 to 64 characters of a-z, 0-9 and _"]
        );
        let exactly_64 = "a".repeat(64);
        assert!(
            verify_problems(with_hypothesis(json!({ "id": exactly_64, "source": "prom", "query": "q", "expect": { "max": 1 } })))
                .is_empty()
        );
    }

    #[test]
    fn criterion_ids_are_unique_across_hypotheses_and_guardrails() {
        let v = json!({
            "hypotheses": [hypothesis()],
            "guardrails": [{ "id": "p95", "source": "prom", "query": "q", "max": 5 }],
            "observation": { "minimumWindow": "24h" }
        });
        assert_eq!(verify_problems(v), ["guardrails[0].id: \"p95\" is already used by another criterion"]);
    }

    #[test]
    fn source_names_match_the_pattern_or_are_manual() {
        let source_problem = |source: &str| {
            format!(
                "hypotheses[0].source: \"{source}\" must be \"manual\" or 1 to 32 characters of a-z, 0-9, _ and -, starting with a-z or 0-9"
            )
        };
        let too_long = "a".repeat(33);
        let max_length = "a".repeat(32);
        for bad in ["Prom", "-prom", "_prom", "prom.x", "", too_long.as_str()] {
            let h = json!({ "id": "p95", "source": bad, "query": "q", "expect": { "max": 1 } });
            assert_eq!(verify_problems(with_hypothesis(h)), [source_problem(bad)], "source {bad:?}");
        }
        for good in ["prom", "my-source_1", "0sentry", "manual", max_length.as_str()] {
            let h = json!({ "id": "p95", "source": good, "query": "q", "expect": { "max": 1 } });
            assert!(verify_problems(with_hypothesis(h)).is_empty(), "source {good:?}");
        }
    }

    #[test]
    fn a_query_is_required_unless_the_source_is_manual_and_is_bounded() {
        assert_eq!(
            verify_problems(with_hypothesis(json!({ "id": "p95", "source": "prom", "query": "  ", "expect": { "max": 1 } }))),
            ["hypotheses[0].query: required unless the source is manual"]
        );
        assert!(verify_problems(with_hypothesis(json!({ "id": "p95", "source": "manual", "expect": { "max": 1 } }))).is_empty());
        let too_long = "q".repeat(4001);
        assert_eq!(
            verify_problems(with_hypothesis(json!({ "id": "p95", "source": "prom", "query": too_long, "expect": { "max": 1 } }))),
            ["hypotheses[0].query: at most 4000 characters"]
        );
        let exactly = "q".repeat(4000);
        assert!(
            verify_problems(with_hypothesis(json!({ "id": "p95", "source": "prom", "query": exactly, "expect": { "max": 1 } }))).is_empty()
        );
    }

    #[test]
    fn a_samples_query_is_non_empty_and_bounded_when_present() {
        let with_samples = |samples: String| {
            with_hypothesis(json!({
                "id": "p95", "source": "prom", "query": "q", "samplesQuery": samples, "expect": { "max": 1 }
            }))
        };
        assert_eq!(verify_problems(with_samples(String::new())), ["hypotheses[0].samplesQuery: must not be empty when present"]);
        assert_eq!(verify_problems(with_samples("s".repeat(4001))), ["hypotheses[0].samplesQuery: at most 4000 characters"]);
        assert!(verify_problems(with_samples("s".into())).is_empty());
    }

    #[test]
    fn a_hypothesis_expects_exactly_one_of_change_max_or_min() {
        for expect in
            [json!({}), json!({ "change": "-20%", "max": 5 }), json!({ "change": "-20%", "min": 5 }), json!({ "max": 1, "min": 0 })]
        {
            let h = json!({ "id": "p95", "source": "prom", "query": "q", "expect": expect.clone() });
            assert_eq!(verify_problems(with_hypothesis(h)), ["hypotheses[0].expect: exactly one of change, max, min"], "expect {expect}");
        }
        let valid_max = json!({ "id": "p95", "source": "prom", "query": "q", "expect": { "min": 0.5 } });
        assert!(verify_problems(with_hypothesis(valid_max)).is_empty());
    }

    #[test]
    fn a_hypothesis_change_parses_and_is_not_zero() {
        let with_change = |change: &str| {
            verify_problems(with_hypothesis(json!({ "id": "p95", "source": "prom", "query": "q", "expect": { "change": change } })))
        };
        assert_eq!(with_change("20%"), ["hypotheses[0].expect.change: \"20%\" must start with + or -, such as -20% or +0.1pp"]);
        assert_eq!(with_change("+0%"), ["hypotheses[0].expect.change must not be zero"]);
        assert_eq!(with_change("+0"), ["hypotheses[0].expect.change must not be zero"]);
        assert!(with_change("-0.25%").is_empty());
    }

    #[test]
    fn a_guardrail_names_at_least_one_limit_and_its_changes_parse() {
        assert_eq!(
            verify_problems(with_guardrail(json!({ "id": "err", "source": "prom", "query": "q" }))),
            ["guardrails[0]: needs at least one of maxChange, minChange, max, min"]
        );
        assert_eq!(
            verify_problems(with_guardrail(json!({ "id": "err", "source": "prom", "query": "q", "maxChange": "x" }))),
            ["guardrails[0].maxChange: \"x\" must start with + or -, such as -20% or +0.1pp"]
        );
        assert_eq!(
            verify_problems(with_guardrail(json!({ "id": "err", "source": "prom", "query": "q", "minChange": "-1pp%" }))),
            ["guardrails[0].minChange: \"-1pp%\" has an unknown unit; use % for a relative change, pp for an absolute one, or no unit"]
        );
    }

    #[test]
    fn a_guardrail_may_forbid_growth_with_zero() {
        let ok = with_guardrail(json!({ "id": "err", "source": "prom", "query": "q", "maxChange": "+0" }));
        assert!(verify_problems(ok).is_empty());
    }

    #[test]
    fn durations_must_parse_and_be_positive() {
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "1 day" }))),
            ["observation.minimumWindow: \"1 day\" is not a duration; use a whole number with s, m, h or d, such as 30s, 15m, 24h or 2d"]
        );
        assert_eq!(verify_problems(with_windows(json!({ "minimumWindow": "0h" }))), ["observation.minimumWindow: must be positive"]);
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "24h", "evaluationInterval": "soon" }))),
            [
                "observation.evaluationInterval: \"soon\" is not a duration; use a whole number with s, m, h or d, such as 30s, 15m, 24h or 2d"
            ]
        );
        // chrono::Duration::days panics on a count this large; it is reported as an ordinary problem.
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "9999999999999999d" }))),
            [
                "observation.minimumWindow: \"9999999999999999d\" is not a duration; use a whole number with s, m, h or d, such as 30s, 15m, 24h or 2d"
            ]
        );
    }

    #[test]
    fn guardrail_interval_is_at_least_one_minute() {
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "24h", "guardrailInterval": "30s" }))),
            ["observation.guardrailInterval: must be at least 1m"]
        );
        assert!(verify_problems(with_windows(json!({ "minimumWindow": "24h", "guardrailInterval": "1m" }))).is_empty());
    }

    #[test]
    fn preferred_window_is_at_least_the_minimum_and_at_most_ninety_days() {
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "24h", "preferredWindow": "12h" }))),
            ["observation.preferredWindow: must be at least minimumWindow"]
        );
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "24h", "preferredWindow": "91d" }))),
            ["observation.preferredWindow: must be at most 90d"]
        );
        assert!(verify_problems(with_windows(json!({ "minimumWindow": "24h", "preferredWindow": "90d" }))).is_empty());
    }

    #[test]
    fn minimum_window_is_at_most_ninety_days_when_it_is_the_preferred_window() {
        assert_eq!(verify_problems(with_windows(json!({ "minimumWindow": "91d" }))), ["observation.minimumWindow: must be at most 90d"]);
    }

    #[test]
    fn baseline_window_is_at_most_ninety_days() {
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "24h", "baselineWindow": "120d" }))),
            ["observation.baselineWindow: must be at most 90d"]
        );
    }

    #[test]
    fn guardrail_window_does_not_exceed_the_preferred_window() {
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "24h", "preferredWindow": "72h", "guardrailWindow": "96h" }))),
            ["observation.guardrailWindow: must not exceed preferredWindow"]
        );
        // The default 1h is not compared, so a short contract stays valid.
        assert!(verify_problems(with_windows(json!({ "minimumWindow": "30m" }))).is_empty());
    }

    #[test]
    fn minimum_samples_is_at_least_one() {
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "24h", "minimumSamples": 0 }))),
            ["observation.minimumSamples: must be at least 1"]
        );
    }

    #[test]
    fn intervals_must_not_exceed_the_preferred_window() {
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "24h", "preferredWindow": "72h", "evaluationInterval": "73h" }))),
            ["observation.evaluationInterval: must not exceed preferredWindow"]
        );
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "24h", "guardrailInterval": "25h" }))),
            ["observation.guardrailInterval: must not exceed minimumWindow"]
        );
        // A huge count is accepted by the digit guard but still bounded by the preferred window.
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "24h", "evaluationInterval": "999999999d" }))),
            ["observation.evaluationInterval: must not exceed minimumWindow"]
        );
        assert!(
            verify_problems(with_windows(
                json!({ "minimumWindow": "24h", "preferredWindow": "72h", "evaluationInterval": "72h", "guardrailInterval": "1h" })
            ))
            .is_empty()
        );
    }

    #[test]
    fn explicit_guardrail_window_names_the_compared_window() {
        assert_eq!(
            verify_problems(with_windows(json!({ "minimumWindow": "24h", "guardrailWindow": "48h" }))),
            ["observation.guardrailWindow: must not exceed minimumWindow"]
        );
    }

    #[test]
    fn a_short_contract_defaults_its_guardrail_window_to_the_preferred_window() {
        let r = contract(with_windows(json!({ "minimumWindow": "30m" }))).resolved();
        assert_eq!(r.preferred_window, Duration::minutes(30));
        assert_eq!(r.guardrail_window, Duration::minutes(30));
        assert!(r.guardrail_window <= r.preferred_window);
        // A long contract keeps the 1h default.
        let r = contract(valid_verify()).resolved();
        assert_eq!(r.guardrail_window, Duration::hours(1));
    }

    #[test]
    fn criteria_are_limited_to_twenty_of_each_kind() {
        let hypotheses: Vec<Value> =
            (0..21).map(|i| json!({ "id": format!("h{i}"), "source": "prom", "query": "q", "expect": { "max": 1 } })).collect();
        let v = json!({ "hypotheses": hypotheses, "observation": { "minimumWindow": "24h" } });
        assert_eq!(verify_problems(v), ["hypotheses: at most 20 hypotheses, got 21"]);

        let guardrails: Vec<Value> = (0..21).map(|i| json!({ "id": format!("g{i}"), "source": "prom", "query": "q", "max": 1 })).collect();
        let v = json!({ "hypotheses": [hypothesis()], "guardrails": guardrails, "observation": { "minimumWindow": "24h" } });
        assert_eq!(verify_problems(v), ["guardrails: at most 20 guardrails, got 21"]);
    }

    #[test]
    fn every_problem_is_reported_in_rule_order() {
        let v = with_windows(json!({ "minimumWindow": "soon", "minimumSamples": 0 }));
        assert_eq!(
            verify_problems(v),
            [
                "observation.minimumWindow: \"soon\" is not a duration; use a whole number with s, m, h or d, such as 30s, 15m, 24h or 2d",
                "observation.minimumSamples: must be at least 1",
            ]
        );
    }

    #[test]
    fn validate_fails_with_the_contract_is_invalid_prefix() {
        let c = contract(with_windows(json!({ "minimumWindow": "0h" })));
        match c.validate(Verify) {
            Err(GenieError::Invalid(m)) => assert_eq!(m, "contract is invalid: observation.minimumWindow: must be positive"),
            other => panic!("expected invalid input, got {other:?}"),
        }
        let c = contract(with_windows(json!({ "minimumWindow": "0h", "minimumSamples": 0 })));
        match c.validate(Verify) {
            Err(GenieError::Invalid(m)) => assert_eq!(
                m,
                "contract is invalid: observation.minimumWindow: must be positive; observation.minimumSamples: must be at least 1"
            ),
            other => panic!("expected invalid input, got {other:?}"),
        }
        assert!(contract(valid_verify()).validate(Verify).is_ok());
    }

    #[test]
    fn resolved_applies_the_documented_defaults() {
        let r = contract(valid_verify()).resolved();
        assert_eq!(r.baseline_window, Duration::days(7));
        assert_eq!(r.minimum_window, Duration::hours(24));
        assert_eq!(r.preferred_window, r.minimum_window);
        assert_eq!(r.minimum_samples, None);
        assert_eq!(r.evaluation_interval, Duration::hours(6));
        assert_eq!(r.guardrail_interval, Duration::minutes(15));
        assert_eq!(r.guardrail_window, Duration::hours(1));
    }

    #[test]
    fn resolved_reads_the_explicit_windows() {
        let r = contract(serde_json::from_str::<Value>(SPEC_EXAMPLE).unwrap()).resolved();
        assert_eq!(r.baseline_window, Duration::days(7));
        assert_eq!(r.minimum_window, Duration::hours(24));
        assert_eq!(r.preferred_window, Duration::hours(72));
        assert_eq!(r.minimum_samples, Some(10000));
        assert_eq!(r.evaluation_interval, Duration::hours(6));
        assert_eq!(r.guardrail_interval, Duration::minutes(15));
        assert_eq!(r.guardrail_window, Duration::hours(1));
    }

    #[test]
    #[should_panic(expected = "call validate first")]
    fn resolved_refuses_an_invalid_contract() {
        contract(with_windows(json!({ "minimumWindow": "soon" }))).resolved();
    }

    #[test]
    fn needs_baseline_for_each_criterion_form() {
        let c = contract(json!({
            "hypotheses": [
                { "id": "change_h", "source": "prom", "query": "q", "expect": { "change": "-20%" } },
                { "id": "bound_h", "source": "prom", "query": "q", "expect": { "max": 1 } }
            ],
            "guardrails": [
                { "id": "rel_g", "source": "prom", "query": "q", "maxChange": "+0.1pp" },
                { "id": "min_g", "source": "prom", "query": "q", "minChange": "-2%" },
                { "id": "bound_g", "source": "prom", "query": "q", "max": 50 }
            ],
            "observation": { "minimumWindow": "24h" }
        }));
        assert!(c.needs_baseline("change_h"));
        assert!(!c.needs_baseline("bound_h"));
        assert!(c.needs_baseline("rel_g"));
        assert!(c.needs_baseline("min_g"));
        assert!(!c.needs_baseline("bound_g"));
        assert!(!c.needs_baseline("unknown"));
    }

    #[test]
    fn criterion_finds_hypotheses_and_guardrails_by_id() {
        let c: Contract = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        match c.criterion("checkout_latency_p95") {
            Some(CriterionRef::Hypothesis(h)) => assert_eq!(h.source, "prom"),
            other => panic!("expected the hypothesis, got {other:?}"),
        }
        match c.criterion("checkout_errors") {
            Some(CriterionRef::Guardrail(g)) => assert_eq!(g.source, "sentry"),
            other => panic!("expected the guardrail, got {other:?}"),
        }
        assert!(c.criterion("nope").is_none());
    }

    #[test]
    fn sources_are_the_non_manual_source_names() {
        let c: Contract = serde_json::from_str(SPEC_EXAMPLE).unwrap();
        assert_eq!(c.sources().into_iter().collect::<Vec<_>>(), ["prom", "sentry"]);

        let manual = contract(json!({
            "hypotheses": [{ "id": "p95", "source": "manual", "expect": { "max": 1 } }],
            "guardrails": [{ "id": "err", "source": "prom", "query": "q", "max": 5 }],
            "observation": { "minimumWindow": "24h" }
        }));
        assert_eq!(manual.sources().into_iter().collect::<Vec<_>>(), ["prom"]);
    }
}
