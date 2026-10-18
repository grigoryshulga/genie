//! The rootless Podman runner: workspaces as pods on the user's own Podman.
//!
//! A workspace is the pod `genie-<id>`. Its main container runs `sleep infinity` and every process
//! execs into it; each service is a container of the same pod. A step runs in a short-lived
//! container of its own. Everything the runner creates carries `genie.workspace=<id>` and
//! `genie.kind=...` labels, so `list` and `remove` find it without state of their own.
//!
//! Command lines come from pure builders (`run_args`, `main_run_args`, `exec_args`, ...) that the
//! unit tests read without running Podman. Secret values never reach a command line: a secret is a
//! read-only bind mount of its host file.
//!
//! Known limits: a command that times out is killed on the host side. For a step the container is
//! removed by name; for `exec` only the `podman exec` client is killed, so the process inside may
//! keep running. Podman exits 125 for its own failures, so a command exiting 125 is reported as an
//! error rather than as a step failure.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use genie_core::env::caps::{Capabilities, Limit, Limits, ViewMode};
use genie_core::env::spec::{NetworkMode, Probe, valid_name};
use serde_json::Value;

use super::layout::HOME;
use super::runner::{BaseRef, BaseSpec, Exec, ExecResult, Mount, MountMode, Resource, Runner, ServiceSpec, StepRun, WorkspaceSpec};
use crate::config::EnvironmentsConfig;

/// Where a workspace's run directory appears in its main container: pid files and process logs.
const RUN_DIR: &str = "/opt/genie-run";
/// The tail of a command's output kept in an `ExecResult`, and of Podman's stderr in an error.
const OUTPUT_TAIL: usize = 64 * 1024;
/// Podman's exit code for its own failures: no such container, no image, bad options.
const PODMAN_ERROR: i32 = 125;
/// The exit code of a probe script whose check cannot run in the base (no curl or wget, no bash or nc).
const PROBE_CANNOT_RUN: i32 = 98;
/// How long one readiness attempt may take when the probe names no timeout.
const PROBE_ATTEMPT: Duration = Duration::from_secs(10);
/// The longest a tool probe (`cargo --version` and the like) may take.
const TOOL_PROBE: Duration = Duration::from_secs(300);
/// How often a running command is checked for its exit and its timeout.
const POLL: Duration = Duration::from_millis(50);

static UNIQUE: AtomicU64 = AtomicU64::new(0);

/// The settings the command lines need besides the specs.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Options {
    /// The network full-network containers join (`environments.podman.network`).
    network: String,
    /// The address they reach the server's host at (`GENIE_HOST_ADDRESS`).
    host_address: String,
    /// The uid and gid containers run as. `--userns=keep-id` maps them to the same ids inside, so
    /// files a workspace writes belong to the server's user on the host.
    user: (u32, u32),
}

/// The configured network value that asks for the network to be chosen from the Podman version.
const AUTO_NETWORK: &str = "auto";

/// The rootless Podman runner. Construction runs nothing: each method asks Podman when it needs to.
#[derive(Debug)]
pub struct PodmanRunner {
    binary: String,
    /// `environments.podman.network` as configured: `auto` or a `--network` value.
    network: String,
    host_address: String,
    user: Option<(u32, u32)>,
    /// The network `auto` resolved to, found once from `podman info` and kept for the runner's life.
    resolved: OnceLock<String>,
}

impl PodmanRunner {
    /// A runner for the Podman program and network of `cfg`. Nothing is run until a method is called.
    pub fn new(cfg: &EnvironmentsConfig) -> PodmanRunner {
        PodmanRunner {
            binary: cfg.podman.binary.clone(),
            network: cfg.podman.network.clone(),
            host_address: cfg.podman.host_address.clone(),
            user: current_user(),
            resolved: OnceLock::new(),
        }
    }

    /// The `--network` value of full-network containers: the configured one, or for `auto` the one
    /// the Podman version needs. `auto` asks Podman once; a failed query is not cached, so it is retried.
    fn full_network(&self) -> Result<String, String> {
        if self.network != AUTO_NETWORK {
            return Ok(self.network.clone());
        }
        if let Some(network) = self.resolved.get() {
            return Ok(network.clone());
        }
        let version = info_version(&self.checked(["info", "--format", "json"])?)?;
        let network = auto_network(&version, &self.host_address)?;
        // A concurrent caller may have set it first; both answers are the same function of the version.
        let _ = self.resolved.set(network.clone());
        Ok(network)
    }

    fn options(&self) -> Result<Options, String> {
        let user = self.user.ok_or("cannot read this process's user id from /proc/self/status")?;
        Ok(Options { network: self.full_network()?, host_address: self.host_address.clone(), user })
    }

    fn podman(&self, args: &[String]) -> Result<Output, String> {
        Command::new(&self.binary).args(args).stdin(Stdio::null()).output().map_err(|e| self.cannot_run(&e))
    }

    fn cannot_run(&self, e: &std::io::Error) -> String {
        format!("cannot run {}: {e}; install rootless Podman or set environments.podman.binary", self.binary)
    }

    /// Run podman and return its stdout. A failure names the subcommand, the exit code and the tail of stderr.
    fn checked<S: AsRef<str>>(&self, args: impl IntoIterator<Item = S>) -> Result<String, String> {
        let args = owned(args);
        let out = self.podman(&args)?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        let code = out.status.code().unwrap_or(-1);
        let stderr = keep_tail(&out.stderr);
        Err(format!("podman {} failed (exit {code}): {}", args.first().map_or("", String::as_str), stderr.trim()))
    }

    /// Stdout of a query, trimmed.
    fn query<S: AsRef<str>>(&self, args: impl IntoIterator<Item = S>) -> Result<String, String> {
        self.checked(args).map(|s| s.trim().to_string())
    }

    /// Whether an object exists. Podman's `exists` commands exit 0 or 1; anything else is an error.
    fn exists<S: AsRef<str>>(&self, args: impl IntoIterator<Item = S>) -> Result<bool, String> {
        let args = owned(args);
        let out = self.podman(&args)?;
        match out.status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            code => {
                Err(format!("podman {} failed (exit {code:?}): {}", args.first().map_or("", String::as_str), keep_tail(&out.stderr).trim()))
            }
        }
    }

    fn container_running(&self, container: &str) -> Result<bool, String> {
        Ok(self.query(["container", "inspect", "--format", "{{.State.Running}}", container])? == "true")
    }

    fn pod_running(&self, pod: &str) -> Result<bool, String> {
        Ok(self.query(["pod", "inspect", "--format", "{{.State}}", pod])? == "Running")
    }

    /// The reference and digest of a pulled image, pulling it first when it is missing. A repository
    /// digest pins the reference; a local image without one is run by its name.
    fn ensure_image(&self, image: &str) -> Result<(String, String), String> {
        if !self.exists(["image", "exists", image])? {
            self.checked(["pull", image])?;
        }
        match normalize_digest(&self.query(["image", "inspect", "--format", "{{.Digest}}", image])?) {
            Some(digest) => Ok((pinned(image, &digest), digest)),
            None => Ok((image.to_string(), self.digest_of(image)?)),
        }
    }

    /// The digest of a local image: its repository digest, else its id.
    fn digest_of(&self, reference: &str) -> Result<String, String> {
        if let Some(digest) = normalize_digest(&self.query(["image", "inspect", "--format", "{{.Digest}}", reference])?) {
            return Ok(digest);
        }
        let id = self.query(["image", "inspect", "--format", "{{.Id}}", reference])?;
        normalize_digest(&id).ok_or_else(|| format!("podman reports no digest or id for {reference}"))
    }

    /// Build a base under `tag` unless an image of that name exists. Podman names a local build
    /// `localhost/<tag>`, so the base is referred to by that name.
    fn ensure_built(&self, tag: &str, source: Source) -> Result<BaseRef, String> {
        let name = local_name(tag);
        if !self.exists(["image", "exists", name.as_str()])? {
            match source {
                Source::Project { context, file } => {
                    self.checked(build_args(&name, file, context)?)?;
                }
                Source::Generated(text) => {
                    // The generated Containerfile lives in a scratch directory, removed whatever the outcome.
                    let context = scratch_path("base");
                    let file = context.join("Containerfile");
                    let built = fs::create_dir_all(&context)
                        .and_then(|()| fs::write(&file, text))
                        .map_err(|e| format!("cannot write the base's Containerfile in {}: {e}", context.display()))
                        .and_then(|()| build_args(&name, &file, &context))
                        .and_then(|args| self.checked(args));
                    let _ = fs::remove_dir_all(&context);
                    built?;
                }
            }
        }
        Ok(BaseRef { reference: name.clone(), digest: self.digest_of(&name)? })
    }

    /// Run a container or command with an optional timeout and log. Output goes to `log` (appended),
    /// or to a scratch file when there is no log. Returns `None` when the timeout fired: the
    /// client is killed and `cleanup` (a podman command) runs. A timed-out command is not an exit code.
    fn run_capture(
        &self,
        args: &[String],
        timeout: Option<Duration>,
        log: Option<&Path>,
        cleanup: Option<&[String]>,
    ) -> Result<Option<ExecResult>, String> {
        let temporary = log.is_none();
        let path = match log {
            Some(p) => p.to_path_buf(),
            None => scratch_path("output"),
        };
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        // Where this run's output starts: the log may already hold earlier runs.
        let start = file.metadata().map_or(0, |m| m.len());
        let errors = file.try_clone().map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let mut child = Command::new(&self.binary)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(file))
            .stderr(Stdio::from(errors))
            .spawn()
            .map_err(|e| self.cannot_run(&e))?;
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| format!("cannot wait for {}: {e}", self.binary))? {
                break status;
            }
            if timeout.is_some_and(|t| started.elapsed() >= t) {
                let _ = child.kill();
                let _ = child.wait();
                if let Some(cleanup) = cleanup {
                    let _ = self.podman(cleanup);
                }
                if temporary {
                    let _ = fs::remove_file(&path);
                }
                return Ok(None);
            }
            std::thread::sleep(POLL);
        };
        let output = read_tail(&path, start);
        if temporary {
            let _ = fs::remove_file(&path);
        }
        Ok(Some(ExecResult { code: status.code().unwrap_or(-1), output }))
    }

    /// Run a step's container and return its result; the step's own timeout removes the container.
    fn run_step_with(&self, step: &StepRun, log: Option<&Path>) -> Result<ExecResult, String> {
        let opts = self.options()?;
        create_overlay_dirs(&step.mounts)?;
        let name = step_name(&step.labels);
        let args = run_args(step, &name, &opts)?;
        let cleanup = ["rm".to_string(), "-f".to_string(), name.clone()];
        match self.run_capture(&args, Some(step.timeout), log, Some(cleanup.as_slice()))? {
            None => Err(format!("{name} timed out after {}s and was removed", step.timeout.as_secs())),
            Some(r) if r.code == PODMAN_ERROR => {
                Err(format!("{name} could not run in its container (podman exit {PODMAN_ERROR}): {}", r.output.trim()))
            }
            Some(r) => Ok(r),
        }
    }
}

impl Runner for PodmanRunner {
    fn kind(&self) -> &'static str {
        "podman"
    }

    fn capabilities(&self) -> Result<Capabilities, String> {
        parse_info(&self.checked(["info", "--format", "json"])?)
    }

    fn ensure_base(&self, spec: &BaseSpec) -> Result<BaseRef, String> {
        if let Some((context, file)) = &spec.containerfile {
            if spec.image.is_some() {
                return Err("a base takes an image or a Containerfile, not both".into());
            }
            if !spec.packages.is_empty() {
                return Err("packages cannot be added to a base built from a Containerfile; install them in that Containerfile".into());
            }
            return self.ensure_built(&spec.tag, Source::Project { context, file });
        }
        let image = spec.image.as_deref().ok_or("a base needs an image or a Containerfile")?;
        if spec.packages.is_empty() {
            let (reference, digest) = self.ensure_image(image)?;
            return Ok(BaseRef { reference, digest });
        }
        let text = containerfile_for(image, &spec.packages)?;
        self.ensure_built(&spec.tag, Source::Generated(text))
    }

    fn image_digest(&self, image: &str) -> Result<String, String> {
        Ok(self.ensure_image(image)?.1)
    }

    fn probe_tool(&self, base: &BaseRef, command: &str) -> Result<String, String> {
        let step = StepRun {
            base: base.clone(),
            mounts: Vec::new(),
            env: BTreeMap::new(),
            secrets: Vec::new(),
            cwd: String::new(),
            command: command.to_string(),
            network: NetworkMode::None,
            // Zero means no limit: a probe runs for seconds, not under the workspace's budget.
            limits: Limits { memory: 0, cpus: 0.0, pids: 0, disk: 0 },
            timeout: TOOL_PROBE,
            log: PathBuf::new(),
            labels: BTreeMap::from([("genie.kind".to_string(), "probe".to_string())]),
        };
        let r = self.run_step_with(&step, None)?;
        if r.code != 0 {
            return Err(format!("tool probe `{command}` failed (exit {}): {}", r.code, r.output.trim()));
        }
        Ok(r.output.trim().to_string())
    }

    fn run_step(&self, step: &StepRun) -> Result<ExecResult, String> {
        self.run_step_with(step, Some(&step.log))
    }

    fn create(&self, ws: &WorkspaceSpec) -> Result<(), String> {
        let opts = self.options()?;
        fs::create_dir_all(&ws.run_dir).map_err(|e| format!("cannot create the run directory {}: {e}", ws.run_dir.display()))?;
        create_overlay_dirs(&ws.mounts)?;
        let pod = pod_name(&ws.id);
        if !self.exists(["pod", "exists", pod.as_str()])? {
            self.checked(pod_create_args(&ws.id, ws.network, &opts)?)?;
        }
        if !self.pod_running(&pod)? {
            self.checked(["pod", "start", pod.as_str()])?;
        }
        let main = main_name(&ws.id);
        if self.exists(["container", "exists", main.as_str()])? {
            if !self.container_running(&main)? {
                self.checked(["start", main.as_str()])?;
            }
        } else {
            self.checked(main_run_args(ws, &opts)?)?;
        }
        Ok(())
    }

    fn exec(&self, ws: &str, e: &Exec) -> Result<ExecResult, String> {
        let args = exec_args(&main_name(ws), &e.cwd, &e.env, false, &["sh", "-c", e.command.as_str()])?;
        match self.run_capture(&args, e.timeout, e.log.as_deref(), None)? {
            None => Err(format!("the command in {ws} timed out after {}s", e.timeout.map_or(0, |t| t.as_secs()))),
            Some(r) if r.code == PODMAN_ERROR => {
                Err(format!("the command in {ws} could not run (podman exit {PODMAN_ERROR}): {}", r.output.trim()))
            }
            Some(r) => Ok(r),
        }
    }

    fn start_service(&self, ws: &WorkspaceSpec, name: &str) -> Result<(), String> {
        let svc = ws.services.iter().find(|s| s.name == name).ok_or_else(|| format!("workspace {} defines no service {name}", ws.id))?;
        let container = service_name(&ws.id, name);
        if !self.exists(["container", "exists", container.as_str()])? {
            self.checked(service_run_args(&ws.id, svc)?)?;
        } else if !self.container_running(&container)? {
            self.checked(["start", container.as_str()])?;
        }
        Ok(())
    }

    fn start_process(&self, ws: &str, name: &str, e: &Exec) -> Result<(), String> {
        self.checked(start_process_args(&main_name(ws), name, e)?).map(|_| ())
    }

    fn probe(&self, ws: &str, p: &Probe) -> Result<bool, String> {
        let script = probe_script(p)?;
        let limit = p.timeout_secs.map_or(PROBE_ATTEMPT, Duration::from_secs);
        // The container's own working directory: a base-only workspace has no /workspace to enter.
        let args = exec_args(&main_name(ws), "", &BTreeMap::new(), false, &["sh", "-c", script.as_str()])?;
        match self.run_capture(&args, Some(limit), None, None)? {
            // An attempt that runs out of time is simply not ready yet.
            None => Ok(false),
            Some(r) => probe_verdict(ws, p, r.code, &r.output),
        }
    }

    fn stop(&self, ws: &str) -> Result<(), String> {
        let pod = pod_name(ws);
        if self.exists(["pod", "exists", pod.as_str()])? {
            self.checked(["pod", "stop", pod.as_str()])?;
        }
        Ok(())
    }

    fn remove(&self, ws: &str) -> Result<(), String> {
        let pod = pod_name(ws);
        if self.exists(["pod", "exists", pod.as_str()])? {
            self.checked(["pod", "rm", "-f", pod.as_str()])?;
        }
        // Anything labelled for the workspace that the pod removal did not reach.
        let filter = format!("label=genie.workspace={ws}");
        let stray = self.checked(["ps", "-a", "--filter", filter.as_str(), "--format", "json"])?;
        for r in parse_containers(&stray)? {
            self.checked(["rm", "-f", r.name.as_str()])?;
        }
        let pods = self.checked(["pod", "ps", "--filter", filter.as_str(), "--format", "json"])?;
        for r in parse_pods(&pods)? {
            self.checked(["pod", "rm", "-f", r.name.as_str()])?;
        }
        Ok(())
    }

    fn list(&self) -> Result<Vec<Resource>, String> {
        let containers = self.checked(["ps", "-a", "--filter", "label=genie.workspace", "--format", "json"])?;
        let pods = self.checked(["pod", "ps", "--filter", "label=genie.workspace", "--format", "json"])?;
        let mut out = parse_containers(&containers)?;
        out.extend(parse_pods(&pods)?);
        Ok(out)
    }

    fn remove_tree(&self, path: &Path) -> Result<(), String> {
        self.checked(unshare_rm_args(path)?).map(|_| ())
    }

    fn prune_bases(&self, keep: &BTreeSet<String>) -> Result<Vec<String>, String> {
        let images = self.checked(["images", "--filter", "label=genie.kind=base", "--format", "json"])?;
        let mut removed = Vec::new();
        // A base still used by a container is refused by Podman: it stays, and the next prune tries again.
        for name in stale_bases(&images, keep)? {
            if self.podman(&["rmi".to_string(), name.clone()])?.status.success() {
                removed.push(name);
            }
        }
        Ok(removed)
    }

    fn harness(&self, ws: &str, cwd: &str, env_names: &[String], argv: &[String]) -> Result<(String, Vec<String>), String> {
        Ok((self.binary.clone(), harness_args(&main_name(ws), cwd, env_names, argv)?))
    }
}

/// Where a base's image is built from: a project's Containerfile with its context directory, or the
/// Containerfile generated for an image and packages.
enum Source<'a> {
    Project { context: &'a Path, file: &'a Path },
    Generated(String),
}

/// The name a locally built base is stored under: a bare tag becomes `localhost/<tag>`, the name Podman gives a local build.
fn local_name(tag: &str) -> String {
    if tag.contains('/') { tag.to_string() } else { format!("localhost/{tag}") }
}

/// The `--network` value for full network from a Podman version. Podman 5 and later have no
/// slirp4netns (it fails with "slirp4netns support has been removed"), so they use pasta, which maps
/// the host's loopback to `host_address`. Earlier versions keep slirp4netns, whose default gateway
/// is `10.0.2.2`, the host address by default.
fn auto_network(version: &str, host_address: &str) -> Result<String, String> {
    let major = major(version).ok_or_else(|| format!("podman version {version:?} is not a version number"))?;
    if major >= 5 { Ok(format!("pasta:--map-host-loopback,{host_address}")) } else { Ok("slirp4netns:allow_host_loopback=true".into()) }
}

/// The Podman server version from `podman info --format json`.
fn info_version(text: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("podman info did not print JSON: {e}"))?;
    v["version"]["Version"].as_str().map(str::to_string).ok_or_else(|| "podman info has no version".to_string())
}

/// The uid and gid of this process, from its effective ids in `/proc/self/status`.
fn current_user() -> Option<(u32, u32)> {
    parse_status_ids(&fs::read_to_string("/proc/self/status").ok()?)
}

/// Effective uid and gid from the `Uid:` and `Gid:` lines of a `/proc/<pid>/status`.
fn parse_status_ids(status: &str) -> Option<(u32, u32)> {
    let field = |key: &str| -> Option<u32> {
        status.lines().find_map(|line| line.strip_prefix(key)).and_then(|rest| rest.split_whitespace().nth(1)).and_then(|v| v.parse().ok())
    };
    Some((field("Uid:")?, field("Gid:")?))
}

fn pod_name(ws: &str) -> String {
    format!("genie-{ws}")
}

fn main_name(ws: &str) -> String {
    format!("genie-{ws}-main")
}

fn service_name(ws: &str, service: &str) -> String {
    format!("genie-{ws}-svc-{service}")
}

/// A name no other step or scratch file has in this process or on this machine, as container-name safe text.
fn unique() -> String {
    let n = UNIQUE.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    format!("{}-{nanos:x}-{n}", std::process::id())
}

fn scratch_path(kind: &str) -> PathBuf {
    std::env::temp_dir().join(format!("genie-{kind}-{}", unique()))
}

fn step_name(labels: &BTreeMap<String, String>) -> String {
    match labels.get("genie.workspace") {
        Some(ws) => format!("genie-{ws}-step-{}", unique()),
        None => format!("genie-step-{}", unique()),
    }
}

fn owned<S: AsRef<str>>(args: impl IntoIterator<Item = S>) -> Vec<String> {
    args.into_iter().map(|a| a.as_ref().to_string()).collect()
}

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

/// A flag and its value as two arguments.
fn pair(flag: &str, value: impl Into<String>) -> Vec<String> {
    vec![flag.to_string(), value.into()]
}

fn label_args(labels: &[(&str, &str)]) -> Vec<String> {
    labels.iter().flat_map(|(k, v)| ["--label".to_string(), format!("{k}={v}")]).collect()
}

fn map_pairs(map: &BTreeMap<String, String>) -> Vec<(&str, &str)> {
    map.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect()
}

/// The network value for a mode: `none`, or the configured network for full access. `registries`
/// parses but no runner supports it in this release.
fn network_value(mode: NetworkMode, opts: &Options) -> Result<String, String> {
    match mode {
        NetworkMode::None => Ok("none".into()),
        NetworkMode::Full => Ok(opts.network.clone()),
        NetworkMode::Registries => Err("network mode registries: not supported in this release".into()),
    }
}

fn user_args(opts: &Options) -> Vec<String> {
    pair("--user", format!("{}:{}", opts.user.0, opts.user.1))
}

/// Resource limits as podman flags. Zero means no limit. Swap is off: memory and memory-swap are equal.
fn limit_args(l: &Limits) -> Vec<String> {
    let mut a = Vec::new();
    if l.memory > 0 {
        a.extend(pair("--memory", l.memory.to_string()));
        a.extend(pair("--memory-swap", l.memory.to_string()));
    }
    if l.cpus > 0.0 {
        a.extend(pair("--cpus", format!("{}", l.cpus)));
    }
    if l.pids > 0 {
        a.extend(pair("--pids-limit", l.pids.to_string()));
    }
    a
}

fn workdir_args(cwd: &str) -> Vec<String> {
    if cwd.is_empty() { Vec::new() } else { pair("-w", cwd) }
}

fn check_env_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if first_ok && chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(format!("{name:?} is not an environment variable name"))
    }
}

/// `-e KEY=VALUE` for each entry. Values are not secrets: secrets are files, never environment.
fn env_args(env: &BTreeMap<String, String>) -> Result<Vec<String>, String> {
    let mut a = Vec::new();
    for (k, v) in env {
        check_env_name(k)?;
        a.extend(pair("-e", format!("{k}={v}")));
    }
    Ok(a)
}

/// The environment of a container: `HOME`, then the host address for full network, then the caller's
/// own variables, which win.
fn container_env(opts: &Options, network: NetworkMode, env: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut all = BTreeMap::from([("HOME".to_string(), HOME.to_string())]);
    if network == NetworkMode::Full {
        all.insert("GENIE_HOST_ADDRESS".to_string(), opts.host_address.clone());
    }
    all.extend(env.clone());
    all
}

/// Creates the upper and work directories of the overlay views among `mounts`. Podman needs them to
/// exist before it mounts a view, and a view's upper directory is the only place its writes go.
fn create_overlay_dirs(mounts: &[Mount]) -> Result<(), String> {
    for m in mounts {
        if let MountMode::Overlay { upper, work } = &m.mode {
            for dir in [upper, work] {
                fs::create_dir_all(dir).map_err(|e| format!("cannot create the overlay directory {}: {e}", dir.display()))?;
            }
        }
    }
    Ok(())
}

/// Secrets as read-only bind mounts of their host files.
fn secret_mounts(secrets: &[(PathBuf, String)]) -> Vec<Mount> {
    secrets.iter().map(|(source, target)| Mount { source: source.clone(), target: target.clone(), mode: MountMode::ReadOnly }).collect()
}

/// A host path as podman takes it in a `-v` spec: absolute, and free of the `:` and `,` that separate
/// the spec's fields and options.
fn mount_path(text: &str, what: &str) -> Result<String, String> {
    if !text.starts_with('/') || text.contains(':') || text.contains(',') || text.contains('\n') {
        return Err(format!("{what} {text:?} must be an absolute path without ':' or ','"));
    }
    Ok(text.to_string())
}

fn host_path(path: &Path, what: &str) -> Result<String, String> {
    let text = path.to_str().ok_or_else(|| format!("{what} {} is not UTF-8", path.display()))?;
    mount_path(text, what)
}

/// The nesting depth of a mount target. Podman applies `-v` options in order, so a parent must come
/// before the mounts beneath it.
fn depth(target: &str) -> usize {
    target.split('/').filter(|s| !s.is_empty()).count()
}

/// `-v` flags for the mounts, parents before children. Read-only, read-write and overlay views.
fn mount_args(mounts: &[Mount]) -> Result<Vec<String>, String> {
    let mut targets = BTreeSet::new();
    let mut specs = Vec::with_capacity(mounts.len());
    for m in mounts {
        let target = mount_path(&m.target, "a mount target")?;
        if !targets.insert(target.clone()) {
            return Err(format!("two mounts target {target}"));
        }
        let source = host_path(&m.source, "a mount source")?;
        let spec = match &m.mode {
            MountMode::ReadOnly => format!("{source}:{target}:ro"),
            MountMode::ReadWrite => format!("{source}:{target}:rw"),
            MountMode::Overlay { upper, work } => format!(
                "{source}:{target}:O,upperdir={},workdir={}",
                host_path(upper, "an overlay upper directory")?,
                host_path(work, "an overlay work directory")?
            ),
        };
        specs.push((depth(&target), spec));
    }
    specs.sort_by_key(|(d, _)| *d);
    Ok(specs.into_iter().flat_map(|(_, spec)| ["-v".to_string(), spec]).collect())
}

/// The podman arguments that run one step in its own container and remove it on exit.
fn run_args(step: &StepRun, name: &str, opts: &Options) -> Result<Vec<String>, String> {
    let mut labels = step.labels.clone();
    labels.entry("genie.kind".to_string()).or_insert_with(|| "step".to_string());
    let mut mounts = step.mounts.clone();
    mounts.extend(secret_mounts(&step.secrets));

    // `--init` reaps the children a command leaves behind and forwards signals to them.
    let mut a = strings(&["run", "--rm", "--init", "--name", name, "--userns=keep-id"]);
    a.extend(user_args(opts));
    a.extend(label_args(&map_pairs(&labels)));
    a.extend(pair("--network", network_value(step.network, opts)?));
    a.extend(limit_args(&step.limits));
    a.extend(env_args(&container_env(opts, step.network, &step.env))?);
    a.extend(workdir_args(&step.cwd));
    a.extend(mount_args(&mounts)?);
    a.push(step.base.reference.clone());
    a.extend(strings(&["sh", "-c"]));
    a.push(step.command.clone());
    Ok(a)
}

/// The pod of a workspace: labelled, sharing the network the workspace's mode asks for.
fn pod_create_args(ws: &str, network: NetworkMode, opts: &Options) -> Result<Vec<String>, String> {
    let mut a = strings(&["pod", "create", "--name"]);
    a.push(pod_name(ws));
    a.extend(label_args(&[("genie.workspace", ws), ("genie.kind", "pod")]));
    a.push("--userns=keep-id".to_string());
    a.extend(pair("--network", network_value(network, opts)?));
    Ok(a)
}

/// The main container of a workspace: it runs `sleep infinity` in the pod, and processes exec into it.
fn main_run_args(ws: &WorkspaceSpec, opts: &Options) -> Result<Vec<String>, String> {
    let mut mounts = ws.mounts.clone();
    mounts.push(Mount { source: ws.run_dir.clone(), target: RUN_DIR.to_string(), mode: MountMode::ReadWrite });
    mounts.extend(secret_mounts(&ws.secrets));

    let mut a = strings(&["run", "-d", "--pod"]);
    a.push(pod_name(&ws.id));
    a.push("--name".to_string());
    a.push(main_name(&ws.id));
    a.push("--init".to_string());
    a.extend(user_args(opts));
    a.extend(label_args(&[("genie.workspace", &ws.id), ("genie.kind", "main")]));
    a.extend(limit_args(&ws.limits));
    a.extend(env_args(&container_env(opts, ws.network, &ws.env))?);
    a.extend(workdir_args(&ws.cwd));
    a.extend(mount_args(&mounts)?);
    a.push(ws.base.reference.clone());
    a.extend(strings(&["sleep", "infinity"]));
    Ok(a)
}

/// A service container in the workspace's pod, from its image, with its command and data mount.
fn service_run_args(ws: &str, svc: &ServiceSpec) -> Result<Vec<String>, String> {
    let mut a = strings(&["run", "-d", "--pod"]);
    a.push(pod_name(ws));
    a.push("--name".to_string());
    a.push(service_name(ws, &svc.name));
    a.extend(label_args(&[("genie.workspace", ws), ("genie.kind", "service")]));
    a.extend(env_args(&svc.env)?);
    let data: Vec<Mount> = svc.data.iter().cloned().collect();
    a.extend(mount_args(&data)?);
    a.push(svc.image.clone());
    a.extend(svc.command.iter().cloned());
    Ok(a)
}

/// `podman exec` into a container: working directory, variables, then the command.
fn exec_args(container: &str, cwd: &str, env: &BTreeMap<String, String>, detached: bool, command: &[&str]) -> Result<Vec<String>, String> {
    let mut a = strings(&["exec"]);
    if detached {
        a.push("-d".to_string());
    }
    a.extend(workdir_args(cwd));
    a.extend(env_args(env)?);
    a.push(container.to_string());
    a.extend(command.iter().map(|s| (*s).to_string()));
    Ok(a)
}

/// The shell script that a detached process runs: its pid goes to `<run dir>/<name>.pid` and its
/// output to `<run dir>/<name>.log`. `exec` makes the pid the command's own.
fn start_process_script(name: &str, command: &str) -> String {
    format!("echo $$ > {RUN_DIR}/{name}.pid; exec sh -c {} >> {RUN_DIR}/{name}.log 2>&1", sh_quote(command))
}

/// The detached exec that starts a named process. Its `Exec.log` and `Exec.timeout` are not used:
/// the output always goes to the run directory, and a detached process has no timeout.
fn start_process_args(container: &str, name: &str, e: &Exec) -> Result<Vec<String>, String> {
    if !valid_name(name) {
        return Err(format!("process name {name:?} must be lowercase letters, digits and dashes"));
    }
    let script = start_process_script(name, &e.command);
    exec_args(container, &e.cwd, &e.env, true, &["sh", "-c", script.as_str()])
}

/// The `podman exec` arguments that run `argv` in workspace `ws`. Only the names of `env_names` are
/// passed: podman reads their values from this process, so they never appear on the command line.
fn harness_args(container: &str, cwd: &str, env_names: &[String], argv: &[String]) -> Result<Vec<String>, String> {
    if argv.is_empty() {
        return Err("the harness command is empty".into());
    }
    let mut a = strings(&["exec", "-i"]);
    a.extend(workdir_args(cwd));
    for name in env_names {
        check_env_name(name)?;
        a.extend(pair("-e", name.clone()));
    }
    a.push(container.to_string());
    a.extend(argv.iter().cloned());
    Ok(a)
}

/// `podman unshare rm -rf <path>`: removes files a container user owns, which the host user cannot.
fn unshare_rm_args(path: &Path) -> Result<Vec<String>, String> {
    let text = path.to_str().ok_or_else(|| format!("{} is not UTF-8", path.display()))?;
    if !text.starts_with('/') {
        return Err(format!("refusing to remove {text:?}: the path must be absolute"));
    }
    let mut a = strings(&["unshare", "rm", "-rf"]);
    a.push(text.to_string());
    Ok(a)
}

/// The Containerfile of a base made of packages on an image. It detects the package manager at build
/// time: apt-get, dnf or apk.
fn containerfile_for(image: &str, packages: &[String]) -> Result<String, String> {
    if packages.is_empty() {
        return Err("a base with packages needs at least one package".into());
    }
    if image.is_empty() || image.contains(char::is_whitespace) {
        return Err(format!("base image {image:?} is not a valid image reference"));
    }
    for p in packages {
        if p.is_empty() || p.contains(char::is_whitespace) {
            return Err(format!("package {p:?} is not a valid package name"));
        }
    }
    let list = packages.iter().map(|p| sh_quote(p)).collect::<Vec<_>>().join(" ");
    Ok(format!(
        "FROM {image}\n\
         RUN set -eu; \\\n\
         \x20   if command -v apt-get >/dev/null 2>&1; then \\\n\
         \x20       apt-get update && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends {list} && rm -rf /var/lib/apt/lists/*; \\\n\
         \x20   elif command -v dnf >/dev/null 2>&1; then \\\n\
         \x20       dnf install -y {list} && dnf clean all; \\\n\
         \x20   elif command -v apk >/dev/null 2>&1; then \\\n\
         \x20       apk add --no-cache {list}; \\\n\
         \x20   else \\\n\
         \x20       echo 'genie: the base image has no apt-get, dnf or apk' >&2; exit 1; \\\n\
         \x20   fi\n"
    ))
}

/// `podman build` of a base: labelled as a genie base so `prune_bases` can find it.
fn build_args(tag: &str, file: &Path, context: &Path) -> Result<Vec<String>, String> {
    let file = file.to_str().ok_or_else(|| format!("{} is not UTF-8", file.display()))?;
    let context = context.to_str().ok_or_else(|| format!("{} is not UTF-8", context.display()))?;
    let mut a = strings(&["build", "--label", "genie.kind=base", "-t"]);
    a.push(tag.to_string());
    a.extend(pair("-f", file));
    a.push(context.to_string());
    Ok(a)
}

/// The probe script for one readiness attempt. A command probe is its own command; an http or tcp probe
/// is a script that exits 0 when the target answers, and `PROBE_CANNOT_RUN` when the base has no tool
/// for the check.
fn probe_script(p: &Probe) -> Result<String, String> {
    let kinds = usize::from(p.command.is_some()) + usize::from(p.http.is_some()) + usize::from(p.tcp.is_some());
    if kinds != 1 {
        return Err("a probe names exactly one of command, http or tcp".into());
    }
    if let Some(command) = &p.command {
        if command.trim().is_empty() {
            return Err("a command probe is empty".into());
        }
        return Ok(command.clone());
    }
    if let Some(url) = &p.http {
        let (host, port) = http_target(url)?;
        let q = sh_quote(url);
        return Ok(format!(
            "if command -v curl >/dev/null 2>&1; then exec curl -fsS -o /dev/null --max-time 5 {q}; \
             elif command -v wget >/dev/null 2>&1; then exec wget -q -O /dev/null -T 5 {q}; \
             else {}; fi",
            tcp_script(&host, port)
        ));
    }
    let (host, port) = host_port(p.tcp.as_deref().unwrap_or_default())?;
    Ok(tcp_script(&host, port))
}

/// What one probe attempt's exit means. Podman's own failure and, for http and tcp probes, the
/// "no tool in the base" code mean the check cannot run (an error); any other non-zero code is
/// "not ready yet". A command probe's exit codes are its own, so only Podman's failure is an error there.
fn probe_verdict(ws: &str, p: &Probe, code: i32, output: &str) -> Result<bool, String> {
    let cannot_run = code == PODMAN_ERROR || (p.command.is_none() && code == PROBE_CANNOT_RUN);
    match code {
        0 => Ok(true),
        _ if cannot_run => Err(format!("the probe cannot run in {ws} (exit {code}): {}", output.trim())),
        _ => Ok(false),
    }
}

/// A tcp connect: bash's `/dev/tcp` when bash is in the base, else `nc -z`, else `PROBE_CANNOT_RUN`.
fn tcp_script(host: &str, port: u16) -> String {
    let dev = sh_quote(&format!("exec 3<>/dev/tcp/{host}/{port}"));
    format!(
        "if command -v bash >/dev/null 2>&1; then exec bash -c {dev}; \
         elif command -v nc >/dev/null 2>&1; then exec nc -z {host} {port}; \
         else echo 'genie: neither bash nor nc is in the base, so tcp probes cannot run' >&2; exit {PROBE_CANNOT_RUN}; fi"
    )
}

/// The host and port of an http URL. The port defaults from the scheme; IPv6 and user info are refused.
fn http_target(url: &str) -> Result<(String, u16), String> {
    let invalid = || format!("http probe {url:?} must start with http:// or https://");
    let (scheme, rest) = url.split_once("://").ok_or_else(invalid)?;
    let default = match scheme {
        "http" => 80,
        "https" => 443,
        _ => return Err(invalid()),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err(format!("http probe {url:?} must not carry user information"));
    }
    if authority.contains(':') { host_port(authority) } else { Ok((checked_host(authority)?, default)) }
}

/// `host:port` of a tcp probe.
fn host_port(text: &str) -> Result<(String, u16), String> {
    let (host, port) = text.rsplit_once(':').ok_or_else(|| format!("tcp probe {text:?} must be host:port"))?;
    let port = port.parse::<u16>().map_err(|_| format!("tcp probe {text:?} has no valid port"))?;
    Ok((checked_host(host)?, port))
}

/// A host name or IPv4 address, as it may appear unquoted in a probe script.
fn checked_host(host: &str) -> Result<String, String> {
    if !host.is_empty() && host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) {
        Ok(host.to_string())
    } else {
        Err(format!("probe host {host:?} is not a plain host name or IPv4 address"))
    }
}

/// Quote text for `sh`: single quotes, with embedded quotes escaped.
fn sh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The digest in `sha256:<hex>` form, from a digest or a bare image id. Empty or unknown text is `None`.
fn normalize_digest(text: &str) -> Option<String> {
    let t = text.trim();
    if let Some(hex) = t.strip_prefix("sha256:") {
        return (!hex.is_empty()).then(|| t.to_string());
    }
    (!t.is_empty() && t.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("sha256:{t}"))
}

/// A run reference that pins a pulled image by its digest. An image that already names a digest stays as it is.
fn pinned(image: &str, digest: &str) -> String {
    if image.contains('@') { image.to_string() } else { format!("{image}@{digest}") }
}

fn arch_name(podman: &str) -> String {
    match podman {
        "amd64" => "x86_64".into(),
        "arm64" => "aarch64".into(),
        other => other.into(),
    }
}

/// The major version of a podman version string, such as `4` for `4.9.3`.
fn major(version: &str) -> Option<u32> {
    version.split('.').next()?.parse().ok()
}

/// The capabilities of the podman that `podman info --format json` describes. Rootful podman is
/// refused: environments run only as the server's own user. Overlay views need podman 4 or later.
fn parse_info(text: &str) -> Result<Capabilities, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("podman info did not print JSON: {e}"))?;
    let host = &v["host"];
    if host["security"]["rootless"].as_bool() != Some(true) {
        return Err("rootful podman is not supported: run environments with rootless Podman as your own user".into());
    }
    let version = v["version"]["Version"].as_str().unwrap_or("unknown").to_string();
    let os = host["os"].as_str().unwrap_or("unknown").to_string();
    let arch = arch_name(host["arch"].as_str().unwrap_or("unknown"));
    let controllers: Vec<&str> =
        host["cgroupControllers"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();

    let mut notes = vec![format!("rootless podman {version}")];
    let mut limits = Vec::new();
    if host["cgroupVersion"].as_str() == Some("v2") {
        for (name, limit) in [("cpu", Limit::Cpu), ("memory", Limit::Memory), ("pids", Limit::Pids)] {
            if controllers.contains(&name) {
                limits.push(limit);
            } else {
                notes.push(format!("the {name} controller is not delegated to this user: {name} limits are not enforced"));
            }
        }
    } else {
        notes.push("cgroup v2 is not in use: cpu, memory and pids limits are not enforced".into());
    }
    let views = if major(&version).is_some_and(|m| m >= 4) { vec![ViewMode::Overlay, ViewMode::Copy] } else { vec![ViewMode::Copy] };
    Ok(Capabilities {
        runner: "podman".into(),
        version,
        os,
        arch,
        base_images: true,
        services: true,
        canonical_paths: true,
        harness: true,
        views,
        network: vec![NetworkMode::None, NetworkMode::Full],
        limits,
        secrets: true,
        capabilities: Vec::new(),
        notes,
    })
}

/// The objects of a `--format json` listing. Empty output is an empty list.
fn json_items(text: &str) -> Result<Vec<Value>, String> {
    if text.trim().is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(text).map_err(|e| format!("podman printed unexpected JSON: {e}"))
}

/// The containers labelled for a workspace, as resources. Unlabelled containers are skipped.
fn parse_containers(text: &str) -> Result<Vec<Resource>, String> {
    let mut out = Vec::new();
    for item in json_items(text)? {
        let labels = &item["Labels"];
        let Some(ws) = labels["genie.workspace"].as_str() else { continue };
        let name =
            item["Names"].as_array().and_then(|n| n.first()).and_then(Value::as_str).or_else(|| item["Id"].as_str()).unwrap_or_default();
        out.push(Resource {
            workspace: ws.to_string(),
            kind: labels["genie.kind"].as_str().unwrap_or("container").to_string(),
            name: name.to_string(),
            running: item["State"].as_str() == Some("running"),
        });
    }
    Ok(out)
}

/// The pods labelled for a workspace, as resources.
fn parse_pods(text: &str) -> Result<Vec<Resource>, String> {
    let mut out = Vec::new();
    for item in json_items(text)? {
        let labels = &item["Labels"];
        let Some(ws) = labels["genie.workspace"].as_str() else { continue };
        out.push(Resource {
            workspace: ws.to_string(),
            kind: labels["genie.kind"].as_str().unwrap_or("pod").to_string(),
            name: item["Name"].as_str().unwrap_or_default().to_string(),
            running: item["Status"].as_str().is_some_and(|s| s.eq_ignore_ascii_case("running")),
        });
    }
    Ok(out)
}

/// The names to remove from a listing of genie base images: every base whose names and id are not in
/// `keep`. A tag in `keep` is written as `genie-base:<hash>`, without the `localhost/` podman adds.
fn stale_bases(text: &str, keep: &BTreeSet<String>) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for item in json_items(text)? {
        let names: Vec<String> =
            item["Names"].as_array().map(|n| n.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
        let id = item["Id"].as_str().unwrap_or_default();
        let kept = names.iter().any(|n| keep.contains(n.strip_prefix("localhost/").unwrap_or(n)))
            || (!id.is_empty() && (keep.contains(id) || keep.contains(&format!("sha256:{id}"))));
        if kept {
            continue;
        }
        match names.first() {
            Some(name) => out.push(name.clone()),
            None if !id.is_empty() => out.push(id.to_string()),
            None => {}
        }
    }
    Ok(out)
}

/// The last `OUTPUT_TAIL` bytes of a command's output, as text.
fn keep_tail(bytes: &[u8]) -> String {
    let from = bytes.len().saturating_sub(OUTPUT_TAIL);
    String::from_utf8_lossy(&bytes[from..]).into_owned()
}

/// The output written to a log file since `start`, at most its last `OUTPUT_TAIL` bytes.
fn read_tail(path: &Path, start: u64) -> String {
    let Ok(mut file) = File::open(path) else { return String::new() };
    let len = file.metadata().map_or(0, |m| m.len());
    let from = start.max(len.saturating_sub(OUTPUT_TAIL as u64));
    let mut buf = Vec::new();
    if file.seek(SeekFrom::Start(from)).is_ok() {
        let _ = file.read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO_V4: &str = r#"{
      "host": {
        "arch": "amd64",
        "cgroupControllers": ["cpu", "io", "memory", "pids"],
        "cgroupManager": "systemd",
        "cgroupVersion": "v2",
        "os": "linux",
        "security": { "rootless": true, "apparmorEnabled": false }
      },
      "version": { "APIVersion": "4.9.3", "Version": "4.9.3", "OsArch": "linux/amd64" }
    }"#;

    fn opts() -> Options {
        Options { network: "slirp4netns:allow_host_loopback=true".into(), host_address: "10.0.2.2".into(), user: (1000, 1001) }
    }

    fn limits() -> Limits {
        Limits { memory: 8 << 30, cpus: 4.0, pids: 4096, disk: 50 << 30 }
    }

    fn base() -> BaseRef {
        BaseRef { reference: "docker.io/library/busybox@sha256:abc".into(), digest: "sha256:abc".into() }
    }

    fn step(network: NetworkMode) -> StepRun {
        StepRun {
            base: base(),
            mounts: Vec::new(),
            env: BTreeMap::new(),
            secrets: Vec::new(),
            cwd: "/workspace".into(),
            command: "make test".into(),
            network,
            limits: limits(),
            timeout: Duration::from_secs(60),
            log: "/tmp/step.log".into(),
            labels: BTreeMap::from([("genie.workspace".to_string(), "shop--api".to_string())]),
        }
    }

    fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2).any(|w| w[0] == flag && w[1] == value)
    }

    fn ro(source: &str, target: &str) -> Mount {
        Mount { source: source.into(), target: target.into(), mode: MountMode::ReadOnly }
    }

    #[test]
    fn a_step_is_labelled_for_its_workspace_and_defaults_to_the_step_kind() {
        let args = run_args(&step(NetworkMode::None), "genie-shop--api-step-1", &opts()).unwrap();
        assert!(has_pair(&args, "--label", "genie.workspace=shop--api"), "{args:?}");
        assert!(has_pair(&args, "--label", "genie.kind=step"), "{args:?}");
        assert!(has_pair(&args, "--name", "genie-shop--api-step-1"));
        assert!(args.contains(&"--rm".to_string()));
    }

    #[test]
    fn a_step_runs_as_the_server_user_mapped_with_keep_id() {
        let args = run_args(&step(NetworkMode::None), "n", &opts()).unwrap();
        assert!(args.contains(&"--userns=keep-id".to_string()), "{args:?}");
        assert!(has_pair(&args, "--user", "1000:1001"), "{args:?}");
    }

    #[test]
    fn a_step_ends_with_its_base_and_the_command_run_by_sh() {
        let args = run_args(&step(NetworkMode::None), "n", &opts()).unwrap();
        assert_eq!(&args[args.len() - 4..], ["docker.io/library/busybox@sha256:abc", "sh", "-c", "make test"]);
    }

    #[test]
    fn network_none_is_none_and_has_no_host_address() {
        let args = run_args(&step(NetworkMode::None), "n", &opts()).unwrap();
        assert!(has_pair(&args, "--network", "none"), "{args:?}");
        assert!(!args.iter().any(|a| a.starts_with("GENIE_HOST_ADDRESS")), "{args:?}");
    }

    #[test]
    fn network_full_uses_the_configured_network_and_names_the_host_address() {
        let args = run_args(&step(NetworkMode::Full), "n", &opts()).unwrap();
        assert!(has_pair(&args, "--network", "slirp4netns:allow_host_loopback=true"), "{args:?}");
        assert!(has_pair(&args, "-e", "GENIE_HOST_ADDRESS=10.0.2.2"), "{args:?}");
    }

    #[test]
    fn network_registries_is_refused_by_name() {
        let err = run_args(&step(NetworkMode::Registries), "n", &opts()).unwrap_err();
        assert!(err.contains("registries") && err.contains("not supported in this release"), "{err}");
    }

    #[test]
    fn limits_are_bytes_cpus_and_pids_with_swap_off() {
        let args = run_args(&step(NetworkMode::None), "n", &opts()).unwrap();
        assert!(has_pair(&args, "--memory", "8589934592"), "{args:?}");
        assert!(has_pair(&args, "--memory-swap", "8589934592"), "{args:?}");
        assert!(has_pair(&args, "--cpus", "4"), "{args:?}");
        assert!(has_pair(&args, "--pids-limit", "4096"), "{args:?}");
    }

    #[test]
    fn a_fractional_cpu_limit_is_written_as_a_decimal() {
        assert_eq!(limit_args(&Limits { memory: 0, cpus: 0.5, pids: 0, disk: 0 }), ["--cpus", "0.5"]);
    }

    #[test]
    fn zero_limits_are_not_passed() {
        let none = Limits { memory: 0, cpus: 0.0, pids: 0, disk: 0 };
        assert!(limit_args(&none).is_empty());
    }

    #[test]
    fn home_is_the_canonical_home_unless_the_caller_overrides_it() {
        let args = run_args(&step(NetworkMode::None), "n", &opts()).unwrap();
        assert!(has_pair(&args, "-e", "HOME=/home/genie"), "{args:?}");
        let mut s = step(NetworkMode::None);
        s.env.insert("HOME".into(), "/home/other".into());
        s.env.insert("CI".into(), "1".into());
        let args = run_args(&s, "n", &opts()).unwrap();
        assert!(has_pair(&args, "-e", "HOME=/home/other"), "{args:?}");
        assert!(has_pair(&args, "-e", "CI=1"), "{args:?}");
        assert!(!args.iter().any(|a| a == "HOME=/home/genie"), "{args:?}");
    }

    #[test]
    fn an_invalid_environment_name_is_refused() {
        let mut s = step(NetworkMode::None);
        s.env.insert("BAD-NAME".into(), "x".into());
        assert!(run_args(&s, "n", &opts()).unwrap_err().contains("BAD-NAME"));
    }

    #[test]
    fn the_working_directory_is_set_only_when_given() {
        let args = run_args(&step(NetworkMode::None), "n", &opts()).unwrap();
        assert!(has_pair(&args, "-w", "/workspace"), "{args:?}");
        let mut s = step(NetworkMode::None);
        s.cwd = String::new();
        assert!(!run_args(&s, "n", &opts()).unwrap().contains(&"-w".to_string()));
    }

    #[test]
    fn read_only_read_write_and_overlay_mounts_have_their_mode_flags() {
        let mounts = vec![
            Mount {
                source: "/src/lower".into(),
                target: "/workspace".into(),
                mode: MountMode::Overlay { upper: "/ws/upper".into(), work: "/ws/work".into() },
            },
            Mount { source: "/data/ro".into(), target: "/ro".into(), mode: MountMode::ReadOnly },
            Mount { source: "/data/out".into(), target: "/out".into(), mode: MountMode::ReadWrite },
        ];
        let args = mount_args(&mounts).unwrap();
        assert!(has_pair(&args, "-v", "/src/lower:/workspace:O,upperdir=/ws/upper,workdir=/ws/work"), "{args:?}");
        assert!(has_pair(&args, "-v", "/data/ro:/ro:ro"), "{args:?}");
        assert!(has_pair(&args, "-v", "/data/out:/out:rw"), "{args:?}");
    }

    #[test]
    fn secrets_are_read_only_bind_mounts_and_never_environment() {
        let mut s = step(NetworkMode::None);
        s.secrets = vec![(PathBuf::from("/run/secrets/host/token"), "/run/secrets/token".into())];
        let args = run_args(&s, "n", &opts()).unwrap();
        assert!(has_pair(&args, "-v", "/run/secrets/host/token:/run/secrets/token:ro"), "{args:?}");
        assert!(!args.windows(2).any(|w| w[0] == "-e" && w[1].contains("token")), "no secret in the environment: {args:?}");
    }

    #[test]
    fn a_parent_mount_comes_before_the_mounts_below_it_whatever_the_input_order() {
        let mounts = vec![ro("/src/api", "/workspace/api"), ro("/src/root", "/workspace"), ro("/src/x", "/workspace/api/x")];
        let specs: Vec<String> = mount_args(&mounts).unwrap().into_iter().filter(|a| a.starts_with('/')).collect();
        assert_eq!(specs, ["/src/root:/workspace:ro", "/src/api:/workspace/api:ro", "/src/x:/workspace/api/x:ro"]);
    }

    #[test]
    fn two_mounts_on_one_target_are_refused() {
        let err = mount_args(&[ro("/a", "/workspace"), ro("/b", "/workspace")]).unwrap_err();
        assert!(err.contains("two mounts target /workspace"), "{err}");
    }

    #[test]
    fn a_relative_target_or_a_colon_or_comma_in_a_path_is_refused() {
        assert!(mount_args(&[ro("/a", "workspace")]).unwrap_err().contains("absolute"));
        assert!(mount_args(&[ro("/a:b", "/workspace")]).unwrap_err().contains("':' or ','"));
        assert!(mount_args(&[ro("/a", "/x,y")]).unwrap_err().contains("':' or ','"));
        let overlay = Mount {
            source: "/a".into(),
            target: "/workspace".into(),
            mode: MountMode::Overlay { upper: "/u,v".into(), work: "/w".into() },
        };
        assert!(mount_args(&[overlay]).is_err());
    }

    #[test]
    fn the_pod_shares_the_network_the_workspace_asks_for() {
        let none = pod_create_args("shop--api", NetworkMode::None, &opts()).unwrap();
        assert_eq!(
            none,
            [
                "pod",
                "create",
                "--name",
                "genie-shop--api",
                "--label",
                "genie.workspace=shop--api",
                "--label",
                "genie.kind=pod",
                "--userns=keep-id",
                "--network",
                "none"
            ]
        );
        let full = pod_create_args("shop--api", NetworkMode::Full, &opts()).unwrap();
        assert!(has_pair(&full, "--network", "slirp4netns:allow_host_loopback=true"), "{full:?}");
    }

    fn workspace(network: NetworkMode) -> WorkspaceSpec {
        WorkspaceSpec {
            id: "shop--api".into(),
            base: base(),
            mounts: vec![Mount { source: "/srv/repos/api".into(), target: "/workspace".into(), mode: MountMode::ReadWrite }],
            env: BTreeMap::from([("CI".to_string(), "1".to_string())]),
            secrets: Vec::new(),
            network,
            limits: limits(),
            services: Vec::new(),
            cwd: "/workspace".into(),
            run_dir: PathBuf::from("/data/env/workspaces/shop--api/run"),
        }
    }

    #[test]
    fn the_main_container_joins_the_pod_and_sleeps_with_init_and_the_run_directory() {
        let args = main_run_args(&workspace(NetworkMode::None), &opts()).unwrap();
        assert!(has_pair(&args, "--pod", "genie-shop--api"), "{args:?}");
        assert!(has_pair(&args, "--name", "genie-shop--api-main"), "{args:?}");
        assert!(args.contains(&"--init".to_string()), "{args:?}");
        assert!(has_pair(&args, "--user", "1000:1001"), "{args:?}");
        assert!(has_pair(&args, "-v", "/data/env/workspaces/shop--api/run:/opt/genie-run:rw"), "{args:?}");
        assert!(has_pair(&args, "-v", "/srv/repos/api:/workspace:rw"), "{args:?}");
        assert_eq!(&args[args.len() - 3..], ["docker.io/library/busybox@sha256:abc", "sleep", "infinity"]);
        assert!(!args.contains(&"--network".to_string()), "the pod owns the network: {args:?}");
        assert!(!args.iter().any(|a| a == "--userns=keep-id"), "the pod owns the user namespace: {args:?}");
    }

    #[test]
    fn the_main_container_of_a_full_network_workspace_names_the_host_address() {
        let args = main_run_args(&workspace(NetworkMode::Full), &opts()).unwrap();
        assert!(has_pair(&args, "-e", "GENIE_HOST_ADDRESS=10.0.2.2"), "{args:?}");
        let none = main_run_args(&workspace(NetworkMode::None), &opts()).unwrap();
        assert!(!none.iter().any(|a| a.starts_with("GENIE_HOST_ADDRESS")), "{none:?}");
    }

    #[test]
    fn the_main_container_carries_its_limits_its_variables_and_its_workdir() {
        let args = main_run_args(&workspace(NetworkMode::None), &opts()).unwrap();
        assert!(has_pair(&args, "--cpus", "4") && has_pair(&args, "--memory", "8589934592"), "{args:?}");
        assert!(has_pair(&args, "-e", "CI=1") && has_pair(&args, "-e", "HOME=/home/genie"), "{args:?}");
        assert!(has_pair(&args, "-w", "/workspace"), "{args:?}");
    }

    #[test]
    fn a_secret_of_the_workspace_is_a_read_only_mount_of_the_main_container() {
        let mut ws = workspace(NetworkMode::None);
        ws.secrets = vec![(PathBuf::from("/s/token"), "/run/secrets/token".into())];
        let args = main_run_args(&ws, &opts()).unwrap();
        assert!(has_pair(&args, "-v", "/s/token:/run/secrets/token:ro"), "{args:?}");
    }

    #[test]
    fn a_service_runs_its_image_with_its_command_and_data_in_the_pod() {
        let svc = ServiceSpec {
            name: "db".into(),
            image: "docker.io/library/postgres:16".into(),
            env: BTreeMap::from([("POSTGRES_HOST_AUTH_METHOD".to_string(), "trust".to_string())]),
            command: vec!["postgres".into(), "-c".into(), "fsync=off".into()],
            data: Some(Mount {
                source: "/data/env/workspaces/shop--api/services/db".into(),
                target: "/var/lib/postgresql/data".into(),
                mode: MountMode::ReadWrite,
            }),
        };
        let args = service_run_args("shop--api", &svc).unwrap();
        assert!(has_pair(&args, "--pod", "genie-shop--api"), "{args:?}");
        assert!(has_pair(&args, "--name", "genie-shop--api-svc-db"), "{args:?}");
        assert!(has_pair(&args, "--label", "genie.kind=service"), "{args:?}");
        assert!(has_pair(&args, "-v", "/data/env/workspaces/shop--api/services/db:/var/lib/postgresql/data:rw"), "{args:?}");
        assert_eq!(&args[args.len() - 4..], ["docker.io/library/postgres:16", "postgres", "-c", "fsync=off"]);
    }

    #[test]
    fn exec_runs_sh_in_the_main_container_with_the_workdir_and_variables() {
        let env = BTreeMap::from([("RUST_LOG".to_string(), "info".to_string())]);
        let args = exec_args("genie-shop--api-main", "/workspace/api", &env, false, &["sh", "-c", "cargo test"]).unwrap();
        assert_eq!(args, ["exec", "-w", "/workspace/api", "-e", "RUST_LOG=info", "genie-shop--api-main", "sh", "-c", "cargo test"]);
    }

    #[test]
    fn a_detached_exec_has_the_d_flag() {
        let args = exec_args("c", "", &BTreeMap::new(), true, &["true"]).unwrap();
        assert_eq!(args, ["exec", "-d", "c", "true"]);
    }

    #[test]
    fn the_start_process_script_writes_a_pid_file_and_execs_the_command_with_its_output_logged() {
        let script = start_process_script("web", "npm start");
        assert_eq!(script, "echo $$ > /opt/genie-run/web.pid; exec sh -c 'npm start' >> /opt/genie-run/web.log 2>&1");
    }

    #[test]
    fn a_process_command_with_quotes_is_quoted_once_for_the_inner_shell() {
        let script = start_process_script("p", "echo 'hi there'");
        assert!(script.contains(r"exec sh -c 'echo '\''hi there'\'''"), "{script}");
    }

    #[test]
    fn a_process_needs_a_valid_name_and_is_started_detached_in_the_main_container() {
        let e = Exec { cwd: "/workspace".into(), command: "sleep 9".into(), env: BTreeMap::new(), timeout: None, log: None };
        let args = start_process_args("genie-shop--api-main", "web", &e).unwrap();
        assert_eq!(&args[..5], ["exec", "-d", "-w", "/workspace", "genie-shop--api-main"]);
        assert!(start_process_args("genie-shop--api-main", "Web Server", &e).unwrap_err().contains("lowercase"));
    }

    #[test]
    fn a_command_probe_is_its_own_command() {
        let p = Probe { command: Some("test -f /workspace/ready".into()), http: None, tcp: None, timeout_secs: None };
        assert_eq!(probe_script(&p).unwrap(), "test -f /workspace/ready");
    }

    #[test]
    fn an_empty_command_probe_is_refused() {
        let p = Probe { command: Some("  ".into()), http: None, tcp: None, timeout_secs: None };
        assert!(probe_script(&p).unwrap_err().contains("empty"));
    }

    #[test]
    fn a_probe_names_exactly_one_kind() {
        let none = Probe { command: None, http: None, tcp: None, timeout_secs: None };
        assert!(probe_script(&none).unwrap_err().contains("exactly one"));
        let two = Probe { command: Some("true".into()), http: None, tcp: Some("db:5432".into()), timeout_secs: None };
        assert!(probe_script(&two).unwrap_err().contains("exactly one"));
    }

    #[test]
    fn an_http_probe_tries_curl_then_wget_then_the_tcp_check_of_its_host() {
        let p = Probe { command: None, http: Some("http://127.0.0.1:8080/health".into()), tcp: None, timeout_secs: None };
        let script = probe_script(&p).unwrap();
        assert!(script.contains("command -v curl") && script.contains("curl -fsS"), "{script}");
        assert!(script.contains("wget -q -O /dev/null -T 5 'http://127.0.0.1:8080/health'"), "{script}");
        assert!(script.contains("exec 3<>/dev/tcp/127.0.0.1/8080"), "{script}");
        assert!(script.contains(&format!("exit {PROBE_CANNOT_RUN}")), "{script}");
    }

    #[test]
    fn an_http_probe_without_a_port_uses_the_scheme_default() {
        let (host, port) = http_target("http://api/health").unwrap();
        assert_eq!((host.as_str(), port), ("api", 80));
        let (_, port) = http_target("https://api.example.test").unwrap();
        assert_eq!(port, 443);
        let (_, port) = http_target("http://api:9000?x=1").unwrap();
        assert_eq!(port, 9000);
    }

    #[test]
    fn an_http_probe_must_be_http_without_user_info_or_ipv6() {
        assert!(http_target("ftp://api/").unwrap_err().contains("http:// or https://"));
        assert!(http_target("api/health").unwrap_err().contains("http:// or https://"));
        assert!(http_target("http://user@api/").unwrap_err().contains("user information"));
        assert!(http_target("http://[::1]:8080/").is_err());
    }

    #[test]
    fn a_tcp_probe_checks_bash_then_nc() {
        let p = Probe { command: None, http: None, tcp: Some("db:5432".into()), timeout_secs: None };
        let script = probe_script(&p).unwrap();
        assert!(script.contains("command -v bash") && script.contains("exec 3<>/dev/tcp/db/5432"), "{script}");
        assert!(script.contains("exec nc -z db 5432"), "{script}");
    }

    #[test]
    fn a_tcp_probe_needs_a_host_and_a_valid_port() {
        assert!(host_port("db").unwrap_err().contains("host:port"));
        assert!(host_port("db:99999").unwrap_err().contains("valid port"));
        assert!(host_port("db/x:5432").unwrap_err().contains("plain host name"));
        assert!(host_port(":5432").unwrap_err().contains("plain host name"));
    }

    #[test]
    fn a_containerfile_installs_the_packages_with_the_detected_manager() {
        let text = containerfile_for("docker.io/library/debian:bookworm-slim", &["jq".to_string(), "git".to_string()]).unwrap();
        assert!(text.starts_with("FROM docker.io/library/debian:bookworm-slim\n"), "{text}");
        assert!(text.contains("apt-get install -y --no-install-recommends 'jq' 'git'"), "{text}");
        assert!(text.contains("dnf install -y 'jq' 'git'"), "{text}");
        assert!(text.contains("apk add --no-cache 'jq' 'git'"), "{text}");
        assert!(text.contains("no apt-get, dnf or apk"), "{text}");
    }

    #[test]
    fn a_containerfile_refuses_no_packages_and_packages_with_spaces() {
        assert!(containerfile_for("img", &[]).is_err());
        assert!(containerfile_for("img", &["two words".to_string()]).unwrap_err().contains("not a valid package name"));
        assert!(containerfile_for("img", &[String::new()]).is_err());
        assert!(containerfile_for("", &["jq".to_string()]).is_err());
    }

    #[test]
    fn the_build_is_labelled_as_a_genie_base_with_its_tag_and_context() {
        let args = build_args("genie-base:abc", Path::new("/tmp/ctx/Containerfile"), Path::new("/tmp/ctx")).unwrap();
        assert_eq!(args, ["build", "--label", "genie.kind=base", "-t", "genie-base:abc", "-f", "/tmp/ctx/Containerfile", "/tmp/ctx"]);
    }

    #[test]
    fn shell_quoting_keeps_text_as_one_word() {
        assert_eq!(sh_quote("jq"), "'jq'");
        assert_eq!(sh_quote("a'b"), r"'a'\''b'");
        assert_eq!(sh_quote("$HOME; rm -rf /"), "'$HOME; rm -rf /'");
    }

    #[test]
    fn a_digest_is_normalized_to_sha256_hex() {
        assert_eq!(normalize_digest("sha256:abc\n").as_deref(), Some("sha256:abc"));
        assert_eq!(normalize_digest("abc").as_deref(), Some("sha256:abc"));
        assert_eq!(normalize_digest(""), None);
        assert_eq!(normalize_digest("<none>"), None);
        assert_eq!(normalize_digest("sha256:"), None);
    }

    #[test]
    fn a_pulled_image_is_pinned_by_its_digest_unless_it_already_is() {
        assert_eq!(pinned("docker.io/library/busybox:latest", "sha256:abc"), "docker.io/library/busybox:latest@sha256:abc");
        assert_eq!(pinned("docker.io/library/busybox@sha256:old", "sha256:abc"), "docker.io/library/busybox@sha256:old");
    }

    #[test]
    fn podman_info_of_a_rootless_v2_host_gives_its_capabilities() {
        let caps = parse_info(INFO_V4).unwrap();
        assert_eq!(
            (caps.runner.as_str(), caps.version.as_str(), caps.os.as_str(), caps.arch.as_str()),
            ("podman", "4.9.3", "linux", "x86_64")
        );
        assert_eq!(caps.views, [ViewMode::Overlay, ViewMode::Copy]);
        assert_eq!(caps.network, [NetworkMode::None, NetworkMode::Full]);
        assert_eq!(caps.limits, [Limit::Cpu, Limit::Memory, Limit::Pids]);
        assert!(caps.base_images && caps.services && caps.canonical_paths && caps.harness && caps.secrets);
        assert!(caps.notes.iter().any(|n| n.contains("rootless")), "{:?}", caps.notes);
    }

    #[test]
    fn rootful_podman_is_refused_by_name() {
        let rootful = INFO_V4.replace(r#""rootless": true"#, r#""rootless": false"#);
        assert!(parse_info(&rootful).unwrap_err().contains("rootful podman is not supported"));
    }

    #[test]
    fn podman_info_that_is_not_json_is_an_error() {
        assert!(parse_info("not json").unwrap_err().contains("did not print JSON"));
    }

    #[test]
    fn a_missing_controller_removes_its_limit_and_says_so() {
        let no_memory = INFO_V4.replace(r#""memory", "pids""#, r#""pids""#);
        let caps = parse_info(&no_memory).unwrap();
        assert_eq!(caps.limits, [Limit::Cpu, Limit::Pids]);
        assert!(caps.notes.iter().any(|n| n.contains("memory") && n.contains("not enforced")), "{:?}", caps.notes);
    }

    #[test]
    fn cgroup_v1_enforces_no_limits() {
        let v1 = INFO_V4.replace(r#""cgroupVersion": "v2""#, r#""cgroupVersion": "v1""#);
        let caps = parse_info(&v1).unwrap();
        assert!(caps.limits.is_empty());
        assert!(caps.notes.iter().any(|n| n.contains("cgroup v2")), "{:?}", caps.notes);
    }

    #[test]
    fn podman_three_has_no_overlay_views_and_arm_maps_to_aarch64() {
        let v3 = INFO_V4.replace(r#""Version": "4.9.3""#, r#""Version": "3.4.4""#).replace(r#""arch": "amd64""#, r#""arch": "arm64""#);
        let caps = parse_info(&v3).unwrap();
        assert_eq!(caps.views, [ViewMode::Copy]);
        assert_eq!(caps.arch, "aarch64");
    }

    #[test]
    fn a_listing_of_containers_keeps_only_the_labelled_ones() {
        let text = r#"[
          {"Id": "a1", "Names": ["genie-shop--api-main"], "State": "running",
           "Labels": {"genie.workspace": "shop--api", "genie.kind": "main"}},
          {"Id": "b2", "Names": ["genie-shop--api-svc-db"], "State": "exited",
           "Labels": {"genie.workspace": "shop--api", "genie.kind": "service"}},
          {"Id": "c3", "Names": ["unrelated"], "State": "running", "Labels": {}}
        ]"#;
        let found = parse_containers(text).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(
            found[0],
            Resource { workspace: "shop--api".into(), kind: "main".into(), name: "genie-shop--api-main".into(), running: true }
        );
        assert!(!found[1].running);
        assert_eq!(found[1].kind, "service");
    }

    #[test]
    fn a_listing_of_pods_reads_their_name_and_status() {
        let text = r#"[{"Id": "p1", "Name": "genie-shop--api", "Status": "Running",
                       "Labels": {"genie.workspace": "shop--api", "genie.kind": "pod"}}]"#;
        let found = parse_pods(text).unwrap();
        assert_eq!(found, [Resource { workspace: "shop--api".into(), kind: "pod".into(), name: "genie-shop--api".into(), running: true }]);
        assert!(parse_pods("").unwrap().is_empty());
        assert!(parse_containers("[]").unwrap().is_empty());
    }

    #[test]
    fn stale_bases_keep_the_pinned_tags_and_name_the_rest() {
        let text = r#"[
          {"Id": "1111", "Names": ["localhost/genie-base:keep1"], "Labels": {"genie.kind": "base"}},
          {"Id": "2222", "Names": ["localhost/genie-base:old"], "Labels": {"genie.kind": "base"}},
          {"Id": "3333", "Names": [], "Labels": {"genie.kind": "base"}}
        ]"#;
        let keep: BTreeSet<String> = ["genie-base:keep1".to_string()].into();
        assert_eq!(stale_bases(text, &keep).unwrap(), ["localhost/genie-base:old", "3333"]);
    }

    #[test]
    fn stale_bases_keep_an_image_whose_id_is_pinned() {
        let text = r#"[{"Id": "abcd", "Names": ["localhost/genie-base:x"]}]"#;
        let keep: BTreeSet<String> = ["sha256:abcd".to_string()].into();
        assert!(stale_bases(text, &keep).unwrap().is_empty());
    }

    #[test]
    fn a_tail_keeps_the_end_of_long_output_only() {
        let mut bytes = vec![b'a'; 100 * 1024];
        bytes.extend_from_slice(b"END");
        let tail = keep_tail(&bytes);
        assert_eq!(tail.len(), OUTPUT_TAIL);
        assert!(tail.ends_with("END"));
        assert_eq!(keep_tail(b"short"), "short");
    }

    #[test]
    fn the_uid_and_gid_come_from_the_effective_ids_of_proc_status() {
        let status = "Name:\tgenie\nUid:\t1000\t1002\t1000\t1000\nGid:\t1001\t1003\t1001\t1001\nFDSize:\t64\n";
        assert_eq!(parse_status_ids(status), Some((1002, 1003)));
        assert_eq!(parse_status_ids("Name:\tx\n"), None);
    }

    #[test]
    fn the_harness_forwards_variable_names_only() {
        let argv = vec!["/opt/genie/bin/genie".to_string(), "agent".to_string()];
        let names = vec!["ANTHROPIC_API_KEY".to_string(), "GENIE_URL".to_string()];
        let args = harness_args("genie-shop--api-main", "/workspace/api", &names, &argv).unwrap();
        assert_eq!(
            args,
            [
                "exec",
                "-i",
                "-w",
                "/workspace/api",
                "-e",
                "ANTHROPIC_API_KEY",
                "-e",
                "GENIE_URL",
                "genie-shop--api-main",
                "/opt/genie/bin/genie",
                "agent"
            ]
        );
        assert!(!args.iter().any(|a| a.contains('=')), "no value and no assignment on the command line: {args:?}");
    }

    #[test]
    fn the_harness_refuses_an_empty_command_and_a_name_that_is_not_a_variable() {
        assert!(harness_args("c", "", &[], &[]).unwrap_err().contains("empty"));
        let argv = vec!["x".to_string()];
        assert!(harness_args("c", "", &["A=B".to_string()], &argv).unwrap_err().contains("not an environment variable"));
    }

    #[test]
    fn remove_tree_asks_unshare_to_remove_an_absolute_path_and_refuses_a_relative_one() {
        assert_eq!(unshare_rm_args(Path::new("/data/env/workspaces/x")).unwrap(), ["unshare", "rm", "-rf", "/data/env/workspaces/x"]);
        assert!(unshare_rm_args(Path::new("workspaces/x")).unwrap_err().contains("absolute"));
    }

    #[test]
    fn a_step_name_carries_its_workspace_when_it_has_one() {
        let named = step_name(&BTreeMap::from([("genie.workspace".to_string(), "shop--api".to_string())]));
        assert!(named.starts_with("genie-shop--api-step-"), "{named}");
        assert!(step_name(&BTreeMap::new()).starts_with("genie-step-"));
        assert_ne!(step_name(&BTreeMap::new()), step_name(&BTreeMap::new()));
    }

    #[test]
    fn the_arch_names_map_to_their_kernel_names() {
        assert_eq!(arch_name("amd64"), "x86_64");
        assert_eq!(arch_name("arm64"), "aarch64");
        assert_eq!(arch_name("riscv64"), "riscv64");
        assert_eq!(major("4.9.3"), Some(4));
        assert_eq!(major("dev"), None);
    }

    #[test]
    fn a_runner_with_no_user_refuses_to_build_commands() {
        let runner =
            PodmanRunner { binary: "podman".into(), network: "n".into(), host_address: "h".into(), user: None, resolved: OnceLock::new() };
        assert!(runner.options().unwrap_err().contains("user id"));
    }

    #[test]
    fn a_step_runs_under_an_init_process_and_removes_itself() {
        let args = run_args(&step(NetworkMode::None), "n", &opts()).unwrap();
        assert!(args.contains(&"--init".to_string()) && args.contains(&"--rm".to_string()), "{args:?}");
    }

    #[test]
    fn a_base_built_here_is_named_under_localhost_unless_it_already_has_a_registry() {
        assert_eq!(local_name("genie-base:abc"), "localhost/genie-base:abc");
        assert_eq!(local_name("registry.test/genie-base:abc"), "registry.test/genie-base:abc");
    }

    #[test]
    fn a_probe_verdict_is_ready_on_zero_and_not_ready_on_other_codes_of_a_command() {
        let command = Probe { command: Some("test -f ready".into()), http: None, tcp: None, timeout_secs: None };
        assert_eq!(probe_verdict("w", &command, 0, ""), Ok(true));
        assert_eq!(probe_verdict("w", &command, 1, ""), Ok(false));
        // A command's own 98 is not the probe tool's "cannot run" code.
        assert_eq!(probe_verdict("w", &command, PROBE_CANNOT_RUN, ""), Ok(false));
        let err = probe_verdict("w", &command, PODMAN_ERROR, "no such container").unwrap_err();
        assert!(err.contains("cannot run in w") && err.contains("no such container"), "{err}");
    }

    #[test]
    fn a_tcp_or_http_probe_reports_a_base_without_the_tool_as_unable_to_run() {
        let tcp = Probe { command: None, http: None, tcp: Some("db:5432".into()), timeout_secs: None };
        assert_eq!(probe_verdict("w", &tcp, 1, ""), Ok(false));
        let err = probe_verdict("w", &tcp, PROBE_CANNOT_RUN, "neither bash nor nc").unwrap_err();
        assert!(err.contains("neither bash nor nc"), "{err}");
    }

    #[test]
    fn auto_picks_pasta_on_podman_five_and_later_and_slirp4netns_before() {
        assert_eq!(auto_network("6.1.4", "10.0.2.2").unwrap(), "pasta:--map-host-loopback,10.0.2.2");
        assert_eq!(auto_network("5.0.0", "10.0.2.2").unwrap(), "pasta:--map-host-loopback,10.0.2.2");
        assert_eq!(auto_network("4.9.3", "10.0.2.2").unwrap(), "slirp4netns:allow_host_loopback=true");
        assert_eq!(auto_network("3.4.4", "10.0.2.2").unwrap(), "slirp4netns:allow_host_loopback=true");
    }

    #[test]
    fn the_pasta_mapping_uses_the_configured_host_address() {
        assert_eq!(auto_network("5.1.0", "172.16.0.1").unwrap(), "pasta:--map-host-loopback,172.16.0.1");
    }

    #[test]
    fn a_version_that_is_not_a_number_is_an_error_not_a_guess() {
        assert!(auto_network("dev", "10.0.2.2").unwrap_err().contains("not a version number"));
    }

    #[test]
    fn the_version_comes_from_podman_info() {
        assert_eq!(info_version(INFO_V4).unwrap(), "4.9.3");
        assert!(info_version("{}").unwrap_err().contains("no version"));
        assert!(info_version("nope").unwrap_err().contains("did not print JSON"));
    }

    #[test]
    fn an_explicit_network_is_used_verbatim_without_asking_podman() {
        // The binary does not exist: an explicit value must not run it.
        let runner = PodmanRunner {
            binary: "/nonexistent/podman".into(),
            network: "slirp4netns:allow_host_loopback=true".into(),
            host_address: "10.0.2.2".into(),
            user: Some((1000, 1000)),
            resolved: OnceLock::new(),
        };
        assert_eq!(runner.options().unwrap().network, "slirp4netns:allow_host_loopback=true");
        let custom = PodmanRunner { network: "pasta:--map-host-loopback,192.0.2.7".into(), ..runner };
        assert_eq!(custom.options().unwrap().network, "pasta:--map-host-loopback,192.0.2.7");
    }

    #[test]
    fn auto_asks_podman_and_a_failed_question_is_an_error_naming_the_binary() {
        let runner = PodmanRunner {
            binary: "/nonexistent/podman".into(),
            network: AUTO_NETWORK.into(),
            host_address: "10.0.2.2".into(),
            user: Some((1000, 1000)),
            resolved: OnceLock::new(),
        };
        let err = runner.options().unwrap_err();
        assert!(err.contains("/nonexistent/podman"), "{err}");
        assert!(runner.resolved.get().is_none(), "a failed query is not cached");
    }

    #[test]
    fn the_runner_is_named_podman() {
        let runner = PodmanRunner::new(&EnvironmentsConfig::default());
        assert_eq!(runner.kind(), "podman");
    }
}
