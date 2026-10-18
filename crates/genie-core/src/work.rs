//! Agent turns and one-shot agent jobs in the server database.
//!
//! A *turn* is one run of an agent process: a team member or the orchestrator
//! reading its leased mail, or a job working on its goal. Turns are recorded so
//! a restart can tell which ones were interrupted, and so the UI can show what
//! agents did and why a run failed.

use rusqlite::{OptionalExtension, Row, params};
use serde::Serialize;
use serde_json::Value;

use crate::db::now;
use crate::error::{GenieError, Result};
use crate::server_db::ServerDb;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    pub id: i64,
    pub project: String,
    pub agent: String,
    pub team: Option<String>,
    pub member: Option<String>,
    pub job: Option<i64>,
    pub status: String,
    pub pid: Option<i64>,
    pub started: String,
    pub finished: Option<String>,
    pub exit_code: Option<i64>,
    pub error: Option<String>,
    pub log: Option<String>,
}

impl Turn {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Turn> {
        Ok(Turn {
            id: r.get("id")?,
            project: r.get("project")?,
            agent: r.get("agent")?,
            team: r.get("team")?,
            member: r.get("member")?,
            job: r.get("job")?,
            status: r.get("status")?,
            pid: r.get("pid")?,
            started: r.get("started")?,
            finished: r.get("finished")?,
            exit_code: r.get("exit_code")?,
            error: r.get("error")?,
            log: r.get("log")?,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    pub id: i64,
    pub project: String,
    pub task: Option<String>,
    pub run_step: Option<i64>,
    pub role: String,
    pub model: Option<String>,
    pub goal: String,
    pub inputs: Value,
    pub output_schema: Option<Value>,
    pub workspace: String,
    pub status: String,
    pub attempts: i64,
    pub output: Option<Value>,
    pub error: Option<String>,
    pub created: String,
    pub finished: Option<String>,
    /// The person the job runs on behalf of (whose LiteLLM key it uses).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initiator: Option<String>,
}

impl Job {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Job> {
        let json = |s: Option<String>| s.and_then(|s| serde_json::from_str(&s).ok());
        Ok(Job {
            id: r.get("id")?,
            project: r.get("project")?,
            task: r.get("task")?,
            run_step: r.get("run_step")?,
            role: r.get("role")?,
            model: r.get("model")?,
            goal: r.get("goal")?,
            inputs: json(r.get("inputs")?).unwrap_or(Value::Null),
            output_schema: json(r.get("output_schema")?),
            workspace: r.get("workspace")?,
            status: r.get("status")?,
            attempts: r.get("attempts")?,
            output: json(r.get("output")?),
            error: r.get("error")?,
            created: r.get("created")?,
            finished: r.get("finished")?,
            initiator: r.get("initiator")?,
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct NewJob {
    pub project: String,
    pub task: Option<String>,
    pub run_step: Option<i64>,
    pub role: String,
    pub model: Option<String>,
    pub goal: String,
    pub inputs: Value,
    pub output_schema: Option<Value>,
    pub workspace: String,
    /// The person the job runs on behalf of (a login).
    pub initiator: Option<String>,
}

impl ServerDb {
    pub fn start_turn(&self, project: &str, agent: &str, team: Option<&str>, member: Option<&str>, job: Option<i64>) -> Result<i64> {
        self.conn().execute(
            "INSERT INTO turns(project, agent, team, member, job, status, started) VALUES (?1, ?2, ?3, ?4, ?5, 'running', ?6)",
            params![project, agent, team, member, job, now()],
        )?;
        Ok(self.conn().last_insert_rowid())
    }

    pub fn set_turn_mail(&self, turn: i64, mail: &[i64]) -> Result<()> {
        self.conn().execute("UPDATE turns SET mail = ?1 WHERE id = ?2", params![serde_json::to_string(mail)?, turn])?;
        Ok(())
    }

    pub fn set_turn_pid(&self, turn: i64, pid: u32) -> Result<()> {
        self.conn().execute("UPDATE turns SET pid = ?1 WHERE id = ?2", params![pid, turn])?;
        Ok(())
    }

    pub fn finish_turn(&self, turn: i64, status: &str, exit_code: Option<i64>, error: Option<&str>, log: Option<&str>) -> Result<()> {
        self.conn().execute(
            "UPDATE turns SET status = ?1, exit_code = ?2, error = ?3, log = ?4, finished = ?5 WHERE id = ?6",
            params![status, exit_code, error, log, now(), turn],
        )?;
        Ok(())
    }

    pub fn turn(&self, id: i64) -> Result<Turn> {
        self.conn()
            .query_row("SELECT * FROM turns WHERE id = ?1", [id], Turn::from_row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("turn {id} not found")))
    }

    /// Turns still marked running: after a restart they were interrupted.
    pub fn interrupt_running_turns(&self) -> Result<Vec<Turn>> {
        let mut stmt = self.conn().prepare("SELECT * FROM turns WHERE status = 'running'")?;
        let rows = stmt.query_map([], Turn::from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        self.conn().execute("UPDATE turns SET status = 'interrupted', finished = ?1 WHERE status = 'running'", [now()])?;
        Ok(rows)
    }

    pub fn turns(&self, project: &str, agent: Option<&str>, limit: i64) -> Result<Vec<Turn>> {
        let mut stmt = self.conn().prepare(
            "SELECT * FROM (SELECT * FROM turns WHERE project = ?1 AND (?2 IS NULL OR agent = ?2) ORDER BY id DESC LIMIT ?3) ORDER BY id",
        )?;
        Ok(stmt.query_map(params![project, agent, limit], Turn::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    // --- jobs ----------------------------------------------------------------

    pub fn create_job(&self, j: NewJob) -> Result<Job> {
        // Any configured role; the server checks it exists before queueing the job.
        let valid = j.role.chars().next().is_some_and(|c| c.is_ascii_lowercase())
            && j.role.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !valid {
            return Err(GenieError::invalid(format!("unknown agent role {}", j.role)));
        }
        if j.goal.trim().is_empty() {
            return Err(GenieError::invalid("a job needs a goal"));
        }
        if !matches!(j.workspace.as_str(), "none" | "read-only" | "worktree" | "scratch") {
            return Err(GenieError::invalid("workspace must be none, read-only, worktree or scratch"));
        }
        self.conn().execute(
            "INSERT INTO agent_jobs(project, task, run_step, role, model, goal, inputs, output_schema, workspace, status, created, initiator)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'queued', ?10, ?11)",
            params![
                j.project,
                j.task,
                j.run_step,
                j.role,
                j.model,
                j.goal,
                j.inputs.to_string(),
                j.output_schema.map(|s| s.to_string()),
                j.workspace,
                now(),
                j.initiator
            ],
        )?;
        self.job(self.conn().last_insert_rowid())
    }

    pub fn job(&self, id: i64) -> Result<Job> {
        self.conn()
            .query_row("SELECT * FROM agent_jobs WHERE id = ?1", [id], Job::from_row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("job {id} not found")))
    }

    pub fn queued_jobs(&self) -> Result<Vec<Job>> {
        let mut stmt = self.conn().prepare("SELECT * FROM agent_jobs WHERE status = 'queued' ORDER BY id")?;
        Ok(stmt.query_map([], Job::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn jobs(&self, project: &str, limit: i64) -> Result<Vec<Job>> {
        let mut stmt =
            self.conn().prepare("SELECT * FROM (SELECT * FROM agent_jobs WHERE project = ?1 ORDER BY id DESC LIMIT ?2) ORDER BY id")?;
        Ok(stmt.query_map(params![project, limit], Job::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Agent jobs about a task that are queued or running.
    pub fn open_jobs_for_task(&self, project: &str, task: &str) -> Result<i64> {
        Ok(self.conn().query_row(
            "SELECT COUNT(*) FROM agent_jobs WHERE project = ?1 AND task = ?2 AND status IN ('queued', 'running')",
            params![project, task],
            |r| r.get(0),
        )?)
    }

    pub fn start_job(&self, id: i64) -> Result<()> {
        self.conn().execute("UPDATE agent_jobs SET status = 'running', attempts = attempts + 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    /// The agent reports its structured result (`genie job output`).
    pub fn set_job_output(&self, id: i64, output: &Value) -> Result<()> {
        let n = self
            .conn()
            .execute("UPDATE agent_jobs SET output = ?1 WHERE id = ?2 AND status = 'running'", params![output.to_string(), id])?;
        if n == 0 {
            return Err(GenieError::invalid(format!("job {id} is not running")));
        }
        Ok(())
    }

    /// Finish a job attempt: succeeded, requeued for another attempt, or failed for good.
    pub fn finish_job(&self, id: i64, ok: bool, error: Option<&str>, max_attempts: i64) -> Result<Job> {
        let job = self.job(id)?;
        let status = if ok {
            "succeeded"
        } else if job.attempts < max_attempts {
            "queued"
        } else {
            "failed"
        };
        let finished = (status != "queued").then(now);
        self.conn()
            .execute("UPDATE agent_jobs SET status = ?1, error = ?2, finished = ?3 WHERE id = ?4", params![status, error, finished, id])?;
        self.job(id)
    }

    pub fn cancel_job(&self, id: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE agent_jobs SET status = 'cancelled', finished = ?1 WHERE id = ?2 AND status IN ('queued', 'running')",
            params![now(), id],
        )?;
        Ok(())
    }

    /// After a restart, jobs that were running go back to the queue (their attempt counts).
    pub fn requeue_running_jobs(&self) -> Result<usize> {
        Ok(self.conn().execute("UPDATE agent_jobs SET status = 'queued' WHERE status = 'running'", [])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jobs_retry_then_fail_and_turns_recover() {
        let dir = tempfile::tempdir().unwrap();
        let db = ServerDb::open(&dir.path().join("server.db")).unwrap();
        let job = db
            .create_job(NewJob {
                project: "shop".into(),
                role: "documenter".into(),
                goal: "write docs".into(),
                workspace: "none".into(),
                inputs: json!({}),
                ..Default::default()
            })
            .unwrap();
        assert!(
            db.create_job(NewJob { role: "Wizard!".into(), goal: "x".into(), workspace: "none".into(), ..Default::default() }).is_err()
        );
        db.start_job(job.id).unwrap();
        assert!(db.set_job_output(job.id, &json!({ "ok": 1 })).is_ok());
        assert_eq!(db.finish_job(job.id, false, Some("crash"), 2).unwrap().status, "queued");
        db.start_job(job.id).unwrap();
        assert_eq!(db.finish_job(job.id, false, Some("crash"), 2).unwrap().status, "failed");
        assert!(db.set_job_output(job.id, &json!({})).is_err(), "output only while running");

        let t = db.start_turn("shop", "G-1/bender", Some("G-1"), Some("bender"), None).unwrap();
        let interrupted = db.interrupt_running_turns().unwrap();
        assert_eq!(interrupted.iter().map(|x| x.id).collect::<Vec<_>>(), vec![t]);
        assert_eq!(db.turn(t).unwrap().status, "interrupted");
    }
}

// --- knowledge proposals ----------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub id: i64,
    pub path: String,
    pub base_hash: Option<String>,
    pub content: String,
    pub author: String,
    pub author_kind: String,
    pub project: Option<String>,
    pub task: Option<String>,
    pub note: String,
    pub status: String,
    pub created: String,
    pub decided_by: Option<String>,
    pub decided_at: Option<String>,
    pub decision_note: Option<String>,
}

impl Proposal {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Proposal> {
        Ok(Proposal {
            id: r.get("id")?,
            path: r.get("path")?,
            base_hash: r.get("base_hash")?,
            content: r.get("content")?,
            author: r.get("author")?,
            author_kind: r.get("author_kind")?,
            project: r.get("project")?,
            task: r.get("task")?,
            note: r.get("note")?,
            status: r.get("status")?,
            created: r.get("created")?,
            decided_by: r.get("decided_by")?,
            decided_at: r.get("decided_at")?,
            decision_note: r.get("decision_note")?,
        })
    }
}

impl ServerDb {
    #[allow(clippy::too_many_arguments)]
    pub fn create_proposal(
        &self,
        path: &str,
        base_hash: Option<&str>,
        content: &str,
        author: &str,
        author_kind: &str,
        project: Option<&str>,
        task: Option<&str>,
        note: &str,
    ) -> Result<Proposal> {
        self.conn().execute(
            "INSERT INTO proposals(path, base_hash, content, author, author_kind, project, task, note, status, created)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'open', ?9)",
            params![path, base_hash, content, author, author_kind, project, task, note, now()],
        )?;
        self.proposal(self.conn().last_insert_rowid())
    }

    pub fn proposal(&self, id: i64) -> Result<Proposal> {
        self.conn()
            .query_row("SELECT * FROM proposals WHERE id = ?1", [id], Proposal::from_row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("proposal {id} not found")))
    }

    pub fn proposals(&self, status: Option<&str>, limit: i64) -> Result<Vec<Proposal>> {
        self.proposals_scoped(status, None, limit)
    }

    /// Filter by configured roots before the result limit; persisted project metadata may be stale after root reconfiguration.
    pub fn proposals_in_roots(&self, status: Option<&str>, roots: &[String], limit: i64) -> Result<Vec<Proposal>> {
        self.proposals_scoped(status, Some(roots), limit)
    }

    fn proposals_scoped(&self, status: Option<&str>, roots: Option<&[String]>, limit: i64) -> Result<Vec<Proposal>> {
        let roots_json = serde_json::to_string(roots.unwrap_or_default())?;
        let mut stmt = self.conn().prepare(
            "SELECT * FROM proposals WHERE (?1 IS NULL OR status = ?1)
             AND (?3 = 0 OR substr(path, 1, instr(path, '/') - 1) IN (SELECT value FROM json_each(?4)))
             ORDER BY id DESC LIMIT ?2",
        )?;
        Ok(stmt.query_map(params![status, limit, roots.is_some(), roots_json], Proposal::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Close an open proposal: `approved`, `rejected` or `superseded`.
    pub fn decide_proposal(&self, id: i64, status: &str, by: &str, note: Option<&str>) -> Result<Proposal> {
        let n = self.conn().execute(
            "UPDATE proposals SET status = ?1, decided_by = ?2, decided_at = ?3, decision_note = ?4 WHERE id = ?5 AND status = 'open'",
            params![status, by, now(), note, id],
        )?;
        if n == 0 {
            return Err(GenieError::invalid(format!("proposal {id} is not open")));
        }
        self.proposal(id)
    }
}
