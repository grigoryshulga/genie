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

/// A limit retains its task fence until the project-side pause is acknowledged.
#[derive(Debug, Clone)]
pub struct ExecutionPause {
    pub id: String,
    pub turn: Option<i64>,
    pub project: String,
    pub task: String,
    pub reason: String,
    pub note: String,
}

impl ExecutionPause {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self { id: r.get(0)?, turn: r.get(1)?, project: r.get(2)?, task: r.get(3)?, reason: r.get(4)?, note: r.get(5)? })
    }
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
    pub fn queue_turn_pause(&self, turn: i64, task: &str, note: &str) -> Result<ExecutionPause> {
        self.tx(|| {
            let run = self.turn(turn)?;
            self.conn().execute(
                "INSERT OR IGNORE INTO execution_pauses(id,turn,project,task,reason,note) VALUES (?1,?2,?3,?4,'time',?5)",
                params![crate::server_db::new_code(), turn, run.project, task, note],
            )?;
            let pause = self.conn().query_row(
                "SELECT id,turn,project,task,reason,note FROM execution_pauses WHERE turn=?1",
                [turn],
                ExecutionPause::from_row,
            )?;
            if pause.task != task || pause.note != note {
                return Err(GenieError::invalid("a turn's hard-limit pause cannot change"));
            }
            Ok(pause)
        })
    }

    pub fn queue_budget_pause(&self, project: &str, task: &str, note: &str) -> Result<ExecutionPause> {
        self.tx(|| {
            self.conn().execute(
                "INSERT OR IGNORE INTO execution_pauses(id,project,task,reason,note) VALUES (?1,?2,?3,'budget',?4)",
                params![crate::server_db::new_code(), project, task, note],
            )?;
            Ok(self.conn().query_row(
                "SELECT id,turn,project,task,reason,note FROM execution_pauses WHERE project=?1 AND task=?2 AND reason='budget' AND applied=0",
                params![project, task], ExecutionPause::from_row,
            )?)
        })
    }

    pub fn pending_execution_pauses(&self, project: Option<&str>) -> Result<Vec<ExecutionPause>> {
        let mut stmt = self.conn().prepare(
            "SELECT id,turn,project,task,reason,note FROM execution_pauses WHERE applied=0 AND (?1 IS NULL OR project=?1) ORDER BY rowid",
        )?;
        Ok(stmt.query_map([project], ExecutionPause::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn execution_pause_applied(&self, id: &str) -> Result<()> {
        self.conn().execute("UPDATE execution_pauses SET applied=1 WHERE id=?1", [id])?;
        Ok(())
    }

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
        self.jobs_in_state("queued")
    }

    pub fn running_jobs(&self) -> Result<Vec<Job>> {
        self.jobs_in_state("running")
    }

    fn jobs_in_state(&self, state: &str) -> Result<Vec<Job>> {
        let mut stmt = self.conn().prepare("SELECT * FROM agent_jobs WHERE status = ?1 ORDER BY id")?;
        Ok(stmt.query_map([state], Job::from_row)?.collect::<rusqlite::Result<_>>()?)
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
        if job.status == "cancelled" {
            return Ok(job);
        }
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

    /// An intentional execution pause preserves a job and its retry allowance.
    pub fn interrupt_job(&self, id: i64) -> Result<()> {
        self.conn().execute(
            "UPDATE agent_jobs SET status='queued', attempts=MAX(0,attempts-1), finished=NULL, error=NULL WHERE id=?1 AND status='running'",
            [id],
        )?;
        Ok(())
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

    const SCHEMA_14_PAUSES: &str = "DROP TABLE execution_pauses;
        CREATE TABLE turn_pauses(id TEXT PRIMARY KEY, turn INTEGER NOT NULL UNIQUE REFERENCES turns(id) ON DELETE CASCADE, task TEXT NOT NULL, note TEXT NOT NULL, applied INTEGER NOT NULL DEFAULT 0 CHECK(applied IN (0,1)));
        UPDATE meta SET value='14' WHERE key='schema';";

    #[test]
    fn limit_intents_upgrade_survive_restart_and_keep_one_frozen_binding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.db");
        let db = ServerDb::open(&path).unwrap();
        let turn = db.start_turn("shop", "worker", None, None, None).unwrap();
        db.conn().execute_batch("DROP TABLE execution_pauses; UPDATE meta SET value='13' WHERE key='schema';").unwrap();
        drop(db);
        let db = ServerDb::open(&path).unwrap();
        let pause = db.queue_turn_pause(turn, "G-1", "run limit reached").unwrap();
        assert_eq!(db.queue_turn_pause(turn, "G-1", "run limit reached").unwrap().id, pause.id);
        assert!(db.queue_turn_pause(turn, "G-2", "run limit reached").is_err());
        drop(db);
        let db = ServerDb::open(&path).unwrap();
        assert_eq!(db.pending_execution_pauses(Some("shop")).unwrap()[0].id, pause.id);
        assert!(db.pending_execution_pauses(Some("other")).unwrap().is_empty());
        db.execution_pause_applied(&pause.id).unwrap();
        assert!(db.pending_execution_pauses(None).unwrap().is_empty());
        assert_eq!(db.queue_turn_pause(turn, "G-1", "run limit reached").unwrap().id, pause.id);
        assert!(db.pending_execution_pauses(None).unwrap().is_empty(), "retry cannot reopen an acknowledged intent");
    }

    #[test]
    fn schema_14_pause_intents_keep_their_receipts_and_acknowledgements() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.db");
        let db = ServerDb::open(&path).unwrap();
        let turn = db.start_turn("shop", "worker", None, None, None).unwrap();
        let settled = db.start_turn("shop", "other", None, None, None).unwrap();
        db.conn().execute_batch(SCHEMA_14_PAUSES).unwrap();
        for (id, turn, applied) in [("pending", turn, 0), ("settled", settled, 1)] {
            db.conn()
                .execute(
                    "INSERT INTO turn_pauses(id,turn,task,note,applied) VALUES (?1,?2,'G-1','Time limit',?3)",
                    params![id, turn, applied],
                )
                .unwrap();
        }
        drop(db);
        let db = ServerDb::open(&path).unwrap();
        let pending = db.pending_execution_pauses(Some("shop")).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "pending");
        assert_eq!(pending[0].reason, "time");
        assert_eq!(pending[0].turn, Some(turn));
        assert_eq!(db.queue_turn_pause(settled, "G-1", "Time limit").unwrap().id, "settled");
        assert_eq!(db.pending_execution_pauses(None).unwrap().len(), 1);
        assert_eq!(db.conn().query_row("PRAGMA integrity_check", [], |r| r.get::<_, String>(0)).unwrap(), "ok");
        assert!(
            !db.conn()
                .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='turn_pauses')", [], |r| r.get::<_, bool>(0))
                .unwrap()
        );
    }

    #[test]
    fn budget_intents_deduplicate_pending_scopes_and_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.db");
        let db = ServerDb::open(&path).unwrap();
        let first = db.queue_budget_pause("shop", "G-1", "Daily budget exhausted").unwrap();
        assert!(first.turn.is_none());
        assert_eq!(first.reason, "budget");
        assert_eq!(db.queue_budget_pause("shop", "G-1", "A later cost report").unwrap().id, first.id);
        drop(db);
        let db = ServerDb::open(&path).unwrap();
        assert_eq!(db.pending_execution_pauses(Some("shop")).unwrap()[0].id, first.id);
        db.execution_pause_applied(&first.id).unwrap();
        let next = db.queue_budget_pause("shop", "G-1", "Another exhausted allowance").unwrap();
        assert_ne!(next.id, first.id);
        assert_eq!(db.pending_execution_pauses(None).unwrap().len(), 1);
    }

    #[test]
    fn a_declared_pause_table_move_still_refuses_row_loss_without_touching_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("server.db");
        let db = ServerDb::open(&path).unwrap();
        let turn = db.start_turn("shop", "worker", None, None, None).unwrap();
        db.conn().execute_batch(SCHEMA_14_PAUSES).unwrap();
        db.conn().execute_batch("PRAGMA foreign_keys=OFF").unwrap();
        for (id, turn) in [("valid", turn), ("orphan", turn + 1)] {
            db.conn().execute("INSERT INTO turn_pauses(id,turn,task,note) VALUES (?1,?2,'G-1','Time limit')", params![id, turn]).unwrap();
        }
        drop(db);
        let before = std::fs::read(&path).unwrap();
        let error = ServerDb::open(&path).err().expect("rehearsal must reject lost records");
        assert!(error.to_string().contains("row count of execution_pauses: 2 → 1"), "{error}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let original = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(original.query_row("SELECT COUNT(*) FROM turn_pauses", [], |r| r.get::<_, i64>(0)).unwrap(), 2);
    }

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
    /// Only an older database has the retired live table. Fresh installations never create it.
    pub fn has_legacy_proposals(&self) -> Result<bool> {
        Ok(self
            .conn()
            .query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'proposals')", [], |r| r.get(0))?)
    }

    /// All pending drafts, without a feed limit: each must survive the one-time conversion.
    pub fn legacy_proposals(&self) -> Result<Vec<Proposal>> {
        if !self.has_legacy_proposals()? {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn().prepare("SELECT * FROM proposals WHERE status = 'open' ORDER BY id")?;
        Ok(stmt.query_map([], Proposal::from_row)?.collect::<rusqlite::Result<_>>()?)
    }

    /// Decided legacy proposals remain available as immutable audit records, not a write API.
    pub fn archived_proposal(&self, id: i64) -> Result<Proposal> {
        self.conn()
            .query_row("SELECT * FROM doc_proposal_archive WHERE id = ?1", [id], Proposal::from_row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("archived proposal {id} not found")))
    }

    /// Allocate a changeset and its durable migration identity together. A crash before the git
    /// ref is written resumes this exact changeset; two drafts on one page never share it.
    pub fn proposal_migration(&self, id: i64, new: &crate::changesets::NewChangeset<'_>) -> Result<crate::changesets::Changeset> {
        self.tx(|| {
            if let Some(changeset) =
                self.conn().query_row("SELECT changeset FROM doc_proposal_migrations WHERE proposal = ?1", [id], |r| r.get(0)).optional()?
            {
                return self.changeset(changeset);
            }
            // Before migration receipts existed, the title recorded the proposal's exact ID.
            // Adopt only an unambiguous match; the caller verifies its git content before closing
            // the source record. Author/path alone can conflate independent drafts.
            let mut stmt = self.conn().prepare(
                "SELECT id FROM doc_changesets WHERE origin = 'migration' AND title = ?1 AND author = ?2
                 AND author_kind = ?3 AND project IS ?4 AND task IS ?5 AND status = 'open'
                 AND NOT EXISTS(SELECT 1 FROM doc_proposal_migrations m WHERE m.changeset = doc_changesets.id) LIMIT 2",
            )?;
            let previous: Vec<i64> = stmt
                .query_map(params![new.title, new.author, new.author_kind, new.project, new.task], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            if previous.len() > 1 {
                return Err(GenieError::invalid(format!(
                    "proposal {id} has ambiguous prior migration changesets; original draft retained"
                )));
            }
            let changeset = match previous.first() {
                Some(previous) => self.changeset(*previous)?,
                None => self.create_changeset(new)?,
            };
            self.conn().execute("INSERT INTO doc_proposal_migrations(proposal, changeset) VALUES (?1, ?2)", params![id, changeset.id])?;
            Ok(changeset)
        })
    }

    /// Called only after the draft's content is durably reachable from its changeset ref.
    pub fn finish_proposal_migration(&self, id: i64, changeset: i64, path: &str) -> Result<()> {
        self.tx(|| {
            self.touch_changeset(changeset, &[path.to_string()])?;
            let n = self.conn().execute(
                "UPDATE proposals SET status = 'migrated', decided_by = 'genie', decided_at = ?1, decision_note = ?2
                 WHERE id = ?3 AND status = 'open' AND EXISTS
                   (SELECT 1 FROM doc_proposal_migrations WHERE proposal = ?3 AND changeset = ?4)",
                params![now(), format!("changeset #{changeset}"), id, changeset],
            )?;
            if n != 1 {
                return Err(GenieError::invalid(format!("proposal {id} has no pending migration to changeset {changeset}")));
            }
            Ok(())
        })
    }

    /// Retire the old table only when every open draft has been converted. Copy and drop commit
    /// together: an interrupted or failed migration retains all original records for retry.
    pub fn retire_legacy_proposals(&self) -> Result<bool> {
        self.tx(|| {
            if !self.has_legacy_proposals()? {
                return Ok(true);
            }
            let open: bool = self.conn().query_row("SELECT EXISTS(SELECT 1 FROM proposals WHERE status = 'open')", [], |r| r.get(0))?;
            if open {
                return Ok(false);
            }
            self.conn().execute_batch(
                "INSERT INTO doc_proposal_archive(id, path, base_hash, content, author, author_kind, project, task,
                   note, status, created, decided_by, decided_at, decision_note)
                 SELECT id, path, base_hash, content, author, author_kind, project, task,
                   note, status, created, decided_by, decided_at, decision_note FROM proposals;
                 DROP TABLE proposals;",
            )?;
            Ok(true)
        })
    }
}
