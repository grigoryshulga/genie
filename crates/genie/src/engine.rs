//! Automation engine and built-in notifications.
//!
//! The engine sleeps until it is woken (`App::wake_engine`: journal events, answered questions,
//! finished jobs, rule changes) or until the soonest deadline of time-based work: a cron rule, a
//! questionnaire's reminder or due time, a `wait`, a step's timeout, a retry (see [`Plan`]).
//! Each pass:
//! 1. **Intake**: read new journal events of every project after the engine's
//!    cursor, create a run for each matching rule (unique per event, so a
//!    re-read never starts a rule twice), then move the cursor.
//! 2. **Schedules**: start runs of cron rules that became due.
//! 3. **Advance**: execute the next steps of active runs. Every step's state is
//!    stored; steps that wait (agent jobs, questionnaires, teams, pauses) are
//!    polled, so runs survive restarts.
//! 4. **Notify**: tell people about decisions they owe, closed tasks, proposals.
//! 5. **Team flow**: handoffs bound to a status in a team's relations (`on`) are
//!    delivered by genie itself when the task enters that status.
//!
//! Protections: no rule triggers itself, cascades stop at depth 3, per-rule
//! concurrency and hourly limits, dry-run mode.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use genie_core::automation::{self, Automation, Run, StepState};
use genie_core::events::Event;
use genie_core::inbox::NewQuestion;
use genie_core::work::NewJob;
use genie_core::{Actor, GenieError, Role, Status};
use serde_json::{Value, json};

use crate::notify::{self, Message};
use crate::runtime::{self, SpawnRequest};
use crate::state::{App, AppError, AppResult, SAFETY_NET, sleep_until};
use crate::tasks::{self, Caller, StatusBody};

const CURSOR: &str = "automations";
const NOTIFY_CURSOR: &str = "notifier";
const FLOW_CURSOR: &str = "team-flow";

/// Passes in a row that ask for another at once; a bug that keeps asking falls back to the deadlines.
const MAX_BACK_TO_BACK: u32 = 20;
/// Wait before looking again at a step that failed and has retries left.
const RETRY_AFTER: chrono::Duration = chrono::Duration::seconds(2);

/// What a pass learned about when the engine has to run again without being woken.
#[derive(Default)]
pub struct Plan {
    /// Something a run waits for happened inside the pass (a run ended and freed a slot of its
    /// rule's concurrency): pass again at once.
    again: bool,
    /// The soonest moment something is due by itself.
    at: Option<DateTime<Utc>>,
}

impl Plan {
    /// When the engine has to run again unless woken before: `None` means only the safety net.
    pub fn next_at(&self) -> Option<DateTime<Utc>> {
        self.at
    }

    fn due(&mut self, at: DateTime<Utc>) {
        self.at = Some(self.at.map_or(at, |t| t.min(at)));
    }

    /// The `deadline` or `until` a waiting step carries.
    fn due_wait(&mut self, wait: &Value) {
        for key in ["deadline", "until"] {
            if let Some(at) = wait[key].as_str().and_then(|t| DateTime::parse_from_rfc3339(t).ok()) {
                self.due(at.with_timezone(&Utc));
            }
        }
    }
}

pub fn start(app: &Arc<App>) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut streak = 0;
        loop {
            let plan = match app.blocking(pass).await {
                Ok(plan) => plan,
                Err(e) => {
                    eprintln!("genie engine: {e}");
                    Plan { at: Some(Utc::now() + chrono::Duration::seconds(10)), ..Plan::default() }
                }
            };
            streak = if plan.again { streak + 1 } else { 0 };
            if plan.again && streak < MAX_BACK_TO_BACK {
                continue;
            }
            tokio::select! {
                _ = app.wake_engine.notified() => {}
                _ = tokio::time::sleep(sleep_until(plan.at, SAFETY_NET)) => {}
            }
        }
    });
}

/// One engine pass (also used directly by tests).
pub fn tick(app: &App) -> AppResult<()> {
    pass(app).map(|_| ())
}

pub fn pass(app: &App) -> AppResult<Plan> {
    let mut plan = Plan::default();
    for p in app.with_server(|db| db.projects())? {
        if let Err(e) = intake(app, &p.slug) {
            eprintln!("genie engine: {}: {e}", p.slug);
        }
        if let Err(e) = notify_intake(app, &p.slug) {
            eprintln!("genie notifier: {}: {e}", p.slug);
        }
        if let Err(e) = flow_intake(app, &p.slug) {
            eprintln!("genie team flow: {}: {e}", p.slug);
        }
    }
    schedules(app, &mut plan)?;
    if let Some(at) = crate::questions::tick(app)? {
        plan.due(at);
    }
    for run in app.with_server(|db| db.active_runs())? {
        if let Err(e) = advance(app, &run, &mut plan) {
            let msg = e.to_string();
            let _ = app.with_server(|db| db.set_run_status(run.id, "failed", Some(&msg)));
            plan.again = true;
        }
    }
    Ok(plan)
}

// --- intake ----------------------------------------------------------------------------

/// Context of an event: payload fields at the top level, plus `task`, `actor`, `event`.
fn event_context(app: &App, project: &str, e: &Event) -> Value {
    let task = e.subject.as_deref().and_then(|s| app.with_tracker(project, |t| t.get(s)).ok());
    let role = if e.actor_role == "human" {
        app.with_server(|db| {
            Ok(match db.user_by_login(&e.actor)? {
                Some(u) => db.project_role(project, &u)?.map(|r| r.as_str().to_string()),
                None => None,
            })
        })
        .ok()
        .flatten()
    } else {
        None
    };
    let mut ctx = e.payload.clone();
    if !ctx.is_object() {
        ctx = json!({});
    }
    let ev = json!({ "id": e.id, "type": e.kind, "subject": e.subject, "actor": e.actor, "actorRole": e.actor_role, "at": e.at, "payload": e.payload, "task": task });
    ctx["event"] = ev;
    ctx["task"] = serde_json::to_value(&task).unwrap_or(Value::Null);
    ctx["actor"] = json!({ "name": e.actor, "role": e.actor_role, "project_role": role });
    ctx["project"] = json!(project);
    ctx
}

/// Which run (if any) caused an event: automation actors are `automation:<rule>:<run>`,
/// job agents are `job-<id>` (their job belongs to a run step).
fn causing_run(app: &App, actor: &str) -> Option<Run> {
    if let Some(rest) = actor.strip_prefix("automation:") {
        let run: i64 = rest.split(':').nth(1)?.parse().ok()?;
        return app.with_server(|db| db.run(run)).ok();
    }
    if let Some(job) = actor.strip_prefix("job-").and_then(|j| j.parse::<i64>().ok()) {
        let step = app.with_server(|db| db.job(job)).ok()?.run_step?;
        let run = app.with_server(|db| db.step(step)).ok()?.run;
        return app.with_server(|db| db.run(run)).ok();
    }
    None
}

fn event_matches(a: &Automation, kind: &str) -> bool {
    match a.spec["on"]["event"].as_str() {
        Some(pattern) if pattern.ends_with(".*") => kind.starts_with(&pattern[..pattern.len() - 1]),
        Some(pattern) => pattern == kind,
        None => false,
    }
}

fn intake(app: &App, project: &str) -> AppResult<()> {
    let cursor = app.with_tracker(project, |t| t.event_cursor(CURSOR))?;
    let events = app.with_tracker(project, |t| t.events_after(cursor, 200))?;
    let Some(last) = events.last().map(|e| e.id) else { return Ok(()) };
    // Rules are read after the events: a rule created while this pass runs is
    // already in the list for the events it may concern. A rule applies to the
    // events from its creation on, never to older history in the same batch.
    let rules: Vec<Automation> =
        app.with_server(|db| db.automations(Some(project)))?.into_iter().filter(|a| a.enabled && a.trigger_kind() == "event").collect();
    for e in &events {
        let candidates: Vec<&Automation> = rules.iter().filter(|a| event_matches(a, &e.kind) && e.at >= a.created).collect();
        if candidates.is_empty() {
            continue;
        }
        let ctx = event_context(app, project, e);
        let cause = causing_run(app, &e.actor);
        for a in candidates {
            if !automation::matches(&a.spec["on"]["where"], &ctx) {
                continue;
            }
            let depth = cause.as_ref().map(|r| r.depth + 1).unwrap_or(0);
            let key = format!("event:{project}:{}", e.id);
            let skip = if cause.as_ref().is_some_and(|r| r.automation == a.id) {
                Some("the rule's own action does not trigger it again")
            } else if depth > automation::MAX_DEPTH {
                Some("cascade too deep")
            } else {
                None
            };
            start_run(app, a, &key, ctx.clone(), depth, skip)?;
        }
    }
    app.with_tracker(project, |t| t.ack_events(CURSOR, last))?;
    Ok(())
}

/// Create a run (idempotent per trigger key) honouring dry-run and the hourly limit.
pub fn start_run(app: &App, a: &Automation, key: &str, ctx: Value, depth: i64, skip: Option<&str>) -> AppResult<Option<Run>> {
    let hour_ago = (Utc::now() - chrono::Duration::hours(1)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let limit = a.spec["limits"]["maxRunsPerHour"].as_i64();
    let over = match limit {
        Some(l) => app.with_server(|db| db.runs_since(a.id, &hour_ago))? >= l,
        None => false,
    };
    let reason =
        skip.map(str::to_string).or_else(|| over.then(|| "hourly limit reached".into())).or_else(|| a.dry_run.then(|| "dry run".into()));
    let trigger = json!({ "context": ctx, "spec": a.spec });
    let status = if reason.is_some() { "skipped" } else { "queued" };
    let run = app.with_server(|db| {
        let run = db.create_run(a, key, &trigger, depth, status)?;
        if let (Some(r), Some(why)) = (&run, &reason) {
            db.set_run_status(r.id, "skipped", Some(why))?;
        }
        Ok(run)
    })?;
    Ok(run)
}

fn schedules(app: &App, plan: &mut Plan) -> AppResult<()> {
    let now = Utc::now();
    for a in app.with_server(|db| db.automations(None))?.into_iter().filter(|a| a.enabled && a.trigger_kind() == "schedule") {
        let expr = a.spec["on"]["schedule"].as_str().unwrap_or_default();
        let tz = a.spec["on"]["tz"].as_str();
        let last = app.with_server(|db| db.last_trigger_key(a.id, "schedule:"))?;
        let after = last
            .and_then(|k| {
                k.strip_prefix("schedule:").and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&Utc))
            })
            .or_else(|| chrono::DateTime::parse_from_rfc3339(&a.updated).ok().map(|t| t.with_timezone(&Utc)))
            .unwrap_or(now);
        if let Some(fire) = automation::due_fire(expr, tz, after, now) {
            let key = format!("schedule:{}", fire.to_rfc3339());
            start_run(app, &a, &key, json!({ "project": a.project, "schedule": { "at": fire.to_rfc3339() } }), 0, None)?;
        }
        if let Some(next) = automation::next_fire(expr, tz, now) {
            plan.due(next);
        }
    }
    Ok(())
}

// --- runs ----------------------------------------------------------------------------------

fn automation_actor(run: &Run) -> Actor {
    Actor::new(format!("automation:{}:{}", run.automation, run.id), Role::Orchestrator)
}

fn duration_or(step: &Value, key: &str, default: chrono::Duration) -> chrono::Duration {
    step[key].as_str().and_then(automation::parse_duration).unwrap_or(default)
}

enum Outcome {
    Done(Value),
    Wait(Value),
}

fn advance(app: &App, run: &Run, plan: &mut Plan) -> AppResult<()> {
    let spec = run.trigger["spec"].clone();
    let steps = spec["steps"].as_array().cloned().unwrap_or_default();
    if run.status == "queued" {
        let limit = spec["limits"]["concurrency"].as_i64().unwrap_or(4);
        // Held back by its rule's concurrency: a run of the rule that ends frees it (in a pass of
        // this engine, or by a person's cancel, which wakes it).
        if app.with_server(|db| db.running_count(run.automation))? >= limit {
            return Ok(());
        }
        app.with_server(|db| db.set_run_status(run.id, "running", None))?;
    }
    let mut ctx = run.trigger["context"].clone();
    if !ctx.is_object() {
        ctx = json!({});
    }
    ctx["run"] = json!({ "id": run.id, "automation": run.automation });
    ctx["steps"] = json!({});
    for (idx, step) in steps.iter().enumerate() {
        let kind = automation::step_kind(step).unwrap_or("wait");
        let sid = automation::step_id(step, idx);
        let state = app.with_server(|db| db.ensure_step(run.id, idx as i64, &sid, kind))?;
        match state.status.as_str() {
            "succeeded" | "skipped" => {
                ctx["steps"][&sid] = json!({ "status": state.status, "output": state.output });
                continue;
            }
            "failed" => {
                ctx["steps"][&sid] = json!({ "status": "failed", "error": state.error });
                if step["onError"] == json!("continue") {
                    continue;
                }
                app.with_server(|db| {
                    db.set_run_status(run.id, "failed", Some(&format!("step {sid}: {}", state.error.clone().unwrap_or_default())))
                })?;
                plan.again = true;
                return Ok(());
            }
            _ => {}
        }
        if state.status == "pending" && step.get("if").is_some() && !automation::truthy(&automation::render(&step["if"], &ctx)) {
            app.with_server(|db| db.update_step(state.id, "skipped", None, None, None, None))?;
            ctx["steps"][&sid] = json!({ "status": "skipped" });
            continue;
        }
        let input = automation::render(&step[kind], &ctx);
        let result = if state.status == "waiting" { poll(app, run, step, &state) } else { execute(app, run, step, kind, &input, &state) };
        match result {
            Ok(Outcome::Done(output)) => {
                app.with_server(|db| db.update_step(state.id, "succeeded", Some(&input), Some(&output), None, None))?;
                ctx["steps"][&sid] = json!({ "status": "succeeded", "output": output });
            }
            Ok(Outcome::Wait(wait)) => {
                // Still waiting for the same thing: nothing to write, only its deadline to keep.
                if state.status != "waiting" {
                    app.with_server(|db| {
                        db.update_step(state.id, "waiting", Some(&input), None, Some(&wait), None)?;
                        db.set_run_status(run.id, "waiting", None)
                    })?;
                }
                plan.due_wait(&wait);
                return Ok(());
            }
            Err(e) => {
                let msg = e.to_string();
                let retries = step["retry"].as_i64().unwrap_or(0);
                if state.status != "waiting" && state.attempt < retries {
                    app.with_server(|db| db.update_step(state.id, "pending", Some(&input), None, None, Some(&msg)))?;
                    plan.due(Utc::now() + RETRY_AFTER);
                    return Ok(());
                }
                app.with_server(|db| db.update_step(state.id, "failed", Some(&input), None, None, Some(&msg)))?;
                ctx["steps"][&sid] = json!({ "status": "failed", "error": msg });
                if step["onError"] == json!("continue") {
                    continue;
                }
                app.with_server(|db| db.set_run_status(run.id, "failed", Some(&format!("step {sid}: {msg}"))))?;
                plan.again = true;
                return Ok(());
            }
        }
    }
    app.with_server(|db| db.set_run_status(run.id, "succeeded", None))?;
    plan.again = true;
    Ok(())
}

fn invalid(msg: impl Into<String>) -> AppError {
    AppError::Genie(GenieError::invalid(msg))
}

fn task_of(input: &Value, ctx_run: &Run) -> Option<String> {
    input["task"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| ctx_run.trigger["context"]["event"]["task"]["id"].as_str().map(str::to_string))
        .or_else(|| ctx_run.trigger["context"]["task"]["id"].as_str().map(str::to_string))
}

fn strings(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => {
            a.iter().filter_map(|x| x.as_str().map(str::to_string).or_else(|| (!x.is_null()).then(|| x.to_string()))).collect()
        }
        Value::String(s) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn execute(app: &App, run: &Run, step: &Value, kind: &str, input: &Value, state: &StepState) -> AppResult<Outcome> {
    app.with_server(|db| db.update_step(state.id, "running", Some(input), None, None, None))?;
    let project = run.project.as_str();
    let actor = automation_actor(run);
    let caller = Caller::automation(project, actor.clone());
    let need_task = || task_of(input, run).ok_or_else(|| invalid("no task: pass `task` or trigger on a task event"));
    match kind {
        "task.status" => {
            let task = need_task()?;
            let to = input["to"].as_str().unwrap_or_default().parse().map_err(AppError::Genie)?;
            let note = input["note"].as_str().map(str::to_string);
            let body = StatusBody { force: Some(input["force"] == json!(true)), ..StatusBody::to(to, note) };
            let t = tasks::set_status(app, &caller, &task, body)?;
            Ok(Outcome::Done(json!({ "task": t.id, "status": t.status })))
        }
        "task.comment" => {
            let task = need_task()?;
            let t = tasks::comment(app, &caller, &task, tasks::from_step(input, &[])?)?;
            Ok(Outcome::Done(json!({ "task": t.id })))
        }
        "task.create" => {
            let t = tasks::create(app, &caller, tasks::from_step(input, &[])?)?;
            Ok(Outcome::Done(json!({ "id": t.id })))
        }
        "task.update" => {
            let task = need_task()?;
            let t = tasks::update(app, &caller, &task, tasks::from_step(input, &[])?)?;
            Ok(Outcome::Done(json!({ "task": t.id })))
        }
        "task.get" => {
            let task = need_task()?;
            let t = app.with_tracker(project, |t| t.get(&task))?;
            Ok(Outcome::Done(serde_json::to_value(t).unwrap_or(Value::Null)))
        }
        "task.ready" => {
            // The task itself, and the tasks that wait for it: closing a dependency frees them.
            let task = need_task()?;
            let mut candidates = vec![task.clone()];
            candidates.extend(app.with_tracker(project, |t| t.dependents(&task))?);
            let (mut moved, mut held) = (Vec::new(), Vec::new());
            for id in candidates {
                let t = app.with_tracker(project, |t| t.get(&id))?;
                if !READY_FROM.contains(&t.status) {
                    continue;
                }
                let holds = holds(app, project, &t)?;
                if !holds.is_empty() {
                    held.push(json!({ "task": t.id, "holds": holds }));
                    continue;
                }
                let note =
                    "Nothing holds it: no block, dependencies done, no open questions or jobs; moved to ready automatically".to_string();
                tasks::set_status(app, &caller, &t.id, StatusBody::to(Status::Ready, Some(note)))?;
                moved.push(t.id);
            }
            Ok(Outcome::Done(json!({ "moved": moved, "held": held })))
        }
        "notify" => {
            let ctx = &run.trigger["context"];
            let users = notify::resolve(app, project, &strings(&input["to"]), ctx)?;
            let task = task_of(input, run);
            let msg = Message {
                kind: input["kind"].as_str().unwrap_or("automation").into(),
                title: input["title"]
                    .as_str()
                    .unwrap_or(&format!("genie · {}", run.trigger["spec"]["name"].as_str().unwrap_or("automation")))
                    .into(),
                body: input["text"].as_str().unwrap_or_default().into(),
                project: Some(project.into()),
                link: input["link"].as_str().map(str::to_string).or_else(|| task.as_ref().map(|t| format!("/active?task={t}"))),
                task,
                channels: strings(&input["channels"]).into_iter().filter(|c| c != "preferred").collect(),
                ..Default::default()
            };
            let n = notify::send(app, &users, &msg, Some(&format!("run:{}:{}", run.id, state.idx)))?;
            Ok(Outcome::Done(json!({ "recipients": users.len(), "queued": n })))
        }
        "agent" => {
            let role = input["role"].as_str().unwrap_or("analyst").to_string();
            let def = app.agents().role_for(project, &role)?.clone();
            if def.class == Role::Orchestrator {
                return Err(invalid("agent: the orchestrator does not run one-shot jobs"));
            }
            let goal = input["goal"].as_str().unwrap_or_default().to_string();
            let task = task_of(input, run);
            let initiator = crate::llm_key::automation_initiator(app, run, task.as_deref());
            let job = app.with_server(|db| {
                db.create_job(NewJob {
                    project: project.into(),
                    task,
                    run_step: Some(state.id),
                    role,
                    model: input["model"].as_str().map(str::to_string),
                    goal,
                    inputs: input["inputs"].clone(),
                    output_schema: input.get("output").cloned().filter(|v| !v.is_null()),
                    workspace: input["workspace"].as_str().unwrap_or("none").into(),
                    initiator,
                })
            })?;
            app.wake_runtime.notify_one();
            let deadline = Utc::now() + duration_or(step, "timeout", chrono::Duration::minutes(60));
            Ok(Outcome::Wait(json!({ "job": job.id, "deadline": deadline.to_rfc3339() })))
        }
        "team" => {
            let task = need_task()?;
            let req = SpawnRequest {
                task: task.clone(),
                template: input["template"].as_str().map(str::to_string),
                members: serde_json::from_value(input["members"].clone()).unwrap_or_default(),
                models: serde_json::from_value(input["models"].clone()).unwrap_or_default(),
                note: input["note"].as_str().map(str::to_string),
                by: actor.clone(),
                initiator: crate::llm_key::automation_initiator(app, run, Some(&task)),
            };
            let team = runtime::spawn_team(app, project, req)?;
            let wait_for = strings(&input["waitFor"]);
            if wait_for.is_empty() {
                return Ok(Outcome::Done(json!({ "team": team.id })));
            }
            let deadline = Utc::now() + duration_or(step, "timeout", chrono::Duration::hours(24));
            Ok(Outcome::Wait(json!({ "team": team.id, "task": task, "statuses": wait_for, "deadline": deadline.to_rfc3339() })))
        }
        "ask" => {
            let ctx = &run.trigger["context"];
            let users = notify::resolve(app, project, &strings(&input["to"]), ctx)?;
            let Some(&recipient) = users.first() else { return Err(invalid("ask: nobody to ask (the person has no account?)")) };
            let questions: Vec<NewQuestion> = input["questions"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .filter_map(|q| match q {
                    Value::String(s) => Some(NewQuestion { text: s.clone(), ..Default::default() }),
                    q => q["text"].as_str().map(|t| NewQuestion {
                        text: t.into(),
                        why: q["why"].as_str().unwrap_or_default().into(),
                        options: strings(&q["options"]),
                    }),
                })
                .collect();
            if questions.is_empty() {
                return Ok(Outcome::Done(json!({ "answers": [], "asked": false })));
            }
            let task = task_of(input, run);
            let id = crate::questions::ask(
                app,
                crate::questions::Ask {
                    project,
                    task: task.as_deref(),
                    asked_by: input["from"].as_str().unwrap_or("genie"),
                    recipient,
                    questions,
                    run_step: Some(state.id),
                    remind_after: input["remindAfter"].as_str().and_then(automation::parse_duration),
                    timeout: input["timeout"].as_str().and_then(automation::parse_duration),
                },
            )?;
            Ok(Outcome::Wait(json!({ "questionnaire": id })))
        }
        "wake_orchestrator" => {
            let text = input["text"].as_str().unwrap_or_default().to_string();
            let task = task_of(input, run);
            app.with_tracker(project, |t| t.bus().notify_orchestrator(&actor.name, "system", "system", &text, task.as_deref()))?;
            app.wake_runtime.notify_one();
            Ok(Outcome::Done(json!({})))
        }
        "changelog.add" => {
            let text = input["text"].as_str().unwrap_or_default().trim().to_string();
            if text.is_empty() {
                return Ok(Outcome::Done(json!({ "added": false })));
            }
            let path = crate::knowledge::changelog_add(
                app,
                project,
                input["group"].as_str().unwrap_or("changed"),
                &text,
                task_of(input, run).as_deref(),
            )?;
            Ok(Outcome::Done(json!({ "added": true, "path": path })))
        }
        "release" => {
            let version = input["version"].as_str().unwrap_or_default().to_string();
            let notes = crate::knowledge::release(app, project, &version, &actor.name)?;
            Ok(Outcome::Done(json!({ "version": version, "notes": notes })))
        }
        "http" => {
            let url = input["url"].as_str().unwrap_or_default().to_string();
            if !(url.starts_with("http://") || url.starts_with("https://")) {
                return Err(invalid("http: url must be http(s)"));
            }
            let method = input["method"].as_str().unwrap_or("POST").to_uppercase();
            let body = input["body"].clone();
            let headers = input["headers"].clone();
            let (status, text) = crate::state::block_on(async {
                let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
                let mut req = client.request(method.parse().map_err(|_| "bad method".to_string())?, &url);
                if let Some(h) = headers.as_object() {
                    for (k, v) in h {
                        req = req.header(k, v.as_str().unwrap_or_default());
                    }
                }
                if !body.is_null() {
                    req = req.json(&body);
                }
                let res = req.send().await.map_err(|e| e.to_string())?;
                let status = res.status().as_u16();
                Ok::<_, String>((status, res.text().await.unwrap_or_default()))
            })
            .map_err(invalid)?;
            if status >= 400 {
                return Err(invalid(format!("http: {status}")));
            }
            let parsed: Value = serde_json::from_str(&text).unwrap_or(Value::String(text.chars().take(4000).collect()));
            Ok(Outcome::Done(json!({ "status": status, "body": parsed })))
        }
        "wait" => {
            let d =
                input["for"].as_str().and_then(automation::parse_duration).ok_or_else(|| invalid("wait: `for` duration such as 24h"))?;
            Ok(Outcome::Wait(json!({ "until": (Utc::now() + d).to_rfc3339() })))
        }
        other => Err(invalid(format!("unknown step kind {other}"))),
    }
}

/// Statuses `task.ready` moves a task from: refinement that nothing else waits for.
const READY_FROM: &[Status] = &[Status::Draft, Status::Refining];

/// What keeps a task from being ready: a block, open dependencies, a decision or
/// answers it waits for, work still going on it, the Definition of Ready.
fn holds(app: &App, project: &str, task: &genie_core::Task) -> AppResult<Vec<String>> {
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

fn past(ts: &Value) -> bool {
    ts.as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).is_some_and(|t| t.with_timezone(&Utc) <= Utc::now())
}

fn poll(app: &App, run: &Run, step: &Value, state: &StepState) -> AppResult<Outcome> {
    let wait = state.wait.clone().unwrap_or(Value::Null);
    if let Some(job) = wait["job"].as_i64() {
        let j = app.with_server(|db| db.job(job))?;
        return match j.status.as_str() {
            "succeeded" => Ok(Outcome::Done(j.output.unwrap_or(json!({})))),
            "failed" | "cancelled" => Err(invalid(format!("agent job {job} {}: {}", j.status, j.error.unwrap_or_default()))),
            _ if past(&wait["deadline"]) => {
                app.with_server(|db| db.cancel_job(job))?;
                Err(invalid(format!("agent job {job} timed out")))
            }
            _ => Ok(Outcome::Wait(wait)),
        };
    }
    if let Some(id) = wait["questionnaire"].as_i64() {
        let qn = app.with_server(|db| db.questionnaire(id))?;
        return match qn.status.as_str() {
            "answered" => Ok(Outcome::Done(json!({ "answers": genie_core::inbox::answers_json(&qn), "asked": true }))),
            "expired" => {
                let input = automation::render(&step["ask"], &run.trigger["context"]);
                match input["onTimeout"].as_str().unwrap_or("needs_owner") {
                    "fail" => Err(invalid(format!("questionnaire {id} expired unanswered"))),
                    "continue" => Ok(Outcome::Done(json!({ "answers": genie_core::inbox::answers_json(&qn), "expired": true }))),
                    _ => {
                        if let Some(task) = &qn.task {
                            let open: Vec<String> = qn.questions.iter().filter(|q| q.answer.is_none()).map(|q| q.text.clone()).collect();
                            let note = format!("Нет ответа на вопросы: {}", open.join("; "));
                            let caller = Caller::automation(&run.project, automation_actor(run));
                            let _ = tasks::set_status(app, &caller, task, StatusBody::to(Status::NeedsOwner, Some(note)));
                        }
                        Ok(Outcome::Done(json!({ "answers": genie_core::inbox::answers_json(&qn), "expired": true })))
                    }
                }
            }
            _ => Ok(Outcome::Wait(wait)),
        };
    }
    if let Some(task) = wait["task"].as_str() {
        let statuses = strings(&wait["statuses"]);
        let t = app.with_tracker(&run.project, |t| t.get(task))?;
        if statuses.iter().any(|s| s == t.status.as_str()) {
            return Ok(Outcome::Done(json!({ "team": wait["team"], "status": t.status })));
        }
        if past(&wait["deadline"]) {
            return Err(invalid(format!("{task} did not reach {} in time", statuses.join("/"))));
        }
        return Ok(Outcome::Wait(wait));
    }
    if !wait["until"].is_null() {
        return Ok(if past(&wait["until"]) { Outcome::Done(json!({})) } else { Outcome::Wait(wait) });
    }
    Err(invalid("the step waits for nothing"))
}

// --- team flow ------------------------------------------------------------------------

/// Handoffs bound to a status (`on` in a team's relations): when a task enters
/// the status, genie tells the members the relation names, so the flow does
/// not depend on an agent remembering to write.
fn flow_intake(app: &App, project: &str) -> AppResult<()> {
    use crate::agent_config::{RelKind, TeamSpec};
    let cursor = app.with_tracker(project, |t| t.event_cursor(FLOW_CURSOR))?;
    let events = app.with_tracker(project, |t| t.events_after(cursor, 200))?;
    let Some(last) = events.last().map(|e| e.id) else { return Ok(()) };
    let mut sent = false;
    for e in events.iter().filter(|e| e.kind == "task.status_changed") {
        let (Some(task_id), Some(to)) = (e.subject.as_deref(), e.payload["to"].as_str().and_then(|s| s.parse::<Status>().ok())) else {
            continue;
        };
        let note = e.payload["note"].as_str().map(str::trim).filter(|n| !n.is_empty()).map(|n| format!(": “{n}”")).unwrap_or_default();
        sent |= app.with_tracker(project, |t| {
            let Some(team_id) = t.get(task_id)?.team else { return Ok(false) };
            let team = t.bus().get(&team_id)?;
            // Teams assembled before relations were stored keep their old kickoffs.
            let Some(spec) = team.spec.as_ref().and_then(TeamSpec::from_value) else { return Ok(false) };
            if team.state != genie_core::TeamState::Active || e.at < team.created {
                return Ok(false);
            }
            let actor = spec.by_name(&e.actor).map(|m| genie_core::team::display_name(&m.name)).unwrap_or_else(|| e.actor.clone());
            let mut any = false;
            for r in spec.relations.iter().filter(|r| r.on == Some(to)) {
                let from = spec.by_key(&r.from).map(|m| genie_core::team::display_name(&m.name)).unwrap_or_else(|| r.from.clone());
                let what = r.note.as_deref().map(|n| format!(" ({n})")).unwrap_or_default();
                for key in &r.to {
                    let Some(m) = spec.by_key(key) else { continue };
                    let active = team.members.iter().any(|x| x.name == m.name && x.state == genie_core::MemberState::Active);
                    if !active || m.name == e.actor {
                        continue;
                    }
                    let text = match r.kind {
                        RelKind::Returns => format!(
                            "{actor} moved {task_id} to {to}{note}. {from} returns the work to you{what}: read the findings (the latest review or test-report artifact and the comments), fix them and hand the work over again."
                        ),
                        _ => format!("{actor} moved {task_id} to {to}{note}. {from} hands over to you{what}: your step starts now."),
                    };
                    t.bus().send(genie_core::team::SendMail {
                        team: &team_id,
                        from: "genie",
                        from_role: "system",
                        to: &m.name,
                        text: &text,
                        level: Some("normal"),
                        intent: None,
                        kind: "system",
                        ..Default::default()
                    })?;
                    any = true;
                }
            }
            Ok(any)
        })?;
    }
    app.with_tracker(project, |t| t.ack_events(FLOW_CURSOR, last))?;
    if sent {
        app.wake_runtime.notify_one();
    }
    Ok(())
}

// --- built-in notifications ------------------------------------------------------------

fn notify_intake(app: &App, project: &str) -> AppResult<()> {
    let cursor = app.with_tracker(project, |t| t.event_cursor(NOTIFY_CURSOR))?;
    let events = app.with_tracker(project, |t| t.events_after(cursor, 200))?;
    let Some(last) = events.last().map(|e| e.id) else { return Ok(()) };
    for e in &events {
        let to = e.payload["to"].as_str().unwrap_or_default();
        let task_id = e.subject.clone();
        let title_of = |id: &str| app.with_tracker(project, |t| Ok(t.get(id)?.title)).unwrap_or_default();
        let ctx = json!({ "event": { "actor": e.actor, "task": { "id": task_id } } });
        let (specs, msg): (Vec<&str>, Option<Message>) = match (e.kind.as_str(), to) {
            ("task.status_changed", "needs_owner") if e.actor_role != "human" => {
                let id = task_id.clone().unwrap_or_default();
                // The person responsible decides, with the author; without one, the admins.
                let responsible = app.with_tracker(project, |t| Ok(t.get(&id)?.assignee)).ok().flatten().is_some();
                (
                    if responsible { vec!["task.assignee", "task.author"] } else { vec!["task.author", "project.admins"] },
                    Some(Message {
                        kind: "needs_owner".into(),
                        title: format!("{id} ждёт вашего решения"),
                        body: format!("{}\n\n{}", title_of(&id), e.payload["note"].as_str().unwrap_or_default()),
                        link: Some(format!("/decisions?task={id}")),
                        ..Default::default()
                    }),
                )
            }
            ("task.status_changed", "done") if e.actor_role != "human" => {
                let id = task_id.clone().unwrap_or_default();
                (
                    vec!["task.author"],
                    Some(Message {
                        kind: "done".into(),
                        title: format!("{id} готова",),
                        body: format!("{}{}", title_of(&id), e.payload["note"].as_str().map(|n| format!("\n\n{n}")).unwrap_or_default()),
                        link: Some(format!("/done?task={id}")),
                        ..Default::default()
                    }),
                )
            }
            ("doc.proposal", _) => {
                let path = e.payload["path"].as_str().unwrap_or_default().to_string();
                let owners = app.with_vault(|v| Ok(v.owners_for(&path))).unwrap_or_default();
                let mut specs: Vec<&str> = vec!["project.admins"];
                let owners_ref: Vec<String> = owners.iter().map(|o| format!("@{o}")).collect();
                let users = notify::resolve(app, project, &owners_ref, &ctx)?;
                let m = Message {
                    kind: "proposal".into(),
                    title: format!("Предложение правки: {path}"),
                    body: format!("{} предлагает изменить страницу базы знаний.", e.actor),
                    link: Some(format!("/docs?proposal={}", e.payload["proposal"])),
                    project: Some(project.into()),
                    ..Default::default()
                };
                if !users.is_empty() {
                    notify::send(app, &users, &m, Some(&format!("ev:{project}:{}", e.id)))?;
                    specs.clear();
                }
                (specs, Some(m))
            }
            ("mail.sent", _) if e.payload["kind"] == json!("system") && e.actor == "genie" && e.payload["to"] == json!("orchestrator") => {
                (vec![], None)
            }
            _ => (vec![], None),
        };
        if let Some(mut m) = msg
            && !specs.is_empty()
        {
            m.project = Some(project.into());
            m.task = task_id.clone();
            let users = notify::resolve(app, project, &specs.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &ctx)?;
            notify::send(app, &users, &m, Some(&format!("ev:{project}:{}", e.id)))?;
        }
    }
    app.with_tracker(project, |t| t.ack_events(NOTIFY_CURSOR, last))?;
    Ok(())
}

// --- playbooks ------------------------------------------------------------------------------

/// Ready-made rules. `{{ … }}` inside are evaluated at run time.
pub fn playbooks() -> Vec<(&'static str, &'static str, Value)> {
    vec![
        (
            "task-done-knowledge",
            "Задача закрыта → знания, чейнджлог и уведомление",
            json!({
                "name": "Задача закрыта — знания и чейнджлог",
                "on": { "event": "task.status_changed", "where": { "to": "done", "task.type": ["task", "bug", "spike"], "task.labels": { "not_contains": "no-docs" } } },
                "limits": { "concurrency": 2, "maxRunsPerHour": 20 },
                "steps": [
                    { "id": "docs", "timeout": "45m", "agent": {
                        "role": "documenter",
                        "workspace": "read-only",
                        "goal": "Task {{ event.task.id }} ({{ event.task.title }}) is done. Turn its artifacts, decisions and review into durable knowledge of this project's vault space: update existing pages (search them first with `genie docs search`), add new pages only for genuinely new topics, link back to the task, set `verified` to today, and write pages with `genie docs write` (the section policy may turn them into proposals). Then write one changelog line for people who use the product.",
                        "output": { "pages_changed": ["path"], "summary": "one paragraph for the owner", "changelog": { "group": "added | changed | fixed", "text": "one line, user-facing, in the owner's language" } }
                    } },
                    { "id": "log", "changelog.add": { "group": "{{ steps.docs.output.changelog.group }}", "text": "{{ steps.docs.output.changelog.text }}" } },
                    { "id": "tell", "notify": {
                        "to": ["task.author", "project.admins"],
                        "title": "{{ event.task.id }} готова: знания обновлены",
                        "text": "{{ steps.docs.output.summary }}\n\nСтраницы:\n{{ steps.docs.output.pages_changed | lines }}\n\nЧейнджлог: {{ steps.docs.output.changelog.text }}"
                    } }
                ]
            }),
        ),
        (
            "new-task-triage",
            "Новая задача от человека → аналитик → вопросы автору",
            json!({
                "name": "Разбор новой задачи",
                "on": { "event": "task.created", "where": { "status": "inbox", "actor.role": "human" } },
                "limits": { "concurrency": 3, "maxRunsPerHour": 30 },
                "steps": [
                    { "id": "take", "task.status": { "to": "refining", "note": "Взята в разбор автоматически: аналитик готовит вопросы автору" } },
                    { "id": "analyse", "timeout": "45m", "agent": {
                        "role": "analyst",
                        "workspace": "read-only",
                        "goal": "Analyse task {{ event.task.id }}: study the project (code if any, knowledge base with `genie docs search`), draft scope and verifiable acceptance criteria, and list only the questions that the task's author must answer (product decisions, missing facts). Do not guess product answers. Write the questions in the owner's language, offer options when the answer is a choice. Save your analysis as an `analysis` artifact on the task.",
                        "output": { "questions": [{ "text": "question", "why": "why it matters", "options": ["optional choices"] }], "draft_acceptance": ["criterion"] }
                    } },
                    { "id": "criteria", "if": "{{ steps.analyse.output.draft_acceptance | length }}", "task.comment": { "kind": "decision", "text": "Черновик критериев приёмки от аналитика:\n{{ steps.analyse.output.draft_acceptance | lines }}" } },
                    { "id": "ask", "if": "{{ steps.analyse.output.questions | length }}", "ask": {
                        "to": ["event.actor"],
                        "from": "аналитика",
                        "questions": "{{ steps.analyse.output.questions }}",
                        "remindAfter": "24h",
                        "timeout": "72h",
                        "onTimeout": "needs_owner"
                    } },
                    { "id": "continue", "wake_orchestrator": { "text": "Triage of {{ event.task.id }} is complete: the analysis artifact and draft criteria are on the task, the author's answers are in its comments. Finish refinement (description, acceptance criteria), then move it to ready if the Definition of Ready is met." } }
                ]
            }),
        ),
        (
            "auto-ready",
            "Задачу ничего не держит → «Готово к работе»",
            json!({
                "name": "Задачу ничего не держит — в «Готово к работе»",
                "on": { "event": "task.*", "where": { "task.status": ["draft", "refining", "done"] } },
                "limits": { "concurrency": 1, "maxRunsPerHour": 200 },
                "steps": [
                    { "id": "ready", "task.ready": {} }
                ]
            }),
        ),
        (
            "ready-start",
            "«Готово к работе» → оркестратор запускает работу",
            json!({
                "name": "«Готово к работе» — оркестратор запускает работу",
                "on": { "event": "task.status_changed", "where": { "to": "ready", "task.type": { "not": "epic" } } },
                "limits": { "concurrency": 3, "maxRunsPerHour": 60 },
                "steps": [
                    { "id": "start", "wake_orchestrator": { "text": "Task {{ event.task.id }} ({{ event.task.title }}) is now ready: start work on it. If a team already works on it, do nothing. Check the Definition of Ready first: if something only the owner can settle is missing (the integration, the repositories), ask the owner instead of dispatching. Otherwise compose its team and spawn it now. If the server refuses the spawn because a limit is reached (active teams per project or per epic), do not retry and do not force it: the task stays in ready and starts when a team stops." } }
                ]
            }),
        ),
        (
            "ready-next",
            "Команда остановилась → оркестратор берёт следующую готовую задачу",
            json!({
                "name": "Команда остановилась — следующая готовая задача",
                "on": { "event": "team.stopped" },
                "limits": { "concurrency": 1, "maxRunsPerHour": 60 },
                "steps": [
                    { "id": "next", "wake_orchestrator": { "text": "A team stopped, so there may be room for work. List the tasks that wait in `ready` without a team (`genie task list --status ready`), take them in order of priority and start work on as many as the limits allow: spawn a team for each. If the server refuses a spawn because a limit is reached, stop there: the rest stay in ready. If nothing waits, do nothing." } }
                ]
            }),
        ),
        (
            "needs-owner-escalation",
            "Решение не принято за сутки → напоминание",
            json!({
                "name": "Эскалация решения",
                "on": { "event": "task.status_changed", "where": { "to": "needs_owner" } },
                "steps": [
                    { "id": "pause", "wait": { "for": "24h" } },
                    { "id": "now", "task.get": {} },
                    { "id": "remind", "if": "{{ steps.now.output.needsOwner }}", "notify": {
                        "to": ["task.author", "project.owners"],
                        "title": "{{ event.task.id }} ждёт решения больше суток",
                        "text": "{{ steps.now.output.needsOwner.question }}"
                    } }
                ]
            }),
        ),
    ]
}
