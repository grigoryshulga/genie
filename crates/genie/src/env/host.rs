//! The host runner: workspaces and steps run on this machine, as plain processes or inside
//! bubblewrap when the sandbox is on.
//!
//! It has no base images, services or agent harnesses, and it enforces no resource limits. Its
//! capabilities say so, so the controller refuses what it cannot do rather than weakening it.
//!
//! A workspace's state lives in `<root>/workspaces/<id>/run/`: `state.json` (what its processes run
//! with), one `<name>.pid` per detached process and their logs. Its home is
//! `<root>/workspaces/<id>/home` unless its environment names another. A step has no directory of
//! its own: its home is its HOME, else the home of the workspace it is labelled with.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::env::consts;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use genie_core::env::caps::{Capabilities, ViewMode};
use genie_core::env::spec::{NetworkMode, Probe};
use serde::{Deserialize, Serialize};

use super::runner::{BaseRef, BaseSpec, Exec, ExecResult, Mount, MountMode, Resource, Runner, StepRun, WorkspaceSpec};
use super::store;

/// The output a command keeps: its last 64 KiB.
const OUTPUT_TAIL: usize = 64 * 1024;
/// The exit code of a command killed at its timeout, as `timeout(1)` reports it.
const TIMED_OUT: i32 = 124;
/// How long a probe attempt may take when its profile names no timeout.
const DEFAULT_PROBE_SECS: u64 = 5;
/// How long a tool probe (`cargo --version`) may take.
const TOOL_PROBE_TIMEOUT: Duration = Duration::from_secs(120);
/// The file that holds a workspace's state, in its run directory.
const STATE_FILE: &str = "state.json";

/// A path made visible inside a sandbox: `source` on the host appears at `target`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Bind {
    source: String,
    target: String,
    readonly: bool,
}

/// What a process runs with: its binds, HOME, working directory, network and environment.
/// `sandboxed` records whether the binds were checked for bubblewrap, so a runner started
/// without it does not run a workspace as if it were plain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Job {
    binds: Vec<Bind>,
    home: String,
    cwd: String,
    isolate_network: bool,
    env: BTreeMap<String, String>,
    sandboxed: bool,
}

/// A workspace's state file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct State {
    running: bool,
    job: Job,
}

/// Runs workspaces and steps on this machine. `root` is the environment store; `sandbox` puts
/// every process in bubblewrap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRunner {
    root: PathBuf,
    sandbox: bool,
}

impl HostRunner {
    pub fn new(root: PathBuf, sandbox: bool) -> HostRunner {
        HostRunner { root, sandbox }
    }

    /// The run directory of a workspace, after checking that its name is safe as a path part.
    fn run_dir(&self, ws: &str) -> Result<PathBuf, String> {
        store::check_name(ws)?;
        Ok(store::run_dir(&self.root, ws))
    }

    fn state(&self, ws: &str) -> Result<State, String> {
        let run = self.run_dir(ws)?;
        read_state(&run)?.ok_or_else(|| format!("workspace {ws} has no state: create it first"))
    }

    /// The state of a workspace that is running and was made for this runner's sandbox setting.
    fn running_state(&self, ws: &str) -> Result<State, String> {
        let state = self.state(ws)?;
        if !state.running {
            return Err(format!("workspace {ws} is stopped: create it again to start it"));
        }
        if state.job.sandboxed != self.sandbox {
            return Err(format!(
                "workspace {ws} was created {} the sandbox; create it again after changing runtime.sandbox",
                if state.job.sandboxed { "with" } else { "without" }
            ));
        }
        Ok(state)
    }

    /// The job for a spec's mounts, secrets, home and network, checked against this runner.
    fn job(
        &self,
        mounts: &[Mount],
        secrets: &[(PathBuf, String)],
        home: &Path,
        cwd: &str,
        network: NetworkMode,
        env: &BTreeMap<String, String>,
    ) -> Result<Job, String> {
        if !cwd.starts_with('/') {
            return Err(format!("the working directory {cwd:?} must be an absolute path"));
        }
        let home = path_text(home)?;
        let binds = binds(&home, mounts, secrets, self.sandbox)?;
        Ok(Job { binds, home, cwd: cwd.into(), isolate_network: self.isolate(network)?, env: env.clone(), sandboxed: self.sandbox })
    }

    /// Whether a process is cut off from the network. `none` needs a sandbox that can do that.
    fn isolate(&self, mode: NetworkMode) -> Result<bool, String> {
        match mode {
            NetworkMode::Full => Ok(false),
            NetworkMode::None if self.sandbox => Ok(true),
            NetworkMode::None => Err("network mode none needs bubblewrap: enable runtime.sandbox for the host runner".into()),
            NetworkMode::Registries => Err("network mode registries is not supported in this release".into()),
        }
    }
}

impl Runner for HostRunner {
    fn kind(&self) -> &'static str {
        "host"
    }

    fn capabilities(&self) -> Result<Capabilities, String> {
        let mut notes = vec![
            "limits are not enforced: a workspace or step may use all the cpus, memory and processes of this machine".to_string(),
            "disk is measured by the store, not enforced".to_string(),
            "the environment is cleared: only PATH and LANG come from the server, the rest is the profile's".to_string(),
            "base images, packages and services need the podman runner; agent harnesses run in the agent sandbox instead".to_string(),
        ];
        let (version, network) = if self.sandbox {
            notes
                .push("processes run in bubblewrap: the host is read-only except the mounts, the workspace home and a private /tmp".into());
            notes.push("network none cuts a process off in its own network namespace; full shares the host network".into());
            (bwrap_version()?, vec![NetworkMode::Full, NetworkMode::None])
        } else {
            notes.push("processes run without a sandbox and can read and write all the server user can; read-only mounts and remapped paths are refused".into());
            ("no sandbox".to_string(), vec![NetworkMode::Full])
        };
        Ok(Capabilities {
            runner: "host".into(),
            version,
            os: consts::OS.into(),
            arch: consts::ARCH.into(),
            base_images: false,
            services: false,
            canonical_paths: false,
            harness: false,
            views: vec![ViewMode::Copy],
            network,
            limits: Vec::new(),
            secrets: true,
            capabilities: Vec::new(),
            notes,
        })
    }

    fn ensure_base(&self, spec: &BaseSpec) -> Result<BaseRef, String> {
        if spec.image.is_some() || spec.containerfile.is_some() || !spec.packages.is_empty() {
            return Err("the host runner uses this machine as its base and cannot build or pull an image or install packages: remove base.image, base.containerfile and base.packages, or use the podman runner".into());
        }
        Ok(BaseRef { reference: "host".into(), digest: format!("host:{}-{}", consts::OS, consts::ARCH) })
    }

    fn image_digest(&self, image: &str) -> Result<String, String> {
        Err(format!("image {image}: the host runner has no images; services need the podman runner"))
    }

    fn probe_tool(&self, base: &BaseRef, command: &str) -> Result<String, String> {
        check_base(base)?;
        let home = self.root.join("probe").join("home");
        fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
        let cwd = path_text(&home)?;
        let job = self.job(&[], &[], &home, &cwd, NetworkMode::Full, &BTreeMap::new())?;
        let result = run_capture(command_for(&job, command), Some(TOOL_PROBE_TIMEOUT), None)?;
        if result.code == 0 {
            Ok(result.output.trim_end().to_string())
        } else {
            Err(format!("{command}: exit {}: {}", result.code, result.output.trim()))
        }
    }

    fn run_step(&self, step: &StepRun) -> Result<ExecResult, String> {
        check_base(&step.base)?;
        let home = step_home(&self.root, &step.env, step.labels.get("genie.workspace"))?;
        fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
        let job = self.job(&step.mounts, &step.secrets, &home, &step.cwd, step.network, &step.env)?;
        run_capture(command_for(&job, &step.command), Some(step.timeout), Some(&step.log))
    }

    fn create(&self, ws: &WorkspaceSpec) -> Result<(), String> {
        let run = self.run_dir(&ws.id)?;
        if ws.run_dir != run {
            return Err(format!(
                "workspace {}: the host runner keeps its state in {}, not in {}",
                ws.id,
                run.display(),
                ws.run_dir.display()
            ));
        }
        if !ws.services.is_empty() {
            let names: Vec<&str> = ws.services.iter().map(|s| s.name.as_str()).collect();
            return Err(format!("workspace {}: the host runner cannot run services ({}); use the podman runner", ws.id, names.join(", ")));
        }
        check_base(&ws.base)?;
        let home = match ws.env.get("HOME") {
            Some(home) => PathBuf::from(home),
            None => store::workspace_home(&self.root, &ws.id),
        };
        fs::create_dir_all(&run).map_err(|e| format!("{}: {e}", run.display()))?;
        fs::create_dir_all(&home).map_err(|e| format!("{}: {e}", home.display()))?;
        let job = self.job(&ws.mounts, &ws.secrets, &home, &ws.cwd, ws.network, &ws.env)?;
        write_state(&run, &State { running: true, job })
    }

    fn exec(&self, ws: &str, e: &Exec) -> Result<ExecResult, String> {
        let state = self.running_state(ws)?;
        let job = exec_job(&state.job, e)?;
        run_capture(command_for(&job, &e.command), e.timeout, e.log.as_deref())
    }

    fn start_service(&self, ws: &WorkspaceSpec, name: &str) -> Result<(), String> {
        Err(format!("workspace {}: service {name} cannot run on the host runner; use the podman runner", ws.id))
    }

    fn start_process(&self, ws: &str, name: &str, e: &Exec) -> Result<(), String> {
        store::check_name(name)?;
        let state = self.running_state(ws)?;
        let run = self.run_dir(ws)?;
        let pid_path = run.join(format!("{name}.pid"));
        // A process already running under this name stays as it is.
        if let Some(pid) = read_pid(&pid_path)?
            && group_alive(pid)
        {
            return Ok(());
        }
        let job = exec_job(&state.job, e)?;
        let log = e.log.clone().unwrap_or_else(|| run.join(format!("{name}.log")));
        let pid = spawn_detached(command_for(&job, &e.command), &log)?;
        if let Err(err) = fs::write(&pid_path, pid.to_string()) {
            kill_group(pid);
            return Err(format!("{}: {err}", pid_path.display()));
        }
        Ok(())
    }

    fn probe(&self, ws: &str, p: &Probe) -> Result<bool, String> {
        let timeout = Duration::from_secs(p.timeout_secs.unwrap_or(DEFAULT_PROBE_SECS));
        match (&p.command, &p.http, &p.tcp) {
            (Some(command), None, None) => {
                let state = self.running_state(ws)?;
                let result = self.exec(
                    ws,
                    &Exec { cwd: state.job.cwd, command: command.clone(), env: BTreeMap::new(), timeout: Some(timeout), log: None },
                )?;
                Ok(result.code == 0)
            }
            (None, Some(url), None) => http_ready(url, timeout),
            (None, None, Some(addr)) => Ok(connect(addr, timeout)?.is_some()),
            _ => Err("a readiness probe names exactly one of command, http and tcp".into()),
        }
    }

    fn stop(&self, ws: &str) -> Result<(), String> {
        let run = self.run_dir(ws)?;
        if !run.is_dir() {
            return Ok(());
        }
        for (_, path) in pid_files(&run)? {
            if let Some(pid) = read_pid(&path)?
                && group_alive(pid)
            {
                kill_group(pid);
            }
            remove_file_if_exists(&path)?;
        }
        if let Some(mut state) = read_state(&run)?
            && state.running
        {
            state.running = false;
            write_state(&run, &state)?;
        }
        Ok(())
    }

    fn remove(&self, ws: &str) -> Result<(), String> {
        self.stop(ws)?;
        let run = self.run_dir(ws)?;
        match fs::remove_dir_all(&run) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("{}: {e}", run.display())),
        }
    }

    fn list(&self) -> Result<Vec<Resource>, String> {
        let dir = self.root.join("workspaces");
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))? {
            let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
            let id = entry.file_name().to_string_lossy().into_owned();
            let run = entry.path().join("run");
            let Some(state) = read_state(&run)? else { continue };
            out.push(Resource { workspace: id.clone(), kind: "workspace".into(), name: id.clone(), running: state.running });
            for (name, path) in pid_files(&run)? {
                if let Some(pid) = read_pid(&path)? {
                    out.push(Resource { workspace: id.clone(), kind: "process".into(), name, running: group_alive(pid) });
                }
            }
        }
        Ok(out)
    }

    fn remove_tree(&self, path: &Path) -> Result<(), String> {
        store::remove_tree(path)
    }

    fn prune_bases(&self, _keep: &BTreeSet<String>) -> Result<Vec<String>, String> {
        Ok(Vec::new())
    }

    fn harness(&self, ws: &str, _cwd: &str, _env_names: &[String], _argv: &[String]) -> Result<(String, Vec<String>), String> {
        Err(format!("workspace {ws}: the host runner keeps agents in the agent sandbox; the environment cannot host them"))
    }
}

/// The binds of a job: its home (writable), the mounts and the secrets, as bubblewrap will see them.
///
/// Without bubblewrap nothing can be remapped or made read-only, so a mount must be an identity path,
/// a read-only one is refused, and a secret must already sit at its own path. With bubblewrap every
/// target must exist, or sit below a writable bind that creates it.
fn binds(home: &str, mounts: &[Mount], secrets: &[(PathBuf, String)], sandboxed: bool) -> Result<Vec<Bind>, String> {
    let mut out = vec![Bind { source: home.into(), target: home.into(), readonly: false }];
    for m in mounts {
        let readonly = match m.mode {
            MountMode::ReadOnly => true,
            MountMode::ReadWrite => false,
            MountMode::Overlay { .. } => {
                return Err(format!("mount {}: the host runner has no overlay views; use a copy view", m.target));
            }
        };
        if !sandboxed && readonly {
            return Err(format!("mount {}: a read-only mount needs bubblewrap; enable runtime.sandbox", m.target));
        }
        if !sandboxed && Path::new(&m.target) != m.source.as_path() {
            return Err(format!(
                "mount {}: without bubblewrap the host runner cannot move a path, so the source must be the target ({}); enable runtime.sandbox",
                m.target,
                m.source.display()
            ));
        }
        out.push(Bind { source: path_text(&m.source)?, target: m.target.clone(), readonly });
    }
    for (source, target) in secrets {
        if !sandboxed && Path::new(target) != source.as_path() {
            return Err(format!(
                "secret {target}: without bubblewrap a secret must already be at its own path ({}); enable runtime.sandbox",
                source.display()
            ));
        }
        out.push(Bind { source: path_text(source)?, target: target.clone(), readonly: true });
    }
    if let Some(b) = out.iter().find(|b| !b.target.starts_with('/')) {
        return Err(format!("mount target {:?} must be an absolute path", b.target));
    }
    if sandboxed {
        check_targets(&out)?;
    }
    Ok(out)
}

/// Every target must exist, or sit below a writable bind that creates it: bubblewrap cannot make
/// a mount point under a read-only tree.
fn check_targets(binds: &[Bind]) -> Result<(), String> {
    let mut writable: Vec<&str> = Vec::new();
    for b in sorted_by_depth(binds) {
        let target = Path::new(&b.target);
        if !target.exists() && !writable.iter().any(|w| target.starts_with(w)) {
            return Err(format!(
                "mount target {} does not exist, and no writable mount above it creates it: create it first, since bubblewrap cannot make a mount point under the read-only system",
                b.target
            ));
        }
        if !b.readonly {
            writable.push(&b.target);
        }
    }
    Ok(())
}

/// Binds ordered from the root down, so a deeper bind overrides the one above it. The sort is stable.
fn sorted_by_depth(binds: &[Bind]) -> Vec<&Bind> {
    let mut sorted: Vec<&Bind> = binds.iter().collect();
    sorted.sort_by_key(|b| Path::new(&b.target).components().count());
    sorted
}

/// The bubblewrap arguments for a job: the host read-only, a private `/dev`, `/proc` and `/tmp`, a
/// private process table, then the binds from the root down, HOME and the working directory. The
/// program and its arguments follow the `--`.
fn bwrap_args(binds: &[Bind], home: &str, cwd: &str, isolate_network: bool) -> Vec<String> {
    let mut out: Vec<String> =
        ["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp", "--unshare-pid", "--die-with-parent"]
            .map(String::from)
            .to_vec();
    if isolate_network {
        out.push("--unshare-net".into());
    }
    for b in sorted_by_depth(binds) {
        out.push(if b.readonly { "--ro-bind" } else { "--bind" }.into());
        out.push(b.source.clone());
        out.push(b.target.clone());
    }
    out.extend(["--setenv".into(), "HOME".into(), home.into(), "--chdir".into(), cwd.into(), "--".into()]);
    out
}

/// The command that runs `shell` for a job: `sh` in bubblewrap, or `sh` in the working directory.
/// The environment is cleared; PATH and LANG come from the server, HOME is the job's home, and the
/// job's variables are added. Values are set in the environment, never on the command line.
fn command_for(job: &Job, shell: &str) -> Command {
    let mut cmd = if job.sandboxed {
        let mut c = Command::new("bwrap");
        c.args(bwrap_args(&job.binds, &job.home, &job.cwd, job.isolate_network)).args(["sh", "-c", shell]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", shell]).current_dir(&job.cwd);
        c
    };
    cmd.env_clear();
    cmd.env("PATH", std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()));
    if let Ok(lang) = std::env::var("LANG") {
        cmd.env("LANG", lang);
    }
    cmd.envs(job.env.iter().filter(|(k, _)| k.as_str() != "HOME"));
    cmd.env("HOME", &job.home);
    cmd
}

/// A step's home: its own HOME when its environment gives one, else the home of its workspace.
fn step_home(root: &Path, env: &BTreeMap<String, String>, workspace: Option<&String>) -> Result<PathBuf, String> {
    if let Some(home) = env.get("HOME") {
        return Ok(PathBuf::from(home));
    }
    match workspace {
        Some(ws) => {
            store::check_name(ws)?;
            Ok(store::workspace_home(root, ws))
        }
        None => Err("a step needs HOME in its environment or a genie.workspace label: the host runner has no other home for it".into()),
    }
}

/// The job of one command in a workspace: its working directory and variables replace the workspace's.
fn exec_job(job: &Job, e: &Exec) -> Result<Job, String> {
    if !e.cwd.starts_with('/') {
        return Err(format!("the working directory {:?} must be an absolute path", e.cwd));
    }
    let mut job = job.clone();
    job.cwd = e.cwd.clone();
    job.env.extend(e.env.clone());
    Ok(job)
}

fn check_base(base: &BaseRef) -> Result<(), String> {
    if base.reference == "host" {
        Ok(())
    } else {
        Err(format!("base {} belongs to another runner: the host runner runs on this machine", base.reference))
    }
}

fn path_text(path: &Path) -> Result<String, String> {
    path.to_str().map(str::to_string).ok_or_else(|| format!("{}: the path is not UTF-8, which the host runner cannot use", path.display()))
}

fn bwrap_version() -> Result<String, String> {
    let out = Command::new("bwrap").arg("--version").output().map_err(|e| format!("bubblewrap: {e}"))?;
    if !out.status.success() {
        return Err("bwrap --version failed".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Kill a process group, led by the process `pgid`. Its members, background jobs included, die with it.
fn kill_group(pgid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pgid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Whether any process still belongs to the group led by `pgid`.
fn group_alive(pgid: u32) -> bool {
    Command::new("kill")
        .args(["-0", "--", &format!("-{pgid}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn open_log(path: &Path) -> Result<File, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    OpenOptions::new().create(true).append(true).open(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// What a command printed: the log file gets all of it, `tail` keeps the last part.
#[derive(Default)]
struct Sink {
    log: Option<File>,
    tail: Vec<u8>,
    log_error: Option<String>,
}

impl Sink {
    fn write(&mut self, bytes: &[u8]) {
        if self.log_error.is_none()
            && let Some(log) = self.log.as_mut()
            && let Err(e) = log.write_all(bytes)
        {
            self.log_error = Some(e.to_string());
        }
        self.tail.extend_from_slice(bytes);
        if self.tail.len() > 2 * OUTPUT_TAIL {
            let cut = self.tail.len() - OUTPUT_TAIL;
            self.tail.drain(..cut);
        }
    }
}

/// Read one pipe of a command into the sink until it ends. Dropping `done` tells the waiter.
fn pump<R: Read + Send + 'static>(mut from: R, sink: Arc<Mutex<Sink>>, done: mpsc::Sender<()>) {
    thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match from.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => sink.lock().unwrap_or_else(|p| p.into_inner()).write(&buf[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        drop(done);
    });
}

/// Run a command to its end, or until `timeout` (its process group is then killed). Stdout and
/// stderr are combined into `log` when given. The result's output is the last 64 KiB.
fn run_capture(mut cmd: Command, timeout: Option<Duration>, log: Option<&Path>) -> Result<ExecResult, String> {
    let log = log.map(open_log).transpose()?;
    let program = cmd.get_program().to_string_lossy().into_owned();
    // A process group of its own: a timeout or a finished leader takes the whole group with it.
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).process_group(0);
    let mut child = cmd.spawn().map_err(|e| format!("cannot start {program}: {e}"))?;
    let pid = child.id();
    let sink = Arc::new(Mutex::new(Sink { log, ..Sink::default() }));
    let (done, finished) = mpsc::channel::<()>();
    if let Some(out) = child.stdout.take() {
        pump(out, sink.clone(), done.clone());
    }
    if let Some(err) = child.stderr.take() {
        pump(err, sink.clone(), done.clone());
    }
    drop(done);

    let deadline = timeout.map(|t| Instant::now() + t);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                kill_group(pid);
                return Err(format!("waiting for {program}: {e}"));
            }
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            kill_group(pid);
            timed_out = true;
            break child.wait().map_err(|e| format!("waiting for {program}: {e}"))?;
        }
        thread::sleep(Duration::from_millis(5));
    };
    // Whatever the leader left running in its group goes too, so that the output can end.
    kill_group(pid);
    // A process that left the group may keep a pipe open; the output is not waited for longer than this.
    let _ = finished.recv_timeout(Duration::from_secs(2));

    let mut sink = sink.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(limit) = timeout.filter(|_| timed_out) {
        sink.write(format!("\n[genie] timed out after {limit:?}; the process group was killed\n").as_bytes());
    }
    if let Some(e) = sink.log_error.take() {
        return Err(format!("writing the log failed: {e}"));
    }
    let start = sink.tail.len().saturating_sub(OUTPUT_TAIL);
    let output = String::from_utf8_lossy(&sink.tail[start..]).into_owned();
    let code = if timed_out { TIMED_OUT } else { status.code().unwrap_or_else(|| 128 + status.signal().unwrap_or(0)) };
    Ok(ExecResult { code, output })
}

/// Start a command detached in its own process group, with its output appended to `log`. It is
/// reaped by a thread when it exits, so no zombie is left behind.
fn spawn_detached(mut cmd: Command, log: &Path) -> Result<u32, String> {
    let out = open_log(log)?;
    let err = out.try_clone().map_err(|e| format!("{}: {e}", log.display()))?;
    cmd.stdin(Stdio::null()).stdout(out).stderr(err).process_group(0);
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd.spawn().map_err(|e| format!("cannot start {program}: {e}"))?;
    let pid = child.id();
    thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(pid)
}

/// Connect to `addr` within `timeout`. `None` when nothing accepts the connection yet; an error
/// when the address does not resolve.
fn connect(addr: &str, timeout: Duration) -> Result<Option<TcpStream>, String> {
    let addrs: Vec<_> = addr.to_socket_addrs().map_err(|e| format!("probe address {addr}: {e}"))?.collect();
    Ok(addrs.iter().find_map(|a| TcpStream::connect_timeout(a, timeout).ok()))
}

/// One HTTP readiness attempt: a GET of an `http://` URL. Ready when the status is below 400.
fn http_ready(url: &str, timeout: Duration) -> Result<bool, String> {
    let rest = url.strip_prefix("http://").ok_or_else(|| format!("probe {url}: the host runner probes only http:// URLs"))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => rest.split_at(i),
        None => (rest, "/"),
    };
    let addr = if authority.contains(':') { authority.to_string() } else { format!("{authority}:80") };
    let Some(mut stream) = connect(&addr, timeout)? else { return Ok(false) };
    stream.set_read_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    stream.set_write_timeout(Some(timeout)).map_err(|e| e.to_string())?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return Ok(false);
    }
    let mut head = Vec::new();
    let mut buf = [0u8; 512];
    while !head.windows(2).any(|w| w == b"\r\n") && head.len() < 4096 {
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    let text = String::from_utf8_lossy(&head);
    let status = text.lines().next().unwrap_or("").split_whitespace().nth(1).and_then(|s| s.parse::<u16>().ok());
    Ok(status.is_some_and(|code| code < 400))
}

/// The state file of a run directory, when there is one.
fn read_state(run: &Path) -> Result<Option<State>, String> {
    let path = run.join(STATE_FILE);
    match fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// Write the state file atomically: a reader sees the old state or the new one.
fn write_state(run: &Path, state: &State) -> Result<(), String> {
    let path = run.join(STATE_FILE);
    let tmp = run.join("state.json.tmp");
    let text = serde_json::to_vec_pretty(state).map_err(|e| e.to_string())?;
    fs::write(&tmp, text).map_err(|e| format!("{}: {e}", tmp.display()))?;
    fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
}

/// The process id in a pid file, when there is one.
fn read_pid(path: &Path) -> Result<Option<u32>, String> {
    match fs::read_to_string(path) {
        Ok(text) => text.trim().parse::<u32>().map(Some).map_err(|_| format!("{}: the file does not hold a process id", path.display())),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

/// The `<name>.pid` files of a run directory, with their names.
fn pid_files(run: &Path) -> Result<Vec<(String, PathBuf)>, String> {
    let mut out = Vec::new();
    for entry in fs::read_dir(run).map_err(|e| format!("{}: {e}", run.display()))? {
        let path = entry.map_err(|e| format!("{}: {e}", run.display()))?.path();
        if path.extension().is_some_and(|x| x == "pid")
            && let Some(stem) = path.file_stem()
        {
            out.push((stem.to_string_lossy().into_owned(), path));
        }
    }
    out.sort();
    Ok(out)
}

fn remove_file_if_exists(path: &Path) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn bind(source: &str, target: &str, readonly: bool) -> Bind {
        Bind { source: source.into(), target: target.into(), readonly }
    }

    fn mount(source: &Path, target: &str, mode: MountMode) -> Mount {
        Mount { source: source.to_path_buf(), target: target.into(), mode }
    }

    /// A job for plain processes in `dir`, with HOME in it.
    fn plain_job(dir: &Path) -> Job {
        Job {
            binds: vec![bind(dir.to_str().unwrap(), dir.to_str().unwrap(), false)],
            home: dir.to_str().unwrap().into(),
            cwd: dir.to_str().unwrap().into(),
            isolate_network: false,
            env: BTreeMap::new(),
            sandboxed: false,
        }
    }

    fn position(args: &[String], flag: &str, value: &str) -> usize {
        args.windows(3).position(|w| w[0] == flag && w[2] == value).unwrap_or_else(|| panic!("{flag} {value} in {args:?}"))
    }

    #[test]
    fn the_bubblewrap_arguments_start_with_the_read_only_system_and_end_with_the_program_boundary() {
        let args = bwrap_args(&[], "/data/home", "/data/home", false);
        assert_eq!(&args[..3], ["--ro-bind", "/", "/"]);
        for flag in ["--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp", "--unshare-pid", "--die-with-parent"] {
            assert!(args.iter().any(|a| a == flag), "{flag} in {args:?}");
        }
        assert!(!args.iter().any(|a| a == "--unshare-net"), "full network keeps the host namespace");
        let tail: Vec<&str> = args[args.len() - 6..].iter().map(String::as_str).collect();
        assert_eq!(tail, ["--setenv", "HOME", "/data/home", "--chdir", "/data/home", "--"]);
    }

    #[test]
    fn network_none_adds_the_namespace_flag_and_only_then() {
        assert!(bwrap_args(&[], "/h", "/h", true).iter().any(|a| a == "--unshare-net"));
        assert!(!bwrap_args(&[], "/h", "/h", false).iter().any(|a| a == "--unshare-net"));
    }

    #[test]
    fn binds_are_mounted_from_the_root_down_whatever_order_they_come_in() {
        let binds = vec![
            bind("/w/home/.npmrc", "/w/home/.npmrc", true),
            bind("/w/repo/api", "/w/repo/api", false),
            bind("/w/repo", "/w/repo", true),
            bind("/w/home", "/w/home", false),
        ];
        let args = bwrap_args(&binds, "/w/home", "/w/repo/api", false);
        let at = |flag: &str, value: &str| position(&args, flag, value);
        assert!(at("--bind", "/w/home") < at("--ro-bind", "/w/home/.npmrc"), "a secret overrides the home");
        assert!(at("--ro-bind", "/w/repo") < at("--bind", "/w/repo/api"), "a writable repo below a read-only one overrides it");
        let tail: Vec<&str> = args[args.len() - 6..].iter().map(String::as_str).collect();
        assert_eq!(tail, ["--setenv", "HOME", "/w/home", "--chdir", "/w/repo/api", "--"]);
    }

    #[test]
    fn the_sort_is_stable_for_equal_depths() {
        let binds = vec![bind("/a/x", "/a/x", true), bind("/b/x", "/b/x", false)];
        let sorted: Vec<&str> = sorted_by_depth(&binds).iter().map(|b| b.target.as_str()).collect();
        assert_eq!(sorted, ["/a/x", "/b/x"]);
    }

    #[test]
    fn without_bubblewrap_a_mount_must_be_an_identity_path_and_writable() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let home_text = home.to_str().unwrap();
        let src = t.path().join("src");
        fs::create_dir_all(&src).unwrap();

        let identity = mount(&src, src.to_str().unwrap(), MountMode::ReadWrite);
        assert!(binds(home_text, std::slice::from_ref(&identity), &[], false).is_ok());

        let remapped = mount(&src, "/elsewhere", MountMode::ReadWrite);
        let err = binds(home_text, &[remapped], &[], false).unwrap_err();
        assert!(err.contains("cannot move a path"), "{err}");

        let read_only = mount(&src, src.to_str().unwrap(), MountMode::ReadOnly);
        let err = binds(home_text, &[read_only], &[], false).unwrap_err();
        assert!(err.contains("read-only mount needs bubblewrap"), "{err}");

        let overlay = mount(&src, src.to_str().unwrap(), MountMode::Overlay { upper: home.clone(), work: home.clone() });
        let err = binds(home_text, &[overlay], &[], true).unwrap_err();
        assert!(err.contains("no overlay views"), "{err}");
    }

    #[test]
    fn a_target_that_bubblewrap_cannot_create_is_refused_by_name_and_one_below_a_writable_bind_is_not() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let home_text = home.to_str().unwrap();
        let ro_dir = t.path().join("ro");
        fs::create_dir_all(&ro_dir).unwrap();
        let missing = ro_dir.join("node_modules");
        let ro_mount = mount(&ro_dir, ro_dir.to_str().unwrap(), MountMode::ReadOnly);
        let output = mount(&home, missing.to_str().unwrap(), MountMode::ReadWrite);

        // Below a read-only tree nothing creates the mount point.
        let err = binds(home_text, &[ro_mount.clone(), output.clone()], &[], true).unwrap_err();
        assert!(err.contains("does not exist"), "{err}");
        assert!(err.contains(missing.to_str().unwrap()), "{err}");

        // Below the writable home, bubblewrap creates the same path inside the home.
        let under_home = mount(&home, home.join("deps").join("x").to_str().unwrap(), MountMode::ReadWrite);
        assert!(binds(home_text, &[under_home], &[], true).is_ok());
        assert!(binds(home_text, &[ro_mount], &[], true).is_ok(), "an existing read-only target is fine");
    }

    #[test]
    fn a_secret_must_be_at_its_own_path_without_bubblewrap() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let secret = t.path().join("token");
        fs::write(&secret, "x").unwrap();
        let err = binds(home.to_str().unwrap(), &[], &[(secret.clone(), "/elsewhere/token".into())], false).unwrap_err();
        assert!(err.contains("at its own path"), "{err}");
        let own = (secret.clone(), secret.to_str().unwrap().to_string());
        assert!(binds(home.to_str().unwrap(), &[], &[own], false).is_ok());
    }

    #[test]
    fn a_relative_target_is_refused() {
        let t = tempfile::tempdir().unwrap();
        let err = binds(t.path().to_str().unwrap(), &[mount(t.path(), "relative", MountMode::ReadWrite)], &[], true).unwrap_err();
        assert!(err.contains("absolute"), "{err}");
    }

    #[test]
    fn the_output_of_a_command_is_its_combined_streams_and_its_exit_code() {
        let t = tempfile::tempdir().unwrap();
        let log = t.path().join("logs/step.log");
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo out; echo err >&2; exit 7"]);
        let r = run_capture(cmd, Some(Duration::from_secs(30)), Some(&log)).unwrap();
        assert_eq!(r.code, 7);
        assert!(r.output.contains("out") && r.output.contains("err"), "{:?}", r.output);
        let logged = fs::read_to_string(&log).unwrap();
        assert!(logged.contains("out") && logged.contains("err"), "{logged:?}");
    }

    #[test]
    fn the_output_keeps_only_the_last_64_kib_while_the_log_keeps_everything() {
        let t = tempfile::tempdir().unwrap();
        let log = t.path().join("big.log");
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "head -c 200000 /dev/zero | tr '\\0' a; echo END"]);
        let r = run_capture(cmd, None, Some(&log)).unwrap();
        assert_eq!(r.output.len(), OUTPUT_TAIL);
        assert!(r.output.ends_with("END\n"));
        assert_eq!(fs::metadata(&log).unwrap().len(), 200_004);
    }

    #[test]
    fn a_timeout_kills_the_process_group_and_reports_the_timeout_code() {
        let started = Instant::now();
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 60 & sleep 60; echo never"]);
        let r = run_capture(cmd, Some(Duration::from_millis(300)), None).unwrap();
        assert_eq!(r.code, TIMED_OUT);
        assert!(r.output.contains("timed out"), "{:?}", r.output);
        assert!(started.elapsed() < Duration::from_secs(20), "the background job did not hold the call");
    }

    #[test]
    fn a_background_job_that_outlives_its_command_does_not_hold_the_output_open() {
        let started = Instant::now();
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 60 & echo done"]);
        let r = run_capture(cmd, None, None).unwrap();
        assert_eq!(r.code, 0);
        assert!(r.output.contains("done"));
        assert!(started.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn a_command_runs_with_a_cleared_environment_plus_the_jobs_variables_and_its_own_home() {
        let t = tempfile::tempdir().unwrap();
        let mut job = plain_job(t.path());
        job.env.insert("GREETING".into(), "hi".into());
        job.env.insert("HOME".into(), "/not/this".into());
        let r = run_capture(command_for(&job, "echo \"$GREETING:$HOME:${GENIE_NOT_SET_ANYWHERE-unset}\""), None, None).unwrap();
        assert_eq!(r.output.trim(), format!("hi:{}:unset", t.path().display()));
    }

    #[test]
    fn a_detached_process_is_started_once_and_its_pid_file_names_its_group() {
        let t = tempfile::tempdir().unwrap();
        let log = t.path().join("sleeper.log");
        let pid = spawn_detached(command_for(&plain_job(t.path()), "exec sleep 60"), &log).unwrap();
        assert!(group_alive(pid));
        kill_group(pid);
        let deadline = Instant::now() + Duration::from_secs(10);
        while group_alive(pid) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(!group_alive(pid), "the group is gone after the kill");
    }

    #[test]
    fn a_tcp_probe_is_ready_only_while_something_listens() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        assert!(connect(&addr, Duration::from_secs(2)).unwrap().is_some());
        drop(listener);
        // Port 1 has no listener here; a closed ephemeral port could connect to itself on localhost.
        assert!(connect("127.0.0.1:1", Duration::from_secs(2)).unwrap().is_none());
        assert!(connect("no-port-here", Duration::from_secs(2)).is_err(), "an address that does not resolve is an error");
    }

    #[test]
    fn a_probe_names_exactly_one_check() {
        let runner = HostRunner::new(PathBuf::from("/nonexistent-store"), false);
        let none = Probe { command: None, http: None, tcp: None, timeout_secs: None };
        assert!(runner.probe("w", &none).unwrap_err().contains("exactly one"));
        let two = Probe { command: Some("true".into()), http: Some("http://x/".into()), tcp: None, timeout_secs: None };
        assert!(runner.probe("w", &two).unwrap_err().contains("exactly one"));
    }

    #[test]
    fn an_image_base_or_packages_are_refused_and_the_host_base_is_its_platform() {
        let runner = HostRunner::new(PathBuf::from("/nonexistent-store"), false);
        let spec = |image: Option<&str>, packages: &[&str]| BaseSpec {
            image: image.map(String::from),
            packages: packages.iter().map(|p| p.to_string()).collect(),
            containerfile: None,
            tag: "genie-base:0".into(),
        };
        assert!(runner.ensure_base(&spec(Some("debian"), &[])).is_err());
        assert!(runner.ensure_base(&spec(None, &["gcc"])).is_err());
        let base = runner.ensure_base(&spec(None, &[])).unwrap();
        assert_eq!(base.reference, "host");
        assert_eq!(base.digest, format!("host:{}-{}", consts::OS, consts::ARCH));
    }

    #[test]
    fn the_capabilities_of_a_plain_host_runner_offer_full_network_only() {
        let caps = HostRunner::new(PathBuf::from("/nonexistent-store"), false).capabilities().unwrap();
        assert_eq!(caps.runner, "host");
        assert_eq!(caps.arch, consts::ARCH);
        assert_eq!(caps.network, vec![NetworkMode::Full]);
        assert_eq!(caps.views, vec![ViewMode::Copy]);
        assert!(caps.limits.is_empty() && !caps.canonical_paths && !caps.harness && !caps.services && !caps.base_images);
        assert!(caps.secrets);
    }
}
