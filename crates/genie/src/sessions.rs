//! Live agent sessions: every team member and each project's orchestrator runs as
//! one long-lived harness process — pi in RPC mode (`pi --mode rpc`) with the
//! genie-bus extension — instead of one process per turn.
//!
//! - **Delivery.** The extension pulls the agent's mail at every step boundary
//!   (after a step's tool calls, before the next model request) and acknowledges
//!   it when the model sees it, so mail reaches a busy agent within its current
//!   step. An idle session is woken with the `/genie-mail` command.
//! - **Interrupt.** Mail at level `interrupt` aborts the running step (RPC
//!   `abort`, which also stops a running shell command); the settled session is
//!   then woken and gets the interrupt first.
//! - **Supervision.** The event stream keeps a live view of each agent (state,
//!   running tool, last words, recent activity) for `peek`, the board and the
//!   web. A crashed process gives back its unacknowledged mail and is restarted
//!   with the same conversation; a failed run is retried with backoff and after
//!   `maxAttempts` the agent is put in `error` and the orchestrator is told. A
//!   step without any sign of life for `turnTimeoutSecs` is aborted; an idle
//!   session is stopped after `idleStopSecs` and resumed on its next mail. After a
//!   server restart the interrupted turns' agents are told to continue
//!   (`resume_after_restart`): their mail was acknowledged in the lost step, so
//!   nothing is pending and no new run would start by itself.
//! - **Runs** (agent start → settled) are recorded as turns, so history and logs
//!   look the same as for turn-based harnesses.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use genie_core::db::now;
use genie_core::team::{ORCHESTRATOR, QuietTeam};
use genie_core::work::Turn;
use genie_core::{Role, Status};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::outcome::{self, Outcome};
use crate::runtime::{self, AgentKey};
use crate::state::{App, AppError, AppResult};

/// The pi extension that delivers mail into a session (written to `<data>/runtime`).
pub const EXTENSION: &str = include_str!("../pi/genie-bus.ts");
/// The pi extension that keeps every agent (sessions and turns) within its role.
pub const GUARD: &str = include_str!("../pi/genie-guard.ts");

/// Activity lines kept per session for `peek`.
const RECENT: usize = 30;
/// A wake-up is repeated only after this long (the session may be starting).
const NUDGE_AGAIN: Duration = Duration::from_secs(5);
/// Deliveries not acknowledged after this long are offered again.
const STALE_DELIVERY: Duration = Duration::from_secs(120);
/// Same tool call this many times in a row: the orchestrator is told the agent may loop.
const LOOP_REPEATS: u32 = 5;

#[derive(Default)]
pub struct Registry {
    sessions: Mutex<HashMap<AgentKey, Arc<Session>>>,
}

impl Registry {
    pub fn get(&self, key: &AgentKey) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get(key).cloned()
    }
    pub fn all(&self) -> Vec<Arc<Session>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
    }
    fn insert(&self, s: Arc<Session>) {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).insert(s.key.clone(), s);
    }
    /// Remove the session if it is still this process.
    fn remove(&self, key: &AgentKey, pid: u32) {
        let mut m = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        if m.get(key).is_some_and(|s| s.pid == pid) {
            m.remove(key);
        }
    }
}

/// What a session is doing, as seen from its event stream.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Live {
    /// `starting` | `idle` | `working` | `stopping`
    pub state: String,
    /// Since when (RFC 3339) the session is in this state.
    pub since: String,
    pub pid: u32,
    pub started: String,
    /// The tool call running now: `{name, args, since}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_thinking: Option<String>,
    /// Recent activity, oldest first (`HH:MM:SS text`).
    pub recent: VecDeque<String>,
    /// Runs since the process started.
    pub runs: u64,
    /// Consecutive failed runs.
    pub failures: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Input tokens of the latest model request (the size of the context).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_tokens: Option<u64>,
    #[serde(skip)]
    last_event: Instant,
    #[serde(skip)]
    state_at: Instant,
    #[serde(skip)]
    nudged: Option<Instant>,
    #[serde(skip)]
    interrupting: bool,
    #[serde(skip)]
    stall_abort: Option<Instant>,
    #[serde(skip)]
    turn: Option<i64>,
    #[serde(skip)]
    last_stop: Option<String>,
    #[serde(skip)]
    repeat: (String, u32),
    #[serde(skip)]
    tool_at: Option<Instant>,
    /// Start again once the running step ends (the model changed).
    #[serde(skip)]
    reload: bool,
}

impl Live {
    fn new(pid: u32) -> Live {
        let t = now();
        Live {
            state: "starting".into(),
            since: t.clone(),
            pid,
            started: t,
            tool: None,
            last_text: None,
            last_thinking: None,
            recent: VecDeque::new(),
            runs: 0,
            failures: 0,
            last_error: None,
            context_tokens: None,
            last_event: Instant::now(),
            state_at: Instant::now(),
            nudged: None,
            interrupting: false,
            stall_abort: None,
            turn: None,
            last_stop: None,
            repeat: (String::new(), 0),
            tool_at: None,
            reload: false,
        }
    }
    fn set_state(&mut self, state: &str) {
        if self.state != state {
            self.state = state.into();
            self.since = now();
            self.state_at = Instant::now();
        }
    }
    fn note(&mut self, line: String) {
        let time = now().get(11..19).unwrap_or_default().to_string();
        self.recent.push_back(format!("{time} {line}"));
        while self.recent.len() > RECENT {
            self.recent.pop_front();
        }
    }
}

pub struct Session {
    pub key: AgentKey,
    pub pid: u32,
    pub role: Role,
    /// The configured role (its rules are rewritten when the configuration changes).
    pub role_id: String,
    pub name: String,
    pub team: Option<String>,
    pub task: Option<String>,
    /// The person it works for: its `LITELLM_API_KEY` is theirs ([`crate::llm_key`]).
    pub initiator: Option<String>,
    token: String,
    stdin: Mutex<Option<mpsc::UnboundedSender<String>>>,
    replies: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    seq: AtomicU64,
    live: Mutex<Live>,
    dir: PathBuf,
}

impl Session {
    pub fn live(&self) -> Live {
        self.live.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    fn with_live<T>(&self, f: impl FnOnce(&mut Live) -> T) -> T {
        f(&mut self.live.lock().unwrap_or_else(|e| e.into_inner()))
    }
    fn send(&self, cmd: Value) -> bool {
        match &*self.stdin.lock().unwrap_or_else(|e| e.into_inner()) {
            Some(tx) => tx.send(format!("{cmd}\n")).is_ok(),
            None => false,
        }
    }
    /// An RPC command with a response (`get_messages`, `get_state`…).
    pub async fn request(&self, mut cmd: Value, timeout: Duration) -> Option<Value> {
        let id = format!("g{}", self.seq.fetch_add(1, Ordering::Relaxed));
        cmd["id"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.replies.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), tx);
        if !self.send(cmd) {
            self.replies.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            return None;
        }
        let out = tokio::time::timeout(timeout, rx).await.ok().and_then(Result::ok);
        self.replies.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
        out
    }
    /// Wake an idle session: the extension takes the pending mail.
    fn nudge(&self) {
        let again = self.with_live(|l| l.nudged.is_none_or(|t| t.elapsed() >= NUDGE_AGAIN));
        if again && self.send(json!({ "type": "prompt", "message": "/genie-mail" })) {
            self.with_live(|l| l.nudged = Some(Instant::now()));
        }
    }
    /// Stop the running step (and a running shell command).
    fn abort(&self, why: &str) {
        if self.send(json!({ "type": "abort" })) {
            self.with_live(|l| l.note(format!("■ aborted: {why}")));
        }
    }
    /// Orderly shutdown: close stdin; the process is killed if it does not exit.
    fn stop(self: &Arc<Self>, why: &str) {
        self.with_live(|l| {
            l.set_state("stopping");
            l.note(format!("■ stopping: {why}"));
        });
        self.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
        let pid = self.pid;
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(async move {
                tokio::time::sleep(Duration::from_secs(10)).await;
                kill(pid, "-KILL");
            });
        }
    }
    fn is(&self, state: &str) -> bool {
        self.with_live(|l| l.state == state)
    }
}

fn kill(pid: u32, signal: &str) {
    if std::path::Path::new(&format!("/proc/{pid}")).exists() {
        let _ = std::process::Command::new("kill").arg(signal).arg(pid.to_string()).stderr(Stdio::null()).status();
    }
}

/// Where the pi extension is written for sessions to load.
pub fn extension_path(app: &App) -> PathBuf {
    app.data.join("runtime").join("genie-bus.ts")
}

pub fn write_extension(app: &App) -> std::io::Result<PathBuf> {
    write_runtime_file(extension_path(app), EXTENSION)
}

/// The guard extension, written for agents to load.
pub fn write_guard(app: &App) -> std::io::Result<PathBuf> {
    write_runtime_file(app.data.join("runtime").join("genie-guard.ts"), GUARD)
}

fn write_runtime_file(path: PathBuf, text: &str) -> std::io::Result<PathBuf> {
    if std::fs::read_to_string(&path).ok().as_deref() != Some(text) {
        std::fs::create_dir_all(path.parent().expect("runtime dir"))?;
        std::fs::write(&path, text)?;
    }
    Ok(path)
}

/// Rewrite the guard rules of running sessions from the current configuration, so
/// a changed rule (a revoked MCP connection, a new denied command) applies at once.
pub fn refresh_policies(app: &App) {
    let agents = app.agents();
    for s in app.sessions.all() {
        let Some(role) = agents.roles.get(&s.role_id) else { continue };
        let policy = runtime::policy(&agents, s.key.project(), role, role.files);
        let text = serde_json::to_string_pretty(&policy).unwrap_or_default();
        if let Err(e) = runtime::write_private(&s.dir.join("policy.json"), &text) {
            eprintln!("genie runtime: {}: rules of {}: {e}", s.key.project(), s.key.label());
        }
    }
}

fn session_dir(app: &App, key: &AgentKey) -> PathBuf {
    app.data.join("runtime").join(key.project()).join(key.label().replace('/', "_"))
}

/// Stop session processes left by a previous server run (their pid files).
pub fn recover(app: &App) {
    let Ok(projects) = std::fs::read_dir(app.data.join("runtime")) else { return };
    for p in projects.flatten().filter(|p| p.path().is_dir()) {
        let project = p.file_name().to_string_lossy().into_owned();
        let Ok(agents) = std::fs::read_dir(p.path()) else { continue };
        for a in agents.flatten() {
            let pid_file = a.path().join("session.pid");
            let Ok(raw) = std::fs::read_to_string(&pid_file) else { continue };
            let mut parts = raw.split_whitespace();
            let (Some(pid), Some(name)) = (parts.next().and_then(|x| x.parse::<i64>().ok()), parts.next()) else { continue };
            if runtime::is_our_agent(pid, &project, name) {
                kill(pid as u32, "-TERM");
                println!("genie runtime: stopped stray session process {pid} ({project}/{name})");
            }
            let _ = std::fs::remove_file(pid_file);
        }
    }
}

/// The note a restarted server leaves its stranded agents. Same wording family as the note the
/// agent-crash path writes (`on_exit`): a letter is what starts a run.
const RESTART_NOTE: &str = "[genie] The genie server restarted after a crash. Continue where you left off.";

/// After a restart, resume the agents whose turns were interrupted by a server crash.
///
/// The extension acknowledges a delivery at the request where the model is about to see it, so a
/// `kill -9` of the server leaves the mail `delivered` while its turn is still running
/// (`release_all_leases` then finds nothing to offer again). The scheduler starts a session only
/// for a mailbox with *pending* mail, and mail the session already holds is settled rather than
/// injected — so nothing would ever start the agent again and the board would show a `working`
/// member without a process. One letter fixes both: the same note the agent-crash path writes.
///
/// No-op in `turns` mode: there the interrupted turn released its lease and its mail runs again.
/// Jobs are skipped — `requeue_running_jobs` already re-runs them.
///
/// The already acknowledged mail is *not* offered again: it is in the resumed conversation, and
/// re-delivering it would duplicate it (requirement 2 of G-132).
pub fn resume_after_restart(app: &App, turns: &[Turn]) {
    if !app.cfg.runtime.live_sessions() {
        return;
    }
    let mut told = 0;
    for t in turns.iter().filter(|t| t.job.is_none()) {
        let key = match (&t.member, &t.team) {
            (Some(member), Some(team)) => AgentKey::Member { project: t.project.clone(), team: team.clone(), member: member.clone() },
            _ => AgentKey::Orchestrator { project: t.project.clone() },
        };
        // The board must not report an agent without a process as busy. `error` is kept: a member
        // that gave up stays given up until someone restarts it.
        if let AgentKey::Member { project, team, member } = &key {
            let _ = app.with_tracker(project, |t| t.bus().member_idle(team, member));
        }
        // An earlier crash note nobody read is enough: a server restarted in a loop adds none.
        if note_pending(app, &key) {
            continue;
        }
        match runtime::note_to_self(app, &key, RESTART_NOTE) {
            Ok(()) => told += 1,
            Err(e) => eprintln!("genie runtime: {}: cannot tell {} to continue: {e}", key.project(), key.label()),
        }
    }
    if told > 0 {
        println!("genie runtime: {told} agent(s) told to continue after the restart");
    }
}

/// Does the mailbox already hold an unread letter from a system (an earlier restart note)?
fn note_pending(app: &App, key: &AgentKey) -> bool {
    let (team, recipient) = mailbox(key);
    app.with_tracker(key.project(), |t| Ok(t.bus().pending(team.as_deref(), &recipient)?.iter().any(|m| m.kind == "system")))
        .unwrap_or(false)
}

/// Mail is waiting for `key`: wake its session, or start one.
pub async fn deliver(app: &Arc<App>, key: &AgentKey) {
    if let Some(s) = app.sessions.get(key) {
        if s.is("idle") && app.attempts.failures(key) < app.cfg.runtime.max_attempts.max(1) {
            // The orchestrator answers whoever wrote: another person's mail restarts
            // it (the same conversation) with that person's key.
            if let AgentKey::Orchestrator { project } = key {
                let slug = project.clone();
                let now = app.blocking(move |app| Ok(crate::llm_key::orchestrator_initiator(app, &slug))).await.ok().flatten();
                if now.is_some() && now != s.initiator {
                    s.stop("the mail is from another person");
                    return;
                }
            }
            s.nudge();
        }
        return;
    }
    if !app.attempts.may_start(key) {
        return;
    }
    let live = app.sessions.all();
    if live.len() >= app.cfg.runtime.max_sessions.max(1) {
        // Make room: stop the session idle the longest; this agent starts on a later pass.
        if let Some(oldest) = live.iter().filter(|s| s.is("idle")).max_by_key(|s| s.with_live(|l| l.state_at.elapsed())) {
            oldest.stop("making room for another agent");
        }
        return;
    }
    match start(app, key).await {
        Ok(Some(s)) => s.nudge(),
        Ok(None) => {}
        Err(e) => {
            let n = app.attempts.failed_to_start(key);
            eprintln!("genie runtime: {}: cannot start a session for {} (attempt {n}): {e}", key.project(), key.label());
        }
    }
}

/// Launch the session process for `key` (`None`: nothing to run).
async fn start(app: &Arc<App>, key: &AgentKey) -> AppResult<Option<Arc<Session>>> {
    let k = key.clone();
    let Some(spec) = app.blocking(move |app| runtime::session_spec(app, &k)).await? else { return Ok(None) };
    let dir = session_dir(app, key);
    tokio::fs::create_dir_all(&dir).await.map_err(|e| AppError::Internal(e.to_string()))?;
    let prompt_file = dir.join("prompt.md");
    tokio::fs::write(&prompt_file, &spec.prompt).await.map_err(|e| AppError::Internal(e.to_string()))?;
    let extension = write_extension(app).map_err(|e| AppError::Internal(e.to_string()))?;
    let files = spec.kit.write(app, &dir, &spec.role_id).map_err(|e| AppError::Internal(e.to_string()))?;
    let sessions = app.data.join("sessions").join(key.project());
    tokio::fs::create_dir_all(&sessions).await.map_err(|e| AppError::Internal(e.to_string()))?;
    let (slug, label, initiator, model) = (key.project().to_string(), key.label(), spec.initiator.clone(), spec.model.clone());
    let llm = app
        .blocking(move |app| {
            crate::llm_key::resolve(app, &slug, &label, initiator.as_deref(), model.as_deref())
                .map_err(|e| genie_core::GenieError::invalid(e).into())
        })
        .await?;
    let (slug, role, role_id, name, team) =
        (key.project().to_string(), spec.role, spec.role_id.clone(), spec.name.clone(), spec.team.clone());
    let token = app
        .blocking(move |app| {
            app.with_server(|db| {
                db.create_role_token(&slug, role, Some(&role_id), &name, team.as_deref(), None, chrono::Duration::days(30))
            })
        })
        .await?;
    let readonly = spec.readonly.clone();
    let mut vars: HashMap<&str, String> = HashMap::from([
        ("sessionDir", sessions.to_string_lossy().into_owned()),
        ("sessionId", spec.session_id.clone()),
        ("model", spec.model.clone().unwrap_or_default()),
        ("thinking", spec.thinking.clone().unwrap_or_default()),
        ("promptFile", prompt_file.to_string_lossy().into_owned()),
        ("extension", extension.to_string_lossy().into_owned()),
        ("readonlyTools", readonly),
        ("cwd", spec.cwd.to_string_lossy().into_owned()),
    ]);
    let lists = spec.kit.placeholders(&files, &mut vars);
    let argv = runtime::build_command(&app.cfg.runtime.session_command, &vars, &lists);
    let Some(program) = argv.first() else { return Err(AppError::Internal("runtime.sessionCommand is empty".into())) };
    let mut cmd = match runtime::agent_command(app, &argv, &spec.cwd, &dir, key.project(), &spec.identity(), &token) {
        Ok(c) => c,
        Err(e) => {
            let t = token.clone();
            let _ = app.blocking(move |app| app.with_server(|db| db.revoke_token(&t))).await;
            return Err(AppError::Internal(e));
        }
    };
    llm.apply(&mut cmd);
    runtime::kit_env(&mut cmd, &files, &argv);
    cmd.env("GENIE_SESSION", "1").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let t = token.clone();
            let _ = app.blocking(move |app| app.with_server(|db| db.revoke_token(&t))).await;
            return Err(AppError::Internal(format!("cannot start {program}: {e}")));
        }
    };
    let pid = child.id().unwrap_or_default();
    let _ = tokio::fs::write(dir.join("session.pid"), format!("{pid} {}\n", spec.name)).await;
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let s = Arc::new(Session {
        key: key.clone(),
        pid,
        role: spec.role,
        role_id: spec.role_id.clone(),
        name: spec.name.clone(),
        team: spec.team.clone(),
        task: spec.task.clone(),
        initiator: spec.initiator.clone(),
        token,
        stdin: Mutex::new(Some(tx)),
        replies: Mutex::new(HashMap::new()),
        seq: AtomicU64::new(1),
        live: Mutex::new(Live::new(pid)),
        dir: dir.clone(),
    });
    s.with_live(|l| l.note(format!("▶ session started (pid {pid})")));
    app.sessions.insert(s.clone());

    let mut stdin = child.stdin.take().expect("piped stdin");
    tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if stdin.write_all(line.as_bytes()).await.is_err() || stdin.flush().await.is_err() {
                break;
            }
        }
        // Dropping stdin asks pi for an orderly shutdown.
    });
    // stderr goes to a file as it comes (the extension and pi report problems there).
    let mut stderr = child.stderr.take().expect("piped stderr");
    let err_log = dir.join("stderr.log");
    tokio::spawn(async move {
        let Ok(mut file) = tokio::fs::File::create(&err_log).await else { return };
        let mut buf = [0u8; 8192];
        while let Ok(n) = stderr.read(&mut buf).await {
            if n == 0 || file.write_all(&buf[..n]).await.is_err() {
                break;
            }
            let _ = file.flush().await;
        }
    });
    let stdout = child.stdout.take().expect("piped stdout");
    let (app2, s2) = (app.clone(), s.clone());
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if let Ok(event) = serde_json::from_slice::<Value>(&line) {
                        on_event(&app2, &s2, &event).await;
                    }
                }
            }
        }
        let status = child.wait().await.ok().and_then(|s| s.code());
        on_exit(&app2, &s2, status).await;
    });
    Ok(Some(s))
}

fn clip(text: &str, n: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > n { format!("{}…", flat.chars().take(n).collect::<String>()) } else { flat }
}

fn args_summary(args: &Value) -> String {
    for key in ["command", "path", "file_path", "pattern", "query"] {
        if let Some(v) = args.get(key).and_then(Value::as_str) {
            return clip(v, 160);
        }
    }
    clip(&args.to_string(), 160)
}

async fn on_event(app: &Arc<App>, s: &Arc<Session>, e: &Value) {
    let kind = e["type"].as_str().unwrap_or_default();
    s.with_live(|l| l.last_event = Instant::now());
    match kind {
        "response" => {
            if let Some(id) = e["id"].as_str()
                && let Some(tx) = s.replies.lock().unwrap_or_else(|e| e.into_inner()).remove(id)
            {
                let _ = tx.send(e.clone());
            }
        }
        "agent_start" => {
            let first = s.with_live(|l| {
                let first = l.state != "working";
                l.set_state("working");
                l.nudged = None;
                l.runs += 1;
                l.last_stop = None;
                first
            });
            if first {
                let (k, pid) = (s.key.clone(), s.pid);
                let turn = app
                    .blocking(move |app| {
                        let (team, member) = match &k {
                            AgentKey::Member { team, member, .. } => (Some(team.clone()), Some(member.clone())),
                            _ => (None, None),
                        };
                        let turn =
                            app.with_server(|db| db.start_turn(k.project(), &k.label(), team.as_deref(), member.as_deref(), None))?;
                        app.with_server(|db| db.set_turn_pid(turn, pid))?;
                        on_member(app, &k, |bus, team, member| {
                            bus.member_working(team, member, json!({ "kind": "session", "pid": pid, "turn": turn }))
                        });
                        Ok(turn)
                    })
                    .await
                    .ok();
                s.with_live(|l| l.turn = turn);
            }
        }
        "message_update" => {
            if let Some(input) = e["usage"]["input"].as_u64().filter(|n| *n > 0) {
                let cached = e["usage"]["cacheRead"].as_u64().unwrap_or(0);
                s.with_live(|l| l.context_tokens = Some(input + cached));
            }
        }
        "message_end" if e["message"]["role"] == "assistant" => {
            let m = &e["message"];
            let blocks = m["content"].as_array().cloned().unwrap_or_default();
            let text: Vec<&str> = blocks.iter().filter(|b| b["type"] == "text").filter_map(|b| b["text"].as_str()).collect();
            let thinking: Vec<&str> = blocks.iter().filter(|b| b["type"] == "thinking").filter_map(|b| b["thinking"].as_str()).collect();
            s.with_live(|l| {
                if !text.is_empty() {
                    let t = clip(&text.join(" "), 600);
                    l.note(format!("💬 {}", clip(&t, 160)));
                    l.last_text = Some(t);
                }
                if !thinking.is_empty() {
                    l.last_thinking = Some(clip(&thinking.join(" "), 400));
                }
                l.last_stop = m["stopReason"].as_str().map(str::to_string);
                if m["stopReason"] == "error" {
                    l.last_error = m["errorMessage"].as_str().map(|x| clip(x, 300));
                }
            });
        }
        "tool_execution_start" => {
            let name = e["toolName"].as_str().unwrap_or("tool").to_string();
            let args = args_summary(&e["args"]);
            let signature = format!("{name} {}", e["args"]);
            let looping = s.with_live(|l| {
                l.tool = Some(json!({ "name": name, "args": args, "since": now() }));
                l.tool_at = Some(Instant::now());
                l.note(format!("▶ {name}: {args}"));
                if l.repeat.0 == signature {
                    l.repeat.1 += 1;
                } else {
                    l.repeat = (signature, 1);
                }
                l.repeat.1 == LOOP_REPEATS
            });
            if looping {
                watchdog(
                    app,
                    s,
                    &format!("repeated the same call {LOOP_REPEATS} times in a row ({name}: {args}); it may be stuck in a loop"),
                )
                .await;
            }
        }
        "tool_execution_end" => {
            let name = e["toolName"].as_str().unwrap_or("tool").to_string();
            let failed = e["isError"] == true;
            s.with_live(|l| {
                let took = l.tool_at.take().map(|t| format!(" · {:.1}s", t.elapsed().as_secs_f32())).unwrap_or_default();
                l.tool = None;
                l.note(format!("{} {name}{took}", if failed { "✗" } else { "✓" }));
            });
        }
        "auto_retry_start" => {
            let msg = clip(e["errorMessage"].as_str().unwrap_or_default(), 160);
            s.with_live(|l| l.note(format!("↻ provider retry {}: {msg}", e["attempt"])));
        }
        "compaction_start" => s.with_live(|l| l.note(format!("⇣ compacting the context ({})", e["reason"].as_str().unwrap_or("")))),
        "extension_error" => {
            let msg = clip(e["error"].as_str().unwrap_or_default(), 200);
            s.with_live(|l| l.note(format!("⚠ extension: {msg}")));
        }
        "agent_settled" => settled(app, s).await,
        _ => {}
    }
}

/// A run ended: record it, retry a failed one, then deliver whatever is waiting.
async fn settled(app: &Arc<App>, s: &Arc<Session>) {
    let (turn, failed, error, log, interrupted, stalled) = s.with_live(|l| {
        l.set_state("idle");
        l.tool = None;
        let interrupted = std::mem::take(&mut l.interrupting);
        let stalled = l.stall_abort.take().is_some();
        let we_aborted = interrupted || stalled;
        // A run we aborted (interrupt, stuck step) ends with an error or an abort: not a failure.
        let failed = l.last_stop.as_deref() == Some("error") && !we_aborted;
        (l.turn.take(), failed, l.last_error.clone(), l.recent.iter().cloned().collect::<Vec<_>>().join("\n"), interrupted, stalled)
    });
    let (k, task) = (s.key.clone(), s.task.clone());
    let verdict = app
        .blocking(move |app| {
            if let Some(turn) = turn {
                app.with_server(|db| {
                    db.finish_turn(turn, if failed { "failed" } else { "succeeded" }, None, error.as_deref().filter(|_| failed), Some(&log))
                })?;
            }
            let outcome = if failed { Outcome::Failed { error: error.as_deref().unwrap_or("error"), log: &log } } else { Outcome::Ran };
            Ok(outcome::record(app, &k, task.as_deref(), outcome))
        })
        .await
        .ok();
    s.with_live(|l| l.failures = verdict.map(|v| v.failures).unwrap_or_default());
    if s.with_live(|l| std::mem::take(&mut l.reload)) {
        // The next step runs with the new settings; the conversation goes on.
        s.stop("settings changed");
        app.wake_runtime.notify_one();
        return;
    }
    if stalled && !interrupted {
        let secs = app.cfg.runtime.turn_timeout_secs;
        s.send(json!({
            "type": "prompt",
            "message": format!("[genie] Your last step showed no sign of life for {secs}s and was stopped. Continue your work; run long commands with a timeout or in the background with their output in a file.")
        }));
    }
    if interrupted {
        // The interrupt usually waits in the mailbox and wakes the session next. If it
        // reached the conversation just before the abort, the model has not answered it yet.
        let k = s.key.clone();
        let waiting = app
            .blocking(move |app| {
                app.with_tracker(k.project(), |t| {
                    let (team, recipient) = mailbox(&k);
                    Ok(t.bus().pending(team.as_deref(), &recipient)?.iter().any(|m| m.level == "interrupt"))
                })
            })
            .await
            .unwrap_or(true);
        if !waiting {
            s.send(json!({ "type": "prompt", "message": "[genie] Your step was interrupted: act on the INTERRUPT message above." }));
        }
    }
    if let Some(v) = verdict.filter(|v| v.failures > 0 && !v.gave_up) {
        // The mail of the failed run is in the conversation already: ask it to go on.
        let s = s.clone();
        let error = s.with_live(|l| l.last_error.clone()).unwrap_or_default();
        tokio::spawn(async move {
            tokio::time::sleep(v.retry_in).await;
            if s.is("idle") {
                s.send(json!({
                    "type": "prompt",
                    "message": format!("[genie] Your previous step failed ({error}). Continue where you left off.")
                }));
            }
        });
    }
    app.wake_runtime.notify_one();
    app.wake_engine.notify_one();
}

async fn on_exit(app: &Arc<App>, s: &Arc<Session>, code: Option<i32>) {
    let (state, turn, log) = s.with_live(|l| (l.state.clone(), l.turn.take(), l.recent.iter().cloned().collect::<Vec<_>>().join("\n")));
    let expected = state == "stopping";
    app.sessions.remove(&s.key, s.pid);
    s.replies.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let _ = std::fs::remove_file(s.dir.join("session.pid"));
    let (k, token, task) = (s.key.clone(), s.token.clone(), s.task.clone());
    let stderr = std::fs::read_to_string(s.dir.join("stderr.log")).unwrap_or_default();
    let verdict = app
        .blocking(move |app| {
            app.with_tracker(k.project(), |t| {
                let (team, recipient) = mailbox(&k);
                t.bus().release_deliveries_of(team.as_deref(), &recipient)
            })?;
            if let Some(turn) = turn {
                app.with_server(|db| {
                    db.finish_turn(
                        turn,
                        if expected { "succeeded" } else { "failed" },
                        code.map(i64::from),
                        Some("the session ended"),
                        Some(&log),
                    )
                })?;
            }
            app.with_server(|db| db.revoke_token(&token))?;
            let outcome = if expected { Outcome::Stopped } else { Outcome::Crashed { code, stderr: &stderr, at_work: state == "working" } };
            Ok(outcome::record(app, &k, task.as_deref(), outcome))
        })
        .await;
    if let Some(v) = verdict.ok().filter(|v| v.failures > 0) {
        // Wake the scheduler when the agent may start again, not at its next periodic pass.
        let app = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(v.retry_in + Duration::from_millis(50)).await;
            app.wake_runtime.notify_one();
        });
    }
    app.wake_runtime.notify_one();
}

fn mailbox(k: &AgentKey) -> (Option<String>, String) {
    match k {
        AgentKey::Member { team, member, .. } => (Some(team.clone()), member.clone()),
        _ => (None, ORCHESTRATOR.to_string()),
    }
}

/// A change on the board for a team member (the orchestrator has no row there).
fn on_member(app: &App, k: &AgentKey, f: impl FnOnce(&genie_core::team::Bus, &str, &str) -> genie_core::Result<()>) {
    if let AgentKey::Member { project, team, member } = k {
        let _ = app.with_tracker(project, |t| f(&t.bus(), team, member));
    }
}

/// Tell the orchestrator about an agent (about the orchestrator itself: the server log).
fn tell_orchestrator(app: &App, k: &AgentKey, task: Option<&str>, text: &str) {
    if matches!(k, AgentKey::Orchestrator { .. }) {
        eprintln!("genie runtime: {}: {text}", k.project());
        return;
    }
    let _ = runtime::tell_orchestrator(app, k.project(), task, text);
}

/// Tell the orchestrator (once per streak) that an agent looks stuck.
async fn watchdog(app: &Arc<App>, s: &Arc<Session>, what: &str) {
    s.with_live(|l| l.note(format!("⚠ watchdog: {what}")));
    let (k, task, text) = (
        s.key.clone(),
        s.task.clone(),
        format!("Watchdog: {} {what}. Peek at it (`genie team peek`) and steer or interrupt it.", s.key.label()),
    );
    let _ = app
        .blocking(move |app| {
            tell_orchestrator(app, &k, task.as_deref(), &text);
            Ok(())
        })
        .await;
}

/// Periodic care: interrupts, stuck steps, idle and orphaned sessions, stale deliveries.
pub async fn sweep(app: &Arc<App>) -> AppResult<()> {
    let projects = app.blocking(|app| app.with_server(|db| db.projects())).await?;
    for p in projects {
        let slug = p.slug.clone();
        let interrupted = app.blocking(move |app| app.with_tracker(&slug, |t| t.bus().interrupted_mailboxes())).await.unwrap_or_default();
        for b in interrupted {
            let key = match b.team {
                Some(team) => AgentKey::Member { project: p.slug.clone(), team, member: b.recipient },
                None => AgentKey::Orchestrator { project: p.slug.clone() },
            };
            if let Some(s) = app.sessions.get(&key)
                && s.is("working")
                && !s.with_live(|l| std::mem::replace(&mut l.interrupting, true))
            {
                s.abort("interrupt mail");
            }
        }
        // Stale deliveries: the session died or hung before the model saw them.
        let cutoff = (chrono::Utc::now() - chrono::Duration::from_std(STALE_DELIVERY).unwrap_or_default())
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let slug = p.slug.clone();
        let _ = app
            .blocking(move |app| {
                app.with_tracker(&slug, |t| {
                    for d in t.bus().open_deliveries(&cutoff)? {
                        t.bus().release_delivery(d.id)?;
                    }
                    Ok(())
                })
            })
            .await;
    }
    let idle_stop = Duration::from_secs(app.cfg.runtime.idle_stop_secs.max(1));
    let stall = Duration::from_secs(app.cfg.runtime.turn_timeout_secs.max(1));
    for s in app.sessions.all() {
        let (state, idle_for, silent_for, stall_abort) =
            s.with_live(|l| (l.state.clone(), l.state_at.elapsed(), l.last_event.elapsed(), l.stall_abort));
        if state == "idle" && idle_for >= idle_stop {
            s.stop("idle");
            continue;
        }
        if state == "working" && silent_for >= stall {
            match stall_abort {
                None => {
                    s.with_live(|l| l.stall_abort = Some(Instant::now()));
                    s.abort("no sign of life");
                    watchdog(app, &s, &format!("showed no sign of life for {}s; its step was aborted", stall.as_secs())).await;
                }
                Some(at) if at.elapsed() >= Duration::from_secs(60) => {
                    s.with_live(|l| l.note("■ killed: the abort did not stop it".into()));
                    kill(s.pid, "-KILL");
                }
                _ => {}
            }
            continue;
        }
        if state == "stopping" {
            continue;
        }
        // The agent is gone (team stopped, member removed, project in manual mode).
        let k = s.key.clone();
        let wanted = app.blocking(move |app| runtime::session_spec(app, &k).map(|x| x.is_some())).await.unwrap_or(true);
        if !wanted {
            s.stop("the agent is no longer active");
        }
    }
    Ok(())
}

/// The silent-team watchdog: an active team on a task in progress where nobody is working,
/// no mail is pending and nothing is waited for is reported to the orchestrator — once per
/// silence streak. The step and loop watchdogs watch one `working` session; this one catches
/// the team that read its last letter, answered it and stopped, leaving the task open.
/// Waiting for a person (blocked, `needs_owner`, a questionnaire, a job) or for CI/delivery
/// (checks running, a request waiting for review or merge) is not silence.
pub async fn watch_silent_teams(app: &Arc<App>) -> AppResult<()> {
    let stall = app.cfg.runtime.stall_secs;
    if stall == 0 {
        return Ok(());
    }
    let projects = app.blocking(|app| app.with_server(|db| db.projects())).await?;
    for p in projects {
        let slug = p.slug.clone();
        let quiet = match app.blocking(move |app| app.with_tracker(&slug, |t| t.bus().quiet_teams(Duration::from_secs(stall)))).await {
            Ok(q) => q,
            Err(e) => {
                eprintln!("genie runtime: {}: silent-team watchdog: {e}", p.slug);
                continue;
            }
        };
        for q in quiet {
            match report_if_silent(app, &p.slug, &q) {
                Ok(true) => println!("genie runtime: {}: team {} is silent for {}s — the orchestrator was told", p.slug, q.id, q.idle_secs),
                Ok(false) => {}
                Err(e) => eprintln!("genie runtime: {}: silent-team watchdog: {e}", p.slug),
            }
        }
    }
    Ok(())
}

/// One quiet candidate: is the task really waited on, has the streak already been reported,
/// and if not — a letter to the orchestrator and a `team_silent` marker in the journal.
fn report_if_silent(app: &App, slug: &str, q: &QuietTeam) -> AppResult<bool> {
    // Only a task in progress: a task in review is the reviewer's business, and a task the
    // owner has to decide on is a wait, not silence.
    let Ok(task) = app.with_tracker(slug, |t| t.get(&q.task)) else { return Ok(false) };
    if task.status != Status::InProgress || task.blocked.is_some() || task.needs_owner.is_some() {
        return Ok(false);
    }
    // Questionnaire, agent job or delivery: someone or something is expected to answer.
    let held = app.with_server(|db| {
        let people = db.open_questionnaires_for_task(slug, &q.task)?;
        let jobs = db.open_jobs_for_task(slug, &q.task)?;
        // Checks running, a request waiting for review or merge, and checks that will not settle are
        // all waits: the delivery, not the team, is what has to move.
        let delivery = db
            .task_repos(slug, &q.task)?
            .iter()
            .any(|r| matches!(r.ci_state.as_deref(), Some("pending" | "stalled")) || r.cr_state.as_deref() == Some("open"));
        Ok(people > 0 || jobs > 0 || delivery)
    })?;
    if held {
        return Ok(false);
    }
    // One letter per streak: the journal survives a restart, an in-memory flag would not.
    let told = app.with_tracker(slug, |t| t.bus().last_event_at(&q.id, "team_silent"))?;
    if told.as_deref().is_some_and(|at| at > q.since.as_str()) {
        return Ok(false);
    }
    let text = format!(
        "Watchdog: team {} on {} has been silent for {} min — every member is idle, no mail is pending, and the task \
         waits on neither a person nor CI. Nothing will move it by itself: look at the board (`genie team board`, \
         `genie team peek {} <member>`), then steer or restart a member, or stop the team and take the task over.",
        q.id,
        q.task,
        q.idle_secs / 60,
        q.id
    );
    app.with_tracker(slug, |t| {
        t.bus().notify_orchestrator("genie", "system", "system", &text, Some(&q.task))?;
        t.bus().log(&q.id, "team_silent", json!({ "task": q.task, "members": q.members, "idleSecs": q.idle_secs }))
    })?;
    Ok(true)
}

/// Stop the session of `key` (team stopped, member restarted…) and forget its failures.
pub fn reset(app: &App, key: &AgentKey) {
    app.attempts.clear(key);
    if let Some(s) = app.sessions.get(key) {
        s.stop("restart");
    }
}

/// Agents waiting out a backoff may try again at once (someone set their LiteLLM key).
pub fn retry_now(app: &App) {
    app.attempts.retry_now();
    app.wake_runtime.notify_one();
}

/// Let the session of `key` pick up new settings (a model) without cutting its
/// step short: an idle session stops now, a busy one when its step ends. The
/// next start resumes the same conversation.
pub fn reload(app: &App, key: &AgentKey) {
    let Some(s) = app.sessions.get(key) else { return };
    if s.is("working") {
        s.with_live(|l| l.reload = true);
    } else {
        s.stop("settings changed");
    }
}

/// Whether a live session holds `key` right now.
pub fn is_live(app: &App, key: &AgentKey) -> bool {
    app.sessions.get(key).is_some()
}

/// Stop every session (server shutdown).
pub fn stop_all(app: &App) {
    for s in app.sessions.all() {
        s.stdin.lock().unwrap_or_else(|e| e.into_inner()).take();
    }
}
