//! Central Factory Analytics definitions. Rebuilt from authoritative project
//! events/usage on each read; unknown telemetry is never converted into success.
use crate::spend::Prices;
use chrono::{DateTime, Utc};
use genie_core::intelligence::{RoutingPlan, SignalProfile};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Metric {
    pub value: Option<f64>,
    pub numerator: f64,
    pub denominator: usize,
    pub eligible: usize,
    pub missing: usize,
    pub unit: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FactoryAttempt {
    pub id: i64,
    pub started_at: String,
    pub result: Option<String>,
    pub escalated: Option<bool>,
    pub human_escalated: Option<bool>,
    pub cost_usd: Option<f64>,
    pub routing_ids: Vec<i64>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FactoryCost {
    pub attempt_id: Option<i64>,
    pub stage: String,
    pub role: Option<String>,
    pub model: String,
    pub calls: usize,
    pub cost_usd: Option<f64>,
    pub known_cost_usd: f64,
    pub unpriced_tokens: u64,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FactoryTask {
    pub task: String,
    pub title: String,
    pub result: String,
    pub delivered_at: String,
    pub attempt_ids: Vec<i64>,
    pub attempts: Vec<FactoryAttempt>,
    pub first_pass_accepted: Option<bool>,
    pub review_rounds: usize,
    pub retries: Option<usize>,
    pub human_interventions: Option<usize>,
    pub owner_clarifications: usize,
    pub escalations: Option<usize>,
    pub execution_hours: Option<f64>,
    pub active_hours: Option<f64>,
    pub waiting_hours: Option<f64>,
    pub cost_usd: Option<f64>,
    pub known_cost_usd: f64,
    pub unpriced_tokens: u64,
    /// Historical daily usage cannot be losslessly attributed to execution attempts.
    pub attempt_cost_coverage: String,
    pub costs: Vec<FactoryCost>,
    pub initial_topology: Option<String>,
    pub final_topology: Option<String>,
    pub models: Vec<String>,
    pub roles: Vec<String>,
    pub policy_versions: Vec<String>,
    pub profile_versions: Vec<String>,
    pub router_versions: Vec<String>,
    pub reason_codes: Vec<String>,
    pub initial_signals: BTreeMap<String, String>,
    pub snapshot_ids: Vec<i64>,
    pub routing_ids: Vec<i64>,
    pub missing: Vec<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FactoryCohort {
    pub dimension: String,
    pub key: String,
    pub tasks: Vec<String>,
    pub metrics: BTreeMap<String, Metric>,
    pub small_sample: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FactoryAnalytics {
    pub schema_version: u32,
    pub since: String,
    pub until: String,
    pub definition: String,
    pub metrics: BTreeMap<String, Metric>,
    pub tasks: Vec<FactoryTask>,
    pub cohorts: Vec<FactoryCohort>,
    pub limitations: Vec<String>,
}
fn date(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Utc))
}
fn elapsed(a: &str, b: &str) -> Option<f64> {
    let n = (date(b)? - date(a)?).num_milliseconds();
    (n >= 0).then_some(n as f64 / 3_600_000.0)
}
fn ratio(values: impl Iterator<Item = Option<bool>>, eligible: usize) -> Metric {
    let observed: Vec<_> = values.flatten().collect();
    let n = observed.iter().filter(|v| **v).count();
    Metric {
        value: (!observed.is_empty()).then(|| n as f64 / observed.len() as f64),
        numerator: n as f64,
        denominator: observed.len(),
        eligible,
        missing: eligible - observed.len(),
        unit: "fraction".into(),
    }
}
fn distribution(values: impl Iterator<Item = Option<f64>>, eligible: usize, q: f64, unit: &str) -> Metric {
    let mut observed: Vec<_> = values.flatten().collect();
    observed.sort_by(f64::total_cmp);
    let value = if observed.is_empty() {
        None
    } else {
        let rank = ((q * observed.len() as f64).ceil() as usize).clamp(1, observed.len());
        Some(observed[rank - 1])
    };
    Metric {
        value,
        numerator: observed.iter().sum(),
        denominator: observed.len(),
        eligible,
        missing: eligible - observed.len(),
        unit: unit.into(),
    }
}
pub fn metrics(rows: &[&FactoryTask]) -> BTreeMap<String, Metric> {
    let n = rows.len();
    let delivered = rows.iter().filter(|r| r.result == "accepted").count();
    let mut out = BTreeMap::new();
    out.insert(
        "deliveredTasks".into(),
        Metric {
            value: Some(delivered as f64),
            numerator: delivered as f64,
            denominator: n,
            eligible: n,
            missing: 0,
            unit: "tasks".into(),
        },
    );
    out.insert("firstPassAcceptance".into(), ratio(rows.iter().map(|r| r.first_pass_accepted), n));
    out.insert("retryRate".into(), ratio(rows.iter().map(|r| r.retries.map(|v| v > 0)), n));
    let attempts: Vec<_> = rows.iter().flat_map(|r| &r.attempts).collect();
    out.insert("escalationRate".into(), ratio(attempts.iter().map(|a| a.escalated), attempts.len()));
    out.insert("humanEscalationRate".into(), ratio(attempts.iter().map(|a| a.human_escalated), attempts.len()));
    for result in ["accepted", "rejected", "failed", "cancelled"] {
        out.insert(
            format!("attempt_{}_rate", result),
            ratio(attempts.iter().map(|a| a.result.as_deref().filter(|s| *s != "unknown").map(|value| value == result)), attempts.len()),
        );
    }
    out.insert("attemptTelemetryCoverage".into(), ratio(rows.iter().map(|r| Some(!r.attempts.is_empty())), n));
    out.insert("humanInterventionRate".into(), ratio(rows.iter().map(|r| r.human_interventions.map(|v| v > 0)), n));
    out.insert("ownerClarificationRate".into(), ratio(rows.iter().map(|r| Some(r.owner_clarifications > 0)), n));
    out.insert("acceptanceRate".into(), ratio(rows.iter().map(|r| Some(r.result == "accepted")), n));
    let delivery: Vec<_> = rows.iter().filter(|r| r.result == "accepted").collect();
    out.insert("executionHoursMedian".into(), distribution(delivery.iter().map(|r| r.execution_hours), delivery.len(), 0.5, "hours"));
    out.insert("executionHoursP90".into(), distribution(delivery.iter().map(|r| r.execution_hours), delivery.len(), 0.9, "hours"));
    out.insert("activeHoursMedian".into(), distribution(delivery.iter().map(|r| r.active_hours), delivery.len(), 0.5, "hours"));
    out.insert("waitingHoursMedian".into(), distribution(delivery.iter().map(|r| r.waiting_hours), delivery.len(), 0.5, "hours"));
    let completed = rows.iter().filter(|r| r.first_pass_accepted.is_some()).collect::<Vec<_>>();
    out.insert("reviewRoundsMedian".into(), distribution(completed.iter().map(|r| Some(r.review_rounds as f64)), n, 0.5, "rounds"));
    // All attributable work of the cohort, including rejected/cancelled attempts
    // and work before the report window. No partial sum is labelled a full cost.
    let known = rows.iter().filter_map(|r| r.cost_usd).sum::<f64>();
    let coverage = rows.iter().filter(|r| r.cost_usd.is_some()).count();
    out.insert(
        "costPerAcceptedTask".into(),
        Metric {
            value: (coverage == n && delivered > 0).then(|| known / delivered as f64),
            numerator: known,
            denominator: delivered,
            eligible: n,
            missing: n - coverage,
            unit: "USD".into(),
        },
    );
    // Daily historical usage is not split into failed retries. A cancelled-only
    // task's complete task cost can be attributed; mixed attempts remain unknown.
    let attributable: Vec<_> = rows
        .iter()
        .filter_map(|r| {
            if r.attempt_cost_coverage != "complete" || r.attempts.is_empty() {
                return None;
            }
            let mut total = 0.0;
            for a in &r.attempts {
                match a.result.as_deref() {
                    Some("accepted") => {}
                    Some("cancelled" | "failed" | "rejected") => total += a.cost_usd?,
                    _ => return None,
                }
            }
            Some(total)
        })
        .collect();
    out.insert(
        "failedWorkCost".into(),
        Metric {
            value: (n > 0 && attributable.len() == n).then(|| attributable.iter().sum()),
            numerator: attributable.iter().sum(),
            denominator: attributable.len(),
            eligible: n,
            missing: n - attributable.len(),
            unit: "USD".into(),
        },
    );
    out
}
fn table(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)", [name], |r| r.get(0))
}
fn json_value<T: serde::de::DeserializeOwned>(text: String) -> rusqlite::Result<T> {
    serde_json::from_str(&text).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
}

fn task_costs(
    conn: &Connection,
    usage: &[genie_core::usage::UsageRow],
    prices: &Prices,
    row: &mut FactoryTask,
    until: &str,
) -> rusqlite::Result<()> {
    use genie_core::usage::Tokens;
    let task = &row.task;
    let daily: Vec<_> = usage.iter().filter(|u| u.task.as_deref() == Some(task.as_str())).collect();
    let aggregate_calls: u64 = daily.iter().map(|u| u.calls).sum();
    let aggregate_tokens = daily.iter().fold(Tokens::default(), |mut sum, u| {
        sum.add(&u.tokens);
        sum
    });
    let mut complete = false;
    if table(conn, "usage_attributions")? {
        let total:(i64,Tokens)=conn.query_row("SELECT COUNT(*),COALESCE(SUM(input),0),COALESCE(SUM(output),0),COALESCE(SUM(cache_read),0),COALESCE(SUM(cache_write),0) FROM usage_attributions WHERE task=?1",[task],|r|Ok((r.get(0)?,Tokens {input:r.get::<_,i64>(1)?.max(0) as u64,output:r.get::<_,i64>(2)?.max(0) as u64,cache_read:r.get::<_,i64>(3)?.max(0) as u64,cache_write:r.get::<_,i64>(4)?.max(0) as u64})))?;
        complete = aggregate_calls > 0 && aggregate_calls == total.0 as u64 && aggregate_tokens == total.1;
        let mut q=conn.prepare("SELECT attempt_id,stage,role,model,COUNT(*),SUM(input),SUM(output),SUM(cache_read),SUM(cache_write) FROM usage_attributions WHERE task=?1 AND at<=?2 GROUP BY attempt_id,stage,role,model")?;
        for item in q.query_map(params![task, until], |r| {
            Ok((
                r.get::<_, Option<i64>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)? as usize,
                Tokens {
                    input: r.get::<_, i64>(5)?.max(0) as u64,
                    output: r.get::<_, i64>(6)?.max(0) as u64,
                    cache_read: r.get::<_, i64>(7)?.max(0) as u64,
                    cache_write: r.get::<_, i64>(8)?.max(0) as u64,
                },
            ))
        })? {
            let (attempt_id, stage, role, model, calls, tokens) = item?;
            let (cost, unpriced) = crate::config::price_of(prices, &model).map_or((0.0, tokens.total()), |price| price.split(&tokens));
            row.costs.push(FactoryCost {
                attempt_id,
                stage,
                role,
                model,
                calls,
                cost_usd: (unpriced == 0).then_some(cost),
                known_cost_usd: cost,
                unpriced_tokens: unpriced,
            });
        }
    }
    if complete {
        row.attempt_cost_coverage = "complete".into();
        row.known_cost_usd = row.costs.iter().map(|c| c.known_cost_usd).sum();
        row.unpriced_tokens = row.costs.iter().map(|c| c.unpriced_tokens).sum();
        row.cost_usd = (row.unpriced_tokens == 0).then_some(row.known_cost_usd);
        for attempt in &mut row.attempts {
            let costs: Vec<_> = row.costs.iter().filter(|c| c.attempt_id == Some(attempt.id)).collect();
            if costs.iter().all(|c| c.cost_usd.is_some()) {
                attempt.cost_usd = Some(costs.iter().filter_map(|c| c.cost_usd).sum());
            }
        }
    } else {
        let cutoff_day = &until[..10];
        let ambiguous = daily.iter().any(|u| u.day == cutoff_day);
        let observed: Vec<_> = daily.iter().filter(|u| u.day.as_str() < cutoff_day).collect();
        for u in &observed {
            let (cost, unpriced) = crate::config::price_of(prices, &u.model).map_or((0.0, u.tokens.total()), |p| p.split(&u.tokens));
            row.known_cost_usd += cost;
            row.unpriced_tokens += unpriced;
        }
        // A legacy daily bucket cannot say which side of an intraday cutoff a
        // call occurred on. Only exact new evidence contributes on that day.
        if !ambiguous && !observed.is_empty() && row.unpriced_tokens == 0 {
            row.cost_usd = Some(row.known_cost_usd);
        }
        if ambiguous {
            row.missing.push("intraday_usage_attribution".into());
        }
        if !row.costs.is_empty() {
            row.attempt_cost_coverage = "partial".into();
        }
    }
    Ok(())
}

pub fn collect(conn: &Connection, since: &str, until: &str, prices: &Prices) -> rusqlite::Result<FactoryAnalytics> {
    // One read snapshot reconciles accounting and attribution while agents append.
    let _read = conn.is_autocommit().then(|| conn.unchecked_transaction()).transpose()?;
    let has_intelligence = table(conn, "execution_attempts")?;
    // Last observed terminal delivery per task at the report cutoff. Reopened
    // tasks remain in the cohort for their delivered work; tasks are counted once.
    let mut q=conn.prepare("SELECT t.id,t.title,e.id,e.at,json_extract(e.payload,'$.to') FROM tasks t JOIN events e ON e.id=(SELECT MAX(n.id) FROM events n WHERE n.subject=t.id AND n.type='task.status_changed' AND n.at<=?2 AND json_extract(n.payload,'$.to') IN ('done','cancelled')) WHERE t.type!='epic' AND e.at>=?1 ORDER BY e.at,t.id")?;
    let delivered = q
        .query_map(params![since, until], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let usage = genie_core::usage::usage_since(conn, "")?;
    let mut tasks = Vec::new();
    for (task, title, end_event, end, status) in delivered {
        let mut row = FactoryTask {
            task: task.clone(),
            title,
            result: if status == "done" { "accepted".into() } else { "cancelled".into() },
            delivered_at: end.clone(),
            attempt_cost_coverage: "not_measured".into(),
            ..Default::default()
        };
        let mut q = conn.prepare("SELECT id,at,actor_role,type,payload FROM events WHERE subject=?1 AND id<=?2 ORDER BY id")?;
        let events = q
            .query_map(params![task, end_event], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    json_value::<serde_json::Value>(r.get(4)?)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut review_pending = false;
        let mut first_verdict = None;
        let mut interventions = 0;
        let mut state = None;
        let mut cursor = None;
        let mut active = 0.0;
        let mut waiting = 0.0;
        let mut known_intervals = true;
        for (_, at, actor, kind, payload) in &events {
            if kind == "task.status_changed" {
                let to = payload["to"].as_str().unwrap_or("");
                let from = payload["from"].as_str().unwrap_or("");
                if to == "review" {
                    review_pending = true;
                }
                if review_pending && matches!(to, "approved" | "changes_requested") {
                    row.review_rounds += 1;
                    first_verdict.get_or_insert(to == "approved");
                    review_pending = false;
                }
                if to == "needs_owner" && actor != "human" {
                    row.owner_clarifications += 1;
                }
                if actor == "human" && (to == "changes_requested" || from == "needs_owner") {
                    interventions += 1;
                }
                if let Some(previous) = cursor.as_deref() {
                    if let Some(duration) = elapsed(previous, at) {
                        if state == Some("in_progress".to_string()) {
                            active += duration;
                        } else {
                            waiting += duration;
                        }
                    } else {
                        known_intervals = false;
                    }
                }
                if to == "in_progress" || cursor.is_some() {
                    cursor = Some(at.clone());
                    state = Some(to.to_string());
                }
                // Read from/to, not a person's free-form note, for durations.
                if from.is_empty() {
                    known_intervals = false;
                }
            }
        }
        row.first_pass_accepted = first_verdict.map(|accepted| accepted && row.result == "accepted");
        if cursor.is_some() && known_intervals {
            row.active_hours = Some(active);
            row.waiting_hours = Some(waiting);
        }
        row.human_interventions = (!events.is_empty()).then_some(interventions);
        if has_intelligence {
            let mut q = conn.prepare("SELECT id,started_at FROM execution_attempts WHERE task=?1 AND started_at<=?2 ORDER BY ordinal")?;
            let attempts = q
                .query_map(params![task, until], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            row.attempt_ids = attempts.iter().map(|(id, _)| *id).collect();
            if let Some((_, start)) = attempts.first() {
                row.execution_hours = elapsed(start, &end);
                let mut status = events
                    .iter()
                    .filter(|(_, at, _, kind, _)| kind == "task.status_changed" && at <= start)
                    .next_back()
                    .and_then(|(_, _, _, _, p)| p["to"].as_str())
                    .map(str::to_string);
                let mut at = start.as_str();
                let mut active = 0.0;
                let mut waiting = 0.0;
                let mut measured = status.is_some();
                for (_, next, _, _, payload) in
                    events.iter().filter(|(_, at, _, kind, _)| kind == "task.status_changed" && at > start && at <= &end)
                {
                    if let Some(duration) = elapsed(at, next) {
                        if status.as_deref() == Some("in_progress") {
                            active += duration;
                        } else {
                            waiting += duration;
                        }
                    } else {
                        measured = false;
                    }
                    status = payload["to"].as_str().map(str::to_string);
                    at = next;
                }
                if let Some(duration) = elapsed(at, &end) {
                    if status.as_deref() == Some("in_progress") {
                        active += duration;
                    } else {
                        waiting += duration;
                    }
                } else {
                    measured = false;
                }
                row.active_hours = measured.then_some(active);
                row.waiting_hours = measured.then_some(waiting);
                row.retries = Some(attempts.len().saturating_sub(1));
            }
            let mut q = conn.prepare("SELECT id,data FROM task_signal_snapshots WHERE task=?1 AND created_at<=?2 ORDER BY revision")?;
            let snapshots = q
                .query_map(params![task, until], |r| Ok((r.get::<_, i64>(0)?, json_value::<SignalProfile>(r.get(1)?)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            row.snapshot_ids = snapshots.iter().map(|(id, _)| *id).collect();
            row.profile_versions = snapshots.iter().map(|(_, s)| s.project_profile_version.clone()).collect();
            row.profile_versions.sort();
            row.profile_versions.dedup();
            if let Some((_, profile)) = snapshots.first() {
                row.initial_signals = profile
                    .signals
                    .iter()
                    .map(|s| (serde_json::to_value(s.name).unwrap().as_str().unwrap().to_string(), s.value.clone()))
                    .collect();
            }
            let mut q = conn.prepare("SELECT id,data FROM routing_decisions WHERE task=?1 AND created_at<=?2 ORDER BY id")?;
            let routing = q
                .query_map(params![task, until], |r| Ok((r.get::<_, i64>(0)?, json_value::<RoutingPlan>(r.get(1)?)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            row.routing_ids = routing.iter().map(|(id, _)| *id).collect();
            row.initial_topology = routing.first().map(|(_, r)| r.selected_template.clone().unwrap_or_else(|| "custom".into()));
            row.final_topology = routing.last().map(|(_, r)| {
                if r.reason_codes.iter().any(|c| c == "topology_upgrade") {
                    "custom".into()
                } else {
                    r.selected_template.clone().unwrap_or_else(|| "custom".into())
                }
            });
            if !routing.is_empty() {
                row.escalations = Some(routing.iter().filter(|(_, r)| r.reason_codes.iter().any(|c| c == "topology_upgrade")).count());
            }
            for (id, started_at) in &attempts {
                let result: Option<String> = conn
                    .query_row(
                        "SELECT result FROM execution_assessments WHERE attempt_id=?1 AND created_at<=?2 ORDER BY revision DESC LIMIT 1",
                        params![id, until],
                        |r| r.get(0),
                    )
                    .optional()?;
                let mut q = conn.prepare("SELECT id, data FROM routing_decisions WHERE attempt_id=?1 AND created_at<=?2 ORDER BY id")?;
                let decisions = q
                    .query_map(params![id, until], |r| Ok((r.get::<_, i64>(0)?, json_value::<RoutingPlan>(r.get(1)?)?)))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                let mut human_upgrade = false;
                for (routing_id, decision) in &decisions {
                    if decision.reason_codes.iter().any(|c| c == "topology_upgrade") {
                        human_upgrade |= conn.query_row("SELECT EXISTS(SELECT 1 FROM events WHERE subject=?1 AND type='task.routing_decided' AND actor_role='human' AND json_extract(payload,'$.routingId')=?2)",params![task,routing_id],|r|r.get::<_,bool>(0))?;
                    }
                }
                row.attempts.push(FactoryAttempt {
                    id: *id,
                    started_at: started_at.clone(),
                    result,
                    escalated: (!decisions.is_empty())
                        .then(|| decisions.iter().any(|(_, d)| d.reason_codes.iter().any(|c| c == "topology_upgrade"))),
                    human_escalated: (!decisions.is_empty()).then_some(human_upgrade),
                    cost_usd: None,
                    routing_ids: decisions.iter().map(|(id, _)| *id).collect(),
                });
            }
            for (_, r) in routing {
                row.policy_versions.push(r.policy_version);
                row.router_versions.push(r.router_version);
                row.reason_codes.extend(r.reason_codes);
                for m in r.selected_models {
                    row.roles.push(m.role);
                    if let Some(model) = m.model {
                        row.models.push(model);
                    }
                }
            }
            for v in [&mut row.policy_versions, &mut row.router_versions, &mut row.reason_codes, &mut row.models, &mut row.roles] {
                v.sort();
                v.dedup();
            }
        }
        task_costs(conn, &usage, prices, &mut row, until)?;
        if row.first_pass_accepted.is_none() {
            row.missing.push("completed_first_review".into());
        }
        if row.attempt_ids.is_empty() {
            row.missing.push("execution_attempts".into());
        }
        if row.routing_ids.is_empty() {
            row.missing.push("routing_history".into());
        }
        if row.cost_usd.is_none() {
            row.missing.push("complete_priced_usage".into());
        }
        if row.attempt_cost_coverage != "complete" {
            row.missing.push("per_attempt_cost_attribution".into());
        }
        tasks.push(row);
    }
    let mut groups: BTreeMap<(String, String), BTreeSet<usize>> = BTreeMap::new();
    for (i, row) in tasks.iter().enumerate() {
        let mut dimensions = vec![
            ("initial_topology".into(), row.initial_topology.clone().unwrap_or_else(|| "unknown".into())),
            ("final_topology".into(), row.final_topology.clone().unwrap_or_else(|| "unknown".into())),
        ];
        for (dimension, values) in [
            ("model", &row.models),
            ("role", &row.roles),
            ("policy_version", &row.policy_versions),
            ("profile_version", &row.profile_versions),
            ("router_version", &row.router_versions),
            ("reason", &row.reason_codes),
        ] {
            for value in values {
                if !value.starts_with("reassess:") {
                    dimensions.push((dimension.into(), value.clone()));
                }
            }
        }
        for (name, value) in &row.initial_signals {
            dimensions.push((format!("signal:{name}"), value.clone()));
        }
        let signal_key = ["complexity", "ambiguity", "blast_radius", "verification"]
            .iter()
            .map(|name| format!("{name}={}", row.initial_signals.get(*name).map_or("unknown", String::as_str)))
            .collect::<Vec<_>>()
            .join("; ");
        dimensions.push(("comparable_routing".into(), format!("{}; {signal_key}", row.initial_topology.as_deref().unwrap_or("unknown"))));
        for version in &row.policy_versions {
            dimensions.push(("policy_by_signals".into(), format!("{version}; {signal_key}")));
        }
        for dimension in dimensions {
            groups.entry(dimension).or_default().insert(i);
        }
    }
    let cohorts = groups
        .into_iter()
        .map(|((dimension, key), indexes)| {
            let rows: Vec<_> = indexes.iter().map(|i| &tasks[*i]).collect();
            FactoryCohort {
                dimension,
                key,
                tasks: rows.iter().map(|r| r.task.clone()).collect(),
                metrics: metrics(&rows),
                small_sample: rows.len() < 20,
            }
        })
        .collect();
    Ok(FactoryAnalytics {schema_version:1,since:since.into(),until:until.into(),definition:"Last observed terminal delivery per task within the window; quality and cost include all prior task-attributable work. Observational cohorts are descriptive, not causal.".into(),metrics:metrics(&tasks.iter().collect::<Vec<_>>()),tasks,cohorts,limitations:vec!["Historical daily usage has no lossless per-attempt or lifecycle-stage attribution; new response attribution is recorded transactionally.".into(),"No production outcome monitoring; absence of incidents is not measured success.".into(),"Costs use current configured prices; unknown prices and absent usage remain missing.".into(),"Cohorts below 20 tasks are flagged as small samples, not ranked.".into()]})
}

#[cfg(test)]
mod tests {
    use super::*;
    use genie_core::{
        Actor, CreateInput, Role, Status, StatusOptions, Tracker,
        usage::{ModelPrice, Tokens},
    };
    fn to(t: &Tracker, id: &str, status: Status) {
        t.set_status(&Actor::new("owner", Role::Human), id, status, StatusOptions { force: true, ..Default::default() }).unwrap();
    }
    #[test]
    fn unknown_reviews_are_excluded_and_all_cohort_costs_include_cancelled_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let t = Tracker::init(dir.path(), None, None).unwrap();
        let actor = Actor::new("owner", Role::Human);
        let first = t.create(&actor, CreateInput { title: "Accepted after retry".into(), ..Default::default() }).unwrap().id;
        to(&t, &first, Status::InProgress);
        t.record_usage("first/executor", Some(&first), "fake/model", &Tokens { input: 1_000_000, ..Default::default() }).unwrap();
        to(&t, &first, Status::Cancelled);
        to(&t, &first, Status::InProgress);
        to(&t, &first, Status::Review);
        to(&t, &first, Status::ChangesRequested);
        to(&t, &first, Status::InProgress);
        to(&t, &first, Status::Review);
        to(&t, &first, Status::Approved);
        to(&t, &first, Status::Done);
        t.record_usage("second/reviewer", Some(&first), "fake/model", &Tokens { input: 2_000_000, ..Default::default() }).unwrap();
        let second = t.create(&actor, CreateInput { title: "No completed review".into(), ..Default::default() }).unwrap().id;
        to(&t, &second, Status::Done);
        t.record_usage("solo/executor", Some(&second), "fake/model", &Tokens { input: 1_000_000, ..Default::default() }).unwrap();
        let cancelled = t.create(&actor, CreateInput { title: "Discarded work".into(), ..Default::default() }).unwrap().id;
        to(&t, &cancelled, Status::InProgress);
        to(&t, &cancelled, Status::Cancelled);
        t.record_usage("failed/executor", Some(&cancelled), "fake/model", &Tokens { input: 2_000_000, ..Default::default() }).unwrap();
        let prices = BTreeMap::from([("fake/model".into(), ModelPrice { input: 1.0, ..Default::default() })]);
        let report = collect(t.conn(), "2000-01-01T00:00:00Z", "2100-01-01T00:00:00Z", &prices).unwrap();
        assert_eq!(report.tasks.len(), 3);
        assert_eq!(report.metrics["deliveredTasks"].value, Some(2.0));
        let acceptance = &report.metrics["firstPassAcceptance"];
        assert_eq!(acceptance.denominator, 1);
        assert_eq!(acceptance.missing, 2);
        assert_eq!(acceptance.value, Some(0.0));
        assert_eq!(report.metrics["costPerAcceptedTask"].numerator, 6.0);
        assert_eq!(report.metrics["costPerAcceptedTask"].value, Some(3.0));
        assert_eq!(report.metrics["failedWorkCost"].value, None, "mixed-attempt failed cost cannot be invented");
        assert_eq!(report.tasks.iter().find(|r| r.task == first).unwrap().attempts.len(), 2);
        let rebuilt = collect(t.conn(), "2000-01-01T00:00:00Z", "2100-01-01T00:00:00Z", &prices).unwrap();
        assert_eq!(serde_json::to_value(report).unwrap(), serde_json::to_value(rebuilt).unwrap());
    }
    #[test]
    fn empty_and_unpriced_samples_stay_unknown_and_quantiles_are_nearest_rank() {
        assert_eq!(ratio(std::iter::empty(), 0).value, None);
        assert_eq!(distribution([Some(1.0), None, Some(2.0), Some(9.0)].into_iter(), 4, 0.9, "hours").value, Some(9.0));
        let rows = [FactoryTask { result: "accepted".into(), known_cost_usd: 2.0, unpriced_tokens: 10, ..Default::default() }];
        let metrics = metrics(&rows.iter().collect::<Vec<_>>());
        assert_eq!(metrics["costPerAcceptedTask"].value, None);
        assert_eq!(metrics["costPerAcceptedTask"].missing, 1);
    }
    #[test]
    fn historical_cutoff_excludes_future_spend_and_future_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let t = Tracker::init(dir.path(), None, None).unwrap();
        let task = t
            .create(&Actor::new("owner", Role::Human), CreateInput { title: "Delivered then reopened".into(), ..Default::default() })
            .unwrap()
            .id;
        to(&t, &task, Status::InProgress);
        t.record_usage("executor", Some(&task), "fake/model", &Tokens { input: 1_000_000, ..Default::default() }).unwrap();
        to(&t, &task, Status::Done);
        let boundary: i64 = t.conn().query_row("SELECT MAX(id) FROM events", [], |r| r.get(0)).unwrap();
        to(&t, &task, Status::InProgress);
        t.record_usage("executor", Some(&task), "fake/model", &Tokens { input: 9_000_000, ..Default::default() }).unwrap();
        t.conn()
            .execute(
                "UPDATE events SET at=CASE WHEN id<=?1 THEN '2026-10-19T11:00:00.000Z' ELSE '2026-10-20T11:00:00.000Z' END",
                [boundary],
            )
            .unwrap();
        t.conn().execute("UPDATE execution_attempts SET started_at=CASE WHEN ordinal=1 THEN '2026-10-19T10:00:00.000Z' ELSE '2026-10-20T10:00:00.000Z' END",[]).unwrap();
        t.conn()
            .execute(
                "UPDATE usage_attributions SET at=CASE WHEN id=1 THEN '2026-10-19T10:30:00.000Z' ELSE '2026-10-20T10:30:00.000Z' END",
                [],
            )
            .unwrap();
        t.conn().execute("UPDATE execution_assessments SET created_at='2026-10-19T11:00:00.000Z'", []).unwrap();
        let prices = BTreeMap::from([("fake/model".into(), ModelPrice { input: 1.0, ..Default::default() })]);
        let monday = collect(t.conn(), "2026-10-19T00:00:00.000Z", "2026-10-19T23:59:59.999Z", &prices).unwrap();
        assert_eq!(monday.tasks[0].cost_usd, Some(1.0));
        assert_eq!(monday.tasks[0].attempts.len(), 1);
        assert_eq!(monday.metrics["costPerAcceptedTask"].value, Some(1.0));
        let tuesday = collect(t.conn(), "2026-10-19T00:00:00.000Z", "2026-10-20T23:59:59.999Z", &prices).unwrap();
        assert_eq!(tuesday.tasks[0].cost_usd, Some(10.0));
        assert_eq!(tuesday.tasks[0].attempts.len(), 2);
    }
}
