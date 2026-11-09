//! Durable task execution fences. Pausing keeps the task, team, mail and artifacts;
//! only an explicit human resume removes the fence. Ancestor pauses cover subtasks.

use crate::{Actor, GenieError, Result, Role, Tracker};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::json;

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS task_execution (
  task TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
  paused INTEGER NOT NULL CHECK(paused IN (0,1)),
  generation INTEGER NOT NULL,
  reason TEXT NOT NULL,
  note TEXT NOT NULL,
  actor TEXT NOT NULL,
  changed_at TEXT NOT NULL
);
"#;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutionState {
    pub task: String,
    pub paused: bool,
    pub generation: i64,
    pub reason: String,
    pub note: String,
    pub actor: String,
    pub changed_at: String,
}

pub struct Execution<'a>(&'a Tracker);
impl Tracker {
    pub fn execution(&self) -> Execution<'_> {
        Execution(self)
    }
}
impl Execution<'_> {
    pub fn state(&self, task: &str) -> Result<Option<ExecutionState>> {
        let task = self.0.normalize_id(task)?;
        Ok(self
            .0
            .conn()
            .query_row("SELECT task,paused,generation,reason,note,actor,changed_at FROM task_execution WHERE task=?1", [task], |r| {
                Ok(ExecutionState {
                    task: r.get(0)?,
                    paused: r.get(1)?,
                    generation: r.get(2)?,
                    reason: r.get(3)?,
                    note: r.get(4)?,
                    actor: r.get(5)?,
                    changed_at: r.get(6)?,
                })
            })
            .optional()?)
    }
    pub fn blocking(&self, task: &str) -> Result<Option<ExecutionState>> {
        for id in self.ancestors(task)? {
            if let Some(state) = self.state(&id)?.filter(|s| s.paused) {
                return Ok(Some(state));
            }
        }
        Ok(None)
    }
    pub fn ensure_running(&self, task: &str) -> Result<()> {
        if let Some(state) = self.blocking(task)? {
            return Err(GenieError::Denied(format!(
                "execution paused for {} ({}): {}; a person must explicitly resume it",
                state.task, state.reason, state.note
            )));
        }
        Ok(())
    }
    /// Include each ancestor generation: a stop followed by resume invalidates a
    /// startup prepared before that stop, even though the task is runnable again.
    pub fn stamp(&self, task: &str) -> Result<String> {
        self.ensure_running(task)?;
        let values = self
            .ancestors(task)?
            .into_iter()
            .map(|id| {
                let generation = self.state(&id)?.map_or(0, |s| s.generation);
                Ok((id, generation))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(serde_json::to_string(&values)?)
    }
    pub fn pause(&self, actor: &Actor, task: &str, reason: &str, note: &str) -> Result<ExecutionState> {
        if !matches!(actor.role, Role::Human | Role::Orchestrator) {
            return Err(GenieError::Denied("only a person or the server pauses execution".into()));
        }
        if note.trim().is_empty() || !matches!(reason, "owner" | "budget" | "time") {
            return Err(GenieError::invalid("a pause needs a reason and note"));
        }
        self.0.tx(|| {
            let task = self.0.get(task)?.id;
            if let Some(state) = self.state(&task)?.filter(|s| s.paused) {
                return Ok(state);
            }
            self.write(actor, &task, true, reason, note)
        })
    }
    pub fn resume(&self, actor: &Actor, task: &str) -> Result<Option<ExecutionState>> {
        if actor.role != Role::Human {
            return Err(GenieError::Denied("only a person resumes execution".into()));
        }
        self.0.tx(|| {
            let task = self.0.get(task)?.id;
            if self.state(&task)?.is_none_or(|s| !s.paused) {
                return self.state(&task);
            }
            self.write(actor, &task, false, "owner", "Explicitly resumed by a person").map(Some)
        })
    }
    fn write(&self, actor: &Actor, task: &str, paused: bool, reason: &str, note: &str) -> Result<ExecutionState> {
        self.0.conn().execute("INSERT INTO task_execution(task,paused,generation,reason,note,actor,changed_at) VALUES (?1,?2,1,?3,?4,?5,?6)
            ON CONFLICT(task) DO UPDATE SET paused=excluded.paused,generation=generation+1,reason=excluded.reason,note=excluded.note,actor=excluded.actor,changed_at=excluded.changed_at",
            params![task,paused,reason,note,actor.name,crate::db::now()])?;
        crate::events::append(
            self.0.conn(),
            if paused { "task.execution_paused" } else { "task.execution_resumed" },
            Some(task),
            &actor.name,
            &actor.role.to_string(),
            json!({"reason":reason,"note":note}),
        )?;
        self.state(task)?.ok_or_else(|| GenieError::not_found("execution state"))
    }
    fn ancestors(&self, task: &str) -> Result<Vec<String>> {
        let mut next = Some(self.0.get(task)?.id);
        let mut ids = Vec::new();
        while let Some(id) = next {
            if ids.contains(&id) || ids.len() >= 32 {
                return Err(GenieError::invalid("invalid task ancestry"));
            }
            next = self.0.get(&id)?.parent;
            ids.push(id);
        }
        Ok(ids)
    }
}
