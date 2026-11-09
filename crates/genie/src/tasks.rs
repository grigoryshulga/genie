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
use sha2::{Digest, Sha256};

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
    /// An intake activation; its task scope is persisted with its conversation.
    Ingress {
        response: String,
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
    if !caller.is_person() {
        ensure_running(app, &caller.project, id)?;
    }
    if let Kind::Ingress { response } = &caller.kind {
        return crate::conversations::authorize_task(app, caller, response, id);
    }
    let Kind::Agent { team, job } = &caller.kind else { return Ok(()) };
    if caller.actor.role == Role::Orchestrator {
        return crate::management::authorize_task(app, &caller.project, &caller.actor, id);
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

/// A durable limit intent fences work even while its project DB is unavailable.
pub(crate) fn ensure_running(app: &App, project: &str, task: &str) -> AppResult<()> {
    if crate::runtime::pending_limit(app, project, task)? {
        return Err(AppError::Conflict("a hard-limit pause is awaiting persistence; task execution remains blocked".into()));
    }
    app.with_tracker(project, |t| t.execution().ensure_running(task))
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
    // Stable creation identity within an incoming activation.
    #[serde(default)]
    pub request_key: Option<String>,
}

/// Fields of a task to change; absent ones stay. A null or empty assignee clears
/// the responsible person; a null or empty parent takes the task out of its epic.
#[derive(Debug, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export, optional_fields)]
pub struct UpdateBody {
    #[serde(default)]
    pub expected_version: String,
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
    // The person responsible by login; null or an empty string clears it.
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
    // An epic to move the task into; null or an empty string takes it out.
    #[serde(default, deserialize_with = "lenient::nullable", skip_serializing_if = "Option::is_none")]
    pub parent: Option<Option<String>>,
}

#[derive(Serialize, ts_rs::TS)]
// Share Task's generated type rather than copying all of its fields and docs.
#[ts(export, type = "import('./Task.ts').Task & { editVersion: string }")]
pub struct TaskSnapshot<'a> {
    #[serde(flatten)]
    pub task: &'a Task,
    #[serde(rename = "editVersion")]
    pub edit_version: String,
}

/// The editable snapshot has its own version: unlike an acceptance proposal,
/// replacing notes or labels must also detect a concurrent writer.
pub fn view(task: &Task) -> genie_core::Result<Value> {
    let value = serde_json::to_value(task)?;
    let digest = Sha256::digest(value.to_string().as_bytes());
    let edit_version = digest.iter().map(|b| format!("{b:02x}")).collect::<String>();
    Ok(serde_json::to_value(TaskSnapshot { task, edit_version })?)
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
    let mut incoming_initiator = None;
    let key = body.request_key.as_deref().unwrap_or("primary");
    if key.trim().is_empty() || key.len() > 200 {
        return Err(AppError::Bad("requestKey must be nonempty and at most 200 bytes".into()));
    }
    if matches!(caller.kind, Kind::Ingress { .. }) {
        crate::conversations::validate_agent(app, &caller.project, &caller.actor.name)?;
        if let Kind::Ingress { response } = &caller.kind {
            let r = app.with_tracker(&caller.project, |t| t.conversations().get(response))?;
            incoming_initiator = Some(crate::conversations::current_principal(app, &caller.project, &r.principal, true)?);
        }
        if body.parent.is_some() {
            return Err(AppError::Bad("incoming requests create independent tasks; use explicit task operations for hierarchy".into()));
        }
    }
    if genie_core::management::managed_task(&caller.actor.name).is_some() {
        return Err(AppError::Forbidden("task managers ask the project manager to create or split work".into()));
    }
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
        Kind::Agent { .. } | Kind::Ingress { .. } => false,
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
        quiet: matches!(caller.kind, Kind::Ingress { .. }),
    };
    let task = app.with_tracker(&caller.project, |t| {
        t.tx(|| {
            if let Kind::Ingress { response } = &caller.kind {
                let r = t.conversations().get(response)?;
                if r.status != genie_core::conversations::ResponseStatus::InProgress {
                    return Err(genie_core::GenieError::Denied("this request is no longer running".into()));
                }
                // Creation identity is per activation/key, independent of its conversation's other work.
                if let Some(id) = t.conversations().created_task(response, key)? {
                    return t.get(&id);
                }
                let task = t.create(&caller.actor, input)?;
                t.conversations().link_task(&r.conversation_id, &task.id)?;
                t.conversations().record_created_task(response, key, &task.id)?;
                // The durable requester supplies the task manager's identity and keys too.
                return t.update(
                    &Actor::new("genie", Role::Orchestrator),
                    &task.id,
                    UpdateInput { assignee: Some(incoming_initiator), ..Default::default() },
                );
            }
            t.create(&caller.actor, input)
        })
    })?;
    changed(app);
    Ok(task)
}

pub fn update(app: &App, caller: &Caller, id: &str, body: UpdateBody) -> AppResult<Task> {
    authorize(app, caller, id, Touch::Edit)?;
    if matches!(caller.kind, Kind::Ingress { .. }) && body.parent.is_some() {
        return Err(AppError::Forbidden("incoming work cannot change a task's parent".into()));
    }
    if genie_core::management::managed_task(&caller.actor.name).is_some()
        && (body.task_type.is_some()
            || body.description.is_some()
            || body.title.is_some()
            || body.priority.is_some()
            || body.merge_strategy.is_some()
            || body.assignee.is_some()
            || body.add_acceptance.is_some()
            || body.remove_acceptance.is_some()
            || body.add_deps.is_some()
            || body.remove_deps.is_some()
            || body.parent.is_some())
    {
        return Err(AppError::Forbidden("scope, requirements and priority changes belong to the project manager".into()));
    }
    if body.expected_version.is_empty() {
        return Err(AppError::Bad("expectedVersion is required for task edits; read the task's editVersion first".into()));
    }
    let expected_version = body.expected_version;
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
    let task = app.with_tracker(&caller.project, |t| {
        t.tx(|| {
            crate::conversations::ensure_active(t, &caller.actor.name)?;
            if view(&t.get(id)?)?["editVersion"].as_str() != Some(&expected_version) {
                return Ok(Err(AppError::Conflict(
                    "task changed since it was read; reload it and reconcile the pending changes before saving".into(),
                )));
            }
            t.update(&caller.actor, id, input).map(Ok)
        })
    })??;
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
    set_status_inner(app, caller, id, body, None)
}

/// Owner acceptance and the corresponding version-bound decision commit together.
pub(crate) fn accept_proposal(app: &App, caller: &Caller, proposal: &str) -> AppResult<Task> {
    let p = app.with_tracker(&caller.project, |t| t.conversations().proposal(proposal))?;
    if !caller.is_person() {
        return Err(AppError::Forbidden("a person accepts an incoming result".into()));
    }
    crate::git::delivery::gate(app, &caller.project, &p.task, Status::Done).map_err(AppError::Conflict)?;
    set_status_inner(
        app,
        caller,
        &p.task,
        StatusBody::to(Status::Done, Some(format!("Accepted proposal {} for its recorded result version.", p.id))),
        Some(proposal),
    )
}

fn set_status_inner(app: &App, caller: &Caller, id: &str, body: StatusBody, proposal: Option<&str>) -> AppResult<Task> {
    authorize(app, caller, id, Touch::Edit)?;
    let to = body.status;
    // An agent's documentation stays in a changeset: the task is not done while it is open.
    if to == Status::Done && !caller.is_person() {
        let task = app.with_tracker(&caller.project, |t| t.normalize_id(id))?;
        let open = app.with_server(|db| {
            db.changesets(&genie_core::changesets::ChangesetFilter {
                task: Some(task),
                statuses: vec![genie_core::changesets::ChangesetStatus::Open, genie_core::changesets::ChangesetStatus::Merging],
                ..Default::default()
            })
        })?;
        if let Some(cs) = open.first() {
            return Err(AppError::Conflict(format!("publish or discard documentation changeset #{} before closing the task", cs.id)));
        }
    }
    if to == Status::Done && !caller.is_person() && app.with_tracker(&caller.project, |t| t.management().has_blocking_questions(id))? {
        return Err(AppError::Conflict("answer blocking clarifications before accepting the result".into()));
    }
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
    let policy = crate::decisions::policy(app, &caller.project)?;
    let agents = app.agents();
    let task = app.with_tracker(&caller.project, |t| {
        t.tx(|| {
            crate::conversations::ensure_active(t, &caller.actor.name)?;
            if let Some(proposal) = proposal {
                let p = crate::conversations::check_proposal(t, proposal, genie_core::conversations::ProposalAction::Accept)?;
                if p.task != t.normalize_id(id)? {
                    return Err(genie_core::GenieError::Denied("the proposal names another task".into()));
                }
                if p.status == genie_core::conversations::ProposalStatus::Approved {
                    return t.get(id);
                }
                if t.management().has_blocking_questions(id)? {
                    return Err(genie_core::GenieError::invalid("answer blocking clarifications before accepting this result"));
                }
            }
            if matches!(to, Status::InProgress | Status::Done)
                && let Some(team) = t.get(id)?.team
            {
                crate::intelligence::validate_team_checks(t, &agents, &policy.intelligence, &team)?;
            }
            let task = t.set_status(&caller.actor, id, to, opts)?;
            if let Some(proposal) = proposal {
                t.conversations().decide_as(proposal, true, &caller.actor)?;
            }
            if crate::conversations::has_result(&task)
                && let Some(conversation) = t.conversations().conversation_for_task(&task.id)?
                && !t.management().has_blocking_questions(&task.id)?
            {
                t.conversations().propose(
                    &conversation,
                    &task.id,
                    genie_core::conversations::ProposalAction::Accept,
                    &crate::conversations::task_version(t, &task.id)?,
                )?;
            }
            Ok(task)
        })
    })?;
    if CLOSED.contains(&to) {
        // The task is closed: its team stops, and what it no longer needs goes with
        // it — this task only (a full sweep is the server's start-up job, G-134).
        if let Some(team) = task.team.clone()
            && let Err(e) = crate::runtime::stop_closed_team(app, &caller.project, &team)
        {
            eprintln!("genie runtime: stop team {team} of {}: {e}", caller.project);
        }
        for line in crate::runtime::sweep_task(app, &caller.project, &task.id) {
            println!("genie runtime: {}: {line}", caller.project);
        }
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
    let open: Vec<_> = rows.into_iter().filter(|r| r.cr_number.is_some() && r.cr_state == Some(genie_core::RequestState::Open)).collect();
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
    let task = app.with_tracker(&caller.project, |t| {
        crate::conversations::ensure_active(t, &caller.actor.name)?;
        t.comment(&caller.actor, id, &body.text, kind)
    })?;
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
        let dedupe = task.comments.last().map(|c| format!("comment:{}:{}:{}", caller.project, task.id, c.id));
        crate::notify::send(app, &users, &msg, dedupe.as_deref())?;
    }
    changed(app);
    Ok(task)
}

/// Complete a previously authorized human answer, with the identity frozen on acceptance.
/// This is durable delivery of that decision, so a replay does not solicit new authority.
pub(crate) fn record_answers(app: &App, project: &str, actor: &str, task: &str, receipt: &str, text: &str) -> AppResult<()> {
    app.with_tracker(project, |t| t.comment_once(receipt, &Actor::new(actor, Role::Human), task, text))?;
    changed(app);
    Ok(())
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
        if app.with_tracker(slug, |t| Ok(t.bus().get(team)?.state == genie_core::TeamState::Active))? {
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

/// A personal subscription, independent of assignment. Authors always receive updates.
#[derive(Debug, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TaskWatch {
    pub watching: bool,
    pub author: bool,
}

pub fn watch(app: &App, caller: &Caller, id: &str, value: Option<bool>) -> AppResult<TaskWatch> {
    if !caller.is_person() {
        return Err(AppError::Forbidden("Only people can follow tasks".into()));
    }
    let user = app.with_server(|db| {
        let user = db
            .user_by_login(&caller.actor.name)?
            .ok_or_else(|| genie_core::GenieError::invalid("Create an account before following tasks"))?;
        if user.disabled || db.project_role(&caller.project, &user)?.is_none() {
            return Err(genie_core::GenieError::Denied("No access to this project".into()));
        }
        Ok(user)
    })?;
    let task = app.with_tracker(&caller.project, |tr| tr.get(id))?;
    let author = task.history.iter().find(|h| h.event == "created" && h.role == Role::Human).is_some_and(|h| h.actor == user.login);
    app.with_server(|db| {
        if let Some(value) = value {
            db.set_task_watch(&caller.project, &task.id, user.id, value)?;
        }
        Ok(TaskWatch { author, watching: author || db.task_watchers(&caller.project, &task.id)?.contains(&user.id) })
    })
}

// Task intelligence mutations share this application boundary across HTTP and
// catalog consumers. Provider calls and routing policy live in their own module.
pub fn intelligence_history(app: &App, caller: &Caller, id: &str) -> AppResult<genie_core::intelligence::IntelligenceHistory> {
    authorize(app, caller, id, Touch::Edit)?;
    app.with_tracker(&caller.project, |t| t.intelligence_history(id))
}
pub fn assess_intelligence(
    app: &App,
    caller: &Caller,
    id: &str,
    body: crate::intelligence::AssessBody,
) -> AppResult<genie_core::intelligence::Snapshot> {
    crate::intelligence::assess_task(app, caller, id, body)
}
pub fn reassess_intelligence(app: &App, caller: &Caller, id: &str, body: crate::intelligence::ReassessBody) -> AppResult<Value> {
    crate::intelligence::reassess(app, caller, id, body)
}
pub fn shadow_intelligence(app: &App, caller: &Caller, id: &str, body: crate::intelligence::ShadowBody) -> AppResult<Value> {
    if !caller.is_person() {
        return Err(AppError::Forbidden("only a project administrator can evaluate candidate policies".into()));
    }
    crate::intelligence::shadow(app, caller, id, body)
}
pub fn resolve_intelligence(
    app: &App,
    caller: &Caller,
    id: &str,
    body: crate::intelligence::ResolveBody,
) -> AppResult<genie_core::intelligence::Snapshot> {
    crate::intelligence::resolve(app, caller, id, body)
}

/// Common human-only execution control; workflow status and artifacts are retained.
pub fn pause_execution(app: &App, caller: &Caller, task: &str, reason: &str, note: &str) -> AppResult<serde_json::Value> {
    pause_execution_inner(app, caller, task, reason, note, None)
}

/// A deterministic control reply commits with its execution generation. Replaying
/// the response after a crash must not stop or resume a later generation.
pub(crate) struct ControlReply<'a> {
    pub response: &'a str,
    pub text: &'a str,
}

pub(crate) fn pause_execution_inner(
    app: &App,
    caller: &Caller,
    task: &str,
    reason: &str,
    note: &str,
    reply: Option<ControlReply<'_>>,
) -> AppResult<serde_json::Value> {
    if !caller.is_person() {
        return Err(AppError::Forbidden("only a person stops or resumes task execution".into()));
    }
    pause_inner(app, &caller.project, &caller.actor, task, reason, note, reply)
}

pub(crate) fn pause_for_limit(app: &App, project: &str, task: &str, reason: &str, note: &str) -> AppResult<serde_json::Value> {
    pause_inner(app, project, &Actor::new("genie", Role::Orchestrator), task, reason, note, None)
}

/// Stage a hard-limit intent and, if its server write fails, retain the pause in
/// the project DB. Live and one-shot workers use the same persistence policy.
pub(crate) fn queue_turn_pause(
    app: &App,
    project: &str,
    task: &str,
    note: &str,
    prepare_turn: impl FnOnce(&genie_core::server_db::ServerDb) -> genie_core::Result<i64>,
) -> AppResult<genie_core::work::TurnPause> {
    let queued = {
        let _gate = app.execution_gate.lock().map_err(|_| AppError::Internal("execution gate poisoned".into()))?;
        app.with_server(|db| db.tx(|| db.queue_turn_pause(prepare_turn(db)?, task, note)))
    };
    match queued {
        Ok(pause) => Ok(pause),
        Err(error) => {
            if let Err(pause_error) = pause_for_limit(app, project, task, "time", note) {
                eprintln!("genie runtime: {project}/{task}: project hard-limit fallback pending: {pause_error}");
            }
            Err(error)
        }
    }
}

pub(crate) fn apply_turn_pause(app: &App, pause: &genie_core::work::TurnPause) -> AppResult<()> {
    let _gate = app.execution_gate.lock().map_err(|_| AppError::Internal("execution gate poisoned".into()))?;
    let halt = app.with_tracker(&pause.project, |t| {
        t.tx(|| {
            let applied = t.execution().pause_once(&Actor::new("genie", Role::Orchestrator), &pause.task, &pause.note, &pause.id)?;
            Ok(t.execution().state(&pause.task)?.is_some_and(|s| s.paused && s.generation == applied.generation))
        })
    })?;
    if halt {
        crate::runtime::halt_scope(app, &pause.project, Some(&pause.task))?;
    }
    changed(app);
    Ok(())
}

fn pause_inner(
    app: &App,
    project: &str,
    actor: &Actor,
    task: &str,
    reason: &str,
    note: &str,
    reply: Option<ControlReply<'_>>,
) -> AppResult<serde_json::Value> {
    let _gate = app.execution_gate.lock().map_err(|_| AppError::Internal("execution gate poisoned".into()))?;
    let state = app.with_tracker(project, |t| {
        t.tx(|| {
            let state = t.execution().pause(actor, task, reason, note)?;
            if let Some(reply) = reply {
                t.conversations().complete(reply.response, reply.text)?;
            }
            Ok(state)
        })
    })?;
    let stopped = crate::runtime::halt_scope(app, project, Some(&state.task))?;
    changed(app);
    Ok(serde_json::json!({"execution":state,"stopped":stopped}))
}

pub fn resume_execution(app: &App, caller: &Caller, task: &str) -> AppResult<serde_json::Value> {
    resume_inner(app, caller, task, None, None)
}

pub(crate) fn resume_execution_with_reply(app: &App, caller: &Caller, task: &str, reply: ControlReply<'_>) -> AppResult<serde_json::Value> {
    resume_inner(app, caller, task, None, Some(reply))
}

pub(crate) fn resume_interaction(app: &App, caller: &Caller, task: &str, generation: i64) -> AppResult<serde_json::Value> {
    resume_inner(app, caller, task, Some(generation), None)
}

fn resume_inner(
    app: &App,
    caller: &Caller,
    task: &str,
    generation: Option<i64>,
    reply: Option<ControlReply<'_>>,
) -> AppResult<serde_json::Value> {
    if !caller.is_person() {
        return Err(AppError::Forbidden("only a person stops or resumes task execution".into()));
    }
    let _gate = app.execution_gate.lock().map_err(|_| AppError::Internal("execution gate poisoned".into()))?;
    if crate::runtime::pending_limit(app, &caller.project, task)? {
        return Err(AppError::Conflict("a hard-limit pause is awaiting persistence; inspect it before resuming".into()));
    }
    if let Some(expected) = generation {
        let current = app.with_tracker(&caller.project, |t| t.execution().state(task))?;
        if let Some(current) = current.filter(|s| !s.paused && expected.checked_add(1) == Some(s.generation)) {
            return Ok(serde_json::json!({"execution":current}));
        }
    }
    if let Some(exceeded) = crate::budget::check(app, &caller.project, Some(task))? {
        return Err(AppError::Conflict(exceeded.explain()));
    }
    let state = app.with_tracker(&caller.project, |t| {
        t.tx(|| {
            if let Some(expected) = generation {
                let current = t.execution().state(task)?.ok_or_else(|| genie_core::GenieError::not_found("execution pause"))?;
                if current.paused && current.generation != expected
                    || !current.paused && expected.checked_add(1) != Some(current.generation)
                {
                    return Err(genie_core::GenieError::invalid("this execution pause is obsolete; inspect the current pause"));
                }
            }
            let was_paused = t.execution().state(task)?.is_some_and(|s| s.paused);
            let state = t.execution().resume(&caller.actor, task)?;
            t.execution().ensure_running(task)?;
            if t.bus().active_count()? as usize > app.cfg.limits.max_active_teams {
                return Err(genie_core::GenieError::invalid("limit: resuming would exceed the project's active-team capacity"));
            }
            if app.cfg.limits.max_active_teams_per_epic > 0 {
                let index = crate::spend::TaskIndex::load(t.conn())?;
                let epics =
                    t.bus().list(false)?.iter().filter_map(|team| index.epic_of(&team.task)).collect::<std::collections::BTreeSet<_>>();
                for epic in epics {
                    if t.bus().active_count_in_epic(&epic)? as usize > app.cfg.limits.max_active_teams_per_epic {
                        return Err(genie_core::GenieError::invalid("limit: resuming would exceed active-team capacity in an epic"));
                    }
                }
            }
            if was_paused {
                // Mail already exists for interrupted turns. Settled sessions need one explicit wake-up.
                let index = crate::spend::TaskIndex::load(t.conn())?;
                for manager in t.management().managers()? {
                    if index.is_under(&manager.task, &t.normalize_id(task)?) && manager.state != "completed" {
                        t.bus().notify_manager(
                            &genie_core::management::manager_name(&manager.task),
                            &caller.actor.name,
                            "human",
                            "system",
                            "Execution explicitly resumed. Read current task state and continue retained work.",
                            Some(&manager.task),
                        )?;
                    }
                }
            }
            if let Some(reply) = reply {
                t.conversations().complete(reply.response, reply.text)?;
            }
            Ok(state)
        })
    })?;
    changed(app);
    Ok(serde_json::json!({"execution":state}))
}
