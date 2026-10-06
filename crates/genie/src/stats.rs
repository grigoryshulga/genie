//! What happened on the server over a period, for reviewing a pilot: the tasks
//! people brought and closed and how long they took, how often agents needed a
//! person's decision and how fast people answered, how often work came back from
//! review, how the agents' runs went, and what the agents' models cost (by
//! model, epic, task and chat, with `modelPrices`). Read from the projects' journals and the
//! server database; `genie stats` prints it, `GET /api/stats` gives it to the
//! server's admins.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use chrono::{DateTime, SecondsFormat, Utc};
use genie_core::server_db::ServerDb;
use rusqlite::{Connection, OpenFlags, params};
use serde::{Deserialize, Serialize};

use crate::spend::{self, Item, Prices, Spend, TaskIndex};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStats {
    pub project: String,
    pub name: String,
    /// Tasks (not epics) created in the period, and how many of them by people.
    pub created: usize,
    pub created_by_people: usize,
    pub done: usize,
    pub cancelled: usize,
    /// Tasks open now (not done, not cancelled).
    pub open: usize,
    /// From creation to done, for the tasks done in the period.
    pub cycle_hours_median: Option<f64>,
    pub cycle_hours_p90: Option<f64>,
    /// Agents asked people for a decision (a task moved to needs_owner by an agent).
    pub decisions: usize,
    /// From such a question to the first word of a person on the task.
    pub answer_hours_median: Option<f64>,
    /// Work returned from review (review → changes_requested).
    pub returns: usize,
    pub comments_by_people: usize,
    pub comments_by_agents: usize,
    pub mcp_calls: usize,
    /// Agent runs (session steps and turns) and how many failed.
    pub runs: usize,
    pub runs_failed: usize,
    /// One-shot jobs of automations.
    pub jobs: usize,
    pub jobs_failed: usize,
    /// Knowledge proposals of agents and what people decided.
    pub proposals: usize,
    pub proposals_approved: usize,
    pub proposals_rejected: usize,
    /// People who did something in the project (created, commented, moved, answered).
    pub people: Vec<String>,
    /// The same period day by day (UTC dates, oldest first, every day present), for charts.
    #[serde(default)]
    pub daily: Vec<DayStats>,
    /// What the agents' models cost in the period.
    #[serde(default)]
    pub usage: Usage,
}

/// What the agents' models spent in a project over the period.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub spend: Spend,
    /// Every epic, then `""` (tasks outside epics) and `"-"` (work on no task), most expensive first.
    pub epics: Vec<Item>,
    /// The most expensive tasks.
    pub tasks: Vec<Item>,
    /// The most expensive chats.
    pub chats: Vec<Item>,
}

/// Tasks and chats listed in the statistics, at most.
const TOP: usize = 20;

/// One day of a project's period.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DayStats {
    /// `YYYY-MM-DD`, UTC.
    pub day: String,
    pub created: usize,
    pub done: usize,
    pub runs: usize,
    pub runs_failed: usize,
    /// Dollars spent (models with a price) and tokens (all models).
    #[serde(default)]
    pub cost: f64,
    #[serde(default)]
    pub tokens: u64,
    /// Dollars by model (`provider/model`), models with a price only.
    #[serde(default)]
    pub cost_by_model: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub days: i64,
    pub since: String,
    pub projects: Vec<ProjectStats>,
}

fn hours(from: &str, to: &str) -> Option<f64> {
    let a = DateTime::parse_from_rfc3339(from).ok()?;
    let b = DateTime::parse_from_rfc3339(to).ok()?;
    Some((b - a).num_seconds().max(0) as f64 / 3600.0)
}

/// The value at the fraction `q` of the sorted values (nearest rank).
fn quantile(values: &[f64], q: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    let rank = ((q * v.len() as f64).ceil() as usize).clamp(1, v.len());
    Some((v[rank - 1] * 10.0).round() / 10.0)
}

fn count(conn: &Connection, sql: &str, since: &str) -> rusqlite::Result<usize> {
    conn.query_row(sql, params![since], |r| r.get::<_, i64>(0)).map(|n| n as usize)
}

fn tracker_stats(p: &mut ProjectStats, db: &Path, since: &str) -> rusqlite::Result<()> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    let not_epic = "(SELECT type FROM tasks WHERE id = e.subject) IS NOT 'epic'";
    let (created, by_people): (i64, Option<i64>) = conn.query_row(
        &format!("SELECT COUNT(*), SUM(e.actor_role = 'human') FROM events e WHERE e.type = 'task.created' AND e.at >= ?1 AND {not_epic}"),
        params![since],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    p.created = created as usize;
    p.created_by_people = by_people.unwrap_or(0) as usize;
    let to = |status: &str| {
        format!(
            "SELECT COUNT(*) FROM events e WHERE e.type = 'task.status_changed' AND e.at >= ?1 AND json_extract(e.payload, '$.to') = '{status}' AND {not_epic}"
        )
    };
    p.done = count(&conn, &to("done"), since)?;
    p.cancelled = count(&conn, &to("cancelled"), since)?;
    p.returns = count(&conn, &to("changes_requested"), since)?;
    p.open = conn
        .query_row("SELECT COUNT(*) FROM tasks WHERE status NOT IN ('done', 'cancelled') AND type != 'epic'", [], |r| r.get::<_, i64>(0))?
        as usize;
    // Cycle time of the tasks done in the period.
    let mut stmt = conn.prepare(
        "SELECT t.created, e.at FROM events e JOIN tasks t ON t.id = e.subject
         WHERE e.type = 'task.status_changed' AND e.at >= ?1 AND json_extract(e.payload, '$.to') = 'done' AND t.type != 'epic'",
    )?;
    let cycles: Vec<f64> = stmt
        .query_map(params![since], |r| Ok(hours(&r.get::<_, String>(0)?, &r.get::<_, String>(1)?)))?
        .filter_map(|x| x.ok().flatten())
        .collect();
    p.cycle_hours_median = quantile(&cycles, 0.5);
    p.cycle_hours_p90 = quantile(&cycles, 0.9);
    // Questions of agents, and the first word of a person after each.
    let mut stmt = conn.prepare(
        "SELECT e.id, e.subject, e.at,
                (SELECT MIN(n.at) FROM events n WHERE n.subject = e.subject AND n.id > e.id AND n.actor_role = 'human')
         FROM events e
         WHERE e.type = 'task.status_changed' AND e.at >= ?1 AND e.actor_role != 'human' AND json_extract(e.payload, '$.to') = 'needs_owner'",
    )?;
    let asked: Vec<(String, Option<String>)> = stmt
        .query_map(params![since], |r| Ok((r.get::<_, String>(2)?, r.get::<_, Option<String>>(3)?)))?
        .collect::<rusqlite::Result<_>>()?;
    p.decisions = asked.len();
    let answers: Vec<f64> = asked.iter().filter_map(|(at, answered)| answered.as_deref().and_then(|a| hours(at, a))).collect();
    p.answer_hours_median = quantile(&answers, 0.5);
    let (people_comments, agent_comments): (Option<i64>, Option<i64>) = conn.query_row(
        "SELECT SUM(actor_role = 'human'), SUM(actor_role != 'human') FROM events WHERE type = 'task.commented' AND at >= ?1",
        params![since],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    p.comments_by_people = people_comments.unwrap_or(0) as usize;
    p.comments_by_agents = agent_comments.unwrap_or(0) as usize;
    p.mcp_calls = count(&conn, "SELECT COUNT(*) FROM events WHERE type = 'mcp.called' AND at >= ?1", since)?;
    let mut stmt = conn.prepare("SELECT DISTINCT actor FROM events WHERE actor_role = 'human' AND at >= ?1 ORDER BY actor")?;
    p.people = stmt.query_map(params![since], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<_>>()?;
    let mut stmt = conn.prepare(&format!(
        "SELECT substr(e.at, 1, 10), SUM(e.type = 'task.created'), SUM(e.type = 'task.status_changed' AND json_extract(e.payload, '$.to') = 'done')
         FROM events e WHERE e.at >= ?1 AND e.type IN ('task.created', 'task.status_changed') AND {not_epic} GROUP BY 1"
    ))?;
    for row in stmt.query_map(params![since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<i64>>(2)?)))? {
        let (day, created, done) = row?;
        if let Some(d) = p.daily.iter_mut().find(|d| d.day == day) {
            d.created = created.unwrap_or(0) as usize;
            d.done = done.unwrap_or(0) as usize;
        }
    }
    Ok(())
}

fn server_stats(p: &mut ProjectStats, conn: &Connection, since: &str) -> rusqlite::Result<()> {
    let pair = |sql: &str| -> rusqlite::Result<(usize, usize)> {
        conn.query_row(sql, params![p.project, since], |r| {
            Ok((r.get::<_, i64>(0)? as usize, r.get::<_, Option<i64>>(1)?.unwrap_or(0) as usize))
        })
    };
    (p.runs, p.runs_failed) =
        pair("SELECT COUNT(*), SUM(status = 'failed') FROM turns WHERE project = ?1 AND started >= ?2 AND status != 'skipped'")?;
    (p.jobs, p.jobs_failed) = pair("SELECT COUNT(*), SUM(status = 'failed') FROM agent_jobs WHERE project = ?1 AND created >= ?2")?;
    let (proposals, approved) = pair("SELECT COUNT(*), SUM(status = 'approved') FROM proposals WHERE project = ?1 AND created >= ?2")?;
    let (_, rejected) = pair("SELECT COUNT(*), SUM(status = 'rejected') FROM proposals WHERE project = ?1 AND created >= ?2")?;
    (p.proposals, p.proposals_approved, p.proposals_rejected) = (proposals, approved, rejected);
    let mut stmt = conn.prepare(
        "SELECT substr(started, 1, 10), COUNT(*), SUM(status = 'failed') FROM turns
         WHERE project = ?1 AND started >= ?2 AND status != 'skipped' GROUP BY 1",
    )?;
    for row in
        stmt.query_map(params![p.project, since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?)))?
    {
        let (day, runs, failed) = row?;
        if let Some(d) = p.daily.iter_mut().find(|d| d.day == day) {
            d.runs = runs as usize;
            d.runs_failed = failed.unwrap_or(0) as usize;
        }
    }
    Ok(())
}

fn usage_stats(p: &mut ProjectStats, db: &Path, since: &str, prices: &Prices) -> rusqlite::Result<()> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    let rows = genie_core::usage::usage_since(&conn, &since[..10])?;
    let tasks = TaskIndex::load(&conn)?;
    for r in &rows {
        if let Some(d) = p.daily.iter_mut().find(|d| d.day == r.day) {
            d.tokens += r.tokens.total();
            if let Some(price) = crate::config::price_of(prices, &r.model) {
                let (c, _) = price.split(&r.tokens);
                d.cost += c;
                *d.cost_by_model.entry(r.model.clone()).or_default() += c;
            }
        }
    }
    for d in &mut p.daily {
        d.cost = (d.cost * 10_000.0).round() / 10_000.0;
        d.cost_by_model.values_mut().for_each(|c| *c = (*c * 10_000.0).round() / 10_000.0);
    }
    p.usage = Usage {
        spend: Spend::of(prices, &rows),
        epics: spend::by_epic(prices, &rows, &tasks),
        tasks: spend::by_task(prices, &rows, &tasks, TOP),
        chats: spend::by_chat(prices, &rows, &tasks, TOP),
    };
    Ok(())
}

/// Every day from `since` to `now`, both included, with nothing counted yet.
fn empty_days(since: DateTime<Utc>, now: DateTime<Utc>) -> Vec<DayStats> {
    let mut out = Vec::new();
    let mut day = since.date_naive();
    while day <= now.date_naive() {
        out.push(DayStats { day: day.format("%Y-%m-%d").to_string(), ..Default::default() });
        day = day.succ_opt().unwrap_or(day);
        if out.len() > 400 {
            break;
        }
    }
    out
}

/// The statistics of every project (or one) over the last `days` days, the
/// models' tokens priced with `prices`.
pub fn collect(data: &Path, days: i64, only: Option<&str>, prices: &Prices) -> Result<Stats, String> {
    let now = Utc::now();
    let since_at = now - chrono::Duration::days(days.max(1));
    let since = since_at.to_rfc3339_opts(SecondsFormat::Millis, true);
    let server = ServerDb::open(&data.join("server.db")).map_err(|e| e.to_string())?;
    let raw = Connection::open_with_flags(data.join("server.db"), OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| e.to_string())?;
    let mut projects = Vec::new();
    for pr in server.projects().map_err(|e| e.to_string())? {
        if only.is_some_and(|o| o != pr.slug) {
            continue;
        }
        let mut p =
            ProjectStats { project: pr.slug.clone(), name: pr.name.clone(), daily: empty_days(since_at, now), ..Default::default() };
        tracker_stats(&mut p, &Path::new(&pr.tracker_dir).join("genie.db"), &since).map_err(|e| format!("{}: {e}", pr.slug))?;
        server_stats(&mut p, &raw, &since).map_err(|e| format!("{}: {e}", pr.slug))?;
        usage_stats(&mut p, &Path::new(&pr.tracker_dir).join("genie.db"), &since, prices).map_err(|e| format!("{}: {e}", pr.slug))?;
        projects.push(p);
    }
    if let Some(o) = only
        && projects.is_empty()
    {
        return Err(format!("no project {o}"));
    }
    Ok(Stats { days: days.max(1), since, projects })
}

fn h(v: Option<f64>) -> String {
    match v {
        None => "—".into(),
        Some(x) if x < 48.0 => format!("{x:.1} h"),
        Some(x) => format!("{:.1} d", x / 24.0),
    }
}

fn tokens(n: u64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.0}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

fn spent(s: &Spend) -> String {
    let unpriced = if s.unpriced_tokens > 0 { format!(" ({} of them without a price)", tokens(s.unpriced_tokens)) } else { String::new() };
    format!("${:.2}, {} tokens{unpriced}", s.cost, tokens(s.tokens.total()))
}

/// The statistics as text for the terminal.
pub fn render(s: &Stats) -> String {
    let mut out = vec![format!("last {} day(s), since {}", s.days, &s.since[..10])];
    let mut everyone = BTreeSet::new();
    for p in &s.projects {
        everyone.extend(p.people.iter().cloned());
        out.push(String::new());
        out.push(format!("{} ({})", p.project, p.name));
        out.push(format!(
            "  tasks        {} created ({} by people), {} done, {} cancelled, {} open now",
            p.created, p.created_by_people, p.done, p.cancelled, p.open
        ));
        out.push(format!("  done in      median {}, 90% within {}", h(p.cycle_hours_median), h(p.cycle_hours_p90)));
        out.push(format!("  decisions    agents asked people {} time(s), answered in median {}", p.decisions, h(p.answer_hours_median)));
        out.push(format!("  review       {} time(s) work came back for changes", p.returns));
        out.push(format!("  comments     {} by people, {} by agents", p.comments_by_people, p.comments_by_agents));
        out.push(format!(
            "  agents       {} run(s), {} failed; {} job(s), {} failed; {} MCP call(s)",
            p.runs, p.runs_failed, p.jobs, p.jobs_failed, p.mcp_calls
        ));
        out.push(format!(
            "  knowledge    {} proposal(s): {} approved, {} rejected",
            p.proposals, p.proposals_approved, p.proposals_rejected
        ));
        out.push(format!("  people       {}", if p.people.is_empty() { "—".to_string() } else { p.people.join(", ") }));
        out.push(format!("  spent        {}", spent(&p.usage.spend)));
        for m in &p.usage.spend.models {
            let cost = m.cost.map_or("no price".to_string(), |c| format!("${c:.2}"));
            out.push(format!("    {:<30} {cost}, {} tokens", m.model, tokens(m.tokens.total())));
        }
        for e in p.usage.epics.iter().filter(|e| !e.id.is_empty() && e.id != "-").take(5) {
            out.push(format!("    epic {} {}: {}", e.id, e.title, spent(&e.spend)));
        }
    }
    let mut all = Spend::default();
    for p in &s.projects {
        all.calls += p.usage.spend.calls;
        all.tokens.add(&p.usage.spend.tokens);
        all.cost += p.usage.spend.cost;
        all.unpriced_tokens += p.usage.spend.unpriced_tokens;
    }
    out.push(String::new());
    out.push(format!("{} active people across {} project(s); spent {}", everyone.len(), s.projects.len(), spent(&all)));
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantiles_take_the_nearest_rank() {
        assert_eq!(quantile(&[], 0.5), None);
        assert_eq!(quantile(&[3.0, 1.0, 2.0], 0.5), Some(2.0));
        assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0, 100.0], 0.9), Some(100.0));
        assert_eq!(hours("2026-09-29T10:00:00.000Z", "2026-09-29T13:30:00.000Z"), Some(3.5));
        assert_eq!(h(Some(72.0)), "3.0 d");
    }

    #[test]
    fn a_period_lists_every_day() {
        let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        let days = empty_days(at("2026-09-23T15:00:00Z"), at("2026-09-30T15:00:00Z"));
        assert_eq!(days.len(), 8);
        assert_eq!((days[0].day.as_str(), days[7].day.as_str()), ("2026-09-23", "2026-09-30"));
    }
}
