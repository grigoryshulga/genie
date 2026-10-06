//! Automations: rules "when <trigger> [where <filter>] then <steps>", their
//! durable runs and the pure helpers the engine uses (matching, templates,
//! durations, schedules).
//!
//! A rule is JSON:
//!
//! ```json
//! { "name": "Task closed — knowledge and changelog",
//!   "on": { "event": "task.status_changed", "where": { "to": "done", "task.type": ["task", "bug"] } },
//!   "limits": { "concurrency": 2, "maxRunsPerHour": 20 },
//!   "steps": [ { "id": "docs", "agent": { "role": "documenter", "goal": "…" } },
//!              { "id": "log", "changelog.add": { "group": "{{ steps.docs.output.changelog.group }}", "text": "…" } } ] }
//! ```
//!
//! Triggers: `event` (+ `where`), `schedule` (cron, 5 or 6 fields, `tz`), `manual`, `webhook`.
//! Runs are unique per `(automation, trigger key)`: an event starts a rule at most once.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::db::now;
use crate::error::{GenieError, Result};
use crate::server_db::ServerDb;

/// Step kinds the engine knows.
pub const STEP_KINDS: &[&str] = &[
    "task.status",
    "task.comment",
    "task.create",
    "task.update",
    "task.get",
    "task.ready",
    "notify",
    "agent",
    "team",
    "ask",
    "wake_orchestrator",
    "changelog.add",
    "release",
    "http",
    "wait",
];

pub const MAX_DEPTH: i64 = 3;

// --- spec ----------------------------------------------------------------------

/// Validate a rule; returns a list of problems (empty when valid).
pub fn validate(spec: &Value) -> Vec<String> {
    let mut p = Vec::new();
    if spec["name"].as_str().is_none_or(|n| n.trim().is_empty()) {
        p.push("name is required".into());
    }
    let on = &spec["on"];
    let kinds = ["event", "schedule", "manual", "webhook"].iter().filter(|k| !on[**k].is_null()).count();
    if kinds != 1 {
        p.push("on: exactly one of event, schedule, manual, webhook".into());
    }
    if let Some(expr) = on["schedule"].as_str()
        && let Err(e) = parse_cron(expr)
    {
        p.push(format!("on.schedule: {e}"));
    }
    if let Some(tz) = on["tz"].as_str()
        && chrono_tz::Tz::from_str(tz).is_err()
    {
        p.push(format!("on.tz: unknown time zone {tz}"));
    }
    if !on["where"].is_null() && !on["where"].is_object() {
        p.push("on.where must be an object".into());
    }
    let Some(steps) = spec["steps"].as_array().filter(|s| !s.is_empty()) else {
        p.push("steps: at least one step".into());
        return p;
    };
    let mut ids = std::collections::HashSet::new();
    for (i, s) in steps.iter().enumerate() {
        match step_kind(s) {
            Some(_) => {}
            None => p.push(format!("steps[{i}]: exactly one step kind of {}", STEP_KINDS.join(", "))),
        }
        let id = step_id(s, i);
        if !ids.insert(id.clone()) {
            p.push(format!("steps[{i}]: duplicate id {id}"));
        }
        if let Some(d) = s["timeout"].as_str()
            && parse_duration(d).is_none()
        {
            p.push(format!("steps[{i}].timeout: invalid duration {d}"));
        }
    }
    p
}

pub fn step_kind(step: &Value) -> Option<&'static str> {
    let found: Vec<&'static str> = STEP_KINDS.iter().copied().filter(|k| step.get(*k).is_some()).collect();
    (found.len() == 1).then(|| found[0])
}

pub fn step_id(step: &Value, idx: usize) -> String {
    step["id"].as_str().map(str::to_string).unwrap_or_else(|| format!("step{}", idx + 1))
}

/// `30s`, `15m`, `24h`, `2d`.
pub fn parse_duration(s: &str) -> Option<chrono::Duration> {
    let s = s.trim();
    let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = num.parse().ok()?;
    Some(match unit.trim() {
        "s" => chrono::Duration::seconds(n),
        "m" | "min" => chrono::Duration::minutes(n),
        "h" => chrono::Duration::hours(n),
        "d" => chrono::Duration::days(n),
        _ => return None,
    })
}

/// Standard 5-field cron (minute hour day month weekday) or 6 fields with seconds.
pub fn parse_cron(expr: &str) -> std::result::Result<cron::Schedule, String> {
    let fields = expr.split_whitespace().count();
    let full = match fields {
        5 => format!("0 {expr}"),
        6 | 7 => expr.to_string(),
        _ => return Err("expected 5 fields: minute hour day month weekday".into()),
    };
    cron::Schedule::from_str(&full).map_err(|e| e.to_string())
}

/// Most recent scheduled time in `(after, now]`, if any.
pub fn due_fire(expr: &str, tz: Option<&str>, after: DateTime<Utc>, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let sched = parse_cron(expr).ok()?;
    let tz: chrono_tz::Tz = tz.and_then(|t| chrono_tz::Tz::from_str(t).ok()).unwrap_or(chrono_tz::UTC);
    let mut last = None;
    for t in sched.after(&after.with_timezone(&tz)).take(10_000) {
        let t = t.with_timezone(&Utc);
        if t > now {
            break;
        }
        last = Some(t);
    }
    last
}

/// First scheduled time after `after` (what the engine sleeps until).
pub fn next_fire(expr: &str, tz: Option<&str>, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let sched = parse_cron(expr).ok()?;
    let tz: chrono_tz::Tz = tz.and_then(|t| chrono_tz::Tz::from_str(t).ok()).unwrap_or(chrono_tz::UTC);
    sched.after(&after.with_timezone(&tz)).next().map(|t| t.with_timezone(&Utc))
}

// --- matching and templates ------------------------------------------------------

/// Value at a dotted path (`task.type`, `steps.docs.output.pages.0`).
pub fn lookup<'a>(ctx: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = ctx;
    for part in path.split('.').filter(|p| !p.is_empty()) {
        cur = match cur {
            Value::Object(m) => m.get(part)?,
            Value::Array(a) => a.get(part.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

fn as_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn matches_one(actual: Option<&Value>, expected: &Value) -> bool {
    match expected {
        Value::Array(options) => options.iter().any(|o| matches_one(actual, o)),
        Value::Object(ops) => ops.iter().all(|(op, arg)| {
            let text = actual.map(as_text).unwrap_or_default();
            match op.as_str() {
                "not" => !matches_one(actual, arg),
                "exists" => actual.is_some_and(|a| !a.is_null()) == arg.as_bool().unwrap_or(true),
                "contains" => actual.is_some_and(|a| match a {
                    Value::Array(items) => items.iter().any(|i| as_text(i) == as_text(arg)),
                    other => as_text(other).contains(&as_text(arg)),
                }),
                "not_contains" => !actual.is_some_and(|a| match a {
                    Value::Array(items) => items.iter().any(|i| as_text(i) == as_text(arg)),
                    other => as_text(other).contains(&as_text(arg)),
                }),
                "gt" => {
                    text.parse::<f64>().ok().zip(arg.as_f64()).is_some_and(|(a, b)| a > b)
                        || (arg.is_string() && text.as_str() > arg.as_str().unwrap_or(""))
                }
                "lt" => {
                    text.parse::<f64>().ok().zip(arg.as_f64()).is_some_and(|(a, b)| a < b)
                        || (arg.is_string() && text.as_str() < arg.as_str().unwrap_or(""))
                }
                "prefix" => text.starts_with(&as_text(arg)),
                _ => false,
            }
        }),
        expected => actual.is_some_and(|a| a == expected || as_text(a) == as_text(expected)),
    }
}

/// Does the context satisfy every condition of a `where` filter?
pub fn matches(filter: &Value, ctx: &Value) -> bool {
    match filter {
        Value::Object(conds) => conds.iter().all(|(path, expected)| matches_one(lookup(ctx, path), expected)),
        Value::Null => true,
        _ => false,
    }
}

fn apply_filter(v: Value, filter: &str) -> Value {
    match filter.trim() {
        "length" => json!(match &v {
            Value::Array(a) => a.len(),
            Value::String(s) => s.chars().count(),
            Value::Object(o) => o.len(),
            Value::Null => 0,
            _ => 1,
        }),
        "json" => Value::String(v.to_string()),
        "lines" => Value::String(match &v {
            Value::Array(a) => a.iter().map(|x| format!("- {}", as_text(x))).collect::<Vec<_>>().join("\n"),
            other => as_text(other),
        }),
        "upper" => Value::String(as_text(&v).to_uppercase()),
        _ => v,
    }
}

fn eval(expr: &str, ctx: &Value) -> Value {
    let mut parts = expr.split('|');
    let path = parts.next().unwrap_or_default().trim();
    let mut v = if path.starts_with('"') && path.ends_with('"') && path.len() >= 2 {
        Value::String(path[1..path.len() - 1].to_string())
    } else {
        lookup(ctx, path).cloned().unwrap_or(Value::Null)
    };
    for f in parts {
        v = apply_filter(v, f);
    }
    v
}

/// Render `{{ path | filter }}` placeholders. A string that is exactly one
/// placeholder keeps the value's type (arrays stay arrays); objects and arrays
/// are rendered recursively.
pub fn render(template: &Value, ctx: &Value) -> Value {
    match template {
        Value::String(s) => {
            let t = s.trim();
            if t.starts_with("{{") && t.ends_with("}}") && t.matches("{{").count() == 1 {
                return eval(&t[2..t.len() - 2], ctx);
            }
            let mut out = String::new();
            let mut rest = s.as_str();
            while let Some(start) = rest.find("{{") {
                out.push_str(&rest[..start]);
                let after = &rest[start + 2..];
                let Some(end) = after.find("}}") else {
                    out.push_str(&rest[start..]);
                    rest = "";
                    break;
                };
                out.push_str(&as_text(&eval(&after[..end], ctx)));
                rest = &after[end + 2..];
            }
            out.push_str(rest);
            Value::String(out)
        }
        Value::Array(a) => Value::Array(a.iter().map(|x| render(x, ctx)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), render(v, ctx))).collect::<Map<_, _>>()),
        other => other.clone(),
    }
}

/// Truthiness of a rendered `if`: empty, `0`, `false`, `null`, `[]`, `{}` are false.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|x| x != 0.0),
        Value::String(s) => !matches!(s.trim(), "" | "0" | "false" | "null" | "[]" | "{}"),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

// --- storage -------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Automation {
    pub id: i64,
    pub project: String,
    pub name: String,
    pub enabled: bool,
    pub dry_run: bool,
    pub version: i64,
    pub spec: Value,
    pub created_by: String,
    pub created: String,
    pub updated: String,
}

impl Automation {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Automation> {
        let spec: String = r.get("spec")?;
        Ok(Automation {
            id: r.get("id")?,
            project: r.get("project")?,
            name: r.get("name")?,
            enabled: r.get::<_, i64>("enabled")? != 0,
            dry_run: r.get::<_, i64>("dry_run")? != 0,
            version: r.get("version")?,
            spec: serde_json::from_str(&spec).unwrap_or(Value::Null),
            created_by: r.get("created_by")?,
            created: r.get("created")?,
            updated: r.get("updated")?,
        })
    }

    pub fn trigger_kind(&self) -> &'static str {
        let on = &self.spec["on"];
        if !on["event"].is_null() {
            "event"
        } else if !on["schedule"].is_null() {
            "schedule"
        } else if !on["webhook"].is_null() {
            "webhook"
        } else {
            "manual"
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: i64,
    pub automation: i64,
    pub version: i64,
    pub project: String,
    pub trigger_key: String,
    pub trigger: Value,
    pub depth: i64,
    pub status: String,
    pub started: String,
    pub finished: Option<String>,
    pub error: Option<String>,
}

impl Run {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Run> {
        let trigger: String = r.get("trigger")?;
        Ok(Run {
            id: r.get("id")?,
            automation: r.get("automation")?,
            version: r.get("version")?,
            project: r.get("project")?,
            trigger_key: r.get("trigger_key")?,
            trigger: serde_json::from_str(&trigger).unwrap_or(Value::Null),
            depth: r.get("depth")?,
            status: r.get("status")?,
            started: r.get("started")?,
            finished: r.get("finished")?,
            error: r.get("error")?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepState {
    pub id: i64,
    pub run: i64,
    pub idx: i64,
    pub step_id: String,
    pub kind: String,
    pub status: String,
    pub attempt: i64,
    pub input: Value,
    pub output: Option<Value>,
    pub wait: Option<Value>,
    pub started: Option<String>,
    pub finished: Option<String>,
    pub error: Option<String>,
}

impl StepState {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<StepState> {
        let json = |s: Option<String>| s.and_then(|s| serde_json::from_str(&s).ok());
        Ok(StepState {
            id: r.get("id")?,
            run: r.get("run")?,
            idx: r.get("idx")?,
            step_id: r.get("step_id")?,
            kind: r.get("kind")?,
            status: r.get("status")?,
            attempt: r.get("attempt")?,
            input: json(r.get("input")?).unwrap_or(Value::Null),
            output: json(r.get("output")?),
            wait: json(r.get("wait")?),
            started: r.get("started")?,
            finished: r.get("finished")?,
            error: r.get("error")?,
        })
    }
}

impl ServerDb {
    pub fn create_automation(&self, project: &str, spec: &Value, by: &str) -> Result<Automation> {
        let problems = validate(spec);
        if !problems.is_empty() {
            return Err(GenieError::invalid(format!("invalid automation: {}", problems.join("; "))));
        }
        let t = now();
        self.conn().execute(
            "INSERT INTO automations(project, name, enabled, dry_run, version, spec, created_by, created, updated) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?7)",
            params![project, spec["name"].as_str().unwrap_or_default(), spec["enabled"] != json!(false), spec["dryRun"] == json!(true), spec.to_string(), by, t],
        )?;
        self.automation(self.conn().last_insert_rowid())
    }

    /// Replace a rule's spec: a new version (running runs keep the version they started with).
    pub fn update_automation(&self, id: i64, spec: &Value) -> Result<Automation> {
        let problems = validate(spec);
        if !problems.is_empty() {
            return Err(GenieError::invalid(format!("invalid automation: {}", problems.join("; "))));
        }
        self.conn().execute(
            "UPDATE automations SET spec = ?1, name = ?2, enabled = ?3, dry_run = ?4, version = version + 1, updated = ?5 WHERE id = ?6",
            params![
                spec.to_string(),
                spec["name"].as_str().unwrap_or_default(),
                spec["enabled"] != json!(false),
                spec["dryRun"] == json!(true),
                now(),
                id
            ],
        )?;
        self.automation(id)
    }

    pub fn set_automation_enabled(&self, id: i64, enabled: bool) -> Result<Automation> {
        self.conn().execute("UPDATE automations SET enabled = ?1, updated = ?2 WHERE id = ?3", params![enabled as i64, now(), id])?;
        self.automation(id)
    }

    pub fn delete_automation(&self, id: i64) -> Result<()> {
        self.conn().execute("DELETE FROM automations WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn automation(&self, id: i64) -> Result<Automation> {
        self.conn()
            .query_row("SELECT * FROM automations WHERE id = ?1", [id], Automation::from_row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("automation {id} not found")))
    }

    pub fn automations(&self, project: Option<&str>) -> Result<Vec<Automation>> {
        let mut stmt = self.conn().prepare("SELECT * FROM automations WHERE (?1 IS NULL OR project = ?1) ORDER BY id")?;
        Ok(stmt.query_map([project], Automation::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Start a run unless one exists for this trigger key. Returns the new run.
    pub fn create_run(&self, a: &Automation, trigger_key: &str, trigger: &Value, depth: i64, status: &str) -> Result<Option<Run>> {
        let n = self.conn().execute(
            "INSERT OR IGNORE INTO automation_runs(automation, version, project, trigger_key, trigger, depth, status, started) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![a.id, a.version, a.project, trigger_key, trigger.to_string(), depth, status, now()],
        )?;
        if n == 0 {
            return Ok(None);
        }
        Ok(Some(self.run(self.conn().last_insert_rowid())?))
    }

    pub fn run(&self, id: i64) -> Result<Run> {
        self.conn()
            .query_row("SELECT * FROM automation_runs WHERE id = ?1", [id], Run::from_row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("run {id} not found")))
    }

    pub fn runs(&self, project: &str, automation: Option<i64>, limit: i64) -> Result<Vec<Run>> {
        let mut stmt = self
            .conn()
            .prepare("SELECT * FROM automation_runs WHERE project = ?1 AND (?2 IS NULL OR automation = ?2) ORDER BY id DESC LIMIT ?3")?;
        Ok(stmt.query_map(params![project, automation, limit], Run::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Runs the engine should advance.
    pub fn active_runs(&self) -> Result<Vec<Run>> {
        let mut stmt = self.conn().prepare("SELECT * FROM automation_runs WHERE status IN ('queued', 'running', 'waiting') ORDER BY id")?;
        Ok(stmt.query_map([], Run::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn running_count(&self, automation: i64) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM automation_runs WHERE automation = ?1 AND status IN ('running', 'waiting')",
            [automation],
            |r| r.get(0),
        )?)
    }

    pub fn runs_since(&self, automation: i64, since: &str) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM automation_runs WHERE automation = ?1 AND started >= ?2 AND status <> 'skipped'",
            params![automation, since],
            |r| r.get(0),
        )?)
    }

    pub fn last_trigger_key(&self, automation: i64, prefix: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT trigger_key FROM automation_runs WHERE automation = ?1 AND trigger_key LIKE ?2 ORDER BY id DESC LIMIT 1",
                params![automation, format!("{prefix}%")],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_run_status(&self, id: i64, status: &str, error: Option<&str>) -> Result<()> {
        let finished = matches!(status, "succeeded" | "failed" | "cancelled" | "skipped").then(now);
        self.conn().execute(
            "UPDATE automation_runs SET status = ?1, error = COALESCE(?2, error), finished = COALESCE(?3, finished) WHERE id = ?4",
            params![status, error, finished, id],
        )?;
        Ok(())
    }

    pub fn steps(&self, run: i64) -> Result<Vec<StepState>> {
        let mut stmt = self.conn().prepare("SELECT * FROM run_steps WHERE run = ?1 ORDER BY idx")?;
        Ok(stmt.query_map([run], StepState::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn step(&self, id: i64) -> Result<StepState> {
        self.conn()
            .query_row("SELECT * FROM run_steps WHERE id = ?1", [id], StepState::from_row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("step {id} not found")))
    }

    /// The step row for `idx`, created on first use.
    pub fn ensure_step(&self, run: i64, idx: i64, step_id: &str, kind: &str) -> Result<StepState> {
        self.conn().execute(
            "INSERT OR IGNORE INTO run_steps(run, idx, step_id, kind, status) VALUES (?1, ?2, ?3, ?4, 'pending')",
            params![run, idx, step_id, kind],
        )?;
        Ok(self.conn().query_row("SELECT * FROM run_steps WHERE run = ?1 AND idx = ?2", params![run, idx], StepState::from_row)?)
    }

    pub fn update_step(
        &self,
        id: i64,
        status: &str,
        input: Option<&Value>,
        output: Option<&Value>,
        wait: Option<&Value>,
        error: Option<&str>,
    ) -> Result<()> {
        let started = (status == "running").then(now);
        let finished = matches!(status, "succeeded" | "failed" | "skipped").then(now);
        self.conn().execute(
            "UPDATE run_steps SET status = ?1, input = COALESCE(?2, input), output = COALESCE(?3, output), wait = ?4, error = ?5,
             attempt = attempt + (CASE WHEN ?1 = 'running' THEN 1 ELSE 0 END),
             started = COALESCE(started, ?6), finished = ?7 WHERE id = ?8",
            params![
                status,
                input.map(|v| v.to_string()),
                output.map(|v| v.to_string()),
                wait.map(|v| v.to_string()),
                error,
                started,
                finished,
                id
            ],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn filters_match_payload_and_task_fields() {
        let ctx =
            json!({ "to": "done", "task": { "type": "bug", "labels": ["web", "no-docs"], "priority": 1 }, "actor": { "role": "human" } });
        assert!(matches(&json!({ "to": "done", "task.type": ["task", "bug"] }), &ctx));
        assert!(!matches(&json!({ "to": "review" }), &ctx));
        assert!(!matches(&json!({ "task.labels": { "not_contains": "no-docs" } }), &ctx));
        assert!(matches(&json!({ "task.labels": { "contains": "web" }, "task.priority": { "lt": 2 } }), &ctx));
        assert!(matches(&json!({ "task.parent": { "exists": false }, "actor.role": { "not": "orchestrator" } }), &ctx));
    }

    #[test]
    fn templates_keep_types_and_interpolate_text() {
        let ctx = json!({ "event": { "task": { "id": "G-7" } }, "steps": { "a": { "output": { "questions": [{ "text": "q1" }, { "text": "q2" }] } } } });
        assert_eq!(render(&json!("{{ steps.a.output.questions }}"), &ctx), json!([{ "text": "q1" }, { "text": "q2" }]));
        assert_eq!(
            render(&json!("Answers for {{ event.task.id }} ({{ steps.a.output.questions | length }})"), &ctx),
            json!("Answers for G-7 (2)")
        );
        assert_eq!(render(&json!({ "task": "{{ event.task.id }}", "n": ["{{ missing }}"] }), &ctx), json!({ "task": "G-7", "n": [null] }));
        assert!(truthy(&render(&json!("{{ steps.a.output.questions | length }}"), &ctx)));
        assert!(!truthy(&render(&json!("{{ steps.b.output.questions | length }}"), &ctx)));
    }

    #[test]
    fn specs_are_validated() {
        let ok =
            json!({ "name": "x", "on": { "event": "task.created" }, "steps": [{ "notify": { "to": ["event.actor"], "text": "hi" } }] });
        assert!(validate(&ok).is_empty());
        let bad = json!({ "on": { "event": "a", "schedule": "* * * * *" }, "steps": [{ "notify": {}, "agent": {} }, { "id": "x", "wait": {}, "timeout": "soon" }] });
        let p = validate(&bad);
        assert_eq!(p.len(), 4, "{p:?}");
        assert!(!validate(&json!({ "name": "s", "on": { "schedule": "61 * * * *" }, "steps": [{ "wait": {} }] })).is_empty());
    }

    #[test]
    fn durations_and_schedules() {
        assert_eq!(parse_duration("24h"), Some(chrono::Duration::hours(24)));
        assert_eq!(parse_duration("15m"), Some(chrono::Duration::minutes(15)));
        assert!(parse_duration("soon").is_none());
        let after = Utc.with_ymd_and_hms(2026, 9, 28, 5, 0, 0).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 9, 28, 7, 30, 0).unwrap();
        // 09:00 in Moscow is 06:00 UTC.
        assert_eq!(due_fire("0 9 * * *", Some("Europe/Moscow"), after, now), Some(Utc.with_ymd_and_hms(2026, 9, 28, 6, 0, 0).unwrap()));
        assert_eq!(due_fire("0 9 * * *", Some("Europe/Moscow"), now, now), None);
        // The next one is tomorrow's, 09:00 Moscow time.
        assert_eq!(next_fire("0 9 * * *", Some("Europe/Moscow"), now), Some(Utc.with_ymd_and_hms(2026, 9, 29, 6, 0, 0).unwrap()));
        assert_eq!(next_fire("not cron", None, now), None);
    }

    #[test]
    fn runs_are_unique_per_trigger() {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("s.db")).unwrap();
        db.create_project("shop", "", "/x", None, None).unwrap();
        let a = db
            .create_automation(
                "shop",
                &json!({ "name": "x", "on": { "event": "task.created" }, "steps": [{ "wait": { "for": "1s" } }] }),
                "anna",
            )
            .unwrap();
        assert!(db.create_run(&a, "event:1", &json!({}), 0, "queued").unwrap().is_some());
        assert!(db.create_run(&a, "event:1", &json!({}), 0, "queued").unwrap().is_none(), "an event starts a rule once");
        let a2 =
            db.update_automation(a.id, &json!({ "name": "y", "on": { "manual": true }, "steps": [{ "wait": { "for": "1s" } }] })).unwrap();
        assert_eq!((a2.version, a2.name.as_str(), a2.trigger_kind()), (2, "y", "manual"));
        assert!(db.create_automation("shop", &json!({ "name": "bad" }), "anna").is_err());
    }
}
