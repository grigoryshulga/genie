//! The task tracker: tasks, epics, the role-checked workflow, acceptance,
//! comments, artifacts and history. Ported from the TypeScript tracker: error
//! messages and rules match it; every change also appends to the event journal
//! in the same transaction.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, Row, params, params_from_iter};
use serde::Serialize;
use serde_json::{Value, json};

use crate::db::{Db, SCHEMA_VERSION, now};
use crate::error::{GenieError, Result};
use crate::events::{self, Event};
use crate::model::*;

pub const DB_FILE: &str = "genie.db";
pub const MAX_ARTIFACT_BYTES: usize = 5 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Meta {
    pub prefix: String,
    pub project: String,
    pub created: String,
}

#[derive(Debug, Clone, Default)]
pub struct CreateInput {
    pub title: String,
    pub task_type: Option<TaskType>,
    pub description: Option<String>,
    pub acceptance: Vec<String>,
    pub priority: Option<i64>,
    pub parent: Option<String>,
    pub deps: Vec<String>,
    /// `None` inherits the parent's labels when splitting.
    pub labels: Option<Vec<String>>,
    pub merge_strategy: Option<String>,
    pub plan: Option<String>,
    /// Initial status: `Inbox` for owner submissions, `Draft` otherwise.
    pub status: Option<Status>,
    /// Do not tell the orchestrator: the owner is still shaping the task
    /// (an idea with a planner), or tells it once for a whole batch.
    pub quiet: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UpdateInput {
    pub title: Option<String>,
    pub task_type: Option<TaskType>,
    pub description: Option<String>,
    pub plan: Option<String>,
    /// Appended to notes with a timestamp header.
    pub append_notes: Option<String>,
    pub notes: Option<String>,
    pub priority: Option<i64>,
    pub labels: Option<Vec<String>>,
    pub assignees: Option<Vec<String>>,
    pub merge_strategy: Option<String>,
    /// The person responsible (`Some(None)` clears it).
    pub assignee: Option<Option<String>>,
    pub add_acceptance: Vec<String>,
    pub remove_acceptance: Vec<i64>,
    pub add_deps: Vec<String>,
    pub remove_deps: Vec<String>,
    /// `Some(Some(epic))` moves the task into an epic, `Some(None)` out of it.
    pub parent: Option<Option<String>>,
}

#[derive(Debug, Clone, Default)]
pub struct ListFilter {
    pub task_type: Vec<TaskType>,
    /// Hide epics (task lists and boards show epics separately).
    pub exclude_epics: bool,
    pub status: Vec<Status>,
    pub team: Option<String>,
    pub parent: Option<String>,
    pub label: Option<String>,
    pub include_closed: bool,
    pub search: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct StatusOptions {
    pub note: Option<String>,
    pub force: bool,
    /// needs_owner only: what the owner can do besides answering in words.
    pub action: Option<OwnerAction>,
}

#[derive(Debug, Clone)]
pub enum ArtifactSource {
    File(PathBuf),
    Content(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct ArtifactInput {
    pub kind: Option<ArtifactKind>,
    pub source: ArtifactSource,
    pub name: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ArtifactContent {
    pub name: String,
    pub kind: ArtifactKind,
    pub content: Vec<u8>,
    /// The content as text when it is valid UTF-8.
    pub text: Option<String>,
}

/// Tasks (id, title; the requested one first, then its subtasks) and teams that go with a deletion.
#[derive(Debug, Clone, Default)]
pub struct DeletePlan {
    pub tasks: Vec<(String, String)>,
    pub teams: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct EpicContext {
    pub epic: Option<Task>,
    pub children: Option<Vec<TaskSummary>>,
}

struct TaskRow {
    id: String,
    title: String,
    task_type: TaskType,
    status: Status,
    priority: i64,
    description: String,
    plan: String,
    notes: String,
    parent: Option<String>,
    labels: String,
    assignees: String,
    team: Option<String>,
    worktree: Option<String>,
    blocked: Option<String>,
    needs_owner: Option<String>,
    merge_strategy: String,
    assignee: String,
    created: String,
    updated: String,
}

impl TaskRow {
    fn from_row(r: &Row<'_>) -> rusqlite::Result<TaskRow> {
        Ok(TaskRow {
            id: r.get("id")?,
            title: r.get("title")?,
            task_type: r.get("type")?,
            status: r.get("status")?,
            priority: r.get("priority")?,
            description: r.get("description")?,
            plan: r.get("plan")?,
            notes: r.get("notes")?,
            parent: r.get("parent")?,
            labels: r.get("labels")?,
            assignees: r.get("assignees")?,
            team: r.get("team")?,
            worktree: r.get("worktree")?,
            blocked: r.get("blocked")?,
            needs_owner: r.get("needs_owner")?,
            merge_strategy: r.get("merge_strategy")?,
            assignee: r.get("assignee")?,
            created: r.get("created")?,
            updated: r.get("updated")?,
        })
    }
}

/// The action of a question for the owner, in words for the task's comments.
fn action_line(a: Option<&OwnerAction>) -> String {
    match a {
        Some(OwnerAction::AskOwnerQuestion { options }) => format!(" (options: {})", options.join(" / ")),
        Some(OwnerAction::AskForMergePr { repo, number, .. }) => {
            format!(" (asks to merge {repo}{})", number.map(|n| format!(" #{n}")).unwrap_or_default())
        }
        Some(OwnerAction::AskFreeForm) | None => String::new(),
    }
}

fn parse_json<T: serde::de::DeserializeOwned>(v: Option<&str>) -> Option<T> {
    v.filter(|s| !s.is_empty()).and_then(|s| serde_json::from_str(s).ok())
}

fn deny(actor: &Actor, what: &str) -> GenieError {
    GenieError::Denied(format!("role \"{}\" ({}) is not allowed to {what}", actor.role, actor.name))
}

fn require_role(actor: &Actor, what: &str, roles: &[Role]) -> Result<()> {
    if is_privileged(actor.role) || roles.contains(&actor.role) { Ok(()) } else { Err(deny(actor, what)) }
}

/// A team role needs the permission (people and the orchestrator hold them all).
fn require_cap(actor: &Actor, what: &str, cap: Capability) -> Result<()> {
    if actor.can(cap) { Ok(()) } else { Err(deny(actor, what)) }
}

fn clamp_priority(p: i64) -> i64 {
    p.clamp(0, 4)
}

/// Replace every run of characters outside `[A-Za-z0-9_.-]` with one `_`.
fn sanitize(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut in_run = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
            out.push(c);
            in_run = false;
        } else if !in_run {
            out.push('_');
            in_run = true;
        }
    }
    out
}

fn system_actor() -> Actor {
    Actor::new("genie", Role::Orchestrator)
}

pub struct Tracker {
    dir: PathBuf,
    db: Db,
    pub gates: Gates,
}

impl Tracker {
    /// Open an existing tracker directory (the one holding `genie.db`).
    pub fn open(dir: impl AsRef<Path>) -> Result<Tracker> {
        let dir = dir.as_ref().to_path_buf();
        let file = dir.join(DB_FILE);
        if !file.exists() {
            return Err(GenieError::not_found(format!("no genie tracker in {}; run `genie init` first", dir.display())));
        }
        Ok(Tracker { db: Db::open(&file)?, dir, gates: Gates::default() })
    }

    /// Create (or reopen) a tracker directory. Existing meta values are kept.
    pub fn init(dir: impl AsRef<Path>, prefix: Option<&str>, project: Option<&str>) -> Result<Tracker> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let db = Db::open(&dir.join(DB_FILE))?;
        let default_project = std::fs::canonicalize(&dir)
            .ok()
            .and_then(|p| p.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "project".to_string());
        db.tx(|| {
            let set = |k: &str, v: &str| db.conn().execute("INSERT OR IGNORE INTO meta(key, value) VALUES (?1, ?2)", [k, v]);
            set("schema", &SCHEMA_VERSION.to_string())?;
            set("prefix", &prefix.unwrap_or("G").to_uppercase())?;
            set("project", project.unwrap_or(&default_project))?;
            set("created", &now())?;
            set("next_seq", "1")?;
            Ok(())
        })?;
        Ok(Tracker { db, dir, gates: Gates::default() })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Raw connection, for the team bus and diagnostics that live outside this module.
    pub fn conn(&self) -> &rusqlite::Connection {
        self.db.conn()
    }

    /// Write transaction on the tracker database (nested calls join the outer one).
    pub fn tx<T>(&self, f: impl FnOnce() -> Result<T>) -> Result<T> {
        self.db.tx(f)
    }

    fn meta_value(&self, key: &str) -> Result<String> {
        Ok(self.conn().query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).optional()?.unwrap_or_default())
    }

    pub fn meta(&self) -> Result<Meta> {
        Ok(Meta { prefix: self.meta_value("prefix")?, project: self.meta_value("project")?, created: self.meta_value("created")? })
    }

    /// `7` → `G-7`, `g-7` → `G-7`.
    pub fn normalize_id(&self, id: &str) -> Result<String> {
        let raw = id.trim().to_uppercase();
        if !raw.is_empty() && raw.chars().all(|c| c.is_ascii_digit()) {
            return Ok(format!("{}-{raw}", self.meta_value("prefix")?));
        }
        Ok(raw)
    }

    pub fn exists(&self, id: &str) -> Result<bool> {
        let id = self.normalize_id(id)?;
        Ok(self.conn().query_row("SELECT 1 FROM tasks WHERE id = ?1", [&id], |_| Ok(())).optional()?.is_some())
    }

    fn row(&self, id: &str) -> Result<TaskRow> {
        let nid = self.normalize_id(id)?;
        self.conn()
            .query_row("SELECT * FROM tasks WHERE id = ?1", [&nid], TaskRow::from_row)
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("task {nid} not found")))
    }

    fn strings(&self, sql: &str, id: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn().prepare_cached(sql)?;
        let out = stmt.query_map([id], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(out)
    }

    /// Dependencies of a task that are not done yet.
    pub fn open_deps(&self, id: &str) -> Result<Vec<String>> {
        self.strings("SELECT d.dep FROM deps d JOIN tasks x ON x.id = d.dep WHERE d.task = ?1 AND x.status != 'done' ORDER BY d.dep", id)
    }

    /// Tasks that depend on this one.
    pub fn dependents(&self, id: &str) -> Result<Vec<String>> {
        self.strings("SELECT task FROM deps WHERE dep = ?1 ORDER BY task", id)
    }

    pub fn get(&self, id: &str) -> Result<Task> {
        let r = self.row(id)?;
        let conn = self.conn();
        let acceptance = conn
            .prepare_cached("SELECT n, text, done, checked_by, checked_at FROM acceptance WHERE task = ?1 ORDER BY n")?
            .query_map([&r.id], |a| {
                Ok(AcceptanceCriterion {
                    id: a.get(0)?,
                    text: a.get(1)?,
                    done: a.get::<_, i64>(2)? != 0,
                    checked_by: a.get(3)?,
                    checked_at: a.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let comments = conn
            .prepare_cached("SELECT id, at, author, role, kind, text FROM comments WHERE task = ?1 ORDER BY id")?
            .query_map([&r.id], |c| {
                Ok(Comment { id: c.get(0)?, at: c.get(1)?, author: c.get(2)?, role: c.get(3)?, kind: c.get(4)?, text: c.get(5)? })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let artifacts = conn
            .prepare_cached("SELECT n, at, author, role, kind, name, size, note FROM artifacts WHERE task = ?1 ORDER BY n")?
            .query_map([&r.id], |a| {
                Ok(Artifact {
                    id: a.get(0)?,
                    at: a.get(1)?,
                    author: a.get(2)?,
                    role: a.get(3)?,
                    kind: a.get(4)?,
                    name: a.get(5)?,
                    size: a.get(6)?,
                    note: a.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let history = conn
            .prepare_cached("SELECT at, actor, role, event, from_status, to_status, note FROM history WHERE task = ?1 ORDER BY id")?
            .query_map([&r.id], |h| {
                Ok(HistoryEntry {
                    at: h.get(0)?,
                    actor: h.get(1)?,
                    role: h.get(2)?,
                    event: h.get(3)?,
                    from: h.get(4)?,
                    to: h.get(5)?,
                    note: h.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Task {
            children: self.strings("SELECT id FROM tasks WHERE parent = ?1 ORDER BY seq", &r.id)?,
            deps: self.strings("SELECT dep FROM deps WHERE task = ?1 ORDER BY dep", &r.id)?,
            labels: parse_json(Some(&r.labels)).unwrap_or_default(),
            assignees: parse_json(Some(&r.assignees)).unwrap_or_default(),
            worktree: parse_json(r.worktree.as_deref()),
            blocked: parse_json(r.blocked.as_deref()),
            needs_owner: parse_json(r.needs_owner.as_deref()),
            id: r.id,
            title: r.title,
            task_type: r.task_type,
            status: r.status,
            priority: r.priority,
            description: r.description,
            acceptance,
            plan: r.plan,
            notes: r.notes,
            merge_strategy: r.merge_strategy,
            assignee: Some(r.assignee).filter(|a| !a.is_empty()),
            parent: r.parent,
            team: r.team,
            comments,
            artifacts,
            history,
            created: r.created,
            updated: r.updated,
        })
    }

    pub fn list(&self, filter: &ListFilter) -> Result<Vec<TaskSummary>> {
        let mut wh: Vec<String> = Vec::new();
        let mut params: Vec<String> = Vec::new();
        let placeholders = |n: usize| vec!["?"; n].join(",");
        if !filter.status.is_empty() {
            wh.push(format!("t.status IN ({})", placeholders(filter.status.len())));
            params.extend(filter.status.iter().map(|s| s.as_str().to_string()));
        } else if !filter.include_closed {
            wh.push("t.status NOT IN ('done', 'cancelled')".into());
        }
        if let Some(team) = &filter.team {
            wh.push("t.team = ?".into());
            params.push(team.clone());
        }
        if !filter.task_type.is_empty() {
            wh.push(format!("t.type IN ({})", placeholders(filter.task_type.len())));
            params.extend(filter.task_type.iter().map(|t| t.as_str().to_string()));
        }
        if filter.exclude_epics {
            wh.push("t.type != 'epic'".into());
        }
        if let Some(parent) = &filter.parent {
            wh.push("t.parent = ?".into());
            params.push(self.normalize_id(parent)?);
        }
        if let Some(label) = &filter.label {
            wh.push("EXISTS (SELECT 1 FROM json_each(t.labels) WHERE value = ?)".into());
            params.push(label.clone());
        }
        if let Some(search) = &filter.search {
            wh.push("(t.title LIKE ? OR t.id LIKE ?)".into());
            params.push(format!("%{search}%"));
            params.push(format!("%{search}%"));
        }
        let sql = format!(
            "SELECT t.*,
              (SELECT COUNT(*) FROM acceptance a WHERE a.task = t.id AND a.done = 1) AS ac_done,
              (SELECT COUNT(*) FROM acceptance a WHERE a.task = t.id) AS ac_total,
              (SELECT COUNT(*) FROM tasks c WHERE c.parent = t.id) AS n_children,
              (SELECT COUNT(*) FROM tasks c WHERE c.parent = t.id AND c.status IN ('done', 'cancelled')) AS n_children_closed,
              (SELECT COUNT(*) FROM comments c WHERE c.task = t.id) AS n_comments,
              (SELECT COUNT(*) FROM artifacts a WHERE a.task = t.id) AS n_artifacts,
              (SELECT group_concat(d.dep) FROM deps d WHERE d.task = t.id) AS deps_all,
              (SELECT group_concat(d.dep) FROM deps d JOIN tasks x ON x.id = d.dep WHERE d.task = t.id AND x.status != 'done') AS deps_open
             FROM tasks t {}
             ORDER BY t.priority, t.seq",
            if wh.is_empty() { String::new() } else { format!("WHERE {}", wh.join(" AND ")) }
        );
        let split = |s: Option<String>| s.map(|s| s.split(',').map(str::to_string).collect()).unwrap_or_default();
        let mut stmt = self.conn().prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(params.iter()), |r| {
            let t = TaskRow::from_row(r)?;
            Ok(TaskSummary {
                labels: parse_json(Some(&t.labels)).unwrap_or_default(),
                blocked: parse_json(t.blocked.as_deref()),
                needs_owner: parse_json(t.needs_owner.as_deref()),
                acceptance_done: r.get("ac_done")?,
                acceptance_total: r.get("ac_total")?,
                deps: split(r.get("deps_all")?),
                open_deps: split(r.get("deps_open")?),
                children: r.get("n_children")?,
                children_closed: r.get("n_children_closed")?,
                comments: r.get("n_comments")?,
                artifacts: r.get("n_artifacts")?,
                id: t.id,
                title: t.title,
                task_type: t.task_type,
                status: t.status,
                priority: t.priority,
                parent: t.parent,
                team: t.team,
                assignee: Some(t.assignee).filter(|a| !a.is_empty()),
                created: t.created,
                updated: t.updated,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Ready tasks whose dependencies are done, not blocked and not yet taken by a team.
    pub fn ready_queue(&self) -> Result<Vec<TaskSummary>> {
        let ready = self.list(&ListFilter { status: vec![Status::Ready], ..Default::default() })?;
        Ok(ready.into_iter().filter(|t| t.blocked.is_none() && t.team.is_none() && t.open_deps.is_empty()).collect())
    }

    fn history(&self, task: &str, actor: &Actor, event: &str, from: Option<&str>, to: Option<&str>, note: Option<&str>) -> Result<()> {
        self.conn().execute(
            "INSERT INTO history(task, at, actor, role, event, from_status, to_status, note) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![task, now(), actor.name, actor.role, event, from, to, note],
        )?;
        Ok(())
    }

    /// Who moved the task to `review` most recently.
    fn last_submitter(&self, task: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT actor FROM history WHERE task = ?1 AND event = 'status' AND to_status = 'review' ORDER BY id DESC LIMIT 1",
                [task],
                |r| r.get(0),
            )
            .optional()?)
    }

    fn touch(&self, task: &str) -> Result<()> {
        self.conn().execute("UPDATE tasks SET updated = ?1 WHERE id = ?2", params![now(), task])?;
        Ok(())
    }

    fn event(&self, kind: &str, subject: &str, actor: &Actor, payload: Value) -> Result<()> {
        events::append(self.conn(), kind, Some(subject), &actor.name, actor.role.as_str(), payload)?;
        Ok(())
    }

    /// System mail to the orchestrator's global mailbox (the team bus delivers it).
    fn mail_orchestrator(&self, sender: &str, sender_role: &str, kind: &str, task: &str, text: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO mail(team, at, sender, sender_role, recipient, text, urgent, kind, task) VALUES (NULL, ?1, ?2, ?3, 'orchestrator', ?4, 0, ?5, ?6)",
            params![now(), sender, sender_role, text, kind, task],
        )?;
        let id = self.conn().last_insert_rowid();
        events::append(
            self.conn(),
            events::MAIL_SENT,
            Some(task),
            sender,
            sender_role,
            json!({ "mail": id, "recipient": "orchestrator", "kind": kind }),
        )?;
        Ok(())
    }

    /// Owner activity wakes the orchestrator through its global mailbox.
    fn tell_orchestrator(&self, actor: &Actor, task: &str, text: &str) -> Result<()> {
        if actor.role != Role::Human {
            return Ok(());
        }
        self.mail_orchestrator(&actor.name, "human", "owner", task, text)
    }

    /// Keep an epic in step with its tasks: the first task a team works on starts
    /// the epic; when every task is closed the orchestrator is asked to close it.
    fn follow_epic(&self, epic_id: &str, child_id: &str, child_status: Status) -> Result<()> {
        let epic: Option<(String, TaskType, Status)> = self
            .conn()
            .query_row("SELECT id, type, status FROM tasks WHERE id = ?1", [epic_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .optional()?;
        let Some((epic_id, TaskType::Epic, epic_status)) = epic else {
            return Ok(());
        };
        let system = system_actor();
        if WORKING.contains(&child_status) && matches!(epic_status, Status::Draft | Status::Refining | Status::Ready) {
            self.db.tx(|| {
                self.conn().execute("UPDATE tasks SET status = 'in_progress', updated = ?1 WHERE id = ?2", params![now(), epic_id])?;
                let note = format!("work started on {child_id}");
                self.history(&epic_id, &system, "status", Some(epic_status.as_str()), Some("in_progress"), Some(&note))?;
                self.event(
                    events::TASK_STATUS_CHANGED,
                    &epic_id,
                    &system,
                    json!({ "from": epic_status, "to": Status::InProgress, "note": note, "force": false }),
                )
            })?;
        }
        if CLOSED.contains(&child_status) && !CLOSED.contains(&epic_status) {
            let open: i64 = self.conn().query_row(
                "SELECT COUNT(*) FROM tasks WHERE parent = ?1 AND status NOT IN ('done', 'cancelled')",
                [&epic_id],
                |r| r.get(0),
            )?;
            if open == 0 {
                let text = format!(
                    "All tasks of epic {epic_id} are closed. Check the epic's success criteria and artifacts, then close it (done) or add the missing tasks."
                );
                self.db.tx(|| self.mail_orchestrator("genie", "system", "system", &epic_id, &text))?;
            }
        }
        Ok(())
    }

    /// The epic of a task plus the epic's tasks, for rendering context.
    pub fn epic_context(&self, id: &str) -> Result<EpicContext> {
        let t = self.row(id)?;
        if t.task_type == TaskType::Epic {
            let children = self.list(&ListFilter { parent: Some(t.id), include_closed: true, ..Default::default() })?;
            return Ok(EpicContext { epic: None, children: Some(children) });
        }
        Ok(match self.epic_of(&t.id)? {
            Some(epic) => EpicContext { epic: Some(self.get(&epic)?), children: None },
            None => EpicContext::default(),
        })
    }

    /// The epic a task belongs to (directly or through its parent task), if any.
    pub fn epic_of(&self, id: &str) -> Result<Option<String>> {
        let parent_of = |id: &str| -> Result<Option<(TaskType, Option<String>)>> {
            Ok(self.conn().query_row("SELECT type, parent FROM tasks WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?)
        };
        let mut cur = parent_of(&self.normalize_id(id)?)?.and_then(|(_, p)| p);
        for _ in 0..10 {
            let Some(c) = cur else { return Ok(None) };
            match parent_of(&c)? {
                None => return Ok(None),
                Some((TaskType::Epic, _)) => return Ok(Some(c)),
                Some((_, p)) => cur = p,
            }
        }
        Ok(None)
    }

    pub fn create(&self, actor: &Actor, input: CreateInput) -> Result<Task> {
        require_cap(actor, "create tasks", Capability::TaskCreate)?;
        let title = input.title.trim().to_string();
        if title.is_empty() {
            return Err(GenieError::invalid("title is required"));
        }
        let task_type = input.task_type.unwrap_or(TaskType::Task);
        let plan = input.plan.clone().filter(|p| !p.trim().is_empty());
        if plan.is_some() {
            require_cap(actor, "write the plan", Capability::TaskPlan)?;
        }
        let status = if input.status == Some(Status::Inbox) { Status::Inbox } else { Status::Draft };
        let id = self.db.tx(|| {
            let parent = input.parent.as_deref().map(|p| self.normalize_id(p)).transpose()?;
            if let Some(p) = &parent {
                if !self.exists(p)? {
                    return Err(GenieError::not_found(format!("parent {p} not found")));
                }
                if task_type == TaskType::Epic {
                    return Err(GenieError::invalid("epics cannot be nested"));
                }
            }
            let deps = input.deps.iter().map(|d| self.normalize_id(d)).collect::<Result<Vec<_>>>()?;
            for d in &deps {
                if !self.exists(d)? {
                    return Err(GenieError::not_found(format!("dependency {d} not found")));
                }
            }
            let seq: i64 = self.meta_value("next_seq")?.parse().unwrap_or(1);
            self.conn().execute("UPDATE meta SET value = ?1 WHERE key = 'next_seq'", [(seq + 1).to_string()])?;
            let tid = format!("{}-{seq}", self.meta_value("prefix")?);
            let at = now();
            self.conn().execute(
                "INSERT INTO tasks(id, seq, title, type, status, priority, description, parent, labels, merge_strategy, plan, created, updated)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    tid,
                    seq,
                    title,
                    task_type,
                    status,
                    clamp_priority(input.priority.unwrap_or(2)),
                    input.description.clone().unwrap_or_default(),
                    parent,
                    serde_json::to_string(&input.labels.clone().unwrap_or_default())?,
                    input.merge_strategy.clone().unwrap_or_default(),
                    plan.unwrap_or_default(),
                    at,
                    at,
                ],
            )?;
            for (i, text) in input.acceptance.iter().enumerate() {
                self.conn().execute("INSERT INTO acceptance(task, n, text) VALUES (?1, ?2, ?3)", params![tid, i as i64 + 1, text])?;
            }
            for d in &deps {
                self.conn().execute("INSERT INTO deps(task, dep) VALUES (?1, ?2)", params![tid, d])?;
            }
            self.history(&tid, actor, "created", None, Some(status.as_str()), None)?;
            if let Some(p) = &parent {
                self.history(p, actor, &format!("child {tid} added"), None, None, None)?;
                self.touch(p)?;
            }
            self.event(
                events::TASK_CREATED,
                &tid,
                actor,
                json!({ "title": title, "type": task_type, "status": status, "parent": parent }),
            )?;
            if !input.quiet {
                self.tell_orchestrator(actor, &tid, &format!("New task {tid} in the inbox from the owner: {title}"))?;
            }
            Ok(tid)
        })?;
        self.get(&id)
    }

    pub fn update(&self, actor: &Actor, id: &str, input: UpdateInput) -> Result<Task> {
        let r = self.row(id)?;
        let scope = |fields: &str, cap: Capability| require_cap(actor, &format!("change {fields}"), cap);
        let privileged = |fields: &str| require_role(actor, &format!("change {fields}"), &[]);
        self.db.tx(|| {
            let mut changed: Vec<String> = Vec::new();
            let mut set = |col: &str, value: &dyn rusqlite::ToSql, name: &str| -> Result<()> {
                self.conn().execute(&format!("UPDATE tasks SET {col} = ?1 WHERE id = ?2"), params![value, r.id])?;
                changed.push(name.to_string());
                Ok(())
            };
            if let Some(title) = &input.title {
                scope("title", Capability::TaskScope)?;
                set("title", title, "title")?;
            }
            if let Some(t) = input.task_type {
                scope("type", Capability::TaskScope)?;
                if t == TaskType::Epic && r.parent.is_some() && !matches!(input.parent, Some(None)) {
                    return Err(GenieError::invalid(format!(
                        "{} is inside {}; epics cannot be nested",
                        r.id,
                        r.parent.as_deref().unwrap_or_default()
                    )));
                }
                set("type", &t, "type")?;
            }
            if let Some(d) = &input.description {
                scope("description", Capability::TaskScope)?;
                set("description", d, "description")?;
            }
            if let Some(p) = input.priority {
                privileged("priority")?;
                set("priority", &clamp_priority(p), "priority")?;
            }
            if let Some(m) = &input.merge_strategy {
                privileged("merge strategy")?;
                set("merge_strategy", m, "merge strategy")?;
            }
            if let Some(a) = &input.assignee {
                privileged("the person responsible")?;
                set("assignee", &a.as_deref().map(str::trim).unwrap_or_default(), "assignee")?;
            }
            if let Some(p) = &input.plan {
                scope("plan", Capability::TaskPlan)?;
                set("plan", p, "plan")?;
            }
            if let Some(n) = &input.notes {
                set("notes", n, "notes")?;
            }
            if let Some(extra) = input.append_notes.as_deref().filter(|s| !s.is_empty()) {
                let cur = self.row(&r.id)?.notes;
                let head = if cur.is_empty() { String::new() } else { format!("{}\n\n", cur.trim_end()) };
                let notes = format!("{head}### {} — {} ({})\n\n{}\n", now(), actor.name, actor.role, extra.trim());
                set("notes", &notes, "notes")?;
            }
            if let Some(labels) = &input.labels {
                set("labels", &serde_json::to_string(labels)?, "labels")?;
            }
            if let Some(assignees) = &input.assignees {
                privileged("assignees")?;
                set("assignees", &serde_json::to_string(assignees)?, "assignees")?;
            }
            if !input.add_acceptance.is_empty() {
                scope("acceptance criteria", Capability::TaskScope)?;
                let first: i64 =
                    self.conn().query_row("SELECT COALESCE(MAX(n), 0) FROM acceptance WHERE task = ?1", [&r.id], |x| x.get::<_, i64>(0))?
                        + 1;
                for (n, text) in (first..).zip(&input.add_acceptance) {
                    self.conn().execute("INSERT INTO acceptance(task, n, text) VALUES (?1, ?2, ?3)", params![r.id, n, text])?;
                }
                changed.push("acceptance".into());
            }
            if !input.remove_acceptance.is_empty() {
                scope("acceptance criteria", Capability::TaskScope)?;
                for n in &input.remove_acceptance {
                    self.conn().execute("DELETE FROM acceptance WHERE task = ?1 AND n = ?2", params![r.id, n])?;
                }
                changed.push("acceptance".into());
            }
            if let Some(parent) = &input.parent {
                scope("epic", Capability::TaskScope)?;
                let target = parent.as_deref().map(|p| self.normalize_id(p)).transpose()?;
                if target != r.parent {
                    if let Some(tg) = &target {
                        let epic: Option<TaskType> =
                            self.conn().query_row("SELECT type FROM tasks WHERE id = ?1", [tg], |x| x.get(0)).optional()?;
                        match epic {
                            None => {
                                return Err(GenieError::not_found(format!("epic {tg} not found")));
                            }
                            Some(t) if t != TaskType::Epic => {
                                return Err(GenieError::invalid(format!("{tg} is not an epic (type {t})")));
                            }
                            _ => {}
                        }
                        if *tg == r.id {
                            return Err(GenieError::invalid("a task cannot be its own epic"));
                        }
                        if r.task_type == TaskType::Epic {
                            return Err(GenieError::invalid("epics cannot be nested"));
                        }
                    }
                    if let Some(old) = &r.parent {
                        self.history(old, actor, &format!("child {} moved out", r.id), None, None, None)?;
                    }
                    if let Some(tg) = &target {
                        self.history(tg, actor, &format!("child {} moved in", r.id), None, None, None)?;
                    }
                    self.conn().execute("UPDATE tasks SET parent = ?1 WHERE id = ?2", params![target, r.id])?;
                    changed.push(match &target {
                        Some(tg) => format!("epic → {tg}"),
                        None => "epic removed".into(),
                    });
                }
            }
            if !input.add_deps.is_empty() || !input.remove_deps.is_empty() {
                scope("dependencies", Capability::TaskScope)?;
                for raw in &input.add_deps {
                    let d = self.normalize_id(raw)?;
                    if !self.exists(&d)? {
                        return Err(GenieError::not_found(format!("dependency {d} not found")));
                    }
                    if d == r.id {
                        return Err(GenieError::invalid("a task cannot depend on itself"));
                    }
                    self.conn().execute("INSERT OR IGNORE INTO deps(task, dep) VALUES (?1, ?2)", params![r.id, d])?;
                }
                for raw in &input.remove_deps {
                    self.conn().execute("DELETE FROM deps WHERE task = ?1 AND dep = ?2", params![r.id, self.normalize_id(raw)?])?;
                }
                changed.push("deps".into());
            }
            if changed.is_empty() {
                return Err(GenieError::invalid("nothing to update"));
            }
            let mut unique: Vec<String> = Vec::new();
            for c in changed {
                if !unique.contains(&c) {
                    unique.push(c);
                }
            }
            self.history(&r.id, actor, &format!("updated {}", unique.join(", ")), None, None, None)?;
            self.touch(&r.id)?;
            self.event(events::TASK_UPDATED, &r.id, actor, json!({ "fields": unique }))
        })?;
        self.get(&r.id)
    }

    pub fn set_status(&self, actor: &Actor, id: &str, to: Status, opts: StatusOptions) -> Result<Task> {
        let task = self.get(id)?;
        let from = task.status;
        if from == to {
            return Err(GenieError::invalid(format!("{} is already {to}", task.id)));
        }
        let privileged = is_privileged(actor.role);
        let forced = opts.force && privileged && to != Status::Inbox;
        if !actor.may_move(from, to) && !forced {
            let hint = if actor.role == Role::Orchestrator {
                " (review/approved are the team's verdicts; pass force only if the team cannot)"
            } else {
                ""
            };
            return Err(deny(actor, &format!("move {} from {from} to {to}{hint}", task.id)));
        }
        if opts.force && !privileged {
            return Err(deny(actor, "force status changes"));
        }
        // Separation of duties: whoever submitted this round of work does not approve it.
        if to == Status::Approved && !privileged && self.last_submitter(&task.id)?.as_deref() == Some(actor.name.as_str()) {
            return Err(deny(actor, &format!("approve {}: they submitted this work for review themselves", task.id)));
        }
        let note = opts.note.as_deref().map(str::trim).filter(|n| !n.is_empty());
        if to == Status::NeedsOwner && note.is_none() {
            return Err(GenieError::invalid("needs_owner requires a note with the question for the owner"));
        }
        let mut action = opts.action.clone();
        if let Some(a) = action.as_mut() {
            if to != Status::NeedsOwner {
                return Err(GenieError::invalid("an action goes with needs_owner only"));
            }
            a.validate()?;
        }
        if !opts.force {
            match to {
                Status::Ready => {
                    let mut known = HashSet::new();
                    for d in &task.deps {
                        if self.exists(d)? {
                            known.insert(d.clone());
                        }
                    }
                    let p = readiness_problems(&task, &known);
                    if !p.is_empty() {
                        return Err(GenieError::invalid(format!("{} is not ready: {} (use force to override)", task.id, p.join("; "))));
                    }
                }
                Status::Done => {
                    let mut stmt = self.conn().prepare_cached("SELECT status FROM tasks WHERE parent = ?1")?;
                    let children = stmt.query_map([&task.id], |r| r.get::<_, Status>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                    let p = done_problems(&task, &children);
                    if !p.is_empty() {
                        return Err(GenieError::invalid(format!("{} cannot be closed: {} (use force to override)", task.id, p.join("; "))));
                    }
                }
                Status::InProgress => {
                    let open = self.open_deps(&task.id)?;
                    if !open.is_empty() {
                        return Err(GenieError::invalid(format!("{} depends on unfinished tasks: {}", task.id, open.join(", "))));
                    }
                }
                Status::Review if self.gates.require_test_report && !task.artifacts.iter().any(|a| a.kind == ArtifactKind::TestReport) => {
                    return Err(GenieError::invalid(format!("{}: attach a test-report artifact before review", task.id)));
                }
                Status::Approved
                    if self.gates.require_review_artifact && !task.artifacts.iter().any(|a| a.kind == ArtifactKind::Review) =>
                {
                    return Err(GenieError::invalid(format!("{}: attach a review artifact before approving", task.id)));
                }
                _ => {}
            }
        }
        self.db.tx(|| {
            let at = now();
            let needs_owner = match (to, note) {
                (Status::NeedsOwner, Some(q)) => Some(serde_json::to_string(&NeedsOwner {
                    question: q.to_string(),
                    by: actor.name.clone(),
                    at: at.clone(),
                    previous: from,
                    action: action.clone(),
                })?),
                _ => None,
            };
            let blocked = if CLOSED.contains(&to) { None } else { self.row(&task.id)?.blocked };
            self.conn().execute(
                "UPDATE tasks SET status = ?1, needs_owner = ?2, blocked = ?3, updated = ?4 WHERE id = ?5",
                params![to, needs_owner, blocked, at, task.id],
            )?;
            self.history(&task.id, actor, "status", Some(from.as_str()), Some(to.as_str()), opts.note.as_deref())?;
            if let Some(n) = opts.note.as_deref().filter(|n| !n.is_empty())
                && to != Status::NeedsOwner
            {
                let kind =
                    if matches!(to, Status::ChangesRequested | Status::Approved) { CommentKind::Review } else { CommentKind::Progress };
                self.conn().execute(
                    "INSERT INTO comments(task, at, author, role, kind, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![task.id, at, actor.name, actor.role, kind, format!("[{from} → {to}] {n}")],
                )?;
            }
            if let (Status::NeedsOwner, Some(q)) = (to, note) {
                self.conn().execute(
                    "INSERT INTO comments(task, at, author, role, kind, text) VALUES (?1, ?2, ?3, ?4, 'question', ?5)",
                    params![task.id, at, actor.name, actor.role, format!("Needs owner decision: {q}{}", action_line(action.as_ref()))],
                )?;
            }
            self.event(
                events::TASK_STATUS_CHANGED,
                &task.id,
                actor,
                json!({ "from": from, "to": to, "note": opts.note, "force": opts.force, "action": action.as_ref().map(OwnerAction::kind) }),
            )?;
            let suffix = opts.note.as_deref().filter(|n| !n.is_empty()).map(|n| format!(": {n}")).unwrap_or_default();
            self.tell_orchestrator(actor, &task.id, &format!("The owner moved {} from {from} to {to}{suffix}", task.id))
        })?;
        if let Some(parent) = &task.parent {
            self.follow_epic(parent, &task.id, to)?;
        }
        self.get(&task.id)
    }

    pub fn comment(&self, actor: &Actor, id: &str, text: &str, kind: CommentKind) -> Result<Task> {
        let text = text.trim();
        if text.is_empty() {
            return Err(GenieError::invalid("comment text is empty"));
        }
        let r = self.row(id)?;
        let kind = if actor.role == Role::Human && kind == CommentKind::Note { CommentKind::Owner } else { kind };
        self.db.tx(|| {
            self.conn().execute(
                "INSERT INTO comments(task, at, author, role, kind, text) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![r.id, now(), actor.name, actor.role, kind, text],
            )?;
            let comment_id = self.conn().last_insert_rowid();
            self.touch(&r.id)?;
            self.event(events::TASK_COMMENTED, &r.id, actor, json!({ "comment": comment_id, "kind": kind }))?;
            let waiting = if r.status == Status::NeedsOwner { " (the task is waiting for this decision)" } else { "" };
            self.tell_orchestrator(actor, &r.id, &format!("The owner commented on {}{waiting}: {text}", r.id))
        })?;
        self.get(&r.id)
    }

    pub fn check(&self, actor: &Actor, id: &str, criterion: i64, done: bool) -> Result<Task> {
        require_cap(actor, "check acceptance criteria", Capability::TaskCheck)?;
        let r = self.row(id)?;
        self.db.tx(|| {
            let changes = self.conn().execute(
                "UPDATE acceptance SET done = ?1, checked_by = ?2, checked_at = ?3 WHERE task = ?4 AND n = ?5",
                params![done as i64, done.then(|| actor.name.clone()), done.then(now), r.id, criterion],
            )?;
            if changes == 0 {
                return Err(GenieError::not_found(format!("{} has no acceptance criterion #{criterion}", r.id)));
            }
            let what = if done { "checked" } else { "unchecked" };
            self.history(&r.id, actor, &format!("acceptance #{criterion} {what}"), None, None, None)?;
            self.touch(&r.id)?;
            self.event(events::TASK_CRITERION_CHECKED, &r.id, actor, json!({ "criterion": criterion, "done": done }))
        })?;
        self.get(&r.id)
    }

    /// Attach an artifact: the content lives in the database, not in files agents might read by accident.
    pub fn add_artifact(&self, actor: &Actor, id: &str, input: ArtifactInput) -> Result<Task> {
        let kind = input.kind.unwrap_or(ArtifactKind::Other);
        let r = self.row(id)?;
        let (data, name) = match &input.source {
            ArtifactSource::File(path) => {
                if !path.exists() {
                    return Err(GenieError::not_found(format!("file {} not found", path.display())));
                }
                let fallback = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                (std::fs::read(path)?, input.name.clone().unwrap_or(fallback))
            }
            ArtifactSource::Content(bytes) => (bytes.clone(), input.name.clone().unwrap_or_else(|| format!("{kind}.md"))),
        };
        if data.len() > MAX_ARTIFACT_BYTES {
            return Err(GenieError::invalid(format!("artifact is larger than {MAX_ARTIFACT_BYTES} bytes")));
        }
        let name = sanitize(&name);
        self.db.tx(|| {
            let n: i64 =
                self.conn().query_row("SELECT COALESCE(MAX(n), 0) FROM artifacts WHERE task = ?1", [&r.id], |x| x.get::<_, i64>(0))? + 1;
            self.conn().execute(
                "INSERT INTO artifacts(task, n, at, author, role, kind, name, note, size, content) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![r.id, n, now(), actor.name, actor.role, kind, name, input.note, data.len() as i64, data],
            )?;
            self.history(&r.id, actor, &format!("artifact #{n} {name} ({kind}) added"), None, None, None)?;
            self.touch(&r.id)?;
            self.event(
                events::TASK_ARTIFACT_ADDED,
                &r.id,
                actor,
                json!({ "artifact": n, "kind": kind, "name": name, "size": data.len() }),
            )
        })?;
        self.get(&r.id)
    }

    pub fn read_artifact(&self, id: &str, n: i64) -> Result<ArtifactContent> {
        let r = self.row(id)?;
        let (name, kind, content): (String, ArtifactKind, Vec<u8>) = self
            .conn()
            .query_row("SELECT name, kind, content FROM artifacts WHERE task = ?1 AND n = ?2", params![r.id, n], |a| {
                Ok((a.get(0)?, a.get(1)?, a.get(2)?))
            })
            .optional()?
            .ok_or_else(|| GenieError::not_found(format!("{} has no artifact #{n}", r.id)))?;
        let text = String::from_utf8(content.clone()).ok();
        Ok(ArtifactContent { name, kind, content, text })
    }

    /// Slice a task into atomic children. The parent becomes an epic — unless it
    /// already belongs to an epic (epics are not nested): then the pieces join
    /// that epic and the split task is cancelled.
    pub fn split(&self, actor: &Actor, id: &str, children: Vec<CreateInput>) -> Result<Vec<Task>> {
        require_role(actor, "split tasks", &[])?;
        let parent = self.get(id)?;
        let epic = if parent.task_type == TaskType::Epic { None } else { self.epic_of(&parent.id)? };
        let make = |target: &str| -> Result<Vec<Task>> {
            children
                .iter()
                .map(|c| {
                    self.create(
                        actor,
                        CreateInput {
                            status: None,
                            parent: Some(target.to_string()),
                            labels: Some(c.labels.clone().unwrap_or_else(|| parent.labels.clone())),
                            ..c.clone()
                        },
                    )
                })
                .collect()
        };
        if let Some(epic) = epic {
            if parent.team.is_some() || WORKING.contains(&parent.status) {
                return Err(GenieError::invalid(format!("{} is being worked on; stop its team before splitting it", parent.id)));
            }
            return self.db.tx(|| {
                let out = make(&epic)?;
                let ids = out.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(", ");
                self.conn().execute("UPDATE tasks SET status = 'cancelled', updated = ?1 WHERE id = ?2", params![now(), parent.id])?;
                let note = format!("split into {ids}");
                self.history(&parent.id, actor, "status", Some(parent.status.as_str()), Some("cancelled"), Some(&note))?;
                self.history(&epic, actor, &format!("{} split into {ids}", parent.id), None, None, None)?;
                self.event(
                    events::TASK_STATUS_CHANGED,
                    &parent.id,
                    actor,
                    json!({ "from": parent.status, "to": Status::Cancelled, "note": note, "force": false }),
                )?;
                Ok(out)
            });
        }
        self.db.tx(|| {
            let out = make(&parent.id)?;
            self.conn().execute("UPDATE tasks SET type = 'epic' WHERE id = ?1", [&parent.id])?;
            let ids = out.iter().map(|c| c.id.as_str()).collect::<Vec<_>>().join(", ");
            self.history(&parent.id, actor, &format!("split into {ids}"), None, None, None)?;
            self.event(
                events::TASK_UPDATED,
                &parent.id,
                actor,
                json!({ "fields": ["type"], "splitInto": out.iter().map(|c| &c.id).collect::<Vec<_>>() }),
            )?;
            Ok(out)
        })
    }

    pub fn block(&self, actor: &Actor, id: &str, reason: &str) -> Result<Task> {
        require_cap(actor, "block tasks", Capability::TaskBlock)?;
        let r = self.row(id)?;
        self.db.tx(|| {
            let blocked = Blocked { reason: reason.to_string(), by: actor.name.clone(), at: now() };
            self.conn().execute("UPDATE tasks SET blocked = ?1 WHERE id = ?2", params![serde_json::to_string(&blocked)?, r.id])?;
            self.history(&r.id, actor, &format!("blocked: {reason}"), None, None, None)?;
            self.touch(&r.id)?;
            self.event(events::TASK_BLOCKED, &r.id, actor, json!({ "reason": reason }))
        })?;
        self.get(&r.id)
    }

    pub fn unblock(&self, actor: &Actor, id: &str) -> Result<Task> {
        require_cap(actor, "unblock tasks", Capability::TaskBlock)?;
        let r = self.row(id)?;
        self.db.tx(|| {
            self.conn().execute("UPDATE tasks SET blocked = NULL WHERE id = ?1", [&r.id])?;
            self.history(&r.id, actor, "unblocked", None, None, None)?;
            self.touch(&r.id)?;
            self.event(events::TASK_UNBLOCKED, &r.id, actor, json!({}))
        })?;
        self.get(&r.id)
    }

    /// A person's login changed: the tasks they are responsible for follow the new one.
    pub fn rename_assignee(&self, before: &str, after: &str) -> Result<()> {
        self.conn().execute("UPDATE tasks SET assignee = ?1 WHERE assignee = ?2", params![after, before])?;
        Ok(())
    }

    /// What deleting a task removes: the task, its subtasks (all levels) and the
    /// teams working on any of them. Without `cascade` a task with subtasks is refused.
    pub fn delete_plan(&self, id: &str, cascade: bool) -> Result<DeletePlan> {
        let root = self.row(id)?;
        let mut tasks = vec![(root.id.clone(), root.title.clone())];
        let mut i = 0;
        while i < tasks.len() {
            let mut stmt = self.conn().prepare_cached("SELECT id, title FROM tasks WHERE parent = ?1 ORDER BY seq")?;
            let children = stmt.query_map([&tasks[i].0], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let children = children.collect::<rusqlite::Result<Vec<_>>>()?;
            tasks.extend(children);
            i += 1;
        }
        if tasks.len() > 1 && !cascade {
            return Err(GenieError::invalid(format!(
                "{} has {} subtasks: delete them together with it (cascade) or move them elsewhere first",
                root.id,
                tasks.len() - 1
            )));
        }
        let mut teams = Vec::new();
        for (task, _) in &tasks {
            teams.extend(self.strings("SELECT id FROM teams WHERE task = ?1 ORDER BY created", task)?);
        }
        Ok(DeletePlan { tasks, teams })
    }

    /// Delete a task for good with its subtasks (`cascade`): comments, criteria,
    /// artifacts and history go with it, tasks that depended on it lose that
    /// dependency, mail about it is dropped. The teams working on it are removed
    /// by the caller first (see `delete_plan`). A `task.deleted` event is journaled.
    pub fn delete_tasks(&self, actor: &Actor, id: &str, cascade: bool) -> Result<DeletePlan> {
        require_role(actor, "delete tasks", &[])?;
        let plan = self.delete_plan(id, cascade)?;
        let parent = self.row(id)?.parent;
        self.db.tx(|| {
            // Subtasks first: `parent` is a plain foreign key.
            for (task, title) in plan.tasks.iter().rev() {
                let row = self.row(task)?;
                self.conn().execute("DELETE FROM deps WHERE dep = ?1", [task])?;
                self.conn().execute("DELETE FROM mail WHERE task = ?1", [task])?;
                self.conn().execute("DELETE FROM tasks WHERE id = ?1", [task])?;
                self.event(
                    events::TASK_DELETED,
                    task,
                    actor,
                    json!({ "title": title, "type": row.task_type, "status": row.status, "parent": row.parent, "cascade": cascade }),
                )?;
            }
            if let Some(p) = parent.filter(|p| !plan.tasks.iter().any(|(t, _)| t == p)) {
                self.history(&p, actor, &format!("child {} deleted", plan.tasks[0].0), None, None, None)?;
                self.touch(&p)?;
            }
            Ok(())
        })?;
        Ok(plan)
    }

    pub fn assign_team(
        &self,
        actor: &Actor,
        id: &str,
        team: Option<&str>,
        worktree: Option<&Worktree>,
        assignees: Option<&[String]>,
    ) -> Result<Task> {
        require_role(actor, "assign teams", &[])?;
        let r = self.row(id)?;
        self.db.tx(|| {
            self.conn().execute("UPDATE tasks SET team = ?1 WHERE id = ?2", params![team, r.id])?;
            if let Some(w) = worktree {
                self.conn().execute("UPDATE tasks SET worktree = ?1 WHERE id = ?2", params![serde_json::to_string(w)?, r.id])?;
            }
            if let Some(a) = assignees {
                self.conn().execute("UPDATE tasks SET assignees = ?1 WHERE id = ?2", params![serde_json::to_string(a)?, r.id])?;
            }
            let event = match team {
                Some(t) => format!("assigned to team {t}"),
                None => "team released".to_string(),
            };
            self.history(&r.id, actor, &event, None, None, None)?;
            self.touch(&r.id)?;
            self.event(events::TASK_TEAM_ASSIGNED, &r.id, actor, json!({ "team": team }))
        })?;
        self.get(&r.id)
    }

    /// Counts per status, for sidebars and status lines.
    pub fn counts(&self) -> Result<std::collections::BTreeMap<String, i64>> {
        let mut stmt = self.conn().prepare_cached("SELECT status, COUNT(*) FROM tasks GROUP BY status")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // --- event journal -------------------------------------------------------

    pub fn events_after(&self, after: i64, limit: usize) -> Result<Vec<Event>> {
        events::after(self.conn(), after, limit)
    }

    pub fn last_event_id(&self) -> Result<i64> {
        events::last_id(self.conn())
    }

    pub fn event_cursor(&self, subscriber: &str) -> Result<i64> {
        events::cursor(self.conn(), subscriber)
    }

    pub fn ack_events(&self, subscriber: &str, id: i64) -> Result<()> {
        events::ack(self.conn(), subscriber, id)
    }
}

#[cfg(test)]
mod tests {
    use super::sanitize;

    #[test]
    fn sanitize_matches_the_typescript_regex() {
        assert_eq!(sanitize("review.md"), "review.md");
        assert_eq!(sanitize("zcl_x.clas.abap"), "zcl_x.clas.abap");
        assert_eq!(sanitize("my report (v2).md"), "my_report_v2_.md");
        assert_eq!(sanitize("отчёт.md"), "_.md");
    }
}
