//! The built-in work loop: the server keeps a project's tasks moving without a playbook to install.
//! It runs in every engine pass for each project except `manual` ones, where people dispatch.
//!
//! - **Intake** (journal cursor `work-loop`): a task entering preparation (`draft`, `refining`)
//!   prompts the project manager to prepare it; a task that nothing holds becomes `ready`; a
//!   finished team, a closed task or a new ready task runs a dispatch pass.
//! - **Reconciliation** (at most once a minute per project): the same checks over every task, and a
//!   new prompt for preparation that looks lost, so a missed wake-up does not strand a task.
//! - **Dispatch**: ready tasks get their task manager by priority, within the team limits.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};

use chrono::{DateTime, Duration, Utc};
use genie_core::events::{self, Event};
use genie_core::team::ORCHESTRATOR;
use genie_core::tracker::ListFilter;
use genie_core::{Actor, Role, Status, Task, TaskSummary, TaskType};
use serde_json::{Value, json};

use crate::notify::{self, Message};
use crate::state::{App, AppResult};
use crate::tasks::{self, Caller, StatusBody};

const CURSOR: &str = "work-loop";
/// Sender of the loop's prompts; what it wrote about a task drives the retries.
const SENDER: &str = "work-loop";
/// Journal events read per pass.
const BATCH: usize = 200;
/// Reconciliation runs at most this often per project.
const RECONCILE_EVERY: Duration = Duration::seconds(60);
/// A task that was never prompted is prompted once nothing touched it for this long.
const FIRST_PROMPT_AFTER: Duration = Duration::seconds(120);
/// Prompts for one task before the loop stops and tells the admins.
const MAX_PROMPTS: i64 = 3;
/// The statuses of tasks in preparation (the «Подготовка» lane).
const PREPARATION: &[Status] = &[Status::Draft, Status::Refining];

/// When each project was last reconciled, keyed by data directory and project (one server owns one).
static RECONCILED: Mutex<BTreeMap<(PathBuf, String), DateTime<Utc>>> = Mutex::new(BTreeMap::new());

/// Runs the loop for one project and returns when it must look again without an event.
pub fn tick(app: &App, project: &str) -> AppResult<Option<DateTime<Utc>>> {
    let now = Utc::now();
    let manual = app.with_server(|db| db.project(project))?.autonomy == "manual";
    let (dispatch, more) = intake(app, project, !manual)?;
    if manual {
        return Ok(more.then_some(now));
    }
    if dispatch {
        dispatch_ready(app, project)?;
    }
    let next = reconcile_if_due(app, project, now)?;
    Ok(if more { Some(now) } else { next })
}

/// Reads the journal after the loop's cursor, acts on each event when `act`, and moves the cursor.
/// Returns whether a dispatch pass is due and whether more events are waiting.
fn intake(app: &App, project: &str, act: bool) -> AppResult<(bool, bool)> {
    let cursor = app.with_tracker(project, |t| t.event_cursor(CURSOR))?;
    let events = app.with_tracker(project, |t| t.events_after(cursor, BATCH))?;
    let Some(last) = events.last().map(|e| e.id) else { return Ok((false, false)) };
    let mut dispatch = false;
    if act {
        for e in &events {
            // An event that fails is not retried forever and does not hold the others back:
            // reconciliation finds whatever it missed.
            match handle(app, project, e) {
                Ok(due) => dispatch |= due,
                Err(err) => eprintln!("genie work loop: {project}: event {}: {err}", e.id),
            }
        }
    }
    app.with_tracker(project, |t| t.ack_events(CURSOR, last))?;
    Ok((dispatch, events.len() == BATCH))
}

/// What one journal event asks of the loop, and whether it frees room for a dispatch.
fn handle(app: &App, project: &str, e: &Event) -> AppResult<bool> {
    let to = e.payload["to"].as_str().and_then(|s| s.parse::<Status>().ok());
    if let Some(id) = e.subject.as_deref().filter(|_| e.kind.starts_with("task.")) {
        ready_check(app, project, id)?;
    }
    // After the ready check: a task that nothing holds goes straight to `ready`, without a prompt.
    if let (events::TASK_STATUS_CHANGED, Some(Status::Draft | Status::Refining), Some(id)) = (e.kind.as_str(), to, &e.subject) {
        prompt_if_preparing(app, project, id)?;
    }
    Ok(match e.kind.as_str() {
        "team.stopped" | "manager.retrospective_published" => true,
        events::TASK_STATUS_CHANGED => matches!(to, Some(Status::Ready | Status::Done | Status::Cancelled)),
        _ => false,
    })
}

/// Moves a task in preparation, or a closed one (which frees the tasks waiting on it), that nothing holds.
fn ready_check(app: &App, project: &str, task: &str) -> AppResult<()> {
    let status = app.with_tracker(project, |t| Ok(if t.exists(task)? { Some(t.get(task)?.status) } else { None }))?;
    if matches!(status, Some(s) if PREPARATION.contains(&s) || s == Status::Done) {
        make_ready(app, project, &caller(project), task)?;
    }
    Ok(())
}

/// Prompts the project manager about a task that entered preparation, unless it has left it already
/// or a prompt about it is still unread.
fn prompt_if_preparing(app: &App, project: &str, task: &str) -> AppResult<()> {
    let Some(t) = find_task(app, project, task)? else { return Ok(()) };
    if t.task_type == TaskType::Epic || !PREPARATION.contains(&t.status) {
        return Ok(());
    }
    if app.with_tracker(project, |tr| tr.bus().sent_about(SENDER, &t.id))?.undelivered > 0 {
        return Ok(());
    }
    prompt(app, project, &t.id, &t.title)
}

fn prompt(app: &App, project: &str, id: &str, title: &str) -> AppResult<()> {
    let text = format!(
        "Task {id} ({title}) is in preparation. Prepare it: read it, check project memory and documentation, complete its description, scope and acceptance criteria, and link its dependencies. Ask the author only what only they can answer (owner questions). When the Definition of Ready is met, move the task to `ready`: the server dispatches ready tasks itself. If it is not worth doing, say so to the author instead of guessing."
    );
    app.with_tracker(project, |t| t.bus().notify_manager(ORCHESTRATOR, SENDER, "orchestrator", "system", &text, Some(id)))?;
    app.wake_runtime.notify_one();
    Ok(())
}

/// Reconciliation, when it is due. Returns when it is due next.
fn reconcile_if_due(app: &App, project: &str, now: DateTime<Utc>) -> AppResult<Option<DateTime<Utc>>> {
    let last = RECONCILED.lock().unwrap_or_else(PoisonError::into_inner).get(&(app.data.clone(), project.to_string())).copied();
    if let Some(last) = last
        && last + RECONCILE_EVERY > now
    {
        return Ok(Some(last + RECONCILE_EVERY));
    }
    reconcile(app, project, now)?;
    Ok(Some(now + RECONCILE_EVERY))
}

/// Reconciliation: the ready check over every task in preparation, a new prompt for preparation that
/// looks lost, and a dispatch pass. `now` is the time the checks judge by.
pub fn reconcile(app: &App, project: &str, now: DateTime<Utc>) -> AppResult<()> {
    RECONCILED.lock().unwrap_or_else(PoisonError::into_inner).insert((app.data.clone(), project.to_string()), now);
    let caller = caller(project);
    for task in preparing(app, project)? {
        if let Err(e) = make_ready(app, project, &caller, &task.id) {
            eprintln!("genie work loop: {project}: {}: ready check: {e}", task.id);
        }
    }
    for task in preparing(app, project)? {
        if let Err(e) = reprompt(app, project, &task, now) {
            eprintln!("genie work loop: {project}: {}: prompt: {e}", task.id);
        }
    }
    dispatch_ready(app, project)?;
    Ok(())
}

/// Tasks in preparation, epics left out: they never become ready (see `readiness_problems`).
fn preparing(app: &App, project: &str) -> AppResult<Vec<TaskSummary>> {
    let tasks = app.with_tracker(project, |t| t.list(&ListFilter { status: PREPARATION.to_vec(), ..Default::default() }))?;
    Ok(tasks.into_iter().filter(|t| t.task_type != TaskType::Epic).collect())
}

/// Prompts again a task in preparation whose prompts and journal went quiet for too long, unless
/// something holds it on purpose. After `MAX_PROMPTS` prompts the admins hear of it instead.
fn reprompt(app: &App, project: &str, task: &TaskSummary, now: DateTime<Utc>) -> AppResult<()> {
    // A block, a decision or open questions are what the task waits for: nothing for the manager to do.
    if task.blocked.is_some() || task.needs_owner.is_some() {
        return Ok(());
    }
    if app.with_server(|db| db.open_questionnaires_for_task(project, &task.id))? > 0 {
        return Ok(());
    }
    let sent = app.with_tracker(project, |t| t.bus().sent_about(SENDER, &task.id))?;
    if sent.undelivered > 0 {
        return Ok(());
    }
    let touched = app.with_tracker(project, |t| t.last_event_at(&task.id))?.as_deref().and_then(parse_at);
    let retry = Duration::try_seconds(i64::try_from(app.cfg.runtime.prep_retry_secs).unwrap_or(i64::MAX)).unwrap_or(Duration::MAX);
    let quiet = |since: Option<DateTime<Utc>>, after: Duration| since.is_none_or(|t| now - t >= after);
    let lost = match sent.last.as_deref().and_then(parse_at) {
        None => quiet(touched, FIRST_PROMPT_AFTER),
        Some(prompt_at) => quiet(Some(prompt_at), retry) && quiet(touched, retry),
    };
    if !lost {
        return Ok(());
    }
    if sent.sent >= MAX_PROMPTS {
        return stuck(app, project, task);
    }
    prompt(app, project, &task.id, &task.title)
}

/// Tells the admins, once, that a task's preparation did not move after all the prompts.
fn stuck(app: &App, project: &str, task: &TaskSummary) -> AppResult<()> {
    let users = notify::resolve(app, project, &["project.admins".to_string()], &json!({}))?;
    let msg = Message {
        kind: "stuck".into(),
        title: format!("Подготовка {} застряла", task.id),
        body: format!("{}\n\nМенеджер проекта не довёл задачу до «Готово к работе» после {MAX_PROMPTS} напоминаний.", task.title),
        project: Some(project.into()),
        task: Some(task.id.clone()),
        link: Some(format!("/task/{}", task.id)),
        ..Default::default()
    };
    notify::send(app, &users, &msg, Some(&format!("loop-stuck:{project}:{}", task.id)))?;
    Ok(())
}

/// Dispatches ready tasks by priority (0 first), then by creation, while the team limits allow. An
/// active task manager takes one of `limits.maxActiveTeams` (and one of its epic's
/// `maxActiveTeamsPerEpic`) until its task closes. A task that cannot be dispatched (not ready,
/// dependencies open, budget) is skipped and tried again at the next pass.
pub fn dispatch_ready(app: &App, project: &str) -> AppResult<Vec<String>> {
    let (mut active, mut per_epic, ready) = app.with_tracker(project, |t| {
        let mut active = 0usize;
        let mut per_epic: HashMap<String, usize> = HashMap::new();
        let mut open = HashSet::new();
        for m in t.management().managers()? {
            if m.state == "completed" {
                continue;
            }
            open.insert(m.task.clone());
            if m.state == "active" {
                active += 1;
                if let Some(epic) = t.get(&m.task)?.parent {
                    *per_epic.entry(epic).or_default() += 1;
                }
            }
        }
        let mut ready = t.list(&ListFilter { status: vec![Status::Ready], exclude_epics: true, ..Default::default() })?;
        ready.retain(|s| !open.contains(&s.id));
        ready.sort_by(|a, b| a.priority.cmp(&b.priority).then_with(|| a.created.cmp(&b.created)));
        Ok((active, per_epic, ready))
    })?;
    let limits = &app.cfg.limits;
    let actor = Actor::new("genie", Role::Orchestrator);
    let mut dispatched = Vec::new();
    for task in ready {
        if active >= limits.max_active_teams {
            break;
        }
        let epic_full = limits.max_active_teams_per_epic > 0
            && task.parent.as_ref().is_some_and(|e| per_epic.get(e).copied().unwrap_or(0) >= limits.max_active_teams_per_epic);
        if epic_full {
            continue;
        }
        match crate::management::dispatch(app, project, &actor, &task.id) {
            Ok(_) => {
                active += 1;
                if let Some(epic) = &task.parent {
                    *per_epic.entry(epic.clone()).or_default() += 1;
                }
                dispatched.push(task.id);
            }
            Err(e) => eprintln!("genie work loop: {project}: {} not dispatched: {e}", task.id),
        }
    }
    Ok(dispatched)
}

/// Moves the task to `ready`, and the tasks waiting on it, unless something still holds each one.
pub fn make_ready(app: &App, project: &str, caller: &Caller, task: &str) -> AppResult<Value> {
    // The task itself, and the tasks that wait for it: closing a dependency frees them.
    let mut candidates = vec![task.to_string()];
    candidates.extend(app.with_tracker(project, |t| t.dependents(task))?);
    let (mut moved, mut held) = (Vec::new(), Vec::new());
    for id in candidates {
        let t = app.with_tracker(project, |t| t.get(&id))?;
        if !PREPARATION.contains(&t.status) {
            continue;
        }
        let holds = holds(app, project, &t)?;
        if !holds.is_empty() {
            held.push(json!({ "task": t.id, "holds": holds }));
            continue;
        }
        let note = "Nothing holds it: no block, dependencies done, no open questions or jobs; moved to ready automatically".to_string();
        tasks::set_status(app, caller, &t.id, StatusBody::to(Status::Ready, Some(note)))?;
        moved.push(t.id);
    }
    Ok(json!({ "moved": moved, "held": held }))
}

/// What keeps a task from being ready: a block, open dependencies, a decision or
/// answers it waits for, work still going on it, the Definition of Ready.
fn holds(app: &App, project: &str, task: &Task) -> AppResult<Vec<String>> {
    let mut out = Vec::new();
    if let Some(b) = &task.blocked {
        out.push(format!("blocked: {}", b.reason));
    }
    let open = app.with_tracker(project, |t| t.open_deps(&task.id))?;
    if !open.is_empty() {
        out.push(format!("waits for dependencies: {}", open.join(", ")));
    }
    if task.needs_owner.is_some() {
        out.push("waits for the owner's decision".into());
    }
    if app.with_server(|db| db.open_questionnaires_for_task(project, &task.id))? > 0 {
        out.push("waits for answers to questions".into());
    }
    if app.with_server(|db| db.open_jobs_for_task(project, &task.id))? > 0 {
        out.push("an agent job on it is still going".into());
    }
    if let Some(team) = app.with_tracker(project, |t| t.bus().active_team_of(&task.id))? {
        out.push(format!("team {team} is working on it"));
    }
    let known = task.deps.iter().filter(|d| app.with_tracker(project, |t| t.exists(d)).unwrap_or(false)).cloned().collect();
    out.extend(genie_core::readiness_problems(task, &known));
    Ok(out)
}

/// The loop acts as the orchestrator, like the management's reconciliation.
fn caller(project: &str) -> Caller {
    Caller::automation(project, Actor::new("genie", Role::Orchestrator))
}

fn find_task(app: &App, project: &str, id: &str) -> AppResult<Option<Task>> {
    app.with_tracker(project, |t| if t.exists(id)? { Ok(Some(t.get(id)?)) } else { Ok(None) })
}

fn parse_at(at: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(at).ok().map(|t| t.with_timezone(&Utc))
}
