//! Durable project decision policy and evaluations. Pending calls reserve their allowance
//! before any network I/O; a lost response keeps the reservation instead of becoming free.
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{GenieError, Result, db::now, server_db::ServerDb};

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS decision_policies (
  project TEXT PRIMARY KEY REFERENCES projects(slug) ON DELETE CASCADE,
  policy TEXT NOT NULL,
  updated TEXT NOT NULL,
  actor TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS decisions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL REFERENCES projects(slug) ON DELETE CASCADE,
  task TEXT NOT NULL,
  actor TEXT NOT NULL,
  purpose TEXT NOT NULL,
  request TEXT NOT NULL,
  result TEXT NOT NULL DEFAULT '{}',
  status TEXT NOT NULL DEFAULT 'pending',
  created TEXT NOT NULL,
  finished TEXT,
  elapsed_ms INTEGER NOT NULL DEFAULT 0,
  cost_usd REAL,
  reserved_usd REAL NOT NULL,
  application TEXT NOT NULL DEFAULT '{}',
  request_key TEXT,
  UNIQUE(project, task, request_key)
);
CREATE INDEX IF NOT EXISTS decisions_task ON decisions(project, task, id);
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    pub id: i64,
    pub project: String,
    pub task: String,
    pub actor: String,
    pub purpose: String,
    pub request: Value,
    pub result: Value,
    pub status: String,
    pub created: String,
    pub finished: Option<String>,
    pub elapsed_ms: u64,
    pub cost_usd: Option<f64>,
    pub reserved_usd: f64,
    pub application: Value,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Decision> {
    // JSON is written only by this module; keep database corruption visible to the caller.
    let value = |key: &str| -> rusqlite::Result<Value> {
        let s: String = r.get(key)?;
        serde_json::from_str(&s).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))
    };
    Ok(Decision {
        id: r.get("id")?,
        project: r.get("project")?,
        task: r.get("task")?,
        actor: r.get("actor")?,
        purpose: r.get("purpose")?,
        request: value("request")?,
        result: value("result")?,
        status: r.get("status")?,
        created: r.get("created")?,
        finished: r.get("finished")?,
        elapsed_ms: r.get::<_, i64>("elapsed_ms")?.max(0) as u64,
        cost_usd: r.get("cost_usd")?,
        reserved_usd: r.get("reserved_usd")?,
        application: value("application")?,
    })
}

pub struct NewDecision<'a> {
    pub project: &'a str,
    pub task: &'a str,
    pub actor: &'a str,
    pub purpose: &'a str,
    pub request: &'a Value,
    pub key: Option<&'a str>,
    pub reserve: f64,
    pub max_calls: u32,
    pub max_cost: f64,
}

impl ServerDb {
    pub fn decision_policy(&self, project: &str) -> Result<Option<Value>> {
        let s: Option<String> =
            self.conn().query_row("SELECT policy FROM decision_policies WHERE project=?1", [project], |r| r.get(0)).optional()?;
        s.map(|s| serde_json::from_str(&s).map_err(Into::into)).transpose()
    }

    pub fn set_decision_policy(&self, project: &str, actor: &str, policy: &Value) -> Result<()> {
        self.conn().execute(
            "INSERT INTO decision_policies(project,policy,updated,actor) VALUES (?1,?2,?3,?4)
             ON CONFLICT(project) DO UPDATE SET policy=excluded.policy, updated=excluded.updated, actor=excluded.actor",
            params![project, policy.to_string(), now(), actor],
        )?;
        Ok(())
    }

    pub fn decision(&self, project: &str, id: i64) -> Result<Decision> {
        self.conn()
            .query_row("SELECT * FROM decisions WHERE project=?1 AND id=?2", params![project, id], row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("decision {id}")))
    }

    pub fn decision_by_key(&self, project: &str, task: &str, key: &str) -> Result<Option<Decision>> {
        Ok(self
            .conn()
            .query_row("SELECT * FROM decisions WHERE project=?1 AND task=?2 AND request_key=?3", params![project, task, key], row)
            .optional()?)
    }

    pub fn decisions(&self, project: &str, task: Option<&str>, limit: u32) -> Result<Vec<Decision>> {
        let mut stmt =
            self.conn().prepare("SELECT * FROM decisions WHERE project=?1 AND (?2 IS NULL OR task=?2) ORDER BY id DESC LIMIT ?3")?;
        Ok(stmt.query_map(params![project, task, limit.min(200)], row)?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn begin_decision(&self, n: NewDecision<'_>) -> Result<Decision> {
        self.tx(|| {
            // Failed calls remain evidence, but must not permanently cache a missing
            // key or an outage after the operator fixes it. Active/successful keys stay unique.
            if let Some(key) = n.key {
                self.conn().execute(
                    "UPDATE decisions SET request_key=NULL WHERE project=?1 AND task=?2 AND request_key=?3 AND status='unavailable'",
                    params![n.project,n.task,key],
                )?;
            }
            let (calls, charged): (u32, f64) = self.conn().query_row(
                "SELECT count(*), COALESCE(sum(COALESCE(cost_usd,reserved_usd)),0) FROM decisions WHERE project=?1 AND task=?2",
                params![n.project,n.task], |r| Ok((r.get(0)?,r.get(1)?)),
            )?;
            if calls >= n.max_calls || charged + n.reserve > n.max_cost {
                return Err(GenieError::invalid("Jev allowance for this task is exhausted"));
            }
            self.conn().execute(
                "INSERT INTO decisions(project,task,actor,purpose,request,created,reserved_usd,request_key) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                params![n.project,n.task,n.actor,n.purpose,n.request.to_string(),now(),n.reserve,n.key],
            )?;
            self.decision(n.project, self.conn().last_insert_rowid())
        })
    }

    pub fn finish_decision(
        &self,
        project: &str,
        id: i64,
        status: &str,
        result: &Value,
        cost: Option<f64>,
        elapsed: u64,
    ) -> Result<Decision> {
        self.conn().execute(
            "UPDATE decisions SET status=?3,result=?4,cost_usd=?5,elapsed_ms=?6,finished=?7 WHERE project=?1 AND id=?2 AND status='pending'",
            params![project,id,status,result.to_string(),cost,elapsed.min(i64::MAX as u64) as i64,now()],
        )?;
        self.decision(project, id)
    }

    pub fn apply_decision(&self, project: &str, id: i64, application: &Value) -> Result<()> {
        self.conn()
            .execute("UPDATE decisions SET application=?3 WHERE project=?1 AND id=?2", params![project, id, application.to_string()])?;
        Ok(())
    }
}
