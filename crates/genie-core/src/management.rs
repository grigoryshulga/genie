//! Project knowledge and the durable lifecycle of one task's manager. Mail routing,
//! versioned briefs, clarification and retrospective completion share this transaction.

use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::{Value, json};

use crate::db::now;
use crate::team::ORCHESTRATOR;
use crate::{Actor, ArtifactInput, ArtifactSource, CLOSED, GenieError, Result, Status, StatusOptions, TaskType, Tracker};

pub const PROJECT_MANAGER: &str = "project-manager";
pub const TASK_MANAGER_PREFIX: &str = "task-manager:";

pub fn manager_name(task: &str) -> String {
    format!("{TASK_MANAGER_PREFIX}{task}")
}

pub fn managed_task(name: &str) -> Option<&str> {
    name.strip_prefix(TASK_MANAGER_PREFIX).filter(|s| !s.is_empty())
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct TaskManager {
    pub task: String,
    pub state: String,
    pub brief_version: i64,
    pub created: String,
    pub finished: Option<String>,
    pub report: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Clarification {
    pub id: i64,
    pub task: String,
    pub brief_version: i64,
    pub question: String,
    pub blocking: bool,
    pub state: String,
    pub answer: Option<String>,
    pub source: Option<String>,
    pub created: String,
    pub answered: Option<String>,
}

pub struct Management<'a>(&'a Tracker);

impl Tracker {
    pub fn management(&self) -> Management<'_> {
        Management(self)
    }
}

impl Management<'_> {
    pub fn manager(&self, task: &str) -> Result<Option<TaskManager>> {
        let id = self.0.normalize_id(task)?;
        Ok(self
            .0
            .conn()
            .query_row("SELECT * FROM task_managers WHERE task = ?1", [id], |r| {
                Ok(TaskManager {
                    task: r.get("task")?,
                    state: r.get("state")?,
                    brief_version: r.get("brief_version")?,
                    created: r.get("created")?,
                    finished: r.get("finished")?,
                    report: r.get("report")?,
                })
            })
            .optional()?)
    }

    pub fn managers(&self) -> Result<Vec<TaskManager>> {
        let mut q = self.0.conn().prepare("SELECT task FROM task_managers ORDER BY created, task")?;
        let ids = q.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        ids.iter().map(|id| self.manager(id)?.ok_or_else(|| GenieError::not_found(id))).collect()
    }

    /// Resolve the legacy address only for dispatched tasks. Unmanaged inbox and
    /// project events still reach the project manager; completed managers stay asleep.
    pub fn recipient(&self, task: Option<&str>) -> Result<String> {
        if let Some(task) = task
            && let Some(m) = self.manager(task)?
            && m.state != "completed"
        {
            return Ok(manager_name(&m.task));
        }
        Ok(ORCHESTRATOR.into())
    }

    pub fn notify(&self, recipient: &str, from: &str, text: &str, task: Option<&str>) -> Result<()> {
        self.0.bus().notify_manager(recipient, from, "orchestrator", "management", text, task)
    }

    pub fn dispatch(&self, actor: &Actor, task: &str) -> Result<TaskManager> {
        self.0.tx(|| {
            let task = self.0.get(task)?;
            if let Some(manager) = self.manager(&task.id)?
                && manager.state != "completed"
            {
                return Ok(manager);
            }
            if task.task_type == TaskType::Epic || CLOSED.contains(&task.status) {
                return Err(GenieError::invalid("only an open task can be dispatched"));
            }
            if task.status != Status::Ready {
                return Err(GenieError::invalid("make the task ready before dispatching its manager"));
            }
            let known = task.deps.iter().cloned().collect();
            let problems = crate::readiness_problems(&task, &known);
            if !problems.is_empty() {
                return Err(GenieError::invalid(format!("task is not ready: {}", problems.join("; "))));
            }
            if task.blocked.is_some() || task.needs_owner.is_some() {
                return Err(GenieError::invalid("the task is blocked or needs an owner decision"));
            }
            for dep in &task.deps {
                if self.0.get(dep)?.status != Status::Done {
                    return Err(GenieError::invalid(format!("dependency {dep} is not done")));
                }
            }
            self.adopt(actor, &task.id)
        })
    }

    /// Adoption is also the migration path for already running teams. Insertion
    /// and kickoff are atomic, so retries never create a second manager.
    pub fn adopt(&self, actor: &Actor, task: &str) -> Result<TaskManager> {
        self.0.tx(|| {
            let task = self.0.get(task)?;
            if task.task_type == TaskType::Epic {
                return Err(GenieError::invalid("epics do not have task managers"));
            }
            let state = if CLOSED.contains(&task.status) { "retrospective" } else { "active" };
            let n = self.0.conn().execute(
                "INSERT OR IGNORE INTO task_managers(task, state, created) VALUES (?1, ?2, ?3)", params![task.id, state, now()])?;
            self.sync_brief(actor, &task.id)?;
            if n > 0 {
                // Transfer undelivered work, including old team mail, without consuming it.
                self.0.conn().execute(
                    "UPDATE mail SET recipient = ?1 WHERE recipient = 'orchestrator' AND task = ?2 AND delivered_at IS NULL AND lease IS NULL AND delivery IS NULL",
                    params![manager_name(&task.id), task.id])?;
                self.notify(&manager_name(&task.id), &actor.name,
                    "You own this task from its current brief through acceptance and publication of its retrospective. Read `genie manager show` first; assemble a team only if one is not already active.", Some(&task.id))?;
                self.event("manager.dispatched", &task.id, actor, json!({}))?;
            }
            self.manager(&task.id)?.ok_or_else(|| GenieError::not_found(task.id))
        })
    }

    fn snapshot(&self, task: &str) -> Result<Value> {
        let t = self.0.get(task)?;
        Ok(json!({ "title": t.title, "type": t.task_type, "description": t.description,
            "acceptance": t.acceptance.iter().map(|a| &a.text).collect::<Vec<_>>(), "deps": t.deps,
            "parent": t.parent, "mergeStrategy": t.merge_strategy, "assignee": t.assignee }))
    }

    pub fn sync_brief(&self, actor: &Actor, task: &str) -> Result<()> {
        let Some(m) = self.manager(task)? else { return Ok(()) };
        self.0.tx(|| {
            let content = self.snapshot(&m.task)?.to_string();
            let last: Option<(i64, String)> = self.0.conn().query_row(
                "SELECT version, content FROM task_briefs WHERE task = ?1 ORDER BY version DESC LIMIT 1", [&m.task],
                |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            if last.as_ref().is_some_and(|(_, old)| old == &content) { return Ok(()) }
            let version = last.as_ref().map_or(1, |(v, _)| v + 1);
            self.0.conn().execute("INSERT INTO task_briefs(task, version, content, author, created) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![m.task, version, content, actor.name, now()])?;
            self.0.conn().execute("UPDATE task_managers SET brief_version = ?1 WHERE task = ?2", params![version, m.task])?;
            self.0.conn().execute("UPDATE clarifications SET state = 'obsolete' WHERE task = ?1 AND brief_version <> ?2 AND state IN ('project', 'owner')", params![m.task, version])?;
            if last.is_some() {
                self.notify(&manager_name(&m.task), &actor.name, &format!("Task brief changed to version {version}. Re-read the requirements before continuing; older pending clarifications are obsolete."), Some(&m.task))?;
            }
            self.event("manager.brief_changed", &m.task, actor, json!({ "version": version }))
        })
    }

    pub fn brief(&self, task: &str, version: Option<i64>) -> Result<Value> {
        let task = self.0.normalize_id(task)?;
        let (v, content): (i64, String) = self.0.conn().query_row(
            "SELECT version, content FROM task_briefs WHERE task = ?1 AND (?2 IS NULL OR version = ?2) ORDER BY version DESC LIMIT 1",
            params![task, version],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        Ok(json!({ "version": v, "content": serde_json::from_str::<Value>(&content)? }))
    }

    /// Called within the tracker's status transaction, including non-HTTP moves.
    pub fn on_status(&self, actor: &Actor, task: &str, status: Status) -> Result<()> {
        let Some(m) = self.manager(task)? else { return Ok(()) };
        if CLOSED.contains(&status) && m.state == "active" {
            self.0
                .conn()
                .execute("UPDATE clarifications SET state = 'obsolete' WHERE task = ?1 AND state IN ('project', 'owner')", [&m.task])?;
            self.0.conn().execute("UPDATE task_managers SET state = 'retrospective' WHERE task = ?1", [&m.task])?;
            self.notify(&manager_name(&m.task), "genie", "The task result is closed. Its team can stop; you remain responsible for the mandatory retrospective. Read all task session sources with `genie manager sources`, then publish the report with `genie manager retrospective`.", Some(&m.task))?;
            self.notify(ORCHESTRATOR, &manager_name(&m.task), "Task result closed; retrospective is pending separately.", Some(&m.task))?;
            self.event("manager.retrospective_pending", &m.task, actor, json!({}))?;
        } else if !CLOSED.contains(&status) && m.state != "active" {
            self.0
                .conn()
                .execute("UPDATE task_managers SET state = 'active', report = NULL, finished = NULL WHERE task = ?1", [&m.task])?;
            self.notify(&manager_name(&m.task), &actor.name, "The task was reopened. Resume its lifecycle from the current requirements; a new retrospective is required after the next closure.", Some(&m.task))?;
        }
        Ok(())
    }

    pub fn clarifications(&self, task: &str) -> Result<Vec<Clarification>> {
        let id = self.0.normalize_id(task)?;
        let mut q = self.0.conn().prepare("SELECT * FROM clarifications WHERE task = ?1 ORDER BY id")?;
        Ok(q.query_map([id], |r| {
            Ok(Clarification {
                id: r.get("id")?,
                task: r.get("task")?,
                brief_version: r.get("brief_version")?,
                question: r.get("question")?,
                blocking: r.get("blocking")?,
                state: r.get("state")?,
                answer: r.get("answer")?,
                source: r.get("source")?,
                created: r.get("created")?,
                answered: r.get("answered")?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }

    pub fn ask(&self, actor: &Actor, task: &str, question: &str, blocking: bool) -> Result<Clarification> {
        if question.trim().is_empty() {
            return Err(GenieError::invalid("a clarification needs a question"));
        }
        self.0.tx(|| {
            let m = self.manager(task)?.ok_or_else(|| GenieError::invalid("dispatch a task manager first"))?;
            if m.state != "active" { return Err(GenieError::invalid("the task is no longer executing")) }
            if let Some(q) = self.clarifications(&m.task)?.into_iter().find(|q| q.brief_version == m.brief_version && q.question == question && matches!(q.state.as_str(), "project" | "owner")) { return Ok(q) }
            self.0.conn().execute("INSERT INTO clarifications(task, brief_version, question, blocking, created) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![m.task, m.brief_version, question, blocking, now()])?;
            let id = self.0.conn().last_insert_rowid();
            self.notify(ORCHESTRATOR, &actor.name, &format!("Clarification #{id} for {} (brief {}): {question}\nAnswer with a verified source using `genie manager answer`, or escalate to a person using `genie manager escalate`.", m.task, m.brief_version), Some(&m.task))?;
            self.event("manager.clarification", &m.task, actor, json!({ "id": id }))?;
            self.clarifications(&m.task)?.into_iter().find(|q| q.id == id).ok_or_else(|| GenieError::not_found("clarification"))
        })
    }

    pub fn answer(&self, actor: &Actor, task: &str, id: i64, answer: &str, source: &str) -> Result<()> {
        if answer.trim().is_empty() || source.trim().is_empty() {
            return Err(GenieError::invalid("an answer needs text and a source"));
        }
        self.0.tx(|| {
            let q = self.clarifications(task)?.into_iter().find(|q| q.id == id).ok_or_else(|| GenieError::not_found("clarification"))?;
            if q.state == "answered" && q.answer.as_deref() == Some(answer) && q.source.as_deref() == Some(source) { return Ok(()) }
            let m = self.manager(task)?.ok_or_else(|| GenieError::not_found("manager"))?;
            if !matches!(q.state.as_str(), "project" | "owner") || q.brief_version != m.brief_version || m.state != "active" {
                return Err(GenieError::invalid("this clarification is obsolete or already answered; check the current brief"));
            }
            self.0.conn().execute("UPDATE clarifications SET state = 'answered', answer = ?1, source = ?2, answered = ?3 WHERE id = ?4",
                params![answer, source, now(), id])?;
            let current = self.0.get(&m.task)?;
            if current.status == Status::NeedsOwner && let Some(mut owner) = current.needs_owner
                && owner.question.starts_with("Clarification #") {
                if let Some(next) = self.clarifications(&m.task)?.into_iter().find(|q| q.state == "owner") {
                    owner.question = format!("Clarification #{}: {}", next.id, next.question);
                    self.0.conn().execute("UPDATE tasks SET needs_owner = ?1 WHERE id = ?2", params![serde_json::to_string(&owner)?, m.task])?;
                } else {
                    self.0.set_status(actor, &m.task, owner.previous, StatusOptions { note: Some(format!("Clarification #{id} answered: {answer}")), force: true, ..Default::default() })?;
                }
            }
            self.notify(&manager_name(&m.task), &actor.name, &format!("Clarification #{id} answered: {answer}\nSource: {source}\nRecord the decision and return the answer to the participant who raised it."), Some(&m.task))?;
            self.event("manager.clarification_answered", &m.task, actor, json!({ "id": id }))
        })
    }

    pub fn escalate(&self, actor: &Actor, task: &str, id: i64) -> Result<Clarification> {
        self.0.tx(|| {
            let q = self.clarifications(task)?.into_iter().find(|q| q.id == id).ok_or_else(|| GenieError::not_found("clarification"))?;
            if !matches!(q.state.as_str(), "project" | "owner") {
                return Err(GenieError::invalid("clarification is no longer open"));
            }
            self.0.conn().execute("UPDATE clarifications SET state = 'owner' WHERE id = ?1", [id])?;
            let current = self.0.get(&q.task)?;
            if current.status != Status::NeedsOwner {
                self.0.set_status(
                    actor,
                    &q.task,
                    Status::NeedsOwner,
                    StatusOptions { note: Some(format!("Clarification #{id}: {}", q.question)), ..Default::default() },
                )?;
            }
            self.event("manager.clarification_escalated", &q.task, actor, json!({ "id": id }))?;
            self.clarifications(task)?.into_iter().find(|q| q.id == id).ok_or_else(|| GenieError::not_found("clarification"))
        })
    }

    pub fn has_blocking_questions(&self, task: &str) -> Result<bool> {
        Ok(self.clarifications(task)?.iter().any(|q| q.blocking && matches!(q.state.as_str(), "project" | "owner")))
    }

    pub fn publish_retrospective(&self, actor: &Actor, task: &str, report: &str, complete: bool) -> Result<TaskManager> {
        if report.trim().is_empty() {
            return Err(GenieError::invalid("a retrospective needs a report"));
        }
        self.0.tx(|| {
            let m = self.manager(task)?.ok_or_else(|| GenieError::not_found("manager"))?;
            if m.state == "completed" { return Ok(m) }
            if m.state != "retrospective" { return Err(GenieError::invalid("close the task result before publishing its retrospective")) }
            let t = self.0.add_artifact(actor, &m.task, ArtifactInput { name: Some("retrospective.md".into()),
                note: Some(if complete { "Complete task retrospective" } else { "Incomplete source coverage; retrospective still pending" }.into()),
                source: ArtifactSource::Content(report.as_bytes().to_vec()), kind: None })?;
            let n = t.artifacts.last().map(|a| a.id);
            self.0.conn().execute("UPDATE task_managers SET state = ?1, report = ?2, finished = ?3 WHERE task = ?4",
                params![if complete { "completed" } else { "retrospective" }, n, complete.then(now), m.task])?;
            self.notify(ORCHESTRATOR, &actor.name, &format!("Retrospective {} for {}: artifact #{}. Read its findings and preserve supported lessons in project memory; proposed prompt/role changes are not applied automatically.", if complete { "published" } else { "partially published (sources missing)" }, m.task, n.unwrap_or_default()), Some(&m.task))?;
            self.event("manager.retrospective_published", &m.task, actor, json!({ "complete": complete, "artifact": n }))?;
            self.manager(&m.task)?.ok_or_else(|| GenieError::not_found(m.task))
        })
    }

    pub fn remember(&self, actor: &Actor, text: &str, source: &str, replaces: Option<i64>) -> Result<i64> {
        if text.trim().is_empty() || source.trim().is_empty() {
            return Err(GenieError::invalid("project memory needs text and a source"));
        }
        self.0.tx(|| {
            self.0.conn().execute(
                "INSERT INTO project_memory(text, source, author, created) VALUES (?1, ?2, ?3, ?4)",
                params![text, source, actor.name, now()],
            )?;
            let id = self.0.conn().last_insert_rowid();
            if let Some(old) = replaces {
                let n = self
                    .0
                    .conn()
                    .execute("UPDATE project_memory SET superseded_by = ?1 WHERE id = ?2 AND superseded_by IS NULL", params![id, old])?;
                if n != 1 {
                    return Err(GenieError::invalid("memory to replace is missing or already superseded"));
                }
            }
            self.0.append_event("memory.changed", None, &actor.name, actor.role.as_str(), json!({ "id": id }))?;
            Ok(id)
        })
    }

    pub fn memory(&self, query: &str) -> Result<Vec<Value>> {
        let query = query.to_lowercase();
        let mut q = self
            .0
            .conn()
            .prepare("SELECT id, text, source, author, created FROM project_memory WHERE superseded_by IS NULL ORDER BY id DESC")?;
        let rows = q.query_map([], |r| Ok(json!({ "id": r.get::<_, i64>(0)?, "text": r.get::<_, String>(1)?, "source": r.get::<_, String>(2)?, "author": r.get::<_, String>(3)?, "created": r.get::<_, String>(4)? })))?.collect::<rusqlite::Result<Vec<Value>>>()?;
        Ok(rows.into_iter().filter(|r| r["text"].as_str().is_some_and(|s| s.to_lowercase().contains(&query))).take(100).collect())
    }

    pub fn record_execution(&self, task: &str, agent: &str, content: &Value) -> Result<()> {
        self.0.conn().execute(
            "INSERT INTO execution_events(task, agent, at, content) VALUES (?1, ?2, ?3, ?4)",
            params![task, agent, now(), content.to_string()],
        )?;
        Ok(())
    }

    pub fn execution_sources(&self, task: &str, cutoff: Option<&str>) -> Result<Vec<String>> {
        let mut q = self
            .0
            .conn()
            .prepare("SELECT DISTINCT agent FROM execution_events WHERE task = ?1 AND (?2 IS NULL OR agent NOT LIKE 'task-manager:%' OR at <= ?2) ORDER BY agent")?;
        Ok(q.query_map(params![task, cutoff], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
    }

    pub fn execution_source(&self, task: &str, agent: &str, cutoff: Option<&str>, after: i64) -> Result<Vec<Value>> {
        let mut q = self.0.conn().prepare("SELECT id, at, content FROM execution_events WHERE task = ?1 AND agent = ?2 AND (?3 IS NULL OR agent NOT LIKE 'task-manager:%' OR at <= ?3) AND id > ?4 ORDER BY id LIMIT 20")?;
        Ok(q.query_map(params![task, agent, cutoff, after], |r| {
            Ok(json!({ "id": r.get::<_, i64>(0)?, "at": r.get::<_, String>(1)?, "content": r.get::<_, String>(2)? }))
        })?
        .collect::<rusqlite::Result<_>>()?)
    }

    fn event(&self, name: &str, task: &str, actor: &Actor, payload: Value) -> Result<()> {
        self.0.append_event(name, Some(task), &actor.name, actor.role.as_str(), payload).map(|_| ())
    }
}
