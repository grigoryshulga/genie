//! Deterministic evaluation of a case from its evidence (`docs/platform/observation-plane.md` §9)
//! and the guardrail check (§8). Both are pure: the time and the records come in as parameters, so
//! the scheduler, the API and the tests share one answer.

use chrono::{DateTime, Utc};

use crate::observation::contract::{Change, ChangeUnit, Contract, Guardrail, Hypothesis, parse_change};
use crate::observation::{
    Confidence, CriterionKind, CriterionResult, CriterionState, EvalTrigger, Evidence, EvidencePhase, ObservationPolicy, TrustLevel,
    Verdict,
};

/// Input of one evaluation (§9). `contract` is valid for `policy`; `evidence` holds the case's
/// records of the current contract version; `violated` lists the guardrail IDs that have an open
/// or resolved (not dismissed) incident.
#[derive(Debug, Clone, Copy)]
pub struct EvalInput<'a> {
    pub policy: ObservationPolicy,
    pub contract: &'a Contract,
    pub anchor: DateTime<Utc>,
    pub now: DateTime<Utc>,
    pub trigger: EvalTrigger,
    pub evidence: &'a [Evidence],
    pub violated: &'a [String],
}

/// The verdict of one evaluation with its per-criterion detail (§9).
#[derive(Debug, Clone)]
pub struct EvalOutcome {
    pub verdict: Verdict,
    pub confidence: Option<Confidence>,
    pub summary: String,
    /// Hypotheses first, then guardrails, in contract order.
    pub details: Vec<CriterionResult>,
}

/// A record counts as evidence when it has no collection error, a value, and a trust level the
/// evaluator accepts. `agent` records never count (D9).
pub fn is_usable(record: &Evidence) -> bool {
    record.error.is_none() && record.value.is_some() && matches!(record.trust, TrustLevel::Measured | TrustLevel::Reported)
}

/// The absolute and relative deltas of a value against its baseline (§4.3). Both are `None`
/// without a baseline; the relative delta is also `None` when the baseline is zero. The relative
/// delta divides by the absolute baseline, so a negative baseline gives the same sign convention.
pub fn deltas(value: f64, baseline: Option<f64>) -> (Option<f64>, Option<f64>) {
    match baseline {
        None => (None, None),
        Some(base) => {
            let abs = value - base;
            let rel = (base != 0.0).then(|| abs / base.abs() * 100.0);
            (Some(abs), rel)
        }
    }
}

/// The §8 check of one record against one guardrail. Returns the violation message, or `None`
/// when the record is not usable or no limit is crossed. The limits are checked in order (`max`,
/// `min`, `maxChange`, `minChange`) and the first crossed limit wins. A change limit needs the
/// matching delta, so a record without a baseline never violates it. A change threshold that does
/// not parse (a contract stored before validation) never violates, and the check returns `None`.
pub fn check_guardrail(g: &Guardrail, record: &Evidence) -> Option<String> {
    if !is_usable(record) {
        return None;
    }
    let value = record.value?;
    if let Some(max) = g.max.filter(|max| value > *max) {
        return Some(format!("value {} exceeds max {}", number(value), number(max)));
    }
    if let Some(min) = g.min.filter(|min| value < *min) {
        return Some(format!("value {} is below min {}", number(value), number(min)));
    }
    if let Some(text) = &g.max_change {
        let change = parse_change(text).ok()?;
        if let Some(delta) = delta_of(record, change.unit).filter(|d| *d > change.value) {
            return Some(change_message(text, change, delta, "exceeds"));
        }
    }
    if let Some(text) = &g.min_change {
        let change = parse_change(text).ok()?;
        if let Some(delta) = delta_of(record, change.unit).filter(|d| *d < change.value) {
            return Some(change_message(text, change, delta, "is below"));
        }
    }
    None
}

/// Evaluates a case (§9.1 to §9.4). Pure and deterministic: the same input gives the same outcome.
/// A contract whose durations do not resolve, or whose windows overflow the anchor, is answered
/// with an insufficient outcome instead of a panic.
pub fn evaluate(input: &EvalInput<'_>) -> EvalOutcome {
    let Some(windows) = input.contract.try_resolved() else {
        return invalid_contract();
    };
    let (Some(minimum_end), Some(preferred_end)) =
        (input.anchor.checked_add_signed(windows.minimum_window), input.anchor.checked_add_signed(windows.preferred_window))
    else {
        return invalid_contract();
    };
    let mut details: Vec<CriterionResult> =
        input.contract.hypotheses.iter().map(|h| hypothesis_result(h, input, windows.minimum_samples)).collect();
    details.extend(input.contract.guardrails.iter().map(|g| guardrail_result(g, input)));

    let is_final = input.trigger == EvalTrigger::Final || input.now >= preferred_end;
    let before_minimum = !is_final && input.now < minimum_end;
    let hypotheses = || details.iter().filter(|d| d.kind == CriterionKind::Hypothesis);
    let guardrails = || details.iter().filter(|d| d.kind == CriterionKind::Guardrail);
    let all_hypotheses_met = hypotheses().all(|d| d.state == CriterionState::Met);
    let all_guardrails_ok = guardrails().all(|d| d.state == CriterionState::Ok);
    let any_violated = guardrails().any(|d| d.state == CriterionState::Violated);
    let any_not_met = hypotheses().any(|d| d.state == CriterionState::NotMet);

    // First matching rule of §9.3.
    let verdict = if before_minimum {
        Verdict::Insufficient
    } else if any_violated {
        Verdict::Negative
    } else {
        match input.policy {
            ObservationPolicy::Guardrails => match (is_final, all_guardrails_ok) {
                (false, _) => Verdict::Insufficient,
                (true, true) => Verdict::Confirmed,
                (true, false) => Verdict::Inconclusive,
            },
            ObservationPolicy::Verify => {
                if all_hypotheses_met && all_guardrails_ok {
                    Verdict::Confirmed
                } else if !is_final {
                    Verdict::Insufficient
                } else if any_not_met {
                    Verdict::Negative
                } else {
                    Verdict::Inconclusive
                }
            }
        }
    };

    let summary =
        if before_minimum { "insufficient: minimum window not reached".to_string() } else { summarize(input.policy, verdict, &details) };
    let confidence = confidence(verdict, any_violated, &details, input.evidence);
    EvalOutcome { verdict, confidence, summary, details }
}

/// The outcome for a contract whose windows cannot be resolved or placed on the timeline: nothing
/// can be concluded, so the case keeps observing with no details.
fn invalid_contract() -> EvalOutcome {
    EvalOutcome {
        verdict: Verdict::Insufficient,
        confidence: None,
        summary: "insufficient: contract is invalid".to_string(),
        details: Vec::new(),
    }
}

/// The latest usable observed record of a criterion (highest `id`) and its value (§9.1).
fn latest_usable<'a>(evidence: &'a [Evidence], criterion: &str) -> Option<(&'a Evidence, f64)> {
    evidence
        .iter()
        .filter(|e| e.criterion == criterion && e.phase == EvidencePhase::Observed && is_usable(e))
        .filter_map(|e| e.value.map(|v| (e, v)))
        .max_by_key(|(e, _)| e.id)
}

fn result(criterion: &str, kind: CriterionKind, state: CriterionState, evidence: Option<i64>, reason: String) -> CriterionResult {
    CriterionResult { criterion: criterion.to_string(), kind, state, evidence, reason }
}

/// The state of one hypothesis from its latest usable observed record (§9.1).
fn hypothesis_result(h: &Hypothesis, input: &EvalInput<'_>, minimum_samples: Option<u64>) -> CriterionResult {
    let kind = CriterionKind::Hypothesis;
    let insufficient = |evidence: Option<i64>, reason: String| result(&h.id, kind, CriterionState::Insufficient, evidence, reason);
    let Some((record, value)) = latest_usable(input.evidence, &h.id) else {
        return insufficient(None, "no usable evidence".into());
    };
    let used = Some(record.id);
    if let Some(minimum) = minimum_samples {
        match record.samples {
            None => return insufficient(used, "sample count unknown".into()),
            Some(samples) => match u64::try_from(samples) {
                Ok(n) if n >= minimum => {}
                _ => return insufficient(used, format!("samples {samples} below minimum {minimum}")),
            },
        }
    }
    if let Some(text) = &h.expect.change {
        let change = match parse_change(text) {
            Ok(change) => change,
            Err(problem) => return insufficient(used, format!("expectation is invalid: {problem}")),
        };
        let Some(delta) = delta_of(record, change.unit) else {
            let reason =
                if change.unit == ChangeUnit::Relative && record.baseline == Some(0.0) { "baseline is zero" } else { "no baseline" };
            return insufficient(used, reason.into());
        };
        let target = change.value;
        let met = (target < 0.0 && delta <= target) || (target > 0.0 && delta >= target);
        let suffix = unit_suffix(text, change.unit);
        let (state, verb) = if met { (CriterionState::Met, "meets") } else { (CriterionState::NotMet, "misses") };
        let reason = format!("delta {}{suffix} {verb} target {}{suffix}", signed(delta), signed(target));
        return result(&h.id, kind, state, used, reason);
    }
    let (state, reason) = match (h.expect.max, h.expect.min) {
        (Some(max), _) if value <= max => (CriterionState::Met, format!("value {} meets max {}", number(value), number(max))),
        (Some(max), _) => (CriterionState::NotMet, format!("value {} exceeds max {}", number(value), number(max))),
        (None, Some(min)) if value >= min => (CriterionState::Met, format!("value {} meets min {}", number(value), number(min))),
        (None, Some(min)) => (CriterionState::NotMet, format!("value {} is below min {}", number(value), number(min))),
        (None, None) => return insufficient(used, "expectation is invalid: exactly one of change, max, min is required".into()),
    };
    result(&h.id, kind, state, used, reason)
}

/// The state of one guardrail (§9.1): violated by an incident, ok with a usable record, or never
/// measured.
fn guardrail_result(g: &Guardrail, input: &EvalInput<'_>) -> CriterionResult {
    let kind = CriterionKind::Guardrail;
    let latest = latest_usable(input.evidence, &g.id).map(|(record, _)| record.id);
    if input.violated.contains(&g.id) {
        // The record that opened the incident is not kept with the case, so the latest record is
        // not reported as its evidence.
        result(&g.id, kind, CriterionState::Violated, None, "an incident was opened for this guardrail".into())
    } else if latest.is_some() {
        result(&g.id, kind, CriterionState::Ok, latest, "within its limits".into())
    } else {
        result(&g.id, kind, CriterionState::Insufficient, None, "never measured".into())
    }
}

/// The summary of §9.4. Policy `guardrails` has no hypotheses part.
fn summarize(policy: ObservationPolicy, verdict: Verdict, details: &[CriterionResult]) -> String {
    let count = |kind: CriterionKind, state: CriterionState| details.iter().filter(|d| d.kind == kind && d.state == state).count();
    let hypotheses = details.iter().filter(|d| d.kind == CriterionKind::Hypothesis).count();
    let guardrails = details.iter().filter(|d| d.kind == CriterionKind::Guardrail).count();
    let guardrail_part = format!(
        "guardrails: {}/{guardrails} ok, {} violated; attribution: not assessed",
        count(CriterionKind::Guardrail, CriterionState::Ok),
        count(CriterionKind::Guardrail, CriterionState::Violated),
    );
    match policy {
        ObservationPolicy::Verify => {
            format!("{verdict}: {}/{hypotheses} hypotheses met; {guardrail_part}", count(CriterionKind::Hypothesis, CriterionState::Met))
        }
        ObservationPolicy::Guardrails => format!("{verdict}: {guardrail_part}"),
    }
}

/// The confidence of a conclusion (§9.4). `inconclusive` is low and `insufficient` has none.
/// A negative from rule 2 (a violated guardrail, `violated` is true) is high. Otherwise
/// `confirmed` and `negative` are high when every record behind a decided state is measured, and
/// medium when one is reported.
fn confidence(verdict: Verdict, violated: bool, details: &[CriterionResult], evidence: &[Evidence]) -> Option<Confidence> {
    match verdict {
        Verdict::Inconclusive => Some(Confidence::Low),
        Verdict::Insufficient => None,
        Verdict::Confirmed | Verdict::Negative => {
            if verdict == Verdict::Negative && violated {
                return Some(Confidence::High);
            }
            let reported = details
                .iter()
                .filter(|d| matches!(d.state, CriterionState::Met | CriterionState::NotMet | CriterionState::Ok))
                .filter_map(|d| d.evidence)
                .any(|id| evidence.iter().any(|e| e.id == id && e.trust == TrustLevel::Reported));
            Some(if reported { Confidence::Medium } else { Confidence::High })
        }
    }
}

fn delta_of(record: &Evidence, unit: ChangeUnit) -> Option<f64> {
    match unit {
        ChangeUnit::Relative => record.delta_rel,
        ChangeUnit::Absolute => record.delta_abs,
    }
}

fn change_message(text: &str, change: Change, delta: f64, verb: &str) -> String {
    let suffix = unit_suffix(text, change.unit);
    format!("change {}{suffix} {verb} {}{suffix}", signed(delta), signed(change.value))
}

/// The unit printed after a change: `%` when relative, `pp` when the threshold wrote `pp`, and
/// nothing for a bare absolute threshold (§8).
fn unit_suffix(text: &str, unit: ChangeUnit) -> &'static str {
    match unit {
        ChangeUnit::Relative => "%",
        ChangeUnit::Absolute if text.trim().ends_with("pp") => "pp",
        ChangeUnit::Absolute => "",
    }
}

/// At most four decimals with trailing zeros removed. A value that rounds to zero prints as `0`,
/// never `-0` (§8).
fn number(v: f64) -> String {
    let fixed = format!("{v:.4}");
    let trimmed = fixed.trim_end_matches('0').trim_end_matches('.');
    if trimmed == "-0" { "0".to_string() } else { trimmed.to_string() }
}

/// A number with its sign: `+0.1`, `-0.4`, `+0`.
fn signed(v: f64) -> String {
    let n = number(v);
    if n.starts_with('-') { n } else { format!("+{n}") }
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone};
    use serde_json::{Value, json};

    use super::*;
    use crate::observation::ObservationPolicy::{Guardrails, Verify};

    const HYP: CriterionKind = CriterionKind::Hypothesis;
    const GUARD: CriterionKind = CriterionKind::Guardrail;
    const START: &str = "2026-10-17T12:00:00.000Z";
    const NO_VIOLATIONS: &[String] = &[];

    fn anchor() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 17, 12, 0, 0).unwrap()
    }

    /// The anchor plus `hours`.
    fn at(hours: i64) -> DateTime<Utc> {
        anchor() + Duration::hours(hours)
    }

    fn contract(v: Value) -> Contract {
        serde_json::from_value(v).expect("test contract parses")
    }

    /// One hypothesis `latency` (-20%) and one guardrail `errors` (maxChange +0.1pp); observation
    /// starts at 24h (minimum) and is final from 72h (preferred).
    fn verify_contract() -> Contract {
        contract(json!({
            "hypotheses": [{ "id": "latency", "source": "prom", "query": "q", "expect": { "change": "-20%" } }],
            "guardrails": [{ "id": "errors", "source": "prom", "query": "q", "maxChange": "+0.1pp" }],
            "observation": { "minimumWindow": "24h", "preferredWindow": "72h" }
        }))
    }

    /// Two hypotheses (`latency` -20%, `throughput` +10%) and two guardrails (`errors` maxChange
    /// +0.1pp, `saturation` max 80), in that order, with the same windows as `verify_contract`.
    fn four_criteria() -> Contract {
        contract(json!({
            "hypotheses": [
                { "id": "latency", "source": "prom", "query": "q", "expect": { "change": "-20%" } },
                { "id": "throughput", "source": "prom", "query": "q", "expect": { "change": "+10%" } }
            ],
            "guardrails": [
                { "id": "errors", "source": "prom", "query": "q", "maxChange": "+0.1pp" },
                { "id": "saturation", "source": "prom", "query": "q", "max": 80 }
            ],
            "observation": { "minimumWindow": "24h", "preferredWindow": "72h" }
        }))
    }

    /// `four_criteria` without its hypotheses, which is the shape the guardrails policy needs.
    fn guardrails_only() -> Contract {
        let mut c = four_criteria();
        c.hypotheses.clear();
        c
    }

    fn guardrail(max: Option<f64>, min: Option<f64>, max_change: Option<&str>, min_change: Option<&str>) -> Guardrail {
        Guardrail {
            id: "g".into(),
            source: "prom".into(),
            query: "q".into(),
            max_change: max_change.map(str::to_string),
            min_change: min_change.map(str::to_string),
            max,
            min,
        }
    }

    /// A record of one criterion, with its deltas computed as the scheduler does (§7.2).
    fn record(id: i64, criterion: &str, kind: CriterionKind, phase: EvidencePhase, value: f64, baseline: Option<f64>) -> Evidence {
        let (delta_abs, delta_rel) = deltas(value, baseline);
        Evidence {
            id,
            case_id: 1,
            contract_version: 1,
            criterion: criterion.to_string(),
            kind,
            phase,
            source: "prom".to_string(),
            query: "q".to_string(),
            at: START.to_string(),
            window_start: START.to_string(),
            window_end: START.to_string(),
            value: Some(value),
            baseline,
            delta_abs,
            delta_rel,
            samples: None,
            provenance: json!({}),
            trust: TrustLevel::Measured,
            error: None,
            note: String::new(),
        }
    }

    fn observed(id: i64, criterion: &str, kind: CriterionKind, value: f64, baseline: Option<f64>) -> Evidence {
        record(id, criterion, kind, EvidencePhase::Observed, value, baseline)
    }

    /// Evaluates at `hours` after the anchor, triggered by the clock.
    fn run(policy: ObservationPolicy, contract: &Contract, hours: i64, evidence: &[Evidence], violated: &[String]) -> EvalOutcome {
        run_with(policy, contract, hours, EvalTrigger::Window, evidence, violated)
    }

    fn run_with(
        policy: ObservationPolicy,
        contract: &Contract,
        hours: i64,
        trigger: EvalTrigger,
        evidence: &[Evidence],
        violated: &[String],
    ) -> EvalOutcome {
        evaluate(&EvalInput { policy, contract, anchor: anchor(), now: at(hours), trigger, evidence, violated })
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    /// Asserts two optional pairs of deltas, up to float noise.
    fn assert_deltas(actual: (Option<f64>, Option<f64>), expected: (Option<f64>, Option<f64>)) {
        let close = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => (a - b).abs() < 1e-9,
            _ => false,
        };
        assert!(close(actual.0, expected.0) && close(actual.1, expected.1), "deltas {actual:?}, expected {expected:?}");
    }

    #[test]
    fn deltas_are_absolute_and_relative_to_the_baseline() {
        assert_deltas(deltas(120.0, Some(100.0)), (Some(20.0), Some(20.0)));
        assert_deltas(deltas(80.0, Some(100.0)), (Some(-20.0), Some(-20.0)));
        assert_deltas(deltas(-50.0, Some(-100.0)), (Some(50.0), Some(50.0)));
    }

    #[test]
    fn a_zero_baseline_has_no_relative_delta() {
        assert_deltas(deltas(5.0, Some(0.0)), (Some(5.0), None));
    }

    #[test]
    fn a_missing_baseline_has_no_delta() {
        assert_deltas(deltas(5.0, None), (None, None));
    }

    #[test]
    fn only_measured_and_reported_records_are_usable() {
        let measured = observed(1, "g", GUARD, 1.0, None);
        assert!(is_usable(&measured));

        let mut reported = measured.clone();
        reported.trust = TrustLevel::Reported;
        assert!(is_usable(&reported));

        let mut agent = measured.clone();
        agent.trust = TrustLevel::Agent;
        assert!(!is_usable(&agent));

        let mut failed = measured.clone();
        failed.error = Some("timeout".into());
        assert!(!is_usable(&failed));

        let mut valueless = measured;
        valueless.value = None;
        assert!(!is_usable(&valueless));
    }

    #[test]
    fn check_guardrail_reports_max_and_min_with_the_spec_messages() {
        let max = guardrail(Some(50.0), None, None, None);
        assert_eq!(check_guardrail(&max, &observed(1, "g", GUARD, 61.0, None)).as_deref(), Some("value 61 exceeds max 50"));
        assert_eq!(check_guardrail(&max, &observed(1, "g", GUARD, 50.0, None)), None, "equal to max is not a violation");
        assert_eq!(check_guardrail(&max, &observed(1, "g", GUARD, 49.0, None)), None);

        let min = guardrail(None, Some(5.0), None, None);
        assert_eq!(check_guardrail(&min, &observed(1, "g", GUARD, 3.0, None)).as_deref(), Some("value 3 is below min 5"));
        assert_eq!(check_guardrail(&min, &observed(1, "g", GUARD, 5.0, None)), None, "equal to min is not a violation");
    }

    #[test]
    fn check_guardrail_reports_absolute_and_relative_changes_with_their_units() {
        let absolute = guardrail(None, None, Some("+0.1pp"), None);
        assert_eq!(check_guardrail(&absolute, &observed(1, "g", GUARD, 0.18, Some(0.0))).as_deref(), Some("change +0.18pp exceeds +0.1pp"));
        assert_eq!(check_guardrail(&absolute, &observed(1, "g", GUARD, 0.1, Some(0.0))), None);

        let relative = guardrail(None, None, Some("+10%"), None);
        assert_eq!(check_guardrail(&relative, &observed(1, "g", GUARD, 112.5, Some(100.0))).as_deref(), Some("change +12.5% exceeds +10%"));

        let relative_min = guardrail(None, None, None, Some("-0.2%"));
        assert_eq!(
            check_guardrail(&relative_min, &observed(1, "g", GUARD, 99.6, Some(100.0))).as_deref(),
            Some("change -0.4% is below -0.2%")
        );

        let bare = guardrail(None, None, None, Some("-5"));
        assert_eq!(
            check_guardrail(&bare, &observed(1, "g", GUARD, 90.0, Some(100.0))).as_deref(),
            Some("change -10 is below -5"),
            "a threshold without a unit prints without one"
        );
    }

    #[test]
    fn check_guardrail_needs_the_matching_baseline_for_change_limits() {
        let absolute = guardrail(None, None, Some("+0.1pp"), None);
        assert_eq!(check_guardrail(&absolute, &observed(1, "g", GUARD, 5.0, None)), None, "no baseline");

        let relative = guardrail(None, None, Some("+10%"), None);
        assert_eq!(check_guardrail(&relative, &observed(1, "g", GUARD, 5.0, Some(0.0))), None, "zero baseline");

        let relative_min = guardrail(None, None, None, Some("-0.2%"));
        assert_eq!(check_guardrail(&relative_min, &observed(1, "g", GUARD, 5.0, None)), None, "no baseline for minChange");
    }

    #[test]
    fn check_guardrail_checks_limits_in_order_and_the_first_crossed_one_wins() {
        let both = guardrail(Some(50.0), None, Some("+0.1pp"), None);
        assert_eq!(
            check_guardrail(&both, &observed(1, "g", GUARD, 61.0, Some(60.0))).as_deref(),
            Some("value 61 exceeds max 50"),
            "max is checked before maxChange, although maxChange is also crossed"
        );
    }

    #[test]
    fn check_guardrail_ignores_unusable_records() {
        let max = guardrail(Some(50.0), None, None, None);

        let mut failed = observed(1, "g", GUARD, 61.0, None);
        failed.error = Some("timeout".into());
        assert_eq!(check_guardrail(&max, &failed), None, "a record with an error");

        let mut agent = observed(1, "g", GUARD, 61.0, None);
        agent.trust = TrustLevel::Agent;
        assert_eq!(check_guardrail(&max, &agent), None, "an agent record");

        let mut reported = observed(1, "g", GUARD, 61.0, None);
        reported.trust = TrustLevel::Reported;
        assert_eq!(check_guardrail(&max, &reported).as_deref(), Some("value 61 exceeds max 50"), "a reported record counts");
    }

    #[test]
    fn check_guardrail_formats_numbers_with_at_most_four_decimals() {
        let max = guardrail(Some(1.0), None, None, None);
        assert_eq!(check_guardrail(&max, &observed(1, "g", GUARD, 1.234567, None)).as_deref(), Some("value 1.2346 exceeds max 1"));

        let min = guardrail(None, Some(0.0), None, None);
        assert_eq!(
            check_guardrail(&min, &observed(1, "g", GUARD, -0.00001, None)).as_deref(),
            Some("value 0 is below min 0"),
            "a value that rounds to zero prints as 0, not -0"
        );
    }

    #[test]
    fn rule_one_keeps_the_case_observing_before_the_minimum_window() {
        let contract = verify_contract();
        let evidence = [observed(1, "latency", HYP, 70.0, Some(100.0))];
        let out = run(Verify, &contract, 10, &evidence, &ids(&["errors"]));
        assert_eq!(out.verdict, Verdict::Insufficient, "a violation does not conclude before the minimum window");
        assert_eq!(out.confidence, None);
        assert_eq!(out.summary, "insufficient: minimum window not reached");
        assert_eq!(out.details[1].state, CriterionState::Violated, "the details still show the violation");
    }

    #[test]
    fn rule_two_makes_a_violated_guardrail_negative_with_high_confidence() {
        let contract = verify_contract();
        let evidence = [observed(1, "latency", HYP, 70.0, Some(100.0))];
        let out = run(Verify, &contract, 30, &evidence, &ids(&["errors"]));
        assert_eq!(out.verdict, Verdict::Negative);
        assert_eq!(out.confidence, Some(Confidence::High));
        assert_eq!(out.summary, "negative: 1/1 hypotheses met; guardrails: 0/1 ok, 1 violated; attribution: not assessed");
    }

    #[test]
    fn guardrails_policy_is_insufficient_before_the_preferred_window() {
        let contract = guardrails_only();
        let evidence = [observed(1, "errors", GUARD, 1.0, Some(1.0)), observed(2, "saturation", GUARD, 50.0, None)];
        let out = run(Guardrails, &contract, 30, &evidence, NO_VIOLATIONS);
        assert_eq!(out.verdict, Verdict::Insufficient);
        assert_eq!(out.confidence, None);
        assert_eq!(out.summary, "insufficient: guardrails: 2/2 ok, 0 violated; attribution: not assessed");
    }

    #[test]
    fn guardrails_policy_confirms_when_final_and_every_guardrail_is_ok() {
        let contract = guardrails_only();
        let evidence = [observed(1, "errors", GUARD, 1.0, Some(1.0)), observed(2, "saturation", GUARD, 50.0, None)];
        let out = run(Guardrails, &contract, 80, &evidence, NO_VIOLATIONS);
        assert_eq!(out.verdict, Verdict::Confirmed);
        assert_eq!(out.confidence, Some(Confidence::High));
        assert_eq!(out.summary, "confirmed: guardrails: 2/2 ok, 0 violated; attribution: not assessed");
    }

    #[test]
    fn guardrails_policy_is_inconclusive_when_final_with_a_never_measured_guardrail() {
        let contract = guardrails_only();
        let evidence = [observed(1, "errors", GUARD, 1.0, Some(1.0))];
        let out = run(Guardrails, &contract, 80, &evidence, NO_VIOLATIONS);
        assert_eq!(out.verdict, Verdict::Inconclusive);
        assert_eq!(out.confidence, Some(Confidence::Low));
        assert_eq!(out.summary, "inconclusive: guardrails: 1/2 ok, 0 violated; attribution: not assessed");
        assert_eq!(out.details[1].reason, "never measured");
    }

    #[test]
    fn verify_confirms_before_the_preferred_window_when_every_criterion_passes() {
        let contract = verify_contract();
        let evidence = [observed(1, "latency", HYP, 70.0, Some(100.0)), observed(2, "errors", GUARD, 1.0, Some(1.0))];
        let out = run(Verify, &contract, 30, &evidence, NO_VIOLATIONS);
        assert_eq!(out.verdict, Verdict::Confirmed);
        assert_eq!(out.confidence, Some(Confidence::High));
        assert_eq!(out.summary, "confirmed: 1/1 hypotheses met; guardrails: 1/1 ok, 0 violated; attribution: not assessed");
        assert_eq!(out.details[0].reason, "delta -30% meets target -20%");
    }

    #[test]
    fn verify_is_insufficient_before_the_preferred_window_when_a_hypothesis_is_not_met() {
        let contract = verify_contract();
        let evidence = [observed(1, "latency", HYP, 90.0, Some(100.0)), observed(2, "errors", GUARD, 1.0, Some(1.0))];
        let out = run(Verify, &contract, 30, &evidence, NO_VIOLATIONS);
        assert_eq!(out.verdict, Verdict::Insufficient);
        assert_eq!(out.summary, "insufficient: 0/1 hypotheses met; guardrails: 1/1 ok, 0 violated; attribution: not assessed");
        assert_eq!(out.details[0].reason, "delta -10% misses target -20%");
    }

    #[test]
    fn verify_is_negative_when_final_and_a_hypothesis_is_not_met() {
        let contract = verify_contract();
        let evidence = [observed(1, "latency", HYP, 90.0, Some(100.0)), observed(2, "errors", GUARD, 1.0, Some(1.0))];
        let out = run(Verify, &contract, 80, &evidence, NO_VIOLATIONS);
        assert_eq!(out.verdict, Verdict::Negative);
        assert_eq!(out.confidence, Some(Confidence::High));
        assert_eq!(out.summary, "negative: 0/1 hypotheses met; guardrails: 1/1 ok, 0 violated; attribution: not assessed");
    }

    #[test]
    fn verify_is_inconclusive_when_final_with_a_met_and_an_insufficient_hypothesis() {
        let contract = four_criteria();
        let evidence = [
            observed(1, "latency", HYP, 70.0, Some(100.0)),
            observed(2, "errors", GUARD, 1.0, Some(1.0)),
            observed(3, "saturation", GUARD, 50.0, None),
        ];
        let out = run(Verify, &contract, 80, &evidence, NO_VIOLATIONS);
        assert_eq!(out.verdict, Verdict::Inconclusive);
        assert_eq!(out.confidence, Some(Confidence::Low));
        assert_eq!(out.summary, "inconclusive: 1/2 hypotheses met; guardrails: 2/2 ok, 0 violated; attribution: not assessed");
        assert_eq!(out.details[1].state, CriterionState::Insufficient);
    }

    #[test]
    fn the_latest_usable_observed_record_decides() {
        let contract = verify_contract();
        let evidence = [
            observed(2, "latency", HYP, 70.0, Some(100.0)),
            observed(5, "latency", HYP, 90.0, Some(100.0)),
            observed(3, "errors", GUARD, 1.0, Some(1.0)),
        ];
        let out = run(Verify, &contract, 30, &evidence, NO_VIOLATIONS);
        assert_eq!(out.details[0].state, CriterionState::NotMet);
        assert_eq!(out.details[0].evidence, Some(5), "the highest record id is used, not the first one");
    }

    #[test]
    fn agent_evidence_is_ignored() {
        let contract = verify_contract();
        let mut agent = observed(1, "latency", HYP, 70.0, Some(100.0));
        agent.trust = TrustLevel::Agent;
        let evidence = [agent, observed(2, "errors", GUARD, 1.0, Some(1.0))];
        let out = run(Verify, &contract, 80, &evidence, NO_VIOLATIONS);
        assert_eq!(out.details[0].state, CriterionState::Insufficient);
        assert_eq!(out.details[0].reason, "no usable evidence");
        assert_eq!(out.details[0].evidence, None);
        assert_eq!(out.verdict, Verdict::Inconclusive, "the agent record does not make the hypothesis met");
    }

    #[test]
    fn reported_evidence_gives_medium_confidence() {
        let contract = verify_contract();
        let mut reported = observed(1, "latency", HYP, 70.0, Some(100.0));
        reported.trust = TrustLevel::Reported;
        let evidence = [reported, observed(2, "errors", GUARD, 1.0, Some(1.0))];
        let out = run(Verify, &contract, 80, &evidence, NO_VIOLATIONS);
        assert_eq!(out.verdict, Verdict::Confirmed);
        assert_eq!(out.confidence, Some(Confidence::Medium));
    }

    #[test]
    fn samples_below_the_minimum_or_unknown_are_insufficient() {
        let mut contract = verify_contract();
        contract.observation.minimum_samples = Some(1000);
        let errors = observed(9, "errors", GUARD, 1.0, Some(1.0));

        let mut below = observed(1, "latency", HYP, 70.0, Some(100.0));
        below.samples = Some(500);
        let out = run(Verify, &contract, 80, &[below, errors.clone()], NO_VIOLATIONS);
        assert_eq!(out.details[0].state, CriterionState::Insufficient);
        assert_eq!(out.details[0].reason, "samples 500 below minimum 1000");
        assert_eq!(out.details[0].evidence, Some(1));

        let unknown = observed(2, "latency", HYP, 70.0, Some(100.0));
        let out = run(Verify, &contract, 80, &[unknown, errors.clone()], NO_VIOLATIONS);
        assert_eq!(out.details[0].reason, "sample count unknown");

        let mut at_minimum = observed(3, "latency", HYP, 70.0, Some(100.0));
        at_minimum.samples = Some(1000);
        let out = run(Verify, &contract, 80, &[at_minimum, errors], NO_VIOLATIONS);
        assert_eq!(out.details[0].state, CriterionState::Met, "samples equal to the minimum are enough");
    }

    #[test]
    fn a_relative_change_on_a_zero_or_missing_baseline_is_insufficient() {
        let contract = verify_contract();
        let errors = observed(9, "errors", GUARD, 1.0, Some(1.0));

        let zero = observed(1, "latency", HYP, 5.0, Some(0.0));
        let out = run(Verify, &contract, 80, &[zero, errors.clone()], NO_VIOLATIONS);
        assert_eq!(out.details[0].state, CriterionState::Insufficient);
        assert_eq!(out.details[0].reason, "baseline is zero");

        let missing = observed(2, "latency", HYP, 5.0, None);
        let out = run(Verify, &contract, 80, &[missing, errors], NO_VIOLATIONS);
        assert_eq!(out.details[0].reason, "no baseline");
    }

    #[test]
    fn details_list_hypotheses_then_guardrails_in_contract_order() {
        let contract = four_criteria();
        let out = run(Verify, &contract, 30, &[], NO_VIOLATIONS);
        let order: Vec<(&str, CriterionKind)> = out.details.iter().map(|d| (d.criterion.as_str(), d.kind)).collect();
        assert_eq!(order, [("latency", HYP), ("throughput", HYP), ("errors", GUARD), ("saturation", GUARD)]);
    }

    /// Builds the records and the violated list for one state of each named criterion of
    /// `four_criteria`: a hypothesis met, not met or insufficient; a guardrail ok, violated or
    /// insufficient.
    fn records_for(states: &[(&str, CriterionState)]) -> (Vec<Evidence>, Vec<String>) {
        let mut evidence = Vec::new();
        let mut violated = Vec::new();
        let mut next = 1;
        for &(name, state) in states {
            // Values that meet or miss each hypothesis, and a value inside each guardrail's limit.
            let (kind, met, not_met): (CriterionKind, f64, f64) = match name {
                "latency" => (HYP, 70.0, 90.0),
                "throughput" => (HYP, 120.0, 105.0),
                "errors" => (GUARD, 1.0, 1.0),
                _ => (GUARD, 50.0, 50.0),
            };
            let value = match state {
                CriterionState::Met => met,
                CriterionState::NotMet => not_met,
                CriterionState::Ok => met.min(not_met),
                CriterionState::Violated => {
                    violated.push(name.to_string());
                    continue;
                }
                CriterionState::Insufficient => continue,
            };
            evidence.push(observed(next, name, kind, value, Some(100.0)));
            next += 1;
        }
        (evidence, violated)
    }

    /// Evaluates `states` as final and checks that the setup produced the wanted states and that
    /// the outcome concludes.
    fn check_final(policy: ObservationPolicy, contract: &Contract, trigger: EvalTrigger, hours: i64, states: &[(&str, CriterionState)]) {
        let (evidence, violated) = records_for(states);
        let out = run_with(policy, contract, hours, trigger, &evidence, &violated);
        let got: Vec<CriterionState> = out.details.iter().map(|d| d.state).collect();
        let wanted: Vec<CriterionState> = states.iter().map(|&(_, state)| state).collect();
        assert_eq!(got, wanted, "{policy:?} {trigger:?} at {hours}h: the setup produced the wanted states");
        assert_ne!(out.verdict, Verdict::Insufficient, "{policy:?} {trigger:?} at {hours}h with {got:?} must conclude");
        assert!(out.confidence.is_some(), "{policy:?} {trigger:?} at {hours}h with {got:?} must carry a confidence");
    }

    #[test]
    fn a_final_evaluation_never_returns_insufficient() {
        use CriterionState as S;
        let hypothesis_states = [S::Met, S::NotMet, S::Insufficient];
        let guardrail_states = [S::Ok, S::Violated, S::Insufficient];
        // Final by request at 30h (inside the preferred window), and final by the clock at 80h.
        for (trigger, hours) in [(EvalTrigger::Final, 30), (EvalTrigger::Window, 80)] {
            for h1 in hypothesis_states {
                for h2 in hypothesis_states {
                    for g1 in guardrail_states {
                        for g2 in guardrail_states {
                            let states = [("latency", h1), ("throughput", h2), ("errors", g1), ("saturation", g2)];
                            check_final(Verify, &four_criteria(), trigger, hours, &states);
                        }
                    }
                }
            }
            for g1 in guardrail_states {
                for g2 in guardrail_states {
                    check_final(Guardrails, &guardrails_only(), trigger, hours, &[("errors", g1), ("saturation", g2)]);
                }
            }
        }
    }

    #[test]
    fn a_final_trigger_concludes_before_the_minimum_window() {
        let contract = verify_contract();
        let passing = [observed(1, "latency", HYP, 70.0, Some(100.0)), observed(2, "errors", GUARD, 1.0, Some(1.0))];
        let out = run_with(Verify, &contract, 10, EvalTrigger::Final, &passing, NO_VIOLATIONS);
        assert_eq!(out.verdict, Verdict::Confirmed, "a final request skips rule 1");
        assert_eq!(out.summary, "confirmed: 1/1 hypotheses met; guardrails: 1/1 ok, 0 violated; attribution: not assessed");

        let out = run_with(Verify, &contract, 10, EvalTrigger::Final, &passing, &ids(&["errors"]));
        assert_eq!(out.verdict, Verdict::Negative, "a violation concludes too once the evaluation is final");

        let failing = [observed(1, "latency", HYP, 90.0, Some(100.0)), observed(2, "errors", GUARD, 1.0, Some(1.0))];
        let out = run_with(Verify, &contract, 30, EvalTrigger::Final, &failing, NO_VIOLATIONS);
        assert_eq!(out.verdict, Verdict::Negative);
        assert_eq!(out.summary, "negative: 0/1 hypotheses met; guardrails: 1/1 ok, 0 violated; attribution: not assessed");
    }

    #[test]
    fn the_minimum_window_boundary_is_not_rule_one() {
        let contract = verify_contract();
        let evidence = [observed(1, "latency", HYP, 70.0, Some(100.0)), observed(2, "errors", GUARD, 1.0, Some(1.0))];
        let just_before = run(Verify, &contract, 23, &evidence, NO_VIOLATIONS);
        assert_eq!(just_before.summary, "insufficient: minimum window not reached");

        let at_minimum = run(Verify, &contract, 24, &evidence, NO_VIOLATIONS);
        assert_eq!(at_minimum.verdict, Verdict::Confirmed, "24h ends the minimum window, so rule 1 no longer applies");

        let violated = run(Verify, &contract, 24, &evidence, &ids(&["errors"]));
        assert_eq!(violated.verdict, Verdict::Negative);
    }

    #[test]
    fn the_preferred_window_boundary_is_final() {
        let contract = verify_contract();
        let evidence = [observed(1, "latency", HYP, 90.0, Some(100.0)), observed(2, "errors", GUARD, 1.0, Some(1.0))];
        let before = run(Verify, &contract, 71, &evidence, NO_VIOLATIONS);
        assert_eq!(before.verdict, Verdict::Insufficient);

        let at_preferred = run(Verify, &contract, 72, &evidence, NO_VIOLATIONS);
        assert_eq!(at_preferred.verdict, Verdict::Negative, "72h is the preferred window, so the evaluation is final");
    }

    #[test]
    fn max_and_min_expectations_compare_inclusively_with_exact_reasons() {
        let contract = contract(json!({
            "hypotheses": [
                { "id": "cap", "source": "prom", "query": "q", "expect": { "max": 50 } },
                { "id": "floor", "source": "prom", "query": "q", "expect": { "min": 5 } }
            ],
            "guardrails": [{ "id": "errors", "source": "prom", "query": "q", "max": 10 }],
            "observation": { "minimumWindow": "24h", "preferredWindow": "72h" }
        }));
        let passing = [observed(1, "cap", HYP, 50.0, None), observed(2, "floor", HYP, 5.0, None), observed(3, "errors", GUARD, 1.0, None)];
        let out = run(Verify, &contract, 30, &passing, NO_VIOLATIONS);
        assert_eq!(out.details[0].state, CriterionState::Met, "equal to max is met");
        assert_eq!(out.details[0].reason, "value 50 meets max 50");
        assert_eq!(out.details[1].state, CriterionState::Met, "equal to min is met");
        assert_eq!(out.details[1].reason, "value 5 meets min 5");
        assert_eq!(out.verdict, Verdict::Confirmed);

        let failing = [observed(1, "cap", HYP, 50.5, None), observed(2, "floor", HYP, 4.0, None), observed(3, "errors", GUARD, 1.0, None)];
        let out = run(Verify, &contract, 80, &failing, NO_VIOLATIONS);
        assert_eq!(out.details[0].state, CriterionState::NotMet);
        assert_eq!(out.details[0].reason, "value 50.5 exceeds max 50");
        assert_eq!(out.details[1].state, CriterionState::NotMet);
        assert_eq!(out.details[1].reason, "value 4 is below min 5");
        assert_eq!(out.verdict, Verdict::Negative);
    }

    #[test]
    fn a_min_change_is_not_violated_at_its_threshold() {
        let absolute = guardrail(None, None, None, Some("-0.5"));
        assert_eq!(check_guardrail(&absolute, &observed(1, "g", GUARD, 9.5, Some(10.0))), None, "equal to the threshold");
        assert_eq!(check_guardrail(&absolute, &observed(1, "g", GUARD, 9.4, Some(10.0))).as_deref(), Some("change -0.6 is below -0.5"));

        let relative = guardrail(None, None, None, Some("-25%"));
        assert_eq!(check_guardrail(&relative, &observed(1, "g", GUARD, 75.0, Some(100.0))), None, "equal to -25%");
        assert_eq!(check_guardrail(&relative, &observed(1, "g", GUARD, 74.0, Some(100.0))).as_deref(), Some("change -26% is below -25%"));
    }

    #[test]
    fn a_change_threshold_that_does_not_parse_never_violates() {
        let unsigned = guardrail(None, None, Some("20%"), None);
        assert_eq!(check_guardrail(&unsigned, &observed(1, "g", GUARD, 100.0, Some(0.0))), None);
        let bad_min = guardrail(None, None, None, Some("-1e3"));
        assert_eq!(check_guardrail(&bad_min, &observed(1, "g", GUARD, 100.0, Some(0.0))), None);
    }

    #[test]
    fn an_unusable_expectation_is_insufficient_instead_of_a_panic() {
        let contract = contract(json!({
            "hypotheses": [
                { "id": "latency", "source": "prom", "query": "q", "expect": { "change": "20%" } },
                { "id": "cap", "source": "prom", "query": "q", "expect": {} }
            ],
            "guardrails": [{ "id": "errors", "source": "prom", "query": "q", "maxChange": "+0.1pp" }],
            "observation": { "minimumWindow": "24h", "preferredWindow": "72h" }
        }));
        let evidence = [
            observed(1, "latency", HYP, 70.0, Some(100.0)),
            observed(2, "cap", HYP, 10.0, None),
            observed(3, "errors", GUARD, 1.0, Some(1.0)),
        ];
        let out = run(Verify, &contract, 80, &evidence, NO_VIOLATIONS);
        assert_eq!(out.details[0].state, CriterionState::Insufficient);
        assert_eq!(out.details[0].reason, "expectation is invalid: \"20%\" must start with + or -, such as -20% or +0.1pp");
        assert_eq!(out.details[0].evidence, Some(1));
        assert_eq!(out.details[1].state, CriterionState::Insufficient);
        assert_eq!(out.details[1].reason, "expectation is invalid: exactly one of change, max, min is required");
        assert_eq!(out.verdict, Verdict::Inconclusive);
    }

    #[test]
    fn a_contract_whose_windows_do_not_resolve_is_insufficient() {
        let mut unparseable = verify_contract();
        unparseable.observation.minimum_window = "soon".into();
        // Parses, but the minimum window cannot be placed on the timeline after the anchor.
        let mut overflowing = verify_contract();
        overflowing.observation.minimum_window = "999999999d".into();
        let evidence = [observed(1, "latency", HYP, 70.0, Some(100.0)), observed(2, "errors", GUARD, 1.0, Some(1.0))];
        for contract in [&unparseable, &overflowing] {
            let out = run_with(Verify, contract, 80, EvalTrigger::Final, &evidence, NO_VIOLATIONS);
            assert_eq!(out.verdict, Verdict::Insufficient);
            assert_eq!(out.confidence, None);
            assert_eq!(out.summary, "insufficient: contract is invalid");
            assert!(out.details.is_empty());
        }
    }

    #[test]
    fn a_violated_guardrail_has_no_evidence_record_and_its_negative_is_high() {
        let contract = verify_contract();
        let evidence = [observed(1, "latency", HYP, 70.0, Some(100.0)), observed(4, "errors", GUARD, 1.0, Some(1.0))];
        let out = run(Verify, &contract, 30, &evidence, &ids(&["errors"]));
        assert_eq!(out.details[1].state, CriterionState::Violated);
        assert_eq!(out.details[1].evidence, None, "the latest record is not the violating one");
        assert_eq!(out.details[1].reason, "an incident was opened for this guardrail");
        assert_eq!(out.confidence, Some(Confidence::High));
    }

    #[test]
    fn a_violation_is_high_even_when_a_reported_hypothesis_is_not_met() {
        let contract = verify_contract();
        let mut reported = observed(1, "latency", HYP, 90.0, Some(100.0));
        reported.trust = TrustLevel::Reported;
        let out = run(Verify, &contract, 30, &[reported], &ids(&["errors"]));
        assert_eq!(out.verdict, Verdict::Negative);
        assert_eq!(out.confidence, Some(Confidence::High));
    }

    #[test]
    fn a_final_negative_from_a_reported_hypothesis_is_medium() {
        let contract = verify_contract();
        let mut reported = observed(1, "latency", HYP, 90.0, Some(100.0));
        reported.trust = TrustLevel::Reported;
        let evidence = [reported, observed(2, "errors", GUARD, 1.0, Some(1.0))];
        let out = run(Verify, &contract, 80, &evidence, NO_VIOLATIONS);
        assert_eq!(out.verdict, Verdict::Negative);
        assert_eq!(out.confidence, Some(Confidence::Medium));
    }

    #[test]
    fn a_baseline_record_neither_decides_a_hypothesis_nor_makes_a_guardrail_ok() {
        let contract = verify_contract();
        let evidence = [
            record(1, "latency", HYP, EvidencePhase::Baseline, 70.0, None),
            record(2, "errors", GUARD, EvidencePhase::Baseline, 1.0, None),
        ];
        let out = run(Verify, &contract, 80, &evidence, NO_VIOLATIONS);
        assert_eq!(out.details[0].state, CriterionState::Insufficient);
        assert_eq!(out.details[0].reason, "no usable evidence");
        assert_eq!(out.details[1].state, CriterionState::Insufficient);
        assert_eq!(out.details[1].reason, "never measured");
        assert_eq!(out.verdict, Verdict::Inconclusive);
    }

    #[test]
    fn an_ok_guardrail_is_within_its_limits() {
        let contract = verify_contract();
        let evidence = [observed(1, "latency", HYP, 70.0, Some(100.0)), observed(2, "errors", GUARD, 1.0, Some(1.0))];
        let out = run(Verify, &contract, 30, &evidence, NO_VIOLATIONS);
        assert_eq!(out.details[1].state, CriterionState::Ok);
        assert_eq!(out.details[1].reason, "within its limits");
    }
}
