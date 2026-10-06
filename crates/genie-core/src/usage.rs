//! Tokens the agents' models spent: one row per day, chat (agent), task and
//! model, added up as the agents report each model response. Kept in the
//! project's tracker so a task's subtasks and its epic are a join away; what
//! the tokens cost is worked out when they are shown, from the server's prices.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::tracker::Tracker;

/// Tokens of one or more model responses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Tokens {
    /// Input tokens read fresh (not from the provider's cache).
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Tokens {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    pub fn add(&mut self, o: &Tokens) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
    }
}

/// What a model costs, in dollars per million tokens. A missing cache price
/// leaves that cache's tokens **unpriced** — they are never billed as input:
/// providers price cache far below input, and inventing a price would inflate
/// the spend several times over.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
}

impl ModelPrice {
    /// What `t` costs and how many of its tokens stayed unpriced: every part
    /// without a price (the whole model, or cache without a cache price).
    pub fn split(&self, t: &Tokens) -> (f64, u64) {
        let mut cost = (t.input as f64 * self.input + t.output as f64 * self.output) / 1_000_000.0;
        let mut unpriced = 0u64;
        match self.cache_read {
            Some(p) => cost += t.cache_read as f64 * p / 1_000_000.0,
            None => unpriced += t.cache_read,
        }
        match self.cache_write {
            Some(p) => cost += t.cache_write as f64 * p / 1_000_000.0,
            None => unpriced += t.cache_write,
        }
        (cost, unpriced)
    }
}

/// What one chat spent on one task with one model in one day.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageRow {
    /// `YYYY-MM-DD`, UTC.
    pub day: String,
    /// The chat: `orchestrator`, `<team>/<member>` or `job/<id>`.
    pub agent: String,
    /// The task the agent worked on; `None` for the orchestrator and jobs without a task.
    pub task: Option<String>,
    /// `provider/model`.
    pub model: String,
    /// Model responses.
    pub calls: u64,
    pub tokens: Tokens,
}

const COLUMNS: &str = "day, agent, task, model, calls, input, output, cache_read, cache_write";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<UsageRow> {
    let n = |i: usize| r.get::<_, i64>(i).map(|v| v.max(0) as u64);
    let task: String = r.get(2)?;
    Ok(UsageRow {
        day: r.get(0)?,
        agent: r.get(1)?,
        task: (!task.is_empty()).then_some(task),
        model: r.get(3)?,
        calls: n(4)?,
        tokens: Tokens { input: n(5)?, output: n(6)?, cache_read: n(7)?, cache_write: n(8)? },
    })
}

/// Every row from `day` (`YYYY-MM-DD`) on, from a tracker database opened by hand
/// (one opened before the table existed has none).
pub fn usage_since(conn: &rusqlite::Connection, day: &str) -> rusqlite::Result<Vec<UsageRow>> {
    let exists =
        conn.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'usage'", [], |r| r.get::<_, i64>(0))? > 0;
    if !exists {
        return Ok(Vec::new());
    }
    let mut stmt = conn.prepare(&format!("SELECT {COLUMNS} FROM usage WHERE day >= ?1 ORDER BY day"))?;
    stmt.query_map([day], row)?.collect()
}

impl Tracker {
    /// Add one model response of `agent` to today's row.
    pub fn record_usage(&self, agent: &str, task: Option<&str>, model: &str, t: &Tokens) -> Result<()> {
        let day = crate::db::now()[..10].to_string();
        self.conn().execute(
            "INSERT INTO usage(day, agent, task, model, calls, input, output, cache_read, cache_write)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8)
             ON CONFLICT(day, agent, task, model) DO UPDATE SET
               calls = calls + 1, input = input + excluded.input, output = output + excluded.output,
               cache_read = cache_read + excluded.cache_read, cache_write = cache_write + excluded.cache_write",
            params![day, agent, task.unwrap_or(""), model, t.input as i64, t.output as i64, t.cache_read as i64, t.cache_write as i64],
        )?;
        Ok(())
    }

    /// Every row of a task and of the tasks under it (an epic's tasks, subtasks).
    pub fn usage_of_task(&self, task: &str) -> Result<Vec<UsageRow>> {
        let mut stmt = self.conn().prepare(&format!(
            "WITH RECURSIVE tree(id) AS (SELECT ?1 UNION SELECT t.id FROM tasks t JOIN tree ON t.parent = tree.id)
             SELECT {COLUMNS} FROM usage WHERE task IN (SELECT id FROM tree) ORDER BY day"
        ))?;
        Ok(stmt.query_map([task], row)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Every row of one chat.
    pub fn usage_of_agent(&self, agent: &str) -> Result<Vec<UsageRow>> {
        let mut stmt = self.conn().prepare(&format!("SELECT {COLUMNS} FROM usage WHERE agent = ?1 ORDER BY day"))?;
        Ok(stmt.query_map([agent], row)?.collect::<rusqlite::Result<_>>()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_without_a_price_is_unpriced_not_input() {
        // The case the task was filed about: a manual price for input and output
        // only, and almost all tokens in the cache.
        let p = ModelPrice { input: 0.14, output: 0.28, cache_read: None, cache_write: None };
        let t = Tokens { input: 1_000_000, output: 1_000_000, cache_read: 40_000_000, cache_write: 3_000_000 };
        let (cost, unpriced) = p.split(&t);
        assert!((cost - 0.42).abs() < 1e-12, "only input and output are billed: {cost}");
        assert_eq!(unpriced, 43_000_000, "the cache tokens are counted, not billed");
    }

    #[test]
    fn a_full_price_bills_everything() {
        let p = ModelPrice { input: 2.0, output: 10.0, cache_read: Some(0.2), cache_write: Some(2.5) };
        let t = Tokens { input: 1_000_000, output: 1_000_000, cache_read: 1_000_000, cache_write: 1_000_000 };
        let (cost, unpriced) = p.split(&t);
        assert!((cost - 14.7).abs() < 1e-12, "{cost}");
        assert_eq!(unpriced, 0);
    }

    #[test]
    fn a_zero_cache_tariff_is_a_price() {
        // DeepSeek writes its cache for free: that is a tariff, not a gap.
        let p = ModelPrice { input: 0.14, output: 0.28, cache_read: Some(0.006), cache_write: Some(0.0) };
        let t = Tokens { input: 0, output: 0, cache_read: 1_000_000, cache_write: 1_000_000 };
        let (cost, unpriced) = p.split(&t);
        assert!((cost - 0.006).abs() < 1e-12, "{cost}");
        assert_eq!(unpriced, 0);
    }
}
