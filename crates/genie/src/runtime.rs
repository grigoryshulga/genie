//! Agent runtime: teams, the server-side orchestrator and one-shot jobs, run as
//! *turns*.
//!
//! Every agent is a mailbox plus a harness session. When a mailbox has unread
//! mail (or a job is queued) and a slot is free, the scheduler starts a turn:
//! the mail is leased to the turn, the harness runs once (by default
//! `pi --print` resuming the agent's session), and the mail is marked delivered
//! only if the turn succeeds. A failed turn releases the lease and is retried
//! with exponential backoff; after `maxAttempts` failures the member is put in
//! `error` and the orchestrator is told. After a restart, running turns are
//! marked interrupted and their mail is offered again — nothing is lost and no
//! long-running process has to be babysat. A live session's mail was already
//! acknowledged in the lost step, so offering it again reaches nobody: there the
//! agent is told to continue with a note (`sessions::resume_after_restart`).
//!
//! Agents act through the `genie` command line (HTTP with a per-turn token bound
//! to the project, the team and the role), so any harness with a shell can take part.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use genie_core::server_db::Project;
use genie_core::team::{self, Mail, NewMember, NewTeam, ORCHESTRATOR, TeamWorktree};
use genie_core::work::Job;
use genie_core::{Actor, CLOSED, Capability, GenieError, Role, Status, StatusOptions, Task, TeamState};
use serde_json::json;
use tokio::io::AsyncReadExt;
use tokio::sync::Semaphore;

use crate::agent_config::{AgentConfig, FileAccess, MailMode, RelKind, Relation, RoleDef, SpecMember, Stage, TeamSpec, Workspace};
use crate::config::MemberSpec;
use crate::ops::{self, AgentKind};
use crate::outcome::Outcome;
use crate::sandbox;
use crate::state::{App, AppError, AppResult, SAFETY_NET};

/// Which agent a turn is for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AgentKey {
    Orchestrator { project: String },
    Member { project: String, team: String, member: String },
    Job { project: String, job: i64 },
}

impl AgentKey {
    pub fn project(&self) -> &str {
        match self {
            AgentKey::Orchestrator { project } | AgentKey::Member { project, .. } | AgentKey::Job { project, .. } => project,
        }
    }
    pub fn label(&self) -> String {
        match self {
            AgentKey::Orchestrator { .. } => ORCHESTRATOR.into(),
            AgentKey::Member { team, member, .. } => format!("{team}/{member}"),
            AgentKey::Job { job, .. } => format!("job/{job}"),
        }
    }
}

/// The scheduler's own state: whose turn is running, and who waits after failures.
#[derive(Default)]
pub struct Sched(Mutex<SchedState>);

#[derive(Default)]
struct SchedState {
    running: HashSet<AgentKey>,
}

impl Sched {
    fn with<T>(&self, f: impl FnOnce(&mut SchedState) -> T) -> T {
        f(&mut self.0.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// Start background workers: crash recovery, then the scheduler.
pub fn start(app: &Arc<App>) {
    crate::knowledge::start_watcher(app);
    crate::vault_sync::start(app);
    crate::agent_config::start_watcher(app);
    crate::engine::start(app);
    crate::channels::start(app);
    if let Err(e) = recover(app) {
        eprintln!("genie runtime: recovery failed: {e}");
    }
    crate::sessions::recover(app);
    // Closed tasks from an earlier run may still hold worktrees (and their build
    // directories) — free what the main branch already has.
    for p in app.with_server(|db| db.projects()).unwrap_or_default() {
        for line in sweep_worktrees(app, &p.slug) {
            println!("genie runtime: {}: {line}", p.slug);
        }
    }
    let gateway = app.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            gateway.mcp.close_idle(crate::mcp_gateway::IDLE);
        }
    });
    if app.cfg.runtime.live_sessions()
        && let Err(e) = crate::sessions::write_extension(app)
    {
        eprintln!("genie runtime: cannot write the session extension: {e}");
    }
    if !app.cfg.runtime.enabled {
        println!("genie runtime: agents disabled");
        return;
    }
    let app = app.clone();
    tokio::spawn(async move {
        let slots = Arc::new(Semaphore::new(app.cfg.runtime.max_concurrent.max(1)));
        loop {
            let next = match schedule(&app, &slots).await {
                Ok(next) => next,
                Err(e) => {
                    eprintln!("genie runtime: {e}");
                    Next(Some(Duration::from_secs(5)))
                }
            };
            tokio::select! {
                _ = app.wake_runtime.notified() => {}
                _ = tokio::time::sleep(next.0.map_or(SAFETY_NET, |d| d.min(SAFETY_NET)).max(Duration::from_millis(50))) => {}
            }
        }
    });
}

/// When the scheduler has to look again without being woken: the soonest of the deadlines it
/// meets in a pass (a backoff, a nudge to repeat, an idle session to stop, a console that lapses).
/// New mail, jobs, finished turns and changed teams wake it instead (`App::wake_runtime`).
#[derive(Default)]
pub(crate) struct Next(Option<Duration>);

impl Next {
    pub(crate) fn within(&mut self, d: Duration) {
        self.0 = Some(self.0.map_or(d, |x| x.min(d)));
    }
}

/// After a restart: interrupted turns give their mail back, running jobs are requeued,
/// and agents whose live session was lost mid-step are told to continue.
pub fn recover(app: &App) -> AppResult<()> {
    let interrupted = app.with_server(|db| {
        db.requeue_running_jobs()?;
        db.interrupt_running_turns()
    })?;
    for p in app.with_server(|db| db.projects())? {
        match app.with_tracker(&p.slug, |t| t.bus().release_all_leases()) {
            Ok(n) if n > 0 => println!("genie runtime: {}: {n} message(s) from interrupted turns offered again", p.slug),
            Err(e) => eprintln!("genie runtime: {}: {e}", p.slug),
            _ => {}
        }
    }
    if !interrupted.is_empty() {
        println!("genie runtime: {} turn(s) were interrupted by the restart", interrupted.len());
    }
    // An agent process may have outlived the server; its turn will run again, so
    // stop the stray one — only if /proc shows it is really that agent.
    for t in &interrupted {
        let Some(pid) = t.pid else { continue };
        let name = match (&t.member, t.job) {
            (Some(m), _) => m.clone(),
            (None, Some(j)) => format!("job-{j}"),
            _ => ORCHESTRATOR.to_string(),
        };
        if is_our_agent(pid, &t.project, &name) {
            let _ = std::process::Command::new("kill").arg("-TERM").arg(pid.to_string()).status();
            println!("genie runtime: stopped stray agent process {pid} ({}/{name})", t.project);
        }
    }
    // A live session's mail was acknowledged inside the lost step, so nothing is pending and the
    // scheduler would never start it again: give it the same note the agent-crash path writes.
    crate::sessions::resume_after_restart(app, &interrupted);
    Ok(())
}

/// Does `/proc/<pid>/environ` belong to this project's agent `name`?
pub(crate) fn is_our_agent(pid: i64, project: &str, name: &str) -> bool {
    let Ok(env) = std::fs::read(format!("/proc/{pid}/environ")) else { return false };
    let vars: Vec<&[u8]> = env.split(|b| *b == 0).collect();
    let has = |kv: String| vars.contains(&kv.as_bytes());
    has(format!("GENIE_PROJECT={project}")) && has(format!("GENIE_AGENT_NAME={name}"))
}

async fn schedule(app: &Arc<App>, slots: &Arc<Semaphore>) -> AppResult<Next> {
    let mut next = Next::default();
    let live = app.cfg.runtime.live_sessions();
    // Supervision first, so what it frees (stale deliveries, stopped sessions) is seen below.
    if live {
        crate::sessions::sweep(app, &mut next).await?;
    }
    // The silent-team watchdog rides every pass, in both `sessions` and `turns` mode. What it
    // watches for is quiet, so the safety net (a minute) bounds how late it may be; the last
    // activity of a team (mail, a turn, a team change) wakes the scheduler by itself.
    if let Err(e) = crate::sessions::watch_silent_teams(app).await {
        eprintln!("genie runtime: silent-team watchdog: {e}");
    }
    if app.cfg.runtime.stall_secs > 0 {
        next.within(Duration::from_secs((app.cfg.runtime.stall_secs / 4).max(1)));
    }
    let (candidates, console) = app
        .blocking(|app| {
            let mut out = Vec::new();
            let mut console = None::<Duration>;
            for p in app.with_server(|db| db.projects())? {
                let boxes = match app.with_tracker(&p.slug, |t| t.bus().mailboxes_with_mail()) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("genie runtime: {}: {e}", p.slug);
                        continue;
                    }
                };
                for b in boxes {
                    match b.team {
                        None if orchestrator_runs(app, &p) => out.push(AgentKey::Orchestrator { project: p.slug.clone() }),
                        // A person's console lapses by itself: the orchestrator takes its mail then.
                        None if p.autonomy != "manual" => {
                            let lapses = app.with_server(|db| db.console(&p.slug)).ok().flatten().and_then(|c| {
                                chrono::DateTime::parse_from_rfc3339(&c.until)
                                    .ok()
                                    .and_then(|u| (u.to_utc() - chrono::Utc::now()).to_std().ok())
                            });
                            console = [console, lapses].into_iter().flatten().min();
                        }
                        None => {}
                        Some(team) => out.push(AgentKey::Member { project: p.slug.clone(), team, member: b.recipient }),
                    }
                }
            }
            for j in app.with_server(|db| db.queued_jobs())? {
                out.push(AgentKey::Job { project: j.project.clone(), job: j.id });
            }
            Ok((out, console))
        })
        .await?;
    if let Some(d) = console {
        next.within(d + Duration::from_millis(50));
    }
    for key in candidates {
        if live && !matches!(key, AgentKey::Job { .. }) {
            if let Some(d) = crate::sessions::deliver(app, &key).await {
                next.within(d);
            }
            continue;
        }
        if !app.attempts.may_start(&key) {
            // Waiting out a backoff: nobody wakes the scheduler when it ends.
            if let Some(d) = app.attempts.wait_for(&key) {
                next.within(d + Duration::from_millis(50));
            }
            continue;
        }
        if app.sched.with(|s| s.running.contains(&key)) {
            continue;
        }
        let Ok(permit) = slots.clone().try_acquire_owned() else { break };
        app.sched.with(|s| s.running.insert(key.clone()));
        let app = app.clone();
        tokio::spawn(async move {
            run_turn(&app, &key).await;
            app.sched.with(|s| s.running.remove(&key));
            drop(permit);
            app.wake_runtime.notify_one();
            app.wake_engine.notify_one();
        });
    }
    Ok(next)
}

struct Prepared {
    spec: AgentSpec,
    turn: i64,
    /// The mail (or job) it takes, as its first message.
    message: String,
    /// Per-turn agent token, revoked when the turn ends.
    token: String,
    /// Its `LITELLM_API_KEY`: its initiator's ([`crate::llm_key`]).
    llm: crate::llm_key::Key,
}

/// Run one turn.
/// Run one turn; returns whether it succeeded.
async fn run_turn(app: &Arc<App>, key: &AgentKey) {
    let k = key.clone();
    let prepared = app.blocking(move |app| prepare(app, &k)).await;
    let mut p = match prepared {
        Ok(Some(p)) => p,
        Ok(None) => return,
        Err(e) => {
            let n = app.attempts.failed_to_start(key);
            eprintln!("genie runtime: {}: cannot start {} (attempt {n}): {e}", key.project(), key.label());
            return;
        }
    };
    let turn = p.turn;
    let ttl = chrono::Duration::seconds(app.cfg.runtime.turn_timeout_secs as i64 + 300);
    let (slug, spec) = (key.project().to_string(), p.spec.clone());
    match app.blocking(move |app| spec.token(app, &slug, ttl)).await {
        Ok(t) => p.token = t,
        Err(e) => {
            app.attempts.failed_to_start(key);
            eprintln!("genie runtime: turn {turn}: {e}");
            return;
        }
    }
    let outcome = execute(app, key, &p).await;
    let (ok, code, error, log) = match outcome {
        Ok((code, log)) => (code == Some(0), code, (code != Some(0)).then(|| format!("exit code {code:?}")), log),
        Err(e) => (false, None, Some(e), String::new()),
    };
    let k = key.clone();
    let res = app.blocking(move |app| finish(app, &k, &p, ok, code, error.as_deref(), &log)).await;
    if let Err(e) = res {
        eprintln!("genie runtime: turn {turn}: {e}");
    }
}

fn project_of(app: &App, slug: &str) -> AppResult<Project> {
    app.with_server(|db| db.project(slug))
}

/// Who an agent is for the repository rules.
fn who(def: &RoleDef, team: Option<&String>, job: Option<i64>) -> crate::git::service::AgentId {
    crate::git::service::AgentId { role: def.class, role_id: Some(def.id.clone()), team: team.cloned(), job }
}

/// Working directory for agents of a project without a specific workspace: a read-only
/// view of the project's repositories, else the project's local repository, else an empty directory.
fn project_workspace(app: &App, p: &Project, sub: &str) -> PathBuf {
    if let Some(view) = crate::git::service::view_workspace(app, &p.slug) {
        return view;
    }
    match &p.repo {
        Some(r) => PathBuf::from(r),
        None => {
            let d = app.data.join("workspaces").join(&p.slug).join(sub);
            let _ = std::fs::create_dir_all(&d);
            d
        }
    }
}

/// Where a team member works: the team's directory, or an empty one of its own
/// when its role does not work with files (`files: none`).
fn member_workspace(app: &App, project: &Project, role: &RoleDef, team_cwd: &str, team: &str, member: &str) -> PathBuf {
    let cwd = PathBuf::from(team_cwd);
    if role.files == FileAccess::None {
        project_workspace_dir(app, &project.slug, &format!("{team}-{member}"))
    } else if cwd.is_dir() {
        cwd
    } else {
        project_workspace(app, project, team)
    }
}

/// `<data>/workspaces/<project>/<name>`, created on first use.
pub(crate) fn project_workspace_dir(app: &App, slug: &str, name: &str) -> PathBuf {
    let d = app.data.join("workspaces").join(slug).join(name);
    let _ = std::fs::create_dir_all(&d);
    d
}

/// Model and thinking: an explicit choice (member, job), then the role, then
/// `roleModels` by role id and by class.
fn role_model(app: &App, role: &RoleDef, model: Option<String>, thinking: Option<String>) -> (Option<String>, Option<String>) {
    let by_id = app.cfg.role_models.get(&role.id);
    let by_class = app.cfg.role_models.get(role.class.as_str());
    let model = model
        .or_else(|| role.model.clone())
        .or_else(|| by_id.and_then(|d| d.model.clone()))
        .or_else(|| by_class.and_then(|d| d.model.clone()));
    let thinking = thinking
        .or_else(|| role.thinking.clone())
        .or_else(|| by_id.and_then(|d| d.thinking.clone()))
        .or_else(|| by_class.and_then(|d| d.thinking.clone()));
    (model, thinking)
}

/// The role a running member or job acts in. A role removed from the configuration
/// is an error the orchestrator hears about, not a silent fallback.
fn running_role(agents: &AgentConfig, id: &str) -> AppResult<RoleDef> {
    agents.roles.get(id).cloned().ok_or_else(|| {
        GenieError::invalid(format!("role {id} is no longer in the agent configuration; restore it or replace the member")).into()
    })
}

fn orchestrator_role(agents: &AgentConfig) -> AppResult<RoleDef> {
    agents.orchestrator().cloned().ok_or_else(|| AppError::Internal("the orchestrator role is missing from the configuration".into()))
}

fn prepare(app: &App, key: &AgentKey) -> AppResult<Option<Prepared>> {
    let project = project_of(app, key.project())?;
    let label = key.label();
    match key {
        AgentKey::Orchestrator { project: slug } => {
            let turn = app.with_server(|db| db.start_turn(slug, &label, None, None, None))?;
            let mail = app.with_tracker(slug, |t| t.bus().lease(None, ORCHESTRATOR, turn))?;
            if mail.is_empty() {
                app.with_server(|db| db.finish_turn(turn, "skipped", None, None, None))?;
                return Ok(None);
            }
            app.with_server(|db| db.set_turn_mail(turn, &mail.iter().map(|m| m.id).collect::<Vec<_>>()))?;
            let spec = orchestrator_spec(app, &project, false, crate::llm_key::mail_initiator(app, slug, &mail))?;
            let llm = turn_key(app, slug, turn, &label, spec.initiator.as_deref(), spec.model.as_deref())?;
            Ok(Some(Prepared { spec, turn, message: orchestrator_message(&project, &mail), token: String::new(), llm }))
        }
        AgentKey::Member { project: slug, team, member } => {
            let t = app.with_tracker(slug, |t| t.bus().get(team))?;
            let Some(m) = t.members.iter().find(|m| &m.name == member).cloned() else { return Ok(None) };
            let spec = member_spec(app, &project, &t, &m, false)?;
            let turn = app.with_server(|db| db.start_turn(slug, &label, Some(team), Some(member), None))?;
            let mail = app.with_tracker(slug, |t| t.bus().lease(Some(team), member, turn))?;
            if mail.is_empty() {
                app.with_server(|db| db.finish_turn(turn, "skipped", None, None, None))?;
                return Ok(None);
            }
            app.with_server(|db| db.set_turn_mail(turn, &mail.iter().map(|m| m.id).collect::<Vec<_>>()))?;
            let llm = turn_key(app, slug, turn, &label, spec.initiator.as_deref(), spec.model.as_deref())?;
            app.with_tracker(slug, |t| t.bus().member_working(team, member, json!({ "kind": "turn", "turn": turn })))?;
            Ok(Some(Prepared { spec, turn, message: member_message(&mail), token: String::new(), llm }))
        }
        AgentKey::Job { project: slug, job } => {
            let j = app.with_server(|db| {
                let j = db.job(*job)?;
                if j.status != "queued" {
                    return Ok(None);
                }
                db.start_job(*job)?;
                Ok(Some(j))
            })?;
            let Some(j) = j else { return Ok(None) };
            let turn = app.with_server(|db| db.start_turn(slug, &label, None, None, Some(*job)))?;
            // A job that cannot start (its role removed, no worktree) fails this attempt instead of hanging.
            let prepared = prepare_job(app, &project, &j, turn);
            if let Err(e) = &prepared {
                let (error, max) = (e.to_string(), i64::from(app.cfg.runtime.max_attempts.max(1)));
                app.with_server(|db| {
                    db.finish_turn(turn, "failed", None, Some(&error), None)?;
                    db.finish_job(*job, false, Some(&error), max).map(|_| ())
                })?;
            }
            prepared.map(Some)
        }
    }
}

/// The key of a turn's agent; when it cannot have one, the turn ends without
/// running and its mail waits for the next attempt.
fn turn_key(app: &App, slug: &str, turn: i64, label: &str, initiator: Option<&str>, model: Option<&str>) -> AppResult<crate::llm_key::Key> {
    crate::llm_key::resolve(app, slug, label, initiator, model).map_err(|e| {
        let _ = app.with_tracker(slug, |t| t.bus().release_lease(turn));
        let _ = app.with_server(|db| db.finish_turn(turn, "failed", None, Some(&e), None));
        GenieError::invalid(e).into()
    })
}

fn prepare_job(app: &App, project: &Project, j: &Job, turn: i64) -> AppResult<Prepared> {
    let agents = app.agents();
    let def = running_role(&agents, &j.role)?;
    let (model, thinking) = role_model(app, &def, j.model.clone(), None);
    let initiator = j.initiator.clone().or_else(|| j.task.as_deref().and_then(|t| crate::llm_key::task_person(app, &project.slug, t)));
    let llm = crate::llm_key::resolve(app, &project.slug, &format!("job {}", j.id), initiator.as_deref(), model.as_deref())
        .map_err(GenieError::invalid)?;
    let place = job_workspace(app, project, j, def.files)?;
    let spec = AgentSpec {
        role: def.class,
        role_id: def.id.clone(),
        readonly: if place.files == FileAccess::Write { String::new() } else { "edit,write".into() },
        name: format!("job-{}", j.id),
        team: None,
        task: j.task.clone(),
        job: Some(j.id),
        kit: kit(app, &agents, &project.slug, &def, place.files, &place.cwd),
        cwd: place.cwd,
        // A job starts fresh on each attempt: no hidden state between retries.
        session_id: format!("{}-job-{}-{}", project.slug, j.id, j.attempts + 1),
        model,
        thinking,
        prompt: agent_prompt(app, &agents, project, &def, None, false, AgentKind::Job, &who(&def, None, Some(j.id))),
        initiator,
    };
    Ok(Prepared { spec, turn, message: job_message(j, &place.note), token: String::new(), llm })
}

/// Where a job works and what it may change there.
struct JobPlace {
    cwd: PathBuf,
    files: FileAccess,
    /// For the job's message.
    note: String,
}

/// A job's `workspace`: `read-only` — the repository without edit and write;
/// `worktree` — its own git worktree (kept across attempts); `scratch` and
/// `none` — an empty directory of the job, as for a role with `files: none`.
fn job_workspace(app: &App, project: &Project, j: &Job, role_files: FileAccess) -> AppResult<JobPlace> {
    let own_dir = || project_workspace_dir(app, &project.slug, &format!("job-{}", j.id));
    if role_files == FileAccess::None {
        let cwd = own_dir();
        return Ok(JobPlace {
            note: format!(
                "You work in `{}`, an empty directory of this job: your role does not work with the project's files.",
                cwd.display()
            ),
            cwd,
            files: role_files,
        });
    }
    if j.workspace == "worktree" && !app.with_server(|db| db.repos(&project.slug))?.is_empty() {
        let (root, placed) = crate::git::service::task_workspace(app, &project.slug, &format!("job-{}", j.id), j.task.as_deref())?;
        let list: Vec<String> = placed
            .iter()
            .map(|p| match &p.branch {
                Some(b) => format!("`{}` (branch `{b}`)", p.repo.mount),
                None => format!("`{}` (read-only use)", p.repo.mount),
            })
            .collect();
        return Ok(JobPlace {
            note: format!(
                "You work in `{}`, which holds the project's repositories: {}. Commit your changes in the branches named; the rules for pushing are in your instructions.",
                root.display(),
                list.join(", ")
            ),
            cwd: root,
            files: role_files,
        });
    }
    Ok(match (j.workspace.as_str(), &project.repo) {
        ("read-only", repo) => {
            let cwd = project_workspace(app, project, "jobs");
            let what = if repo.is_some() { "the project's repository" } else { "the project's directory" };
            JobPlace {
                note: format!("You work in {what} `{}` read-only: read, search and run checks, but do not change files.", cwd.display()),
                files: if role_files == FileAccess::Write { FileAccess::Read } else { role_files },
                cwd,
            }
        }
        ("worktree", Some(repo)) => {
            let w = job_worktree(app, Path::new(repo), j)?;
            JobPlace {
                note: format!(
                    "You work in your own git worktree `{}` on branch `{}`, made from the repository's HEAD. Commit your changes there: the branch is your result and stays after the job.",
                    w.path, w.branch
                ),
                cwd: PathBuf::from(w.path),
                files: role_files,
            }
        }
        _ => {
            let cwd = own_dir();
            JobPlace { note: format!("You work in `{}`, an empty directory of this job.", cwd.display()), cwd, files: role_files }
        }
    })
}

/// Launch the harness for a prepared turn and wait for it (with a timeout).
async fn execute(app: &Arc<App>, key: &AgentKey, p: &Prepared) -> Result<(Option<i32>, String), String> {
    let (mut cmd, dir) = agent_process(app, key, &p.spec, Mode::Turn { message: &p.message }, &p.token, &p.llm).await?;
    let program = cmd.as_std().get_program().to_string_lossy().into_owned();
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("cannot start {program}: {e}"))?;
    if let Some(pid) = child.id() {
        let turn = p.turn;
        let _ = app.blocking(move |app| app.with_server(|db| db.set_turn_pid(turn, pid))).await;
    }
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let read = async {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        if let Some(s) = stdout.as_mut() {
            let _ = s.read_to_end(&mut out).await;
        }
        if let Some(s) = stderr.as_mut() {
            let _ = s.read_to_end(&mut err).await;
        }
        (out, err)
    };
    let timeout = Duration::from_secs(app.cfg.runtime.turn_timeout_secs.max(1));
    let result = tokio::time::timeout(timeout, async {
        let (out, err) = read.await;
        let status = child.wait().await;
        (status, out, err)
    })
    .await;
    let (status, out, err) = match result {
        Ok(v) => v,
        Err(_) => return Err(format!("turn timed out after {}s", timeout.as_secs())),
    };
    let mut log = String::from_utf8_lossy(&out).into_owned();
    let err = String::from_utf8_lossy(&err);
    if !err.trim().is_empty() {
        log.push_str("\n--- stderr ---\n");
        log.push_str(&err);
    }
    let _ = tokio::fs::write(dir.join(format!("turn-{}.log", p.turn)), &log).await;
    let tail: String = log.chars().rev().take(4000).collect::<Vec<_>>().into_iter().rev().collect();
    Ok((status.map_err(|e| e.to_string())?.code(), tail))
}

/// How an agent process runs: one turn on its mail, or a live session fed over RPC.
pub(crate) enum Mode<'a> {
    Turn { message: &'a str },
    Session { extension: &'a Path },
}

/// The runtime directory of an agent: its prompt, rules, MCP config, logs and pid file.
pub(crate) fn agent_dir(app: &App, key: &AgentKey) -> PathBuf {
    app.data.join("runtime").join(key.project()).join(key.label().replace('/', "_"))
}

/// The harness command for an agent, ready to spawn — the same for a turn and a session
/// but for the command template (`runtime.command` / `runtime.sessionCommand`) and what
/// it is given (`{message}` / `{extension}`). Returns it with the agent's runtime directory.
pub(crate) async fn agent_process(
    app: &App,
    key: &AgentKey,
    spec: &AgentSpec,
    mode: Mode<'_>,
    token: &str,
    llm: &crate::llm_key::Key,
) -> Result<(tokio::process::Command, PathBuf), String> {
    let dir = agent_dir(app, key);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| e.to_string())?;
    let prompt_file = dir.join("prompt.md");
    tokio::fs::write(&prompt_file, &spec.prompt).await.map_err(|e| e.to_string())?;
    let files = spec.kit.write(app, &dir, &spec.role_id).map_err(|e| e.to_string())?;
    let sessions = app.data.join("sessions").join(key.project());
    tokio::fs::create_dir_all(&sessions).await.map_err(|e| e.to_string())?;
    let mut vars: HashMap<&str, String> = HashMap::from([
        ("sessionDir", sessions.to_string_lossy().into_owned()),
        ("sessionId", spec.session_id.clone()),
        ("model", spec.model.clone().unwrap_or_default()),
        ("thinking", spec.thinking.clone().unwrap_or_default()),
        ("promptFile", prompt_file.to_string_lossy().into_owned()),
        ("readonlyTools", spec.readonly.clone()),
        ("cwd", spec.cwd.to_string_lossy().into_owned()),
    ]);
    let template = match mode {
        Mode::Turn { message } => {
            vars.insert("message", message.to_string());
            &app.cfg.runtime.command
        }
        Mode::Session { extension } => {
            vars.insert("extension", extension.to_string_lossy().into_owned());
            &app.cfg.runtime.session_command
        }
    };
    let lists = spec.kit.placeholders(&files, &mut vars);
    let argv = build_command(template, &vars, &lists);
    let mut cmd = agent_command(app, &argv, &spec.cwd, &dir, key.project(), &spec.identity(), token)?;
    llm.apply(&mut cmd);
    kit_env(&mut cmd, &files);
    Ok((cmd, dir))
}

/// Who an agent process is: its environment for the `genie` command line and the extension.
pub(crate) struct Identity<'a> {
    pub role: Role,
    pub role_id: &'a str,
    pub name: &'a str,
    pub team: Option<&'a str>,
    pub task: Option<&'a str>,
    pub job: Option<i64>,
}

/// The harness command with the agent's environment (`GENIE_URL`, `GENIE_TOKEN`…),
/// in the agent's sandbox when agents run in one. `dir` is the agent's runtime
/// directory (its prompt, rules and MCP config).
pub(crate) fn agent_command(
    app: &App,
    argv: &[String],
    cwd: &Path,
    dir: &Path,
    project: &str,
    who: &Identity<'_>,
    token: &str,
) -> Result<tokio::process::Command, String> {
    let path = std::env::var("PATH").unwrap_or_default();
    let exe_dir = app.exe.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let (program, args) = argv.split_first().ok_or("the agent command is empty")?;
    let is_pi = Path::new(program).file_name().is_some_and(|n| n == "pi");
    let (program, args) = match sandbox_plan(app, project, cwd, dir)? {
        Some(plan) => plan.wrap(program, args),
        None => (program.to_string(), args.to_vec()),
    };
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .env("PATH", format!("{exe_dir}:{path}"))
        .env("GENIE_URL", format!("http://127.0.0.1:{}", app.cfg.port))
        .env("GENIE_TOKEN", token)
        .env("GENIE_PROJECT", project)
        .env("GENIE_AGENT_ROLE", who.role.as_str())
        .env("GENIE_AGENT_ROLE_ID", who.role_id)
        .env("GENIE_AGENT_NAME", who.name)
        .env("GENIE_TASK", who.task.unwrap_or_default())
        .env("GENIE_TEAM", who.team.unwrap_or_default())
        .env("GENIE_JOB", who.job.map(|j| j.to_string()).unwrap_or_default())
        // Keep the TypeScript pi extension (if installed) out of server-run agents.
        .env("GENIE_ROLE", "off")
        .env_remove("GENIE_DIR");
    // Agents hold no credentials for git hosts: their clones talk to the server's proxy.
    let through_proxy = app.with_server(|db| db.repos(project)).is_ok_and(|r| !r.is_empty());
    for var in crate::git::hosts::secret_vars(through_proxy) {
        cmd.env_remove(var);
    }
    // The secrets of connections behind the gateway stay with the server.
    if app.cfg.runtime.mcp_gateway {
        for var in app.agents().mcp_secret_vars() {
            cmd.env_remove(var);
        }
    }
    // Commits need an identity: when the server user has none, agents commit under their own names.
    if !git_identity(cwd) {
        let (name, email) = (format!("{} ({})", who.name, who.role_id), format!("{}@genie.local", who.name));
        cmd.env("GIT_AUTHOR_NAME", &name)
            .env("GIT_AUTHOR_EMAIL", &email)
            .env("GIT_COMMITTER_NAME", &name)
            .env("GIT_COMMITTER_EMAIL", &email);
    }
    for (k, v) in &app.cfg.runtime.env {
        cmd.env(k, v);
    }
    if is_pi {
        let own = app.cfg.runtime.env.get("NODE_OPTIONS").cloned().or_else(|| std::env::var("NODE_OPTIONS").ok());
        if let Some(options) = heap_cap(own.as_deref(), app.cfg.runtime.node_heap_mb) {
            // The guard extension takes the cap off again for what pi runs (builds are not pi).
            cmd.env("NODE_OPTIONS", options).env("GENIE_NODE_HEAP_MB", app.cfg.runtime.node_heap_mb.to_string());
        }
    }
    app.mcp.place(&crate::mcp_gateway::agent_key(project, who.team, who.name), cwd);
    Ok(cmd)
}

/// `NODE_OPTIONS` with a heap cap of `mb` added, or `None` when there is nothing to add: no cap
/// wanted (0) or one already set.
fn heap_cap(own: Option<&str>, mb: u64) -> Option<String> {
    let own = own.unwrap_or_default().trim();
    if mb == 0 || own.contains("--max-old-space-size") {
        return None;
    }
    Some(format!("{own} --max-old-space-size={mb}").trim().to_string())
}

/// The sandbox of an agent working in `cwd` (`dir`: its runtime directory), or
/// `None` when agents run without one ([`crate::sandbox`]).
fn sandbox_plan(app: &App, project: &str, cwd: &Path, dir: &Path) -> Result<Option<sandbox::Plan>, String> {
    use sandbox::Access::{Hidden, ReadOnly, Writable};
    let cfg = &app.cfg.runtime.sandbox;
    if !sandbox::enabled(cfg)? {
        if cfg.mode == "auto" {
            warn_once(
                "genie runtime: agents run without a sandbox: bubblewrap does not work on this machine (apt install bubblewrap); set runtime.sandbox to \"off\" to run without one on purpose",
            );
        }
        return Ok(None);
    }
    let home = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("/"));
    let mut plan = sandbox::Plan::new(cwd);
    // The server's data: only the agent's own files show through.
    plan.set(&app.data, Hidden);
    for f in ["genie-bus.ts", "genie-guard.ts"] {
        plan.set(&app.data.join("runtime").join(f), ReadOnly);
    }
    plan.set(dir, ReadOnly);
    plan.set(&app.data.join("skills"), ReadOnly);
    let sessions = app.data.join("sessions").join(project);
    let tmp = dir.join("tmp");
    for d in [&sessions, &tmp] {
        std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    plan.set(&sessions, Writable);
    plan.tmp(&tmp);
    // The trackers (agents reach tasks through the API) and the other projects.
    for p in app.with_server(|db| db.projects()).map_err(|e| e.to_string())? {
        plan.set(Path::new(&p.tracker_dir), Hidden);
        if p.slug == project {
            continue;
        }
        if let Some(repo) = &p.repo {
            let repo = Path::new(repo);
            plan.set(repo, Hidden);
            if let Some(worktrees) = worktree_place(app, repo, "_", "_").0.parent() {
                plan.set(worktrees, Hidden);
            }
        }
    }
    // `genie …` is the server's own binary.
    if let Some(bin) = app.exe.parent() {
        plan.set(bin, ReadOnly);
    }
    // Where the agent works, and the git repository behind a worktree (its commits go there).
    plan.set(cwd, Writable);
    if let Some(git) = git_common_dir(cwd) {
        plan.set(&git, Writable);
        // Not what the server runs outside the sandbox: hooks and configuration.
        for f in sandbox::GIT_DIR_READONLY {
            plan.set(&git.join(f), ReadOnly);
        }
    }
    let pi_dir = app.cfg.runtime.env.get("PI_CODING_AGENT_DIR").cloned().or_else(|| std::env::var("PI_CODING_AGENT_DIR").ok());
    sandbox::defaults(&mut plan, cfg, &home, pi_dir.map(|d| sandbox::expand(&d, &home)).as_deref());
    Ok(Some(plan))
}

/// The repository's git directory (shared by its worktrees), when `cwd` is in one.
fn git_common_dir(cwd: &Path) -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    (out.status.success() && path.is_dir()).then_some(path)
}

/// Whether git has an identity for commits made in `cwd` (the repository's, the user's or the system's).
fn git_identity(cwd: &Path) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["config", "user.email"])
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.trim_ascii().is_empty())
}

// --- what the harness gets from the role ------------------------------------------

/// An agent's harness setup from its role, besides the prompt: skills, MCP
/// connections and the rules of the genie guard extension.
#[derive(Debug, Clone, Default)]
pub(crate) struct Kit {
    /// `{skill}`: the role's skills, then the repository's skill directories.
    pub skills: Vec<String>,
    /// `{?limitSkills}`: the role lists its skills, so the harness loads no others.
    pub limit_skills: bool,
    /// The role's connections in pi's `mcp.json` format (secrets resolved, exposure chosen).
    pub mcp: serde_json::Value,
    /// The guard's rules (`GENIE_POLICY`).
    pub policy: serde_json::Value,
}

/// The files a started agent reads.
pub(crate) struct KitFiles {
    pub policy: PathBuf,
    /// The role's MCP connections; the guard extension connects them (`GENIE_MCP_CONFIG`).
    pub mcp_config: PathBuf,
    pub guard: PathBuf,
}

/// The rules the guard enforces for a role (`files` may be narrower than the role's: a read-only job).
pub(crate) fn policy(agents: &AgentConfig, project: &str, role: &RoleDef, files: FileAccess) -> serde_json::Value {
    let mcp: serde_json::Map<String, serde_json::Value> = agents
        .mcp_for(project, role)
        .into_iter()
        .map(|(s, tools)| (s.id.clone(), tools.map_or(serde_json::Value::Null, |t| json!(t))))
        .collect();
    json!({ "role": role.id, "files": files, "denyCommands": role.deny_commands, "mcp": mcp })
}

/// The kit of an agent working in `cwd`.
pub(crate) fn kit(app: &App, agents: &AgentConfig, project: &str, role: &RoleDef, files: FileAccess, cwd: &Path) -> Kit {
    let mut skills: Vec<String> =
        role.skills.iter().flatten().filter_map(|name| agents.skills.get(name)).map(|s| s.dir.to_string_lossy().into_owned()).collect();
    skills.extend(repo_skill_dirs(cwd).into_iter().map(|d| d.to_string_lossy().into_owned()));
    let mut codemode = false;
    let servers: serde_json::Map<String, serde_json::Value> = agents
        .mcp_for(project, role)
        .into_iter()
        .map(|(s, tools)| {
            // Through the gateway the harness holds only its address and the agent's own token
            // (the harness fills `${GENIE_TOKEN}` from its environment); the gateway keeps the tools.
            let mut entry = if app.cfg.runtime.mcp_gateway && s.gateway {
                let url = format!("http://127.0.0.1:{}/api/mcp-gateway/{}", app.cfg.port, s.id);
                json!({ "url": url, "headers": { "Authorization": "Bearer ${GENIE_TOKEN}" } })
            } else {
                s.resolved()
            };
            if let Some(o) = entry.as_object_mut() {
                // The admin's choice of exposure is the connection's, whoever opens it.
                for key in ["exposure", "toolExposure"] {
                    if let Some(v) = s.config.get(key) {
                        o.insert(key.into(), v.clone());
                    }
                }
                if !s.description.is_empty() {
                    o.insert("description".into(), json!(s.description));
                }
                codemode |= expose(o, tools.as_deref());
            }
            (s.id.clone(), entry)
        })
        .collect();
    // Codemode (JavaScript calling the tools) only for connections an admin set to it: pi would
    // switch it on for every role otherwise.
    Kit {
        skills,
        limit_skills: role.skills.is_some(),
        mcp: json!({ "mcpServers": servers, "autoEnableCodemode": codemode }),
        policy: policy(agents, project, role, files),
    }
}

/// How the tools of a connection reach the model. `exposure` in `mcp.json` is the admin's choice;
/// by default a connection limited to some tools declares them (`direct`: few, and the model calls
/// them as any tool), a whole one is searched with `tool_search` (`deferred`: its tools stay out of
/// every request until needed). A grant limited to tools hides the rest of the connection:
/// `toolExposure` names the granted patterns, so the others are never exposed (the guard checks
/// the calls all the same). Returns whether a tool is left to codemode.
fn expose(entry: &mut serde_json::Map<String, serde_json::Value>, granted: Option<&[String]>) -> bool {
    let codemode = |how: &str| how.starts_with("codemode");
    let chosen = entry.get("exposure").and_then(|e| e.as_str()).map(str::to_string);
    match granted {
        Some(patterns) => {
            let how = chosen.unwrap_or_else(|| "direct".into());
            // pi's patterns know `*` only.
            let by_tool: serde_json::Map<String, serde_json::Value> = patterns.iter().map(|p| (p.replace('?', "*"), json!(how))).collect();
            entry.insert("exposure".into(), json!("hidden"));
            entry.insert("toolExposure".into(), json!(by_tool));
            codemode(&how)
        }
        None => {
            let how = chosen.unwrap_or_else(|| "deferred".into());
            entry.insert("exposure".into(), json!(how));
            codemode(&how)
                || entry
                    .get("toolExposure")
                    .and_then(|t| t.as_object())
                    .is_some_and(|t| t.values().any(|v| v.as_str().is_some_and(codemode)))
        }
    }
}

/// The project's own skills, which every agent gets: `.pi/skills` and `.agents/skills`
/// from the working directory up to the repository root.
fn repo_skill_dirs(cwd: &Path) -> Vec<PathBuf> {
    let root = cwd.ancestors().find(|d| d.join(".git").exists());
    let mut out: Vec<PathBuf> = vec![cwd.join(".pi").join("skills")];
    for d in cwd.ancestors() {
        out.push(d.join(".agents").join("skills"));
        if root.is_none_or(|r| d == r) {
            break;
        }
    }
    out.retain(|d| d.is_dir());
    out
}

impl Kit {
    /// Write the agent's rules and MCP config into its runtime directory `dir`.
    pub(crate) fn write(&self, app: &App, dir: &Path, role: &str) -> std::io::Result<KitFiles> {
        let text = |v: &serde_json::Value| serde_json::to_string_pretty(v).unwrap_or_default();
        let files =
            KitFiles { policy: dir.join("policy.json"), mcp_config: dir.join("mcp.json"), guard: crate::sessions::write_guard(app)? };
        write_private(&files.policy, &text(&self.policy))?;
        write_private(&files.mcp_config, &text(&self.mcp))?;
        if app.cfg.runtime.mcp_adapter_loaded() && self.mcp["mcpServers"].as_object().is_some_and(|m| !m.is_empty()) {
            warn_once(&format!(
                "genie runtime: pi loads pi-mcp-adapter, which does not belong next to its native MCP support: the guard blocks its tools, so the role {role} has only the connections genie hands it (`pi remove npm:pi-mcp-adapter`)"
            ));
        }
        Ok(files)
    }

    /// Add the kit's placeholders for the harness command to `vars`; returns its lists.
    pub(crate) fn placeholders(&self, files: &KitFiles, vars: &mut HashMap<&str, String>) -> HashMap<&'static str, Vec<String>> {
        let path = |p: &Path| p.to_string_lossy().into_owned();
        vars.insert("guard", path(&files.guard));
        vars.insert("mcpConfig", path(&files.mcp_config));
        vars.insert("limitSkills", if self.limit_skills { "yes".into() } else { String::new() });
        HashMap::from([("skill", self.skills.clone())])
    }
}

/// Print a warning once per server run.
fn warn_once(text: &str) {
    static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if seen.get_or_insert_default().insert(text.to_string()) {
        eprintln!("{text}");
    }
}

/// Tell the started agent where its rules and its MCP connections are (the guard extension reads both).
pub(crate) fn kit_env(cmd: &mut tokio::process::Command, files: &KitFiles) {
    cmd.env("GENIE_POLICY", &files.policy).env("GENIE_MCP_CONFIG", &files.mcp_config);
}

/// Write a file only its owner can read (it may hold secrets), replacing it at once.
pub(crate) fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(&tmp)?.write_all(text.as_bytes())?;
    std::fs::rename(&tmp, path)
}

/// Who an agent runs as and with what — for a turn and a live session alike.
#[derive(Clone)]
pub(crate) struct AgentSpec {
    pub role: Role,
    pub role_id: String,
    /// `--exclude-tools` for read-only roles.
    pub readonly: String,
    pub name: String,
    pub team: Option<String>,
    pub task: Option<String>,
    pub job: Option<i64>,
    pub cwd: PathBuf,
    pub session_id: String,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub prompt: String,
    pub kit: Kit,
    /// The person the agent works for now ([`crate::llm_key`]).
    pub initiator: Option<String>,
}

impl AgentSpec {
    pub fn identity(&self) -> Identity<'_> {
        Identity {
            role: self.role,
            role_id: &self.role_id,
            name: &self.name,
            team: self.team.as_deref(),
            task: self.task.as_deref(),
            job: self.job,
        }
    }

    /// The agent's token, bound to the project and to what it is (role, name, team or job).
    pub fn token(&self, app: &App, project: &str, ttl: chrono::Duration) -> AppResult<String> {
        app.with_server(|db| db.create_role_token(project, self.role, Some(&self.role_id), &self.name, self.team.as_deref(), self.job, ttl))
    }
}

/// The project's orchestrator (`live`: as a session; `initiator`: whom it works for now).
fn orchestrator_spec(app: &App, project: &Project, live: bool, initiator: Option<String>) -> AppResult<AgentSpec> {
    let agents = app.agents();
    let def = orchestrator_role(&agents)?;
    let (model, thinking) = role_model(app, &def, None, None);
    let cwd = project_workspace(app, project, "orchestrator");
    Ok(AgentSpec {
        role: Role::Orchestrator,
        role_id: def.id.clone(),
        readonly: def.excluded_tools().unwrap_or_default(),
        name: ORCHESTRATOR.into(),
        team: None,
        task: None,
        job: None,
        kit: kit(app, &agents, &project.slug, &def, def.files, &cwd),
        cwd,
        session_id: format!("{}-orchestrator", project.slug),
        model,
        thinking,
        prompt: agent_prompt(app, &agents, project, &def, None, live, AgentKind::Orchestrator, &who(&def, None, None)),
        initiator,
    })
}

/// A member of a team (`live`: as a session).
fn member_spec(app: &App, project: &Project, t: &team::Team, m: &team::Member, live: bool) -> AppResult<AgentSpec> {
    let agents = app.agents();
    let def = running_role(&agents, &m.role)?;
    let (model, thinking) = role_model(app, &def, m.model.clone(), m.thinking.clone());
    let cwd = member_workspace(app, project, &def, &t.cwd, &t.id, &m.name);
    Ok(AgentSpec {
        role: def.class,
        role_id: def.id.clone(),
        readonly: def.excluded_tools().unwrap_or_default(),
        name: m.name.clone(),
        team: Some(t.id.clone()),
        task: Some(t.task.clone()),
        job: None,
        kit: kit(app, &agents, &project.slug, &def, def.files, &cwd),
        cwd,
        session_id: m.session_file.clone().unwrap_or_else(|| format!("{}-{}", t.id, m.name).to_lowercase()),
        model,
        thinking,
        prompt: agent_prompt(
            app,
            &agents,
            project,
            &def,
            m.instructions.as_deref(),
            live,
            AgentKind::Member,
            &who(&def, Some(&t.id), None),
        ),
        initiator: crate::llm_key::team_initiator(app, &project.slug, &t.id),
    })
}

/// Whether the server's orchestrator of a project runs: not in `manual` mode, and not while
/// a person's session holds its console.
fn orchestrator_runs(app: &App, project: &Project) -> bool {
    project.autonomy != "manual" && !console_held(app, &project.slug)
}

/// Whether someone's own session holds the orchestrator console of a project
/// (`genie orchestrate`): the server's orchestrator waits meanwhile.
fn console_held(app: &App, slug: &str) -> bool {
    app.with_server(|db| db.console(slug)).ok().flatten().is_some()
}

/// What a person's session at the orchestrator console gets: the orchestrator's
/// configured role, prompt and model.
pub(crate) struct ConsoleSpec {
    pub role_id: String,
    pub prompt: String,
    pub model: Option<String>,
    pub thinking: Option<String>,
}

pub(crate) fn console_spec(app: &App, slug: &str, user: &str) -> AppResult<ConsoleSpec> {
    let project = project_of(app, slug)?;
    let agents = app.agents();
    let def = orchestrator_role(&agents)?;
    let (model, thinking) = role_model(app, &def, None, None);
    let mut prompt = agent_prompt(app, &agents, &project, &def, None, true, AgentKind::Orchestrator, &who(&def, None, None));
    prompt.push_str(&format!(
        "\n## The console\n\nYou run in the pi session of @{user} at the orchestrator console of the project: they talk to you directly and see your work. Team mail arrives in this conversation as `[genie mail]` blocks — between your steps, or on its own when you are idle. The server's orchestrator waits while this session holds the console.\n"
    ));
    Ok(ConsoleSpec { role_id: def.id.clone(), prompt, model, thinking })
}

/// The live session of an agent, or `None` when it should not run now (team
/// stopped, member removed or in error, orchestrator in manual mode or at a
/// person's console, a job).
pub(crate) fn session_spec(app: &App, key: &AgentKey) -> AppResult<Option<AgentSpec>> {
    let project = project_of(app, key.project())?;
    match key {
        AgentKey::Orchestrator { project: slug } if orchestrator_runs(app, &project) => {
            orchestrator_spec(app, &project, true, crate::llm_key::orchestrator_initiator(app, slug)).map(Some)
        }
        AgentKey::Member { project: slug, team, member } => {
            let Ok((t, runnable)) = app.with_tracker(slug, |t| Ok((t.bus().get(team)?, t.bus().runnable(team, member)?))) else {
                return Ok(None);
            };
            match t.members.iter().find(|m| &m.name == member) {
                Some(m) if runnable => member_spec(app, &project, &t, m, true).map(Some),
                _ => Ok(None),
            }
        }
        _ => Ok(None),
    }
}

/// Expand argument groups; a group with an empty placeholder is dropped, and a
/// group with a list placeholder (from `lists`) is repeated for each item.
/// `{?name}` adds nothing and keeps its group only when `name` is set. Single
/// pass over the template, so text inside a substituted value (a message
/// mentioning `{model}`) is never expanded again.
pub fn build_command(groups: &[Vec<String>], vars: &HashMap<&str, String>, lists: &HashMap<&str, Vec<String>>) -> Vec<String> {
    let mut out = Vec::new();
    for g in groups {
        let list = g.iter().find_map(|arg| lists.keys().copied().find(|k| arg.contains(&format!("{{{k}}}"))));
        match list {
            Some(name) => {
                for item in &lists[name] {
                    let mut one = vars.clone();
                    one.insert(name, item.clone());
                    out.extend(expand_group(g, &one, lists).unwrap_or_default());
                }
            }
            None => out.extend(expand_group(g, vars, lists).unwrap_or_default()),
        }
    }
    out
}

/// One argument group; `None` when a placeholder in it is empty.
fn expand_group(g: &[String], vars: &HashMap<&str, String>, lists: &HashMap<&str, Vec<String>>) -> Option<Vec<String>> {
    let set = |name: &str| vars.get(name).is_some_and(|v| !v.is_empty()) || lists.get(name).is_some_and(|l| !l.is_empty());
    let known = |name: &str| vars.contains_key(name) || lists.contains_key(name);
    let mut expanded = Vec::new();
    for arg in g {
        let mut s = String::new();
        let mut rest = arg.as_str();
        let mut condition = false;
        while let Some(start) = rest.find('{') {
            s.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            match after.find('}').map(|end| (&after[..end], end)) {
                Some((name, end)) if name.strip_prefix('?').is_some_and(known) => {
                    if !set(&name[1..]) {
                        return None;
                    }
                    condition = true;
                    rest = &after[end + 1..];
                }
                Some((name, end)) if vars.contains_key(name) => {
                    let v = &vars[name];
                    if v.is_empty() {
                        return None;
                    }
                    s.push_str(v);
                    rest = &after[end + 1..];
                }
                _ => {
                    s.push('{');
                    rest = after;
                }
            }
        }
        s.push_str(rest);
        // An argument made only of conditions adds nothing.
        if !(condition && s.is_empty()) {
            expanded.push(s);
        }
    }
    Some(expanded)
}

/// A turn ended: close its row, its token and its mail lease (or job), then record how it went.
fn finish(app: &App, key: &AgentKey, p: &Prepared, ok: bool, code: Option<i32>, error: Option<&str>, log: &str) -> AppResult<()> {
    let slug = key.project();
    app.with_server(|db| db.finish_turn(p.turn, if ok { "succeeded" } else { "failed" }, code.map(i64::from), error, Some(log)))?;
    app.with_server(|db| db.revoke_token(&p.token))?;
    match key {
        AgentKey::Job { job, .. } => {
            let has_output = app.with_server(|db| db.job(*job))?.output.is_some();
            let ok = ok && has_output;
            let err = if !has_output && error.is_none() { Some("the agent finished without `genie job output`") } else { error };
            let max_attempts = i64::from(app.cfg.runtime.max_attempts.max(1));
            app.with_server(|db| db.finish_job(*job, ok, err, max_attempts))?;
        }
        AgentKey::Orchestrator { .. } | AgentKey::Member { .. } => {
            app.with_tracker(slug, |t| if ok { t.bus().complete_lease(p.turn) } else { t.bus().release_lease(p.turn) })?;
        }
    }
    let outcome = if ok { Outcome::Ran } else { Outcome::Failed { error: error.unwrap_or("error"), log } };
    crate::outcome::record(app, key, p.spec.task.as_deref(), outcome);
    Ok(())
}

// --- prompts -----------------------------------------------------------------

/// How the agent acts in genie: the delivery model (live session or turns) and
/// the command table of its role, from the catalog of operations.
fn tools_section(live: bool, role: &RoleDef, reader: AgentKind, ask_timeout: u64) -> String {
    let orch = reader == AgentKind::Orchestrator;
    let can = |c: Capability| orch || role.can(c);
    let mut out = String::from("\n## How you act in genie (server runtime)\n\n");
    if live {
        out.push_str(
            "You run as a live session. Team mail arrives in your conversation between your steps, as `[genie mail]` blocks with the most urgent first — read them when they appear and adjust your work. A message marked INTERRUPT means your previous step was stopped for it: follow it first. When there is nothing left to do, simply stop: new mail wakes you. Never sleep or poll for mail.\n",
        );
    } else {
        out.push_str(
            "You run in turns: each turn delivers your new messages; do the work they call for, then end the turn by finishing your reply. Teammates' answers arrive as a new turn — never wait or poll with sleep.\n",
        );
    }
    if reader == AgentKind::Member {
        out.push_str("\nYour kickoff says how your team works — who hands work to whom, who reviews it, who reports to the orchestrator. Where it differs from the role guide above, follow the kickoff.\n");
    }
    out.push_str("\nEverything goes through the `genie` command (already configured for you: project, team, task and your identity come from the environment). Wherever the role guide above mentions a tool, use its command from this table:\n\n");
    out.push_str(&ops::command_table(reader, &can));
    if reader != AgentKind::Job {
        let mut tips = Vec::new();
        if live && (orch || role.can(Capability::MailTeam)) {
            tips.push(format!("`genie mail ask` waits up to {ask_timeout}s for the answer; a late answer arrives as mail."));
        }
        tips.push(
            "Someone waiting on you gets `genie mail reply <id> \"…\"`; a clipped message is read in full with `genie mail read <id>`."
                .to_string(),
        );
        tips.push(
            "`genie mail send … --topic <subject>` replaces your earlier update on the same subject instead of adding one.".to_string(),
        );
        out.push_str(&format!(
            "\nTalking to the team: {} Keep messages short and point to the task, comments and artifacts for details; progress goes into the task, not into mail. Do not send acknowledgements.\n",
            tips.join(" ")
        ));
    }
    if orch {
        out.push_str(
            "\nDirecting the team: `genie team board` shows every agent's state, current step and waiting mail; `genie mail send <name> \"…\" --level high --team T` corrects an agent at its next step; `genie team interrupt <team> <name> \"what to do instead\"` stops the step it is running (even a long command); `genie team pause` / `resume` hold an agent and let it go on. Peek (`genie team peek`) before you interrupt; interrupt only when the current step is wrong or wasteful.\n",
        );
    }
    out.push_str("\n`genie <group> <command> --help` prints the details of a command. Output is plain text meant for you.\n");
    out
}

/// The orchestrator's view of the configured templates and roles of a project.
fn catalogue_section(agents: &AgentConfig, project: &Project) -> String {
    let mut out = String::from(
        "\n## Team templates and roles\n\nAssemble a team from a template (`genie team spawn <TASK> --template <id>`) or from roles (`--member <role>`, repeatable); add a member to a running team with `genie team add-member <TEAM> <role>`. Templates are presets, not rules: pick by the descriptions. `genie team templates` and `genie team roles` print the details.\n\nTemplates:\n",
    );
    for t in agents.teams.values().filter(|t| t.available_in(&project.slug)) {
        let roles: Vec<&str> = t.members.iter().map(|m| m.role.as_str()).collect();
        let stage = match t.stage {
            Stage::Refinement => "before ready",
            Stage::Delivery => "ready tasks",
        };
        let ws = match t.workspace {
            Workspace::Worktree => "own worktree",
            Workspace::Repo => "main working copy",
            Workspace::Scratch => "empty workspace",
        };
        out.push_str(&format!("- `{}` — {} ({stage} · {ws} · {})\n", t.id, t.description, roles.join(", ")));
    }
    out.push_str("\nRoles:\n");
    let mut early = Vec::new();
    for r in agents.roles.values().filter(|r| r.class != Role::Orchestrator && r.available_in(&project.slug)) {
        let class = if r.id == r.class.as_str() { String::new() } else { format!(" ({})", r.class) };
        out.push_str(&format!("- `{}`{class} — {}\n", r.id, r.description));
        if r.stages.contains(&Stage::Refinement) {
            early.push(format!("`{}`", r.id));
        }
    }
    out.push_str(&format!("\nBefore a task is `ready`, only these roles may work on it: {}.\n", early.join(", ")));
    out
}

#[allow(clippy::too_many_arguments)]
fn agent_prompt(
    app: &App,
    agents: &AgentConfig,
    project: &Project,
    role: &RoleDef,
    instructions: Option<&str>,
    live: bool,
    reader: AgentKind,
    who: &crate::git::service::AgentId,
) -> String {
    let lang = &app.cfg.language;
    let mut out = role.full_prompt();
    out.push_str(&tools_section(live, role, reader, app.cfg.runtime.ask_timeout_secs));
    out.push_str(&mcp_section(agents, &project.slug, role));
    out.push_str(&format!(
        "\n## Project\n\nProject `{}` ({}). {}\n\nLanguage: write tasks, comments, artifacts and team mail in {}; anything addressed to people (questions for the owner, needs_owner notes) in {}.\n",
        project.slug,
        project.name,
        if project.repo.is_some() || app.with_server(|db| db.repos(&project.slug)).is_ok_and(|r| !r.is_empty()) {
            "It has a code repository."
        } else {
            "It has no code repository: deliver results as task artifacts and knowledge pages."
        },
        lang.internal,
        lang.user,
    ));
    out.push_str(&crate::git::service::prompt_section(app, &project.slug, who));
    out.push_str("\nTo reach a person, mention them as `@login` in a task comment: they get a notification (in the web, Telegram or e-mail). A task's person responsible (`assignee`) is the one to ask about it.\n");
    if (reader == AgentKind::Orchestrator || role.can(Capability::DocsRead))
        && let Some(l0) = crate::context::l0(app, &project.slug)
    {
        out.push_str(&format!("\n{l0}\n"));
    }
    if reader == AgentKind::Orchestrator {
        let people: Vec<String> = app
            .with_server(|db| db.members_of(&project.slug))
            .unwrap_or_default()
            .into_iter()
            .filter(|(u, _)| !u.disabled)
            .map(|(u, r)| {
                format!("@{} ({}{})", u.login, r.as_str(), if u.name.is_empty() { String::new() } else { format!(", {}", u.name) })
            })
            .collect();
        if !people.is_empty() {
            out.push_str(&format!("\nPeople of the project: {}. Set a task's person responsible with `genie task update --assignee login` when someone owns the decision or the review.\n", people.join(", ")));
        }
        if project.autonomy == "assisted" {
            out.push_str("\n## Autonomy: assisted\n\nPeople close tasks in this project. Take tasks into work and see them through as usual, but do not move a task to `done` or `cancelled` yourself (the server refuses): when it meets its Definition of Done, move it to `needs_owner` with a short summary of the result and what to check. A person closes it.\n");
        }
        if !project.integration.trim().is_empty() {
            out.push_str(&format!(
                "\n## Integration\n\nUnless the owner agreed on another way for a task, its result is integrated like this: {}. Use it as the task's integration (`mergeStrategy`) without asking; teams get it with the task.\n",
                project.integration.trim()
            ));
        }
        out.push_str("\n## Automations\n\nSome work is done by the project's automations (their comments and actions are signed `automation:<id>:<run>`). A task in `refining` with the comment \"Взята в разбор автоматически\" is being triaged by an automation: do not start another analysis for it — you will get a message when the author's answers are in. Automations also update the knowledge base and the changelog when a task is done.\n");
        out.push_str(&catalogue_section(agents, project));
    }
    if let Some(i) = instructions.filter(|i| !i.trim().is_empty()) {
        out.push_str(&format!("\n## Instructions for you\n\n{i}\n"));
    }
    out
}

/// The MCP connections the role was granted (their descriptions tell when to use them).
fn mcp_section(agents: &AgentConfig, project: &str, role: &RoleDef) -> String {
    let grants = agents.mcp_for(project, role);
    if grants.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n## MCP connections\n\nYour role may use these MCP connections, and no others. Their tools are named `mcp__<connection>__<tool>` (`-` becomes `_`); those not declared to you yet are found with `tool_search`, or in a `codemode` script when you have it:\n\n",
    );
    for (s, tools) in grants {
        out.push_str(&format!("- `{}`", s.id));
        if !s.description.is_empty() {
            out.push_str(&format!(" — {}", s.description));
        }
        if let Some(t) = tools {
            out.push_str(&format!(" (only the tools {})", t.iter().map(|x| format!("`{x}`")).collect::<Vec<_>>().join(", ")));
        }
        out.push('\n');
    }
    out
}

fn orchestrator_message(project: &Project, mail: &[Mail]) -> String {
    let verbatim: Vec<&Mail> = mail.iter().filter(|m| m.kind != "message").collect();
    let mut out = format!("[genie · project {} · orchestrator turn]\n", project.slug);
    for m in verbatim {
        out.push_str(&format!(
            "\n## {} from {}\n\n{}\n",
            if m.kind == "owner" { "Owner activity" } else { "System" },
            m.from,
            m.text.trim()
        ));
    }
    let digest = team::render_digest(mail);
    if !digest.is_empty() {
        out.push('\n');
        out.push_str(&digest);
        out.push('\n');
    }
    out.push_str("\nAct on these now (take inbox tasks, answer teams, dispatch ready work, accept approved work), then end your turn.");
    out
}

fn member_message(mail: &[Mail]) -> String {
    format!("{}\n\nAct on this now, then end your turn. Replies arrive as your next turn.", team::render_batch(mail))
}

fn job_message(j: &Job, workspace: &str) -> String {
    let mut out = format!("[genie job {} · role {}]\n\n## Goal\n\n{}\n\n## Workspace\n\n{workspace}\n", j.id, j.role, j.goal.trim());
    if let Some(t) = &j.task {
        out.push_str(&format!("\nTask: {t} (read it with `genie task show {t}`).\n"));
    }
    if !j.inputs.is_null() && j.inputs != json!({}) {
        out.push_str(&format!("\n## Inputs\n\n```json\n{}\n```\n", serde_json::to_string_pretty(&j.inputs).unwrap_or_default()));
    }
    match &j.output_schema {
        Some(schema) => out.push_str(&format!(
            "\n## Result\n\nWhen done, report your result exactly once with `genie job output '<json>'`, a JSON object with this shape:\n\n```json\n{}\n```\n\nThe job fails if you finish without reporting a result.",
            serde_json::to_string_pretty(schema).unwrap_or_default()
        )),
        None => out.push_str("\n## Result\n\nWhen done, report a short summary with `genie job output '{\"summary\": \"…\"}'`. The job fails if you finish without reporting a result."),
    }
    out
}

// --- team operations -----------------------------------------------------------

/// Where the worktree of a team (or a job) goes, and its branch (`worktrees` in the config).
fn worktree_place(app: &App, repo: &Path, team: &str, task: &str) -> (PathBuf, String) {
    let repo_name = repo.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "repo".into());
    let fill = |tpl: &str| {
        tpl.replace("{mainRoot}", &repo.to_string_lossy()).replace("{repo}", &repo_name).replace("{team}", team).replace("{task}", task)
    };
    // `{mainRoot}/../…` without the `..` (the repository path is canonical, so this is exact).
    let mut dir = PathBuf::new();
    for c in Path::new(&fill(&app.cfg.worktrees.dir)).components() {
        match c {
            std::path::Component::ParentDir if dir.file_name().is_some() => {
                dir.pop();
            }
            std::path::Component::CurDir => {}
            other => dir.push(other),
        }
    }
    (dir, fill(&app.cfg.worktrees.branch))
}

/// A job's own worktree (`job-<id>`), kept across its attempts.
fn job_worktree(app: &App, repo: &Path, j: &Job) -> AppResult<TeamWorktree> {
    let name = format!("job-{}", j.id);
    let task = j.task.clone().unwrap_or_else(|| name.clone());
    let (dir, branch) = worktree_place(app, repo, &name, &task);
    if dir.join(".git").exists() {
        return Ok(TeamWorktree { path: dir.to_string_lossy().into_owned(), branch, base: None });
    }
    create_worktree(app, repo, &name, &task)
}

/// Worktree for a team: `git worktree add` on a fresh branch from HEAD.
fn create_worktree(app: &App, repo: &Path, team: &str, task: &str) -> AppResult<TeamWorktree> {
    let run = |dir: &Path, args: &[&str]| -> AppResult<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .map_err(|e| AppError::Internal(format!("git: {e}")))?;
        if !out.status.success() {
            return Err(AppError::Internal(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    let (dir, branch) = worktree_place(app, repo, team, task);
    let base = run(repo, &["rev-parse", "HEAD"])?;
    if dir.exists() {
        return Err(AppError::Internal(format!("worktree directory {} already exists", dir.display())));
    }
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::Internal(e.to_string()))?;
    }
    let exists = run(repo, &["rev-parse", "--verify", "--quiet", &format!("refs/heads/{branch}")]).is_ok();
    let dir_s = dir.to_string_lossy().into_owned();
    if exists {
        run(repo, &["worktree", "add", &dir_s, &branch])?;
    } else {
        run(repo, &["worktree", "add", "-b", &branch, &dir_s, &base])?;
    }
    Ok(TeamWorktree { path: dir_s, branch, base: Some(base) })
}

pub struct SpawnRequest {
    pub task: String,
    pub template: Option<String>,
    pub members: Vec<MemberSpec>,
    /// Models for a template's members, by member key.
    pub models: std::collections::BTreeMap<String, String>,
    pub note: Option<String>,
    pub by: Actor,
    /// The person the team works on behalf of; by default `by` when a person,
    /// else the task's person ([`crate::llm_key::initiator_of`]).
    pub initiator: Option<String>,
}

/// A free name for a role: from its pool, else numbered.
fn pick_name(role: &RoleDef, taken: &mut std::collections::HashSet<String>) -> String {
    team::pick_from(&role.names, taken)
}

/// Assemble a team for a task: roster and relations from the template (or from
/// the roles asked for), a workspace, the snapshot of how the team works, kickoffs.
pub fn spawn_team(app: &App, slug: &str, req: SpawnRequest) -> AppResult<genie_core::team::Team> {
    let project = project_of(app, slug)?;
    let agents = app.agents();
    let task = app.with_tracker(slug, |t| t.get(&req.task))?;
    if task.task_type == genie_core::TaskType::Epic {
        return Err(GenieError::invalid("epics are not handed to teams; split it into tasks").into());
    }
    if CLOSED.contains(&task.status) {
        return Err(GenieError::invalid(format!("{} is {}", task.id, task.status)).into());
    }
    // Chosen outside the tracker's lock: the knowledge base has its own.
    let docs = crate::context::l1(app, slug, &task);
    let refinement = matches!(task.status, Status::Inbox | Status::Draft | Status::Refining);
    let template = match (&req.template, req.members.is_empty()) {
        (Some(id), _) => Some(agents.team_for(slug, id)?.clone()),
        (None, true) => Some(agents.team_for(slug, "standard")?.clone()),
        (None, false) => None,
    };
    // The roster: the template's members, or the roles asked for.
    let asked: Vec<(Option<String>, MemberSpec)> = if req.members.is_empty() {
        template
            .as_ref()
            .map(|t| {
                t.members
                    .iter()
                    .map(|m| {
                        (
                            Some(m.key.clone()),
                            MemberSpec {
                                role: m.role.clone(),
                                name: m.name.clone(),
                                model: req.models.get(&m.key).filter(|x| !x.trim().is_empty()).cloned().or_else(|| m.model.clone()),
                                thinking: m.thinking.clone(),
                                instructions: m.instructions.clone(),
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        req.members.iter().map(|m| (None, m.clone())).collect()
    };
    if asked.is_empty() {
        return Err(GenieError::invalid("a team needs at least one member").into());
    }
    let limits = &app.cfg.limits;
    if asked.len() > limits.max_members_per_team {
        return Err(GenieError::invalid(format!("limit: at most {} members per team", limits.max_members_per_team)).into());
    }
    let mut roles: Vec<RoleDef> = Vec::new();
    for (_, m) in &asked {
        let r = agents.role_for(slug, &m.role)?;
        if r.class == Role::Orchestrator {
            return Err(GenieError::invalid("the orchestrator is not a team member").into());
        }
        roles.push(r.clone());
    }
    if refinement && let Some(r) = roles.iter().find(|r| !r.stages.contains(&Stage::Refinement)) {
        let early: Vec<&str> = agents
            .roles
            .values()
            .filter(|r| r.stages.contains(&Stage::Refinement) && r.available_in(slug))
            .map(|r| r.id.as_str())
            .collect();
        return Err(GenieError::invalid(format!(
            "{} is not ready ({}); role {} works only on ready tasks. Before `ready` only these roles may work on it: {} (e.g. template research)",
            task.id,
            task.status,
            r.id,
            early.join(", ")
        ))
        .into());
    }
    if let Some(team) = app.with_tracker(slug, |t| t.bus().active_team_of(&task.id))? {
        return Err(GenieError::invalid(format!("{} already has an active team {team}", task.id)).into());
    }
    let active = app.with_tracker(slug, |t| t.bus().active_count())?;
    if active as usize >= limits.max_active_teams {
        return Err(
            GenieError::invalid(format!("limit: at most {} active teams per project; stop one first", limits.max_active_teams)).into()
        );
    }
    if limits.max_active_teams_per_epic > 0
        && let Some(epic) = &task.parent
    {
        let in_epic = app.with_tracker(slug, |t| t.bus().active_count_in_epic(epic))?;
        if in_epic as usize >= limits.max_active_teams_per_epic {
            return Err(GenieError::invalid(format!(
                "limit: at most {} active teams per epic ({epic}); {} stays in its status until one of them stops",
                limits.max_active_teams_per_epic, task.id
            ))
            .into());
        }
    }
    if let Some(over) = crate::budget::check(app, slug, Some(&task.id))? {
        return Err(GenieError::invalid(over.explain()).into());
    }
    let team_id = app.with_tracker(slug, |t| t.bus().free_id(&task.id))?;
    let workspace = template.as_ref().map(|t| t.workspace).unwrap_or(Workspace::Worktree);
    let has_repos = !app.with_server(|db| db.repos(slug))?.is_empty();
    let worktree = match (&project.repo, workspace) {
        _ if has_repos => match (workspace, refinement) {
            // The project's repositories: a clone of each at its mount, on the task's branch.
            (Workspace::Worktree, false) => {
                let (root, placed) = crate::git::service::task_workspace(app, slug, &team_id, Some(&task.id))?;
                let branch = placed.iter().find_map(|p| p.branch.clone()).unwrap_or_default();
                Some(TeamWorktree { path: root.to_string_lossy().into_owned(), branch, base: None })
            }
            _ => None,
        },
        (Some(repo), Workspace::Worktree) if !refinement => Some(create_worktree(app, Path::new(repo), &team_id, &task.id)?),
        _ => None,
    };
    let cwd = match (&worktree, &project.repo, workspace) {
        (None, _, Workspace::Scratch) if has_repos => {
            let d = app.data.join("workspaces").join(slug).join(&team_id);
            std::fs::create_dir_all(&d).map_err(|e| AppError::Internal(format!("{}: {e}", d.display())))?;
            d.to_string_lossy().into_owned()
        }
        (None, _, _) if has_repos => project_workspace(app, &project, &team_id).to_string_lossy().into_owned(),
        (Some(w), _, _) => w.path.clone(),
        (None, Some(_), Workspace::Scratch) => {
            let d = app.data.join("workspaces").join(slug).join(&team_id);
            std::fs::create_dir_all(&d).map_err(|e| AppError::Internal(format!("{}: {e}", d.display())))?;
            d.to_string_lossy().into_owned()
        }
        (None, Some(repo), _) => repo.clone(),
        (None, None, _) => project_workspace(app, &project, &team_id).to_string_lossy().into_owned(),
    };
    // Names and keys; the spec keeps the template's relations (or derives them).
    let mut taken = app.with_tracker(slug, |t| t.bus().taken_names())?;
    let mut spec_members: Vec<SpecMember> = Vec::new();
    let mut members: Vec<NewMember> = Vec::new();
    for ((key, m), role) in asked.iter().zip(&roles) {
        let name = m.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| pick_name(role, &mut taken));
        taken.insert(name.clone());
        let key = key.clone().unwrap_or_else(|| crate::agent_config::free_key(&spec_members, &role.id));
        spec_members.push(SpecMember { key, name: name.clone(), role: role.id.clone() });
        members.push(NewMember {
            name,
            role: role.id.clone(),
            model: m.model.clone(),
            thinking: m.thinking.clone(),
            instructions: m.instructions.clone(),
        });
    }
    let classes: Vec<Role> = roles.iter().map(|r| r.class).collect();
    let mut spec = match &template {
        Some(t) if req.members.is_empty() => TeamSpec {
            template: Some(t.id.clone()),
            title: Some(t.title.clone()),
            stage: t.stage,
            workspace: t.workspace,
            mail: t.mail,
            members: spec_members,
            relations: t.relations.clone(),
            charter: t.charter.clone(),
            template_hash: Some(crate::agent_config::template_hash(t)),
            initiator: None,
        },
        _ => TeamSpec {
            template: template.as_ref().map(|t| t.id.clone()),
            title: template.as_ref().map(|t| t.title.clone()),
            workspace,
            ..TeamSpec::derived(spec_members, &classes, refinement)
        },
    };
    spec.initiator = req.initiator.clone().or_else(|| crate::llm_key::initiator_of(app, slug, &req.by, Some(&task.id)));
    let system = Actor::new(req.by.name.clone(), if req.by.role == Role::Human { Role::Human } else { Role::Orchestrator });
    let created = app.with_tracker(slug, |t| {
        let team = t.bus().create(
            &req.by.name,
            req.by.role.as_str(),
            NewTeam {
                id: team_id.clone(),
                task: task.id.clone(),
                template: spec.template.clone(),
                cwd: cwd.clone(),
                worktree: worktree.clone(),
                members: members.clone(),
                spec: Some(spec.to_value()),
            },
        )?;
        let wt = worktree.as_ref().map(|w| genie_core::Worktree { path: w.path.clone(), branch: Some(w.branch.clone()) });
        let names: Vec<String> = members.iter().map(|m| m.name.clone()).collect();
        t.assign_team(&system, &task.id, Some(&team_id), wt.as_ref(), Some(&names))?;
        // The project's way of integrating results, unless the task has its own.
        if !refinement && task.merge_strategy.trim().is_empty() && !project.integration.trim().is_empty() {
            let input = genie_core::UpdateInput { merge_strategy: Some(project.integration.clone()), ..Default::default() };
            t.update(&system, &task.id, input)?;
        }
        if matches!(task.status, Status::Inbox | Status::Draft) {
            t.set_status(
                &system,
                &task.id,
                Status::Refining,
                StatusOptions { note: Some(format!("research team {team_id} started")), force: true, ..Default::default() },
            )?;
        }
        let epic = t.epic_context(&task.id)?.epic;
        let k = Kickoff {
            team: &team_id,
            task: &task,
            cwd: &cwd,
            worktree: worktree.as_ref(),
            spec: &spec,
            agents: &agents,
            note: req.note.as_deref(),
            epic: epic.as_ref(),
            joining: false,
            docs: docs.as_deref(),
        };
        for m in &spec.members {
            t.bus().send(team::SendMail {
                team: &team_id,
                from: ORCHESTRATOR,
                from_role: "orchestrator",
                to: &m.name,
                text: &k.text(m),
                level: Some("normal"),
                intent: None,
                kind: "kickoff",
                ..Default::default()
            })?;
        }
        Ok(team)
    })?;
    app.wake_runtime.notify_one();
    Ok(created)
}

/// Add a member to a running team: its role from the configuration, relations
/// derived for it, a kickoff for it and a note for the others.
pub fn add_member(app: &App, slug: &str, team_id: &str, m: MemberSpec, by: &str) -> AppResult<SpecMember> {
    let agents = app.agents();
    let role = agents.role_for(slug, &m.role)?.clone();
    if role.class == Role::Orchestrator {
        return Err(GenieError::invalid("the orchestrator is not a team member").into());
    }
    let max = app.cfg.limits.max_members_per_team;
    let task_id = app.with_tracker(slug, |t| Ok(t.bus().get(team_id)?.task))?;
    let docs = app.with_tracker(slug, |t| t.get(&task_id)).ok().and_then(|task| crate::context::l1(app, slug, &task));
    app.with_tracker(slug, |t| {
        let team = t.bus().get(team_id)?;
        if team.state != TeamState::Active {
            return Err(GenieError::invalid(format!("team {team_id} is stopped")));
        }
        if team.members.len() >= max {
            return Err(GenieError::invalid(format!("limit: at most {max} members per team")));
        }
        let task = t.get(&team.task)?;
        let refinement = matches!(task.status, Status::Inbox | Status::Draft | Status::Refining);
        if refinement && !role.stages.contains(&Stage::Refinement) {
            return Err(GenieError::invalid(format!(
                "{} is not ready ({}); role {} works only on ready tasks",
                task.id, task.status, role.id
            )));
        }
        // Teams assembled before relations were configurable get a derived spec now.
        let mut spec = team.spec.as_ref().and_then(TeamSpec::from_value).unwrap_or_else(|| {
            let mut members = Vec::new();
            let mut classes = Vec::new();
            for x in &team.members {
                members.push(SpecMember {
                    key: crate::agent_config::free_key(&members, &x.role),
                    name: x.name.clone(),
                    role: x.role.clone(),
                });
                classes.push(agents.roles.get(&x.role).map(|r| r.class).unwrap_or(Role::Analyst));
            }
            TeamSpec { template: team.template.clone(), ..TeamSpec::derived(members, &classes, refinement) }
        });
        let mut taken = t.bus().taken_names()?;
        let name = m.name.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| pick_name(&role, &mut taken));
        let me = SpecMember { key: spec.free_key(&role.id), name: name.clone(), role: role.id.clone() };
        // Relations the process implies for the newcomer, next to the current ones.
        let mut keyed: Vec<(String, Role)> =
            spec.members.iter().map(|x| (x.key.clone(), agents.roles.get(&x.role).map(|r| r.class).unwrap_or(Role::Analyst))).collect();
        keyed.push((me.key.clone(), role.class));
        // Only the newcomer's own edges: a relation it merely shares (the executor
        // handing over to every reviewer) is narrowed to it, so nobody else is told twice.
        for r in crate::agent_config::default_relations(&keyed, refinement) {
            let r = if r.from == me.key {
                r
            } else if r.to.contains(&me.key) {
                Relation { to: vec![me.key.clone()], ..r }
            } else {
                continue;
            };
            if !spec.relations.contains(&r) {
                spec.relations.push(r);
            }
        }
        spec.members.push(me.clone());
        let updated = t.bus().add_member(
            team_id,
            NewMember {
                name: name.clone(),
                role: role.id.clone(),
                model: m.model.clone(),
                thinking: m.thinking.clone(),
                instructions: m.instructions.clone(),
            },
        )?;
        t.bus().set_spec(team_id, &spec.to_value())?;
        let k = Kickoff {
            team: team_id,
            task: &task,
            cwd: &team.cwd,
            worktree: team.worktree.as_ref(),
            spec: &spec,
            agents: &agents,
            note: None,
            epic: None,
            joining: true,
            docs: docs.as_deref(),
        };
        let bus = t.bus();
        bus.send(team::SendMail {
            team: team_id,
            from: ORCHESTRATOR,
            from_role: "orchestrator",
            to: &name,
            text: &k.text(&me),
            level: None,
            intent: None,
            kind: "kickoff",
            ..Default::default()
        })?;
        let note = format!("{} — {} joined the team (added by {by}).", team::display_name(&name), role.id);
        for other in updated.members.iter().filter(|x| x.name != name) {
            bus.send(team::SendMail {
                team: team_id,
                from: ORCHESTRATOR,
                from_role: "orchestrator",
                to: &other.name,
                text: &note,
                level: Some("low"),
                intent: Some("fyi"),
                kind: "system",
                ..Default::default()
            })?;
        }
        Ok(me)
    })
}

/// A member's first message, generated from the team's relations.
pub struct Kickoff<'a> {
    pub team: &'a str,
    pub task: &'a Task,
    pub cwd: &'a str,
    pub worktree: Option<&'a TeamWorktree>,
    pub spec: &'a TeamSpec,
    pub agents: &'a AgentConfig,
    pub note: Option<&'a str>,
    pub epic: Option<&'a Task>,
    /// Joining a running team rather than starting with it.
    pub joining: bool,
    /// Pages of the knowledge base chosen for the task (L1, see `crate::context`).
    pub docs: Option<&'a str>,
}

fn and_list(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        n => format!("{} and {}", items[..n - 1].join(", "), items[n - 1]),
    }
}

impl Kickoff<'_> {
    fn who(&self, key: &str) -> String {
        if key == crate::agent_config::ORCHESTRATOR {
            return "the orchestrator".into();
        }
        self.spec.by_key(key).map(|m| team::display_name(&m.name)).unwrap_or_else(|| key.to_string())
    }

    fn whom(&self, keys: &[String]) -> String {
        and_list(&keys.iter().map(|k| self.who(k)).collect::<Vec<_>>())
    }

    fn role_label(&self, role: &str) -> String {
        match self.agents.roles.get(role) {
            Some(r) if r.id != r.class.as_str() => format!("{} ({} class)", r.id, r.class),
            _ => role.to_string(),
        }
    }

    pub fn text(&self, me: &SpecMember) -> String {
        let task = self.task;
        let rels = &self.spec.relations;
        let note = |r: &Relation| r.note.as_deref().map(|n| format!(" ({n})")).unwrap_or_default();
        let roster = self
            .spec
            .members
            .iter()
            .map(|m| format!("{} — {} (`{}`)", team::display_name(&m.name), m.role, m.name))
            .collect::<Vec<_>>()
            .join(", ");
        let mut out = vec![
            format!(
                "{} team {}, {}! You are the {}; teammates address you as \"{}\". Task: {} — {} (status {}). Read it with `genie task show`.",
                if self.joining { "You are joining" } else { "Welcome to" },
                self.team,
                team::display_name(&me.name),
                self.role_label(&me.role),
                me.name,
                task.id,
                task.title,
                task.status
            ),
            match self.worktree {
                Some(w) => format!(
                    "Working directory: {} (branch {}, base {}).",
                    self.cwd,
                    w.branch,
                    w.base.as_deref().unwrap_or("").chars().take(10).collect::<String>()
                ),
                None => format!("Working directory: {}.", self.cwd),
            },
            format!("Team: {roster}, plus orchestrator."),
        ];
        if let Some(e) = self.epic {
            out.push(format!(
                "This task is part of epic {} — {}. Read its goal and shared artifacts (`genie task show {}`) before you start, and attach material useful for the whole epic to the epic itself.",
                e.id, e.title, e.id
            ));
        }
        // The idea planner talks with the owner, not with a team or the orchestrator.
        if self.spec.template.as_deref() == Some(crate::http::ideas::IDEA_TEMPLATE) {
            out.push(format!(
                "{} is an idea in the owner's own words. Start now: read it, look at what already exists, then ask the owner your first question in your reply. \
                 The owner reads your replies in the agent chat and answers by mail; do not mail the orchestrator. \
                 Keep your proposal in the `{}` artifact as your role describes: the owner applies it from the web.",
                task.id,
                crate::http::ideas::PLAN_ARTIFACT
            ));
            if let Some(d) = self.docs {
                out.push(format!("\n{d}"));
            }
            return out.join("\n");
        }
        if matches!(task.status, Status::Inbox | Status::Draft | Status::Refining) {
            out.push(format!(
                "The task is not ready yet (status {}): this team clarifies it — scope, a precise description and verifiable acceptance criteria (`genie task update`), findings as an `analysis` artifact. Nobody implements.",
                task.status
            ));
        }
        let mut how = Vec::new();
        let inbound: Vec<&Relation> = rels.iter().filter(|r| r.kind == RelKind::Handoff && r.to.contains(&me.key)).collect();
        if inbound.is_empty() {
            how.push("Start now.".to_string());
        } else {
            let from: Vec<String> = inbound.iter().map(|r| format!("{}{}", self.who(&r.from), note(r))).collect();
            let auto: Vec<String> = inbound.iter().filter_map(|r| r.on.map(|s| s.to_string())).collect();
            how.push(format!(
                "You start after {} hand{} over: until then set a waiting status (`genie team set-status`) and end your turn without messaging anyone.{}",
                and_list(&from),
                if from.len() == 1 { "s" } else { "" },
                if auto.is_empty() { String::new() } else { format!(" genie tells you when the task moves to {}.", and_list(&auto)) }
            ));
        }
        for r in rels.iter().filter(|r| r.from == me.key) {
            let to = self.whom(&r.to);
            match (r.kind, r.on) {
                (RelKind::Handoff, Some(st)) => how.push(format!(
                    "When your step is done{}, move the task to {st} with a note (`genie task status {st} --note …`): genie passes it to {to}.",
                    note(r)
                )),
                (RelKind::Handoff, None) => {
                    how.push(format!("When your step is done{}, hand over to {to} (`genie mail send <name> … --intent done`).", note(r)))
                }
                (RelKind::Returns, Some(st)) => how.push(format!(
                    "If the work does not meet the criteria, return it to {to}: move the task to {st} with a note{}; genie tells them.",
                    note(r)
                )),
                (RelKind::Returns, None) => how.push(format!("If the work does not meet the criteria, send {to} your findings{} directly.", note(r))),
                (RelKind::Consults, _) => how.push(format!("Ask {to} directly{}: `genie mail ask <name> \"…\"` waits for the answer.", note(r))),
                (RelKind::Reports, _) => {}
            }
        }
        for r in rels.iter().filter(|r| r.to.contains(&me.key)) {
            let from = self.who(&r.from);
            match r.kind {
                RelKind::Returns => how.push(format!(
                    "{from} may return the work to you{}{}: fix it and hand over again.",
                    note(r),
                    r.on.map(|s| format!("; genie tells you when the task moves to {s}")).unwrap_or_default()
                )),
                RelKind::Consults => how.push(format!("{from} may ask you questions{}: answer promptly.", note(r))),
                _ => {}
            }
        }
        let reporters: Vec<&Relation> = rels.iter().filter(|r| r.kind == RelKind::Reports).collect();
        let mine: Vec<String> = reporters.iter().filter(|r| r.from == me.key).filter_map(|r| r.note.clone()).collect();
        if reporters.iter().any(|r| r.from == me.key) {
            how.push(format!(
                "You are the team's voice to the orchestrator: report {} (`genie mail send orchestrator … --intent verdict` or `done`).",
                if mine.is_empty() { "the result".to_string() } else { and_list(&mine) }
            ));
        } else if reporters.is_empty() {
            how.push("Report your result to the orchestrator yourself (`genie mail send orchestrator … --intent done`).".into());
        } else {
            let mut voices: Vec<String> = reporters.iter().map(|r| self.who(&r.from)).collect();
            voices.dedup();
            how.push(format!(
                "{} {} the team's voice to the orchestrator; message the orchestrator yourself only with a scope question or a blocker.",
                and_list(&voices),
                if voices.len() == 1 { "is" } else { "are" }
            ));
        }
        if self.spec.mail == MailMode::Flow {
            let targets: Vec<String> = self.spec.flow_targets(&me.key).iter().map(|m| m.name.clone()).collect();
            how.push(format!(
                "Mail follows the template's route: you may write to {}{}; answer questions with `genie mail reply <id>`. genie refuses other mail.",
                if targets.is_empty() { "no teammate".to_string() } else { and_list(&targets) },
                if self.spec.is_voice(&me.key) {
                    " and the orchestrator"
                } else {
                    ", and to the orchestrator only questions and blockers (`--intent question` or `blocker`)"
                }
            ));
        }
        out.push(format!(
            "How the team works{}:\n{}",
            self.spec.title.as_deref().map(|t| format!(" (template \"{t}\")")).unwrap_or_default(),
            how.iter().map(|h| format!("- {h}")).collect::<Vec<_>>().join("\n")
        ));
        if let Some(c) = self.spec.charter.as_deref().filter(|c| !c.trim().is_empty()) {
            out.push(format!("Team rules: {}", c.trim()));
        }
        if let Some(n) = self.note.filter(|n| !n.trim().is_empty()) {
            out.push(format!("\nFrom {}: {n}", if self.joining { "whoever added you" } else { "the orchestrator" }));
        }
        if let Some(d) = self.docs {
            out.push(format!("\n{d}"));
        }
        out.join("\n")
    }
}

/// Stop a team: members stop receiving turns, agent tokens are revoked, the task is released.
pub fn stop_team(app: &App, slug: &str, team: &str, reason: &str, by: &str) -> AppResult<Vec<String>> {
    let mut report = Vec::new();
    app.with_tracker(slug, |t| {
        let tm = t.bus().get(team)?;
        t.bus().set_state(team, TeamState::Stopped, Some(reason), by)?;
        report.push(format!("team {team} stopped ({reason})"));
        if let Ok(task) = t.get(&tm.task)
            && task.team.as_deref() == Some(team)
            && !CLOSED.contains(&task.status)
        {
            t.assign_team(&Actor::new(by, Role::Orchestrator), &task.id, None, None, None)?;
            report.push(format!("{} released", task.id));
        }
        if reason == "owner" {
            t.bus().notify_orchestrator(by, "human", "owner", &format!("The owner ({by}) stopped team {team}."), Some(&tm.task))?;
        }
        Ok(())
    })?;
    app.with_server(|db| db.revoke_agent_tokens(slug, Some(team), None))?;
    for s in app.sessions.all() {
        if s.key.project() == slug && s.team.as_deref() == Some(team) {
            crate::sessions::reset(app, &s.key);
        }
    }
    Ok(report)
}

/// Stop teams whose task is closed (from blocking code) and free what the closed
/// tasks no longer need: worktrees whose branch is in the main one go, with the
/// branch (G-134 — a closed task used to keep its 17 GB `target/` forever).
pub fn reap_closed_blocking(app: &App, slug: &str) -> AppResult<Vec<String>> {
    let closed: Vec<String> = app.with_tracker(slug, |t| {
        let mut out = Vec::new();
        for team in t.bus().list(false)? {
            if t.get(&team.task).map(|task| CLOSED.contains(&task.status)).unwrap_or(true) {
                out.push(team.id);
            }
        }
        Ok(out)
    })?;
    for team in &closed {
        stop_team(app, slug, team, "task_closed", "genie")?;
    }
    let mut report: Vec<String> = closed.iter().map(|t| format!("team {t} stopped (task_closed)")).collect();
    report.extend(sweep_worktrees(app, slug));
    Ok(report)
}

/// Worktrees of closed tasks (`done`, `cancelled`): a git worktree goes when its
/// branch is in the repository's main branch (the branch goes too); a workspace
/// under the data directory goes as it is (its work is on the git host). Work not
/// in the main branch yet stays — the report says so. Teams still running on a
/// closed task are skipped (`reap_closed_blocking` stops them first).
pub fn sweep_worktrees(app: &App, slug: &str) -> Vec<String> {
    let Ok(ids) = app.with_tracker(slug, |t| {
        let f = genie_core::ListFilter { status: vec![Status::Done, Status::Cancelled], include_closed: true, ..Default::default() };
        t.list(&f).map(|list| list.into_iter().map(|s| s.id).collect::<Vec<_>>())
    }) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for id in ids {
        let Some(task) = app.with_tracker(slug, |t| t.get(&id)).ok() else { continue };
        let Some(w) = task.worktree.clone() else { continue };
        // A live team still works out of it (closing a task stops the team first;
        // a race here means the sweep takes it on the next pass).
        if let Some(team) = &task.team
            && app.with_tracker(slug, |t| Ok(t.bus().get(team).map(|tm| tm.state == TeamState::Active).unwrap_or(false))).unwrap_or(false)
        {
            continue;
        }
        // A workspace (clones of hosted repositories): the work is on the host.
        if Path::new(&w.path).starts_with(app.data.join("workspaces")) {
            match std::fs::remove_dir_all(&w.path) {
                Ok(()) => {
                    let _ = app.with_tracker(slug, |t| t.clear_worktree(&task.id));
                    out.push(format!("{}: workspace {} removed", task.id, w.path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let _ = app.with_tracker(slug, |t| t.clear_worktree(&task.id));
                }
                Err(e) => out.push(format!("{}: workspace {} not removed: {e}", task.id, w.path)),
            }
            continue;
        }
        // A git worktree of the project's repository: only merged work goes.
        let Some(branch) = w.branch.as_deref().filter(|b| !b.is_empty()) else {
            out.push(format!("{}: worktree {} kept (no branch recorded)", task.id, w.path));
            continue;
        };
        let Some(repo) = app.with_server(|db| db.project(slug)).ok().and_then(|p| p.repo) else {
            out.push(format!("{}: worktree {} kept (the project has no repository)", task.id, w.path));
            continue;
        };
        let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&repo).args(args).output();
        // Merged into the main branch (the repository's HEAD)? — `git branch -d`
        // repeats the same check before deleting, so the two cannot disagree.
        let merged = git(&["merge-base", "--is-ancestor", branch, "HEAD"]).map(|o| o.status.success()).unwrap_or(false);
        if !merged {
            out.push(format!("{}: worktree kept, {branch} is not in the main branch yet", task.id));
            continue;
        }
        let mut removed = Vec::new();
        if Path::new(&w.path).exists() {
            match git(&["worktree", "remove", "--force", &w.path]) {
                Ok(o) if o.status.success() => removed.push(format!("worktree {} removed", w.path)),
                Ok(o) => out.push(format!("{}: worktree not removed: {}", task.id, String::from_utf8_lossy(&o.stderr).trim())),
                Err(e) => out.push(format!("{}: worktree not removed: {e}", task.id)),
            }
        }
        match git(&["branch", "-d", branch]) {
            Ok(o) if o.status.success() => removed.push(format!("branch {branch} deleted")),
            // Not there anymore, or git sees unmerged work after all: not an error.
            _ => removed.push(format!("branch {branch} kept")),
        }
        if !removed.is_empty() {
            let _ = app.with_tracker(slug, |t| t.clear_worktree(&task.id));
            out.push(format!("{}: {}", task.id, removed.join(", ")));
        }
    }
    out
}

/// Stop teams whose task is closed.
pub async fn reap_closed(app: &Arc<App>, project: &str) {
    let slug = project.to_string();
    let res = app.blocking(move |app| reap_closed_blocking(app, &slug)).await;
    if let Err(e) = res {
        eprintln!("genie runtime: reap {project}: {e}");
    }
}

/// Remove a team's worktree or workspace (its branches stay); returns what happened, for a report.
pub fn remove_worktree(app: &App, slug: &str, team: &str) -> String {
    let Ok(Some(w)) = app.with_tracker(slug, |t| Ok(t.bus().get(team)?.worktree)) else { return "no worktree".into() };
    // A workspace of the project's repositories (clones of the server's mirrors, not git worktrees):
    // its work is on the git host, so the directory can go.
    if std::path::Path::new(&w.path).starts_with(app.data.join("workspaces")) {
        return match std::fs::remove_dir_all(&w.path) {
            Ok(()) => format!("workspace {} removed (branches stay on the git host)", w.path),
            Err(e) => format!("workspace {} not removed: {e}", w.path),
        };
    }
    let out = std::process::Command::new("git").args(["-C", &w.path, "worktree", "remove", "--force", &w.path]).output();
    match out {
        Ok(o) if o.status.success() => format!("worktree {} removed (branch {} kept)", w.path, w.branch),
        Ok(o) => format!("worktree {} not removed: {}", w.path, String::from_utf8_lossy(&o.stderr).trim()),
        Err(e) => format!("worktree {} not removed: {e}", w.path),
    }
}

/// A system notice to the project's orchestrator about `task`; the orchestrator is woken for it.
pub fn tell_orchestrator(app: &App, project: &str, task: Option<&str>, text: &str) -> AppResult<()> {
    app.with_tracker(project, |t| t.bus().notify_orchestrator("genie", "system", "system", text, task))?;
    app.wake_runtime.notify_one();
    Ok(())
}

/// A note in the agent's own mailbox (a member's, or the orchestrator's): it reads it on its next step.
pub fn note_to_self(app: &App, k: &AgentKey, text: &str) -> AppResult<()> {
    app.with_tracker(k.project(), |t| match k {
        AgentKey::Member { team, member, .. } => t
            .bus()
            .send(genie_core::team::SendMail {
                team,
                from: "genie",
                from_role: "system",
                to: member,
                text,
                level: Some("high"),
                kind: "system",
                ..Default::default()
            })
            .map(|_| ()),
        _ => t.bus().notify_orchestrator("genie", "system", "system", text, None),
    })?;
    app.wake_runtime.notify_one();
    Ok(())
}

/// Let a member in `error` work again (its kept mail is offered on the next tick).
pub fn restart_member(app: &App, slug: &str, team: &str, member: &str) -> AppResult<()> {
    app.with_tracker(slug, |t| t.bus().member_restarted(team, member))?;
    let key = AgentKey::Member { project: slug.to_string(), team: team.to_string(), member: member.to_string() };
    crate::sessions::reset(app, &key);
    app.wake_runtime.notify_one();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_heap_cap_joins_node_options_without_replacing_them() {
        assert_eq!(heap_cap(None, 2048).as_deref(), Some("--max-old-space-size=2048"));
        assert_eq!(heap_cap(Some("  "), 512).as_deref(), Some("--max-old-space-size=512"));
        assert_eq!(heap_cap(Some("--no-warnings"), 2048).as_deref(), Some("--no-warnings --max-old-space-size=2048"));
        assert_eq!(heap_cap(Some("--no-warnings"), 0), None, "0 is no cap");
        assert_eq!(heap_cap(Some("--max-old-space-size=8192"), 2048), None, "the operator's own cap stands");
    }

    #[test]
    fn a_connections_tools_reach_the_model_as_the_grant_and_the_admin_say() {
        let run = |entry: serde_json::Value, granted: Option<&[&str]>| {
            let mut o = entry.as_object().unwrap().clone();
            let granted: Option<Vec<String>> = granted.map(|g| g.iter().map(|s| s.to_string()).collect());
            let codemode = expose(&mut o, granted.as_deref());
            (serde_json::Value::Object(o), codemode)
        };
        // Some tools: those are declared, the rest of the connection is hidden.
        let (e, cm) = run(json!({ "url": "http://x" }), Some(&["get_*", "list_?"]));
        assert_eq!(
            (e["exposure"].clone(), e["toolExposure"].clone(), cm),
            (json!("hidden"), json!({ "get_*": "direct", "list_*": "direct" }), false)
        );
        // A whole connection is searched for.
        assert_eq!(run(json!({ "url": "http://x" }), None).0["exposure"], "deferred");
        // The admin's choice stands, and codemode is switched on only for it.
        let (e, cm) = run(json!({ "url": "http://x", "exposure": "codemode" }), None);
        assert_eq!((e["exposure"].clone(), cm), (json!("codemode"), true));
        let (e, cm) = run(json!({ "url": "http://x", "exposure": "deferred", "toolExposure": { "get_*": "codemode" } }), None);
        assert_eq!((e["exposure"].clone(), cm), (json!("deferred"), true));
        let (e, cm) = run(json!({ "url": "http://x", "exposure": "deferred" }), Some(&["get_*"]));
        assert_eq!((e["toolExposure"].clone(), cm), (json!({ "get_*": "deferred" }), false), "also for the granted tools");
    }

    #[tokio::test]
    async fn a_turn_and_a_session_launch_the_same_agent_but_for_what_they_are_given() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = crate::config::Config::load(dir.path()).unwrap();
        cfg.runtime.enabled = false;
        cfg.runtime.sandbox.mode = "off".into();
        let template = |last: &str| -> Vec<Vec<String>> {
            vec![
                vec!["harness".into()],
                vec!["--session".into(), "{sessionId}".into()],
                vec!["--model".into(), "{model}".into()],
                vec!["--prompt".into(), "{promptFile}".into()],
                vec![last.into()],
            ]
        };
        cfg.runtime.command = template("{message}");
        cfg.runtime.session_command = template("{extension}");
        let app = App::open(dir.path(), cfg, PathBuf::from("/nonexistent")).unwrap();
        app.create_project("shop", "Shop", None, None, None).unwrap();
        let t = app
            .with_tracker("shop", |t| {
                t.create(&Actor::new("anna", Role::Human), genie_core::CreateInput { title: "CSV export".into(), ..Default::default() })?;
                t.bus().create(
                    "anna",
                    "human",
                    NewTeam {
                        id: "G-1".into(),
                        task: "G-1".into(),
                        cwd: ".".into(),
                        members: vec![NewMember {
                            name: "bender".into(),
                            role: "executor".into(),
                            model: Some("m-1".into()),
                            ..Default::default()
                        }],
                        ..Default::default()
                    },
                )?;
                t.bus().get("G-1")
            })
            .unwrap();
        let project = project_of(&app, "shop").unwrap();
        let key = AgentKey::Member { project: "shop".into(), team: "G-1".into(), member: "bender".into() };
        let spec = member_spec(&app, &project, &t, &t.members[0], false).unwrap();
        assert_eq!(
            (spec.name.as_str(), spec.team.as_deref(), spec.task.as_deref(), spec.model.as_deref()),
            ("bender", Some("G-1"), Some("G-1"), Some("m-1"))
        );
        assert_eq!(spec.session_id, "g-1-bender");
        let llm = crate::llm_key::Key::Server;
        let argv =
            |cmd: &tokio::process::Command| -> Vec<String> { cmd.as_std().get_args().map(|a| a.to_string_lossy().into_owned()).collect() };
        let (turn, turn_dir) = agent_process(&app, &key, &spec, Mode::Turn { message: "two letters" }, "tok", &llm).await.unwrap();
        let extension = PathBuf::from("/x/genie-bus.ts");
        let (session, session_dir) = agent_process(&app, &key, &spec, Mode::Session { extension: &extension }, "tok", &llm).await.unwrap();
        assert_eq!(turn_dir, session_dir, "one runtime directory per agent");
        assert_eq!(turn_dir, agent_dir(&app, &key));
        let prompt = turn_dir.join("prompt.md").to_string_lossy().into_owned();
        let common = ["--session", "g-1-bender", "--model", "m-1", "--prompt", prompt.as_str()];
        assert_eq!(argv(&turn), [&common[..], &["two letters"]].concat());
        assert_eq!(argv(&session), [&common[..], &["/x/genie-bus.ts"]].concat());
        assert!(std::fs::read_to_string(&prompt).unwrap().contains("bender"), "the prompt is written for the agent");
    }

    #[test]
    fn optional_groups_disappear_when_empty() {
        let groups: Vec<Vec<String>> = vec![
            vec!["pi".into(), "--print".into()],
            vec!["--model".into(), "{model}".into()],
            vec!["--session-id".into(), "{sessionId}".into()],
            vec!["{message}".into()],
        ];
        let vars = HashMap::from([("model", String::new()), ("sessionId", "s1".to_string()), ("message", "hi {model}".to_string())]);
        assert_eq!(build_command(&groups, &vars, &HashMap::new()), vec!["pi", "--print", "--session-id", "s1", "hi {model}"]);
    }

    #[test]
    fn list_placeholders_repeat_their_group_and_conditions_keep_it() {
        let g = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let groups = vec![g(&["pi"]), g(&["--no-skills", "{?limitSkills}"]), g(&["--skill", "{skill}"]), g(&["-e", "{guard}"])];
        let mut vars = HashMap::from([("limitSkills", "yes".to_string()), ("guard", "/g.ts".to_string())]);
        let mut lists = HashMap::from([("skill", vec!["/s/a".to_string(), "/s/b".to_string()])]);
        assert_eq!(build_command(&groups, &vars, &lists), vec!["pi", "--no-skills", "--skill", "/s/a", "--skill", "/s/b", "-e", "/g.ts"]);
        // A role without its own skill list: no --no-skills; no skill directories: no --skill.
        vars.insert("limitSkills", String::new());
        lists.insert("skill", Vec::new());
        assert_eq!(build_command(&groups, &vars, &lists), vec!["pi", "-e", "/g.ts"]);
        // An unknown placeholder stays as text.
        assert_eq!(build_command(&[g(&["{?nope}", "{x}"])], &vars, &lists), vec!["{?nope}", "{x}"]);
    }
}
