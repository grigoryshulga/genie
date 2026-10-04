//! Task commands: what a person, an agent or an automation asks of a task, with every rule
//! the server holds on top of the tracker's workflow — an agent stays within its task, an
//! `assisted` project is closed by people, the delivery must be in order before review and
//! done, only people and the orchestrator force, the people named hear of it, closing a task
//! stops its team, and the workers wake up.
//!
//! The HTTP routes, the automation engine and questionnaires call these. genie's own
//! bookkeeping inside a tracker transaction (assembling a team, shaping an idea, a host's
//! review) calls the tracker directly: it asks nothing of anyone.
//!
//! The request bodies are the wire contract of the web, the command line and MCP: the catalog
//! sends these structs, the web gets them as generated TypeScript types, and a key nobody knows
//! is refused.

use std::collections::BTreeSet;

use genie_core::{Actor, CLOSED, CommentKind, CreateInput, OwnerAction, Role, Status, StatusOptions, Task, TaskType, UpdateInput};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::state::{App, AppError, AppResult};

/// Who asks for a task command.
#[derive(Debug, Clone)]
pub struct Caller {
    pub project: String,
    pub actor: Actor,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Person,
    /// An agent with its token: a team member, a one-shot job, or the orchestrator.
    Agent {
        team: Option<String>,
        job: Option<i64>,
    },
    /// A step of an automation, acting as the orchestrator.
    Automation,
}

impl Caller {
    pub fn person(project: &str, login: &str) -> Caller {
        Caller { project: project.into(), actor: Actor::new(login, Role::Human), kind: Kind::Person }
    }

    pub fn automation(project: &str, actor: Actor) -> Caller {
        Caller { project: project.into(), actor, kind: Kind::Automation }
    }

    pub fn is_person(&self) -> bool {
        self.kind == Kind::Person
    }

    fn is_orchestrator(&self) -> bool {
        !self.is_person() && self.actor.role == Role::Orchestrator
    }
}

/// What a caller does to a task: change it, or only leave a note (comment, artifact).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Touch {
    Edit,
    Note,
}

/// Agents act within their assignment: a team member (or job) changes only its own task and
/// its subtasks, and may leave notes on that task's epic. The orchestrator, automations and
/// people are not limited here (the workflow rules still apply).
pub fn authorize(app: &App, caller: &Caller, id: &str, touch: Touch) -> AppResult<()> {
    let Kind::Agent { team, job } = &caller.kind else { return Ok(()) };
    if caller.actor.role == Role::Orchestrator {
        return Ok(());
    }
    let home = match (team, job) {
        (Some(t), _) => Some(app.with_tracker(&caller.project, |tr| Ok(tr.bus().get(t)?.task))?),
        (None, Some(j)) => app.with_server(|db| db.job(*j))?.task,
        _ => None,
    };
    let Some(home) = home else { return Err(AppError::Forbidden("this agent has no task to work on".into())) };
    let verdict = app.with_tracker(&caller.project, |t| {
        let target = t.normalize_id(id)?;
        let home = t.normalize_id(&home)?;
        // The task itself or one of its descendants.
        let mut cur = Some(target.clone());
        for _ in 0..10 {
            match cur {
                Some(c) if c == home => return Ok(Ok(())),
                Some(c) => cur = t.get(&c).ok().and_then(|x| x.parent),
                None => break,
            }
        }
        if touch == Touch::Note && t.epic_of(&home)?.as_deref() == Some(target.as_str()) {
            return Ok(Ok(()));
        }
        Ok(Err(format!("agents change only their own task ({home}) and its subtasks, and leave notes on its epic; {target} is outside")))
    })?;
    verdict.map_err(AppError::Forbidden)
}

/// Something changed in a project: wake the workers that react to it.
pub fn changed(app: &App) {
    app.wake_engine.notify_one();
    app.wake_runtime.notify_one();
}

// --- requests -----------------------------------------------------------------------------------

/// A new task.
#[derive(Debug, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct CreateBody {
    #[serde(default)]
    pub title: String,
    #[serde(rename = "type", default, deserialize_with = "lenient::parsed_opt")]
    pub task_type: Option<TaskType>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub acceptance: Option<Vec<String>>,
    #[serde(default, deserialize_with = "lenient::int")]
    pub priority: Option<i64>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub deps: Option<Vec<String>>,
    /// Absent: a subtask takes its parent's labels.
    #[serde(default, deserialize_with = "lenient::list")]
    pub labels: Option<Vec<String>>,
    #[serde(default)]
    pub merge_strategy: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub plan: Option<String>,
    /// A person's task starts as a draft instead of the inbox.
    #[serde(default)]
    pub draft: Option<bool>,
    /// An automation's task goes to the inbox instead of the drafts.
    #[serde(default)]
    pub inbox: Option<bool>,
}

/// Fields of a task to change; absent ones stay.
#[derive(Debug, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct UpdateBody {
    #[serde(default, deserialize_with = "lenient::text")]
    pub title: Option<String>,
    #[serde(rename = "type", default, deserialize_with = "lenient::parsed_opt")]
    pub task_type: Option<TaskType>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub plan: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub append_notes: Option<String>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub notes: Option<String>,
    #[serde(default, deserialize_with = "lenient::int")]
    pub priority: Option<i64>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub labels: Option<Vec<String>>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub assignees: Option<Vec<String>>,
    #[serde(default, deserialize_with = "lenient::text")]
    pub merge_strategy: Option<String>,
    /// The person responsible by login; `null` or `""` clears it.
    #[serde(default, deserialize_with = "lenient::nullable", skip_serializing_if = "Option::is_none")]
    pub assignee: Option<Option<String>>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub add_acceptance: Option<Vec<String>>,
    #[serde(default)]
    pub remove_acceptance: Option<Vec<i64>>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub add_deps: Option<Vec<String>>,
    #[serde(default, deserialize_with = "lenient::list")]
    pub remove_deps: Option<Vec<String>>,
    /// An epic to move the task into; `null` or `""` takes it out.
    #[serde(default, deserialize_with = "lenient::nullable", skip_serializing_if = "Option::is_none")]
    pub parent: Option<Option<String>>,
}

/// A move to another status.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct StatusBody {
    #[serde(deserialize_with = "lenient::parsed")]
    pub status: Status,
    #[serde(default, deserialize_with = "lenient::text")]
    pub note: Option<String>,
    /// Skip the Definition of Ready and Done (the orchestrator; people's moves always do).
    #[serde(default)]
    pub force: Option<bool>,
    /// needs_owner: what the owner can do besides answering in words.
    #[serde(default, deserialize_with = "lenient::owner_action")]
    pub action: Option<OwnerAction>,
}

impl StatusBody {
    pub fn to(status: Status, note: Option<String>) -> StatusBody {
        StatusBody { status, note, force: None, action: None }
    }
}

/// A comment. People's comments are always the owner's.
#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct CommentBody {
    pub text: String,
    #[serde(default, deserialize_with = "lenient::parsed_opt")]
    pub kind: Option<CommentKind>,
}

/// Lenient readers for request fields: the command line, MCP clients and automation templates
/// send numbers as text and a single value for a list.
mod lenient {
    use std::str::FromStr;

    use genie_core::{GenieError, OwnerAction};
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    /// A status, type or kind by its name, with the tracker's own message for an unknown one.
    pub fn parsed<'de, D: Deserializer<'de>, T: FromStr<Err = GenieError>>(d: D) -> Result<T, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }

    pub fn parsed_opt<'de, D: Deserializer<'de>, T: FromStr<Err = GenieError>>(d: D) -> Result<Option<T>, D::Error> {
        match Option::<String>::deserialize(d)? {
            Some(s) if !s.is_empty() => s.parse().map(Some).map_err(serde::de::Error::custom),
            _ => Ok(None),
        }
    }

    pub fn text<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        Ok(match Value::deserialize(d)? {
            Value::Null => None,
            Value::String(s) => Some(s),
            other => Some(other.to_string()),
        })
    }

    /// An array of strings or numbers (empty ones dropped), or one string.
    pub fn list<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
        let item = |v: Value| match v {
            Value::String(s) => Some(s),
            Value::Null => None,
            other => Some(other.to_string()),
        };
        Ok(match Value::deserialize(d)? {
            Value::Null => None,
            Value::Array(a) => Some(a.into_iter().filter_map(item).filter(|s| !s.is_empty()).collect()),
            Value::String(s) if s.is_empty() => Some(Vec::new()),
            other => Some(item(other).into_iter().collect()),
        })
    }

    pub fn int<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
        match Value::deserialize(d)? {
            Value::Null => Ok(None),
            Value::Number(n) => n.as_i64().map(Some).ok_or_else(|| serde::de::Error::custom("expected a whole number")),
            Value::String(s) if s.trim().is_empty() => Ok(None),
            Value::String(s) => {
                s.trim().parse().map(Some).map_err(|_| serde::de::Error::custom(format!("expected a whole number, got {s:?}")))
            }
            other => Err(serde::de::Error::custom(format!("expected a whole number, got {other}"))),
        }
    }

    /// Present: `null` or `""` → `Some(None)`, a value → `Some(Some(v))`. Absent stays `None` (`default`).
    pub fn nullable<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
        Ok(Some(text(d)?.filter(|s| !s.trim().is_empty())))
    }

    pub fn owner_action<'de, D: Deserializer<'de>>(d: D) -> Result<Option<OwnerAction>, D::Error> {
        match Value::deserialize(d)? {
            Value::Null => Ok(None),
            v => OwnerAction::parse(&v).map(Some).map_err(serde::de::Error::custom),
        }
    }
}

// --- commands -----------------------------------------------------------------------------------

pub fn create(app: &App, caller: &Caller, body: CreateBody) -> AppResult<Task> {
    let parent = body.parent.filter(|p| !p.is_empty());
    if matches!(caller.kind, Kind::Agent { .. }) && caller.actor.role != Role::Orchestrator {
        let Some(parent) = &parent else {
            return Err(AppError::Forbidden("agents create only subtasks of their own task (pass parent)".into()));
        };
        authorize(app, caller, parent, Touch::Edit)?;
    }
    // People submit to the inbox (the orchestrator takes it from there); agents create drafts.
    let inbox = match caller.kind {
        Kind::Person => body.draft != Some(true),
        Kind::Automation => body.inbox == Some(true),
        Kind::Agent { .. } => false,
    };
    let input = CreateInput {
        title: body.title,
        task_type: body.task_type,
        description: body.description.filter(|s| !s.is_empty()),
        acceptance: body.acceptance.unwrap_or_default(),
        priority: body.priority,
        parent,
        deps: body.deps.unwrap_or_default(),
        labels: body.labels,
        merge_strategy: body.merge_strategy,
        plan: body.plan,
        status: inbox.then_some(Status::Inbox),
        quiet: false,
    };
    let task = app.with_tracker(&caller.project, |t| t.create(&caller.actor, input))?;
    changed(app);
    Ok(task)
}

pub fn update(app: &App, caller: &Caller, id: &str, body: UpdateBody) -> AppResult<Task> {
    authorize(app, caller, id, Touch::Edit)?;
    let assignee = body.assignee.map(|a| a.map(|s| s.trim().trim_start_matches('@').to_lowercase()).filter(|s| !s.is_empty()));
    let assigned = assignee.clone().flatten();
    // The person responsible is someone who works in the project.
    let before = match &assigned {
        Some(login) => {
            if !works_here(app, &caller.project, login)? {
                return Err(AppError::Bad(format!("{login} is not a member of this project")));
            }
            app.with_tracker(&caller.project, |t| t.get(id))?.assignee
        }
        None => None,
    };
    let input = UpdateInput {
        title: body.title,
        task_type: body.task_type,
        description: body.description,
        plan: body.plan,
        append_notes: body.append_notes,
        notes: body.notes,
        priority: body.priority,
        labels: body.labels,
        assignees: body.assignees,
        merge_strategy: body.merge_strategy,
        assignee,
        add_acceptance: body.add_acceptance.unwrap_or_default(),
        remove_acceptance: body.remove_acceptance.unwrap_or_default(),
        add_deps: body.add_deps.unwrap_or_default(),
        remove_deps: body.remove_deps.unwrap_or_default(),
        parent: body.parent,
    };
    let task = app.with_tracker(&caller.project, |t| t.update(&caller.actor, id, input))?;
    // A new person responsible hears of it (not when they assign themselves).
    if let Some(login) = task.assignee.clone().filter(|l| before.as_ref() != Some(l) && *l != caller.actor.name) {
        let users = crate::notify::resolve(app, &caller.project, &[format!("@{login}")], &json!({}))?;
        let msg = crate::notify::Message {
            kind: "assigned".into(),
            title: format!("{}: вы ответственный", task.id),
            body: format!("{}\n\nНазначил(а): {}", task.title, caller.actor.name),
            project: Some(caller.project.clone()),
            task: Some(task.id.clone()),
            link: Some(format!("/mine?task={}", task.id)),
            ..Default::default()
        };
        crate::notify::send(app, &users, &msg, None)?;
    }
    changed(app);
    Ok(task)
}

/// Move a task. Blocking: the delivery gate asks the git host about the checks.
pub fn set_status(app: &App, caller: &Caller, id: &str, body: StatusBody) -> AppResult<Task> {
    authorize(app, caller, id, Touch::Edit)?;
    let to = body.status;
    // In an `assisted` project people close tasks; the orchestrator asks one instead.
    if caller.is_orchestrator() && CLOSED.contains(&to) && app.with_server(|db| db.project(&caller.project))?.autonomy == "assisted" {
        return Err(AppError::Conflict(
            "people close tasks in this project (assisted): move the task to needs_owner with a short summary of the result and what to check".into(),
        ));
    }
    // What the task delivers to its repositories must be in order before review and close (people decide for themselves).
    if !caller.is_person() {
        crate::git::delivery::gate(app, &caller.project, id, to).map_err(AppError::Conflict)?;
    }
    // The owner's moves are authoritative; agents follow the workflow and only the orchestrator forces, when asked to.
    let force = caller.is_person() || (caller.is_orchestrator() && body.force == Some(true));
    let action = body.action.map(|a| owner_action(app, &caller.project, id, a)).transpose()?;
    let opts = StatusOptions { note: body.note.filter(|n| !n.is_empty()), force, action };
    let task = app.with_tracker(&caller.project, |t| t.set_status(&caller.actor, id, to, opts))?;
    if CLOSED.contains(&to)
        && let Err(e) = crate::runtime::reap_closed_blocking(app, &caller.project)
    {
        eprintln!("genie runtime: reap {}: {e}", caller.project);
    }
    changed(app);
    Ok(task)
}

/// An owner action as the task will keep it: a merge request names the task's open
/// request (the only one when no repository is named) with its number and page.
fn owner_action(app: &App, project: &str, task: &str, action: OwnerAction) -> AppResult<OwnerAction> {
    let OwnerAction::AskForMergePr { repo, .. } = action else { return Ok(action) };
    let task = app.with_tracker(project, |t| t.normalize_id(task))?;
    let rows = app.with_server(|db| db.task_repos(project, &task))?;
    let open: Vec<_> = rows.into_iter().filter(|r| r.cr_number.is_some() && r.cr_state.as_deref() == Some("open")).collect();
    let names = || open.iter().map(|r| r.repo.as_str()).collect::<Vec<_>>().join(", ");
    let invalid = AppError::Bad;
    let row = match repo.trim() {
        "" if open.len() == 1 => &open[0],
        "" if open.is_empty() => return Err(invalid(format!("ask-for-merge-pr: {task} has no open request to merge"))),
        "" => return Err(invalid(format!("ask-for-merge-pr: name the repository (open requests in {})", names()))),
        name => open.iter().find(|r| r.repo == name).ok_or_else(|| {
            invalid(format!(
                "ask-for-merge-pr: {task} has no open request in {name}{}",
                if open.is_empty() { String::new() } else { format!(" (open in {})", names()) }
            ))
        })?,
    };
    Ok(OwnerAction::AskForMergePr { repo: row.repo.clone(), number: row.cr_number, url: row.cr_url.clone() })
}

pub fn comment(app: &App, caller: &Caller, id: &str, body: CommentBody) -> AppResult<Task> {
    authorize(app, caller, id, Touch::Note)?;
    let kind = if caller.is_person() { CommentKind::Owner } else { body.kind.unwrap_or(CommentKind::Note) };
    let task = app.with_tracker(&caller.project, |t| t.comment(&caller.actor, id, &body.text, kind))?;
    // People named with `@login` hear of it — from people, agents and automations alike.
    let me = app.with_server(|db| db.user_by_login(&caller.actor.name))?.map(|u| u.id);
    let users: Vec<i64> = mentioned(app, &caller.project, &body.text)?.into_iter().filter(|u| Some(*u) != me).collect();
    if !users.is_empty() {
        let excerpt: String = body.text.chars().take(500).collect();
        let msg = crate::notify::Message {
            kind: "mention".into(),
            title: format!("Вас упомянули в {}", task.id),
            body: format!("{}: {excerpt}\n\n{}", caller.actor.name, task.title),
            project: Some(caller.project.clone()),
            task: Some(task.id.clone()),
            link: Some(format!("/active?task={}", task.id)),
            ..Default::default()
        };
        crate::notify::send(app, &users, &msg, None)?;
    }
    changed(app);
    Ok(task)
}

/// A task deleted for good, with what happened to the teams that worked on it.
pub struct Deleted {
    pub ids: Vec<String>,
    pub report: Vec<String>,
}

/// Delete a task for good: teams working on it are stopped and removed (with their worktrees;
/// the branches stay), and its footprint on the server goes. Project admins only — the caller
/// checks that; agents, the orchestrator included, never delete tasks.
pub fn delete(app: &App, caller: &Caller, id: &str, cascade: bool) -> AppResult<Deleted> {
    if !caller.is_person() {
        return Err(AppError::Forbidden("project admin rights required".into()));
    }
    let slug = caller.project.as_str();
    let plan = app.with_tracker(slug, |t| t.delete_plan(id, cascade))?;
    let mut report = Vec::new();
    for team in &plan.teams {
        if app.with_tracker(slug, |t| Ok(t.bus().get(team)?.state == "active"))? {
            report.extend(crate::runtime::stop_team(app, slug, team, "owner", &caller.actor.name)?);
        }
        report.push(crate::runtime::remove_worktree(app, slug, team));
        app.with_tracker(slug, |t| t.bus().delete(team))?;
    }
    let plan = app.with_tracker(slug, |t| t.delete_tasks(&caller.actor, id, cascade))?;
    app.with_server(|db| db.tx(|| plan.tasks.iter().try_for_each(|(task, _)| db.forget_task(slug, task))))?;
    changed(app);
    Ok(Deleted { ids: plan.tasks.into_iter().map(|(id, _)| id).collect(), report })
}

/// Whether a login belongs to someone who works in the project.
fn works_here(app: &App, project: &str, login: &str) -> AppResult<bool> {
    app.with_server(|db| match db.user_by_login(login)? {
        Some(u) if !u.disabled => Ok(db.project_role(project, &u)?.is_some()),
        _ => Ok(false),
    })
}

/// People named `@login` in a text who work in the project.
fn mentioned(app: &App, project: &str, text: &str) -> AppResult<Vec<i64>> {
    if !text.contains('@') {
        return Ok(Vec::new());
    }
    let logins: BTreeSet<String> = text
        .split(|c: char| c.is_whitespace() || ",;:!?()[]<>\"'«»".contains(c))
        .filter_map(|w| w.strip_prefix('@'))
        .map(|w| w.trim_end_matches(['.', '-']).to_lowercase())
        .filter(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)))
        .collect();
    app.with_server(|db| {
        let mut out = Vec::new();
        for login in logins {
            if let Some(u) = db.user_by_login(&login)?
                && !u.disabled
                && db.project_role(project, &u)?.is_some()
            {
                out.push(u.id);
            }
        }
        Ok(out)
    })
}

/// A request body read from an automation step's input (the keys `task` and those listed in
/// `skip` belong to the step, not the request).
pub fn from_step<T: serde::de::DeserializeOwned>(input: &Value, skip: &[&str]) -> AppResult<T> {
    let mut v = input.clone();
    if let Some(o) = v.as_object_mut() {
        o.remove("task");
        for k in skip {
            o.remove(*k);
        }
    }
    serde_json::from_value(v).map_err(|e| AppError::Genie(genie_core::GenieError::invalid(e.to_string())))
}
