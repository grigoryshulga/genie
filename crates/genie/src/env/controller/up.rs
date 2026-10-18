//! `up`: bring a task's workspace to ready, stage by stage, and prepare the snapshots its steps
//! produce.
//!
//! Stages: `sources` (the repositories' clones), `environment` (the base, the tools, the snapshots,
//! the views and the runner's workspace), `services` (services and processes). A stage that already
//! succeeded for the same inputs is skipped where skipping is safe; its record stays `done`.
//!
//! Preparation runs in a staging directory, never in the workspace: the step's repositories are a
//! copy of the commit, its earlier outputs are copied in, its outputs are moved into the snapshot
//! when it succeeds. Snapshot keys therefore depend only on the commit, the profile and the runner,
//! so every workspace of a project reuses the snapshots of the others.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use chrono::Utc;
use genie_core::env::caps::{self, Limits, ViewMode};
use genie_core::env::db::{Acquire, EnvWorkspace, PIN_BASE, PIN_SNAPSHOT, StageStart, WsState, workspace_id};
use genie_core::env::key::{self, KEY_SCHEMA, KeyInput, sha256_hex};
use genie_core::env::lock::{BaseLock, RepoLock, WorkspaceLock};
use genie_core::env::spec::{DataPolicy, NetworkMode, Probe, Process, Resolved, ResolvedComponent, ResolvedStep, Tool};
use serde_json::json;

use super::{git_root, load_resolved, policy, secret_names, store_of};
use crate::env::inputs;
use crate::env::layout::Layout;
use crate::env::runner::{BaseRef, BaseSpec, Exec, ExecResult, Mount, MountMode, Runner, ServiceSpec, StepRun, WorkspaceSpec};
use crate::env::store::{self, Store};
use crate::git::store::Placed;
use crate::state::{App, AppError, AppResult};

/// How long a snapshot lease lasts without renewal; its holder renews it every third of this.
const LEASE: Duration = Duration::from_secs(60);
/// How often a process waiting for another's snapshot asks the database, and a probe is retried.
const POLL: Duration = Duration::from_millis(500);
/// The timeout of a step that sets none.
const STEP_TIMEOUT: Duration = Duration::from_secs(3600);
/// How many lines of a failed step's log its error quotes.
const LOG_TAIL: usize = 20;
/// Where a service's data directory appears inside its container.
const SERVICE_DATA: &str = "/data";

/// What `up` is asked to bring up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpRequest {
    pub project: String,
    pub name: String,
    pub task: Option<String>,
    pub profile: String,
    /// Agents run inside the environment: the runner must be able to host the agent harness.
    pub for_agent: bool,
}

/// A failure of one stage of `up`: the stage it happened in, and the error the caller gets.
struct Staged {
    stage: &'static str,
    err: AppError,
}

fn at<T>(stage: &'static str, r: AppResult<T>) -> Result<T, Staged> {
    r.map_err(|err| Staged { stage, err })
}

fn internal<T>(stage: &'static str, r: Result<T, String>) -> Result<T, Staged> {
    r.map_err(|e| Staged { stage, err: AppError::Internal(e) })
}

/// Bring the workspace `req.name` of `req.project` to ready: create it if it is new, otherwise
/// reuse what is already there. On failure the workspace is `failed` at the stage that failed,
/// with the reason, and the error is returned.
pub fn up(app: &App, req: &UpRequest) -> AppResult<EnvWorkspace> {
    let id = workspace_id(&req.project, &req.name);
    let turn = app.envs.turn(&id);
    let _turn = turn.lock().unwrap_or_else(|e| e.into_inner());
    let runner = app.envs.runner(app)?;
    let store = store_of(app)?;
    let ws = app.with_server(|db| db.env_workspace_ensure(&req.project, &req.name, req.task.as_deref(), &req.profile, runner.kind()))?;
    let run = Up { app, req, runner: runner.as_ref(), store: &store, id, owner: app.envs.owner() };
    match run.build(ws) {
        Ok(ws) => Ok(ws),
        Err(Staged { stage, err }) => {
            run.fail(stage, &err);
            Err(err)
        }
    }
}

/// One `up` in progress.
struct Up<'a> {
    app: &'a App,
    req: &'a UpRequest,
    runner: &'a dyn Runner,
    store: &'a Store,
    id: String,
    owner: &'a str,
}

impl Up<'_> {
    fn build(&self, ws: EnvWorkspace) -> Result<EnvWorkspace, Staged> {
        let req = self.req;
        let caps = internal("environment", self.runner.capabilities())?;
        let mut was_live =
            matches!(ws.state, WsState::Ready | WsState::ServicesReady) && internal("environment", is_running(self.runner, &self.id))?;

        // sources: each repository's clone at the task's branch, and the commit it is at
        let (_, placed) = at("sources", crate::git::service::task_workspace(self.app, &req.project, &req.name, req.task.as_deref()))?;
        let repos = internal("sources", repo_locks(&placed))?;
        let begun = self.begin("sources", &digest(&repos))?;
        self.end("sources", begun)?;
        self.transition("sources", WsState::SourcesReady)?;

        // environment: the profile's base, tools, snapshots and the workspace the runner creates
        let (resolved, profile_hash) = at("environment", load_resolved(self.app, &req.project, &req.profile))?;
        let policy = at("environment", policy(self.app))?;
        let secrets = at("environment", secret_names(self.app, &req.project))?;
        let gaps = caps::check(&resolved, &policy, &caps, &|name| secrets.contains(name), req.for_agent);
        if !gaps.is_empty() {
            let reasons: Vec<String> = gaps.iter().map(|g| g.to_string()).collect();
            return Err(Staged { stage: "environment", err: AppError::Bad(reasons.join("\n")) });
        }
        let limits = caps::limits(&resolved, &policy);
        let view = internal("environment", caps::view_mode(&caps).ok_or_else(|| "no view mode is available on this runner".to_string()))?;
        if let Some(used) = ws.size_exclusive.filter(|used| u64::try_from(*used).is_ok_and(|u| u > limits.disk)) {
            return Err(Staged {
                stage: "environment",
                err: AppError::Conflict(format!(
                    "workspace {} uses {used} bytes, over its disk limit of {} bytes: delete work in it, or raise environments.policy.max.disk",
                    req.name, limits.disk
                )),
            });
        }
        let base_spec = internal("environment", base_spec(&resolved, &placed))?;
        let base = internal("environment", self.runner.ensure_base(&base_spec))?;
        let h_env = digest(&(&profile_hash, &repos, &base.digest, caps.runner.as_str(), &caps.version, view.to_string()));
        let begun_env = self.begin("environment", &h_env)?;
        let skip_probes = begun_env == StageStart::Done;
        let old = ws.lock.clone().unwrap_or_default();
        let (tools, comp_tools) = internal("environment", self.tool_versions(&resolved, &base, &old, skip_probes))?;

        let key_base =
            KeyBase { project: &req.project, base: &base.digest, os: &caps.os, arch: &caps.arch, canonical: caps.canonical_paths };
        let prep = Prep {
            app: self.app,
            store: self.store,
            runner: self.runner,
            canonical: caps.canonical_paths,
            project: &req.project,
            owner: self.owner,
            prepare_timeout: Duration::from_secs(self.app.cfg.environments.prepare_timeout_secs),
        };
        let mut snapshots: BTreeMap<String, String> = BTreeMap::new();
        let mut prepared: Vec<(&ResolvedComponent, &ResolvedStep, String)> = Vec::new();
        for comp in &resolved.components {
            let repo = repo_of(&placed, &repos, &comp.repo).map_err(|e| Staged { stage: "environment", err: AppError::Internal(e) })?;
            let comp_tools = comp_tools.get(&comp.repo).cloned().unwrap_or_default();
            let mut earlier: Vec<(String, Vec<String>)> = Vec::new();
            let mut depends: Vec<String> = Vec::new();
            for step in &comp.steps {
                let inputs = internal("environment", inputs::input_hashes(&repo.path, &repo.sha, &step.inputs))?;
                let kin = key_input(&key_base, comp, step, &comp_tools, inputs, depends.clone());
                let key = key::key(&kin);
                let job = Job {
                    key: &key,
                    component: comp,
                    step,
                    earlier: &earlier,
                    repo_dir: &repo.path,
                    sha: &repo.sha,
                    base: &base,
                    limits: &limits,
                };
                internal("environment", prepare_snapshot(&prep, &job))?;
                earlier.push((key.clone(), step.outputs.clone()));
                depends.push(key.clone());
                snapshots.insert(step.qualified.clone(), key.clone());
                prepared.push((comp, step, key));
            }
        }

        // The workspace: what the runner gets for the create, then the lock that records it.
        let ws_dir = self.store.workspace_dir(&self.id);
        let ws_home = store::workspace_home(&self.store.root, &self.id);
        let root = git_root(self.app, &req.project, &req.name);
        let layout = internal("environment", Layout::for_workspace(caps.canonical_paths, &root, &ws_home))?;
        let mut mounts = Vec::new();
        for p in &placed {
            mounts.push(Mount { source: p.path.clone(), target: layout.repo(&p.repo.mount), mode: MountMode::ReadWrite });
        }
        mounts.push(Mount { source: ws_home.clone(), target: layout.home().into(), mode: MountMode::ReadWrite });
        let mut seen_caches = BTreeSet::new();
        for comp in &resolved.components {
            for cache in &comp.caches {
                if !seen_caches.insert(cache.name.clone()) {
                    continue;
                }
                let dir = self.store.cache_dir(&req.project, &cache.name);
                let target = layout.expand(&cache.path);
                internal("environment", ensure_dir(&dir))?;
                if !caps.canonical_paths {
                    internal("environment", ensure_dir(Path::new(&target)))?;
                }
                mounts.push(Mount { source: dir, target, mode: MountMode::ReadWrite });
            }
        }
        for (comp, step, key) in &prepared {
            let repo_mount = &comp.mount;
            for (i, out) in step.outputs.iter().enumerate() {
                let target = output_target(&layout, repo_mount, out);
                let reused = old.snapshots.get(&step.qualified) == Some(key) && Path::new(&target).exists() && !caps.canonical_paths;
                if caps.canonical_paths {
                    let (source, mode) = at("environment", self.view_of(&step.qualified, i, key, view))?;
                    mounts.push(Mount { source, target, mode });
                } else if !reused {
                    // A host workspace holds the output itself: its path is the target, which needs no mount.
                    let snapshot = self.store.snapshot_dir(key).join("out").join(i.to_string());
                    internal("environment", place_copy(self.runner, &snapshot, Path::new(&target)))?;
                }
            }
        }
        let mut secrets = Vec::new();
        for (name, grant) in &resolved.secrets {
            if !grant.steps.iter().any(|s| s == "work") {
                continue;
            }
            let value = at("environment", self.app.with_server(|db| db.env_secret(&req.project, name)))?.ok_or_else(|| Staged {
                stage: "environment",
                err: AppError::Conflict(format!(
                    "secret {name} has no value in project {}: set it with `genie env secret set`",
                    req.project
                )),
            })?;
            let file = ws_dir.join("secrets").join(name);
            internal("environment", write_secret(&file, &value))?;
            secrets.push((file, grant.file.clone()));
        }
        let services: Vec<ServiceSpec> = resolved
            .services
            .iter()
            .map(|(name, s)| {
                let data = match s.data {
                    DataPolicy::Ephemeral => None,
                    DataPolicy::Workspace => Some(ws_dir.join("services").join(name)),
                    DataPolicy::Persistent => {
                        Some(self.store.root.join("volumes").join(&req.project).join(s.volume.as_deref().unwrap_or(name)))
                    }
                };
                ServiceSpec {
                    name: name.clone(),
                    image: s.image.clone(),
                    env: s.env.clone(),
                    command: s.command.clone(),
                    data: data.map(|source| Mount { source, target: SERVICE_DATA.into(), mode: MountMode::ReadWrite }),
                }
            })
            .collect();
        for svc in &services {
            if let Some(m) = &svc.data {
                internal("environment", ensure_dir(&m.source))?;
            }
        }
        let spec = WorkspaceSpec {
            id: self.id.clone(),
            base: base.clone(),
            mounts,
            env: resolved.env.clone(),
            secrets,
            network: resolved.network.work,
            limits: limits.clone(),
            services,
            cwd: layout.root().into(),
            run_dir: store::run_dir(&self.store.root, &self.id),
        };

        // A ready workspace whose inputs changed is restarted, so its processes run what the lock names.
        let lock_changed = ws.lock.is_none()
            || old.profile_hash != profile_hash
            || old.repos != repos
            || old.base.digest != base.digest
            || old.snapshots != snapshots;
        if was_live && lock_changed {
            internal("environment", self.runner.stop(&self.id))?;
            was_live = false;
        }
        internal("environment", self.runner.create(&spec))?;
        let lock = WorkspaceLock {
            profile: req.profile.clone(),
            profile_hash,
            runner: caps.runner.clone(),
            runner_version: caps.version.clone(),
            base: BaseLock { reference: base.reference.clone(), digest: base.digest.clone() },
            repos,
            tools,
            snapshots: snapshots.clone(),
            services: BTreeMap::new(),
            view_mode: view.to_string(),
            created: Utc::now().to_rfc3339(),
        };
        at("environment", self.app.with_server(|db| db.env_set_lock(&self.id, &lock, &view.to_string())))?;
        let mut pins: BTreeSet<(String, String)> = snapshots.values().map(|k| (PIN_SNAPSHOT.to_string(), k.clone())).collect();
        pins.insert((PIN_BASE.to_string(), base.reference.clone()));
        let pins: Vec<(String, String)> = pins.into_iter().collect();
        at("environment", self.app.with_server(|db| db.env_pins_set(&self.id, &pins)))?;
        self.transition("environment", WsState::EnvironmentReady)?;
        self.end("environment", begun_env)?;

        // services: their images, then their processes, each once it is ready
        let svc_hash = digest(&WorkspaceLock { created: String::new(), ..lock.clone() });
        let begun_svc = self.begin("services", &svc_hash)?;
        let start = !(begun_svc == StageStart::Done && was_live);
        let mut digests = BTreeMap::new();
        for name in resolved.services.keys() {
            let image = &resolved.services[name].image;
            digests.insert(name.clone(), internal("services", self.runner.image_digest(image))?);
        }
        if start {
            let ready_timeout = Duration::from_secs(self.app.cfg.environments.ready_timeout_secs);
            for (name, svc) in &resolved.services {
                internal("services", self.runner.start_service(&spec, name))?;
                if let Some(probe) = &svc.ready {
                    internal("services", wait_ready(self.runner, &self.id, &format!("service {name}"), probe, ready_timeout))?;
                }
            }
            let logs = ws_dir.join("logs");
            internal("services", ensure_dir(&logs))?;
            for name in internal("services", process_order(&resolved.processes))? {
                let process: &Process = &resolved.processes[&name];
                let dir = match &process.repo {
                    Some(repo) => layout.repo(&repo_mount_of(&resolved, repo)),
                    None => layout.root().to_string(),
                };
                let exec = Exec {
                    cwd: dir,
                    command: process.command.clone(),
                    env: resolved.env.clone(),
                    timeout: None,
                    log: Some(logs.join(format!("{name}.log"))),
                };
                internal("services", self.runner.start_process(&self.id, &name, &exec))?;
                if let Some(probe) = &process.ready {
                    internal("services", wait_ready(self.runner, &self.id, &format!("process {name}"), probe, ready_timeout))?;
                }
            }
        }
        let mut locked = lock.clone();
        locked.services = digests;
        at("services", self.app.with_server(|db| db.env_set_lock(&self.id, &locked, &view.to_string())))?;
        let keys: Vec<String> = snapshots.values().cloned().collect::<BTreeSet<_>>().into_iter().collect();
        let (exclusive, logical) = internal("services", self.store.sizes(&self.id, &keys))?;
        at("services", self.app.with_server(|db| db.env_set_sizes(&self.id, exclusive as i64, logical as i64)))?;
        self.transition("services", WsState::ServicesReady)?;
        self.end("services", begun_svc)?;
        self.transition("services", WsState::Ready)?;
        at("services", self.app.with_server(|db| db.env_workspace(&self.id)))?
            .ok_or_else(|| Staged { stage: "services", err: AppError::Internal(format!("workspace {} vanished", self.id)) })
    }

    /// The tool versions of the profile's components: probed in the base, or taken from the lock
    /// when the environment stage is done and the lock has them. Returns the lock's map and the
    /// versions of each component's tools.
    #[allow(clippy::type_complexity)]
    fn tool_versions(
        &self,
        resolved: &Resolved,
        base: &BaseRef,
        old: &WorkspaceLock,
        skip_probes: bool,
    ) -> Result<(BTreeMap<String, String>, BTreeMap<String, BTreeMap<String, String>>), String> {
        let mut lock_tools = BTreeMap::new();
        let mut per_component = BTreeMap::new();
        for comp in &resolved.components {
            let mut versions = BTreeMap::new();
            for (name, tool) in &comp.tools {
                let lock_key = format!("{}/{name}", comp.repo);
                let version = match old.tools.get(&lock_key) {
                    Some(v) if skip_probes => v.clone(),
                    _ => probe_tool_version(self.runner, base, &lock_key, tool)?,
                };
                lock_tools.insert(lock_key, version.clone());
                versions.insert(name.clone(), version);
            }
            per_component.insert(comp.repo.clone(), versions);
        }
        Ok((lock_tools, per_component))
    }

    /// The view of output `index` of a snapshot in this workspace (podman: overlay or copy in the store).
    fn view_of(&self, qualified: &str, index: usize, key: &str, mode: ViewMode) -> AppResult<(PathBuf, MountMode)> {
        self.store.view(&self.id, qualified, index, key, mode).map_err(AppError::Internal)
    }

    fn begin(&self, stage: &'static str, input: &str) -> Result<StageStart, Staged> {
        at(stage, self.app.with_server(|db| db.env_stage_begin(&self.id, stage, input, self.owner)))
    }

    /// Record a stage that ran as done. A stage skipped as done has nothing to record.
    fn end(&self, stage: &'static str, begun: StageStart) -> Result<(), Staged> {
        match begun {
            StageStart::Begun => at(stage, self.app.with_server(|db| db.env_stage_finish(&self.id, stage, true, ""))),
            StageStart::Done => Ok(()),
        }
    }

    fn transition(&self, stage: &'static str, state: WsState) -> Result<(), Staged> {
        at(stage, self.app.with_server(|db| db.env_set_state(&self.id, state, None, None)).map(|_| ()))
    }

    /// Record a failed `up`: its stage, its reason, and a stopped runtime when services or
    /// processes may have started. Recording errors are not reported over the original one.
    fn fail(&self, stage: &'static str, err: &AppError) {
        let detail = err.to_string();
        let _ = self.app.with_server(|db| db.env_stage_finish(&self.id, stage, false, &detail));
        let _ = self.app.with_server(|db| db.env_set_state(&self.id, WsState::Failed, Some(stage), Some(&detail)));
        if stage == "services"
            && let Err(e) = self.runner.stop(&self.id)
        {
            eprintln!("genie env: {}: cannot stop after a failed start: {e}", self.id);
        }
    }
}

/// A placed repository of a component, with the commit `up` locked for it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Repo {
    path: PathBuf,
    sha: String,
}

/// The lock of each repository: its commit and the branch it is on.
fn repo_locks(placed: &[Placed]) -> Result<Vec<RepoLock>, String> {
    placed
        .iter()
        .map(|p| {
            let sha = git(&p.path, &["rev-parse", "HEAD"])?;
            let branch = git(&p.path, &["symbolic-ref", "-q", "--short", "HEAD"]).ok().filter(|b| !b.is_empty());
            Ok(RepoLock { name: p.repo.name.clone(), mount: p.repo.mount.clone(), sha, branch })
        })
        .collect()
}

fn repo_of(placed: &[Placed], locks: &[RepoLock], name: &str) -> Result<Repo, String> {
    let p = placed.iter().find(|p| p.repo.name == name).ok_or_else(|| format!("repository {name} is not in the workspace"))?;
    let lock = locks.iter().find(|l| l.name == name).ok_or_else(|| format!("repository {name} has no commit"))?;
    Ok(Repo { path: p.path.clone(), sha: lock.sha.clone() })
}

fn repo_mount_of(resolved: &Resolved, repo: &str) -> String {
    resolved.components.iter().find(|c| c.repo == repo).map(|c| c.mount.clone()).unwrap_or_else(|| ".".into())
}

/// Run `git` in `dir` and return its trimmed standard output.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        return Err(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A digest of plain data: the SHA-256 of its JSON.
fn digest<T: serde::Serialize + ?Sized>(value: &T) -> String {
    // Plain data always serializes; a failure would be a bug in the value's type.
    sha256_hex(&serde_json::to_vec(value).expect("plain data serializes"))
}

/// Whether the runner reports the workspace's main object as running.
fn is_running(runner: &dyn Runner, id: &str) -> Result<bool, String> {
    Ok(runner.list()?.iter().any(|r| r.workspace == id && r.running && (r.kind == "workspace" || r.kind == "main")))
}

/// The base of a profile: the image, or the image with packages, or a Containerfile of a repository.
fn base_spec(resolved: &Resolved, placed: &[Placed]) -> Result<BaseSpec, String> {
    let containerfile = match &resolved.base.containerfile {
        None => None,
        Some(spec) => {
            let (repo, file) = spec.split_once(':').ok_or_else(|| format!("containerfile {spec:?} must be <repo>:<path>"))?;
            let p = placed
                .iter()
                .find(|p| p.repo.name == repo)
                .ok_or_else(|| format!("containerfile repository {repo} is not in the workspace"))?;
            Some((p.path.clone(), p.path.join(file)))
        }
    };
    let content = match &containerfile {
        Some((dir, file)) => {
            let rel = file.strip_prefix(dir).map_err(|e| e.to_string())?.to_string_lossy().into_owned();
            let sha = match placed.iter().find(|p| p.path == *dir) {
                Some(p) => git(&p.path, &["rev-parse", "HEAD"])?,
                None => String::new(),
            };
            git(dir, &["show", &format!("{sha}:{rel}")])?
        }
        None => String::new(),
    };
    let tag_input = json!({ "image": resolved.base.image, "packages": resolved.base.packages, "containerfile": content });
    let hash = digest(&tag_input);
    Ok(BaseSpec {
        image: resolved.base.image.clone(),
        packages: resolved.base.packages.clone(),
        containerfile,
        tag: format!("genie-base:{}", &hash[..16]),
    })
}

/// Probe one tool in the base and check its version against the profile's.
fn probe_tool_version(runner: &dyn Runner, base: &BaseRef, lock_key: &str, tool: &Tool) -> Result<String, String> {
    let output = runner.probe_tool(base, &tool.probe).map_err(|e| format!("tool {lock_key} cannot run in the base: {e}"))?;
    if !version_ok(&tool.version, &output) {
        let found = first_version(&output).unwrap_or_else(|| "no version".into());
        return Err(format!("tool {lock_key} is {found}, the profile needs {}", tool.version));
    }
    Ok(first_version(&output).unwrap_or_else(|| output.lines().next().unwrap_or("").trim().to_string()))
}

/// The first version-like token of a tool's output: `1.79.0` in `cargo 1.79.0 (…)`, `22.3.0` in `v22.3.0`.
pub fn first_version(output: &str) -> Option<String> {
    output.split_whitespace().find_map(|token| {
        let t = token.trim_start_matches(|c: char| !c.is_ascii_digit()).trim_end_matches(|c: char| !c.is_ascii_digit());
        let version = t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.chars().all(|c| c.is_ascii_digit() || c == '.');
        version.then(|| t.to_string())
    })
}

/// Whether a tool's output satisfies the wanted version: `""` or `*` accept any; otherwise the first
/// version of the output must equal the wanted one or start with it followed by a dot.
pub fn version_ok(want: &str, output: &str) -> bool {
    if want.is_empty() || want == "*" {
        return true;
    }
    match first_version(output) {
        Some(found) => found == want || found.starts_with(&format!("{want}.")),
        None => false,
    }
}

/// The inputs that identify a step's key that are the same for every step of a workspace.
struct KeyBase<'a> {
    project: &'a str,
    base: &'a str,
    os: &'a str,
    arch: &'a str,
    canonical: bool,
}

/// The key input of one step: what it runs, from which commit, on which base, with which tools.
/// The layout is `canonical` or `staged`: preparation always runs in a staging root, whatever the
/// workspace's real path is.
fn key_input(
    base: &KeyBase,
    comp: &ResolvedComponent,
    step: &ResolvedStep,
    tools: &BTreeMap<String, String>,
    inputs: BTreeMap<String, String>,
    depends: Vec<String>,
) -> KeyInput {
    KeyInput {
        schema: KEY_SCHEMA,
        scope: format!("project:{}", base.project),
        recipe: comp.recipe.clone(),
        revision: comp.revision,
        recipe_hash: comp.recipe_hash.clone(),
        step: step.qualified.clone(),
        run: step.run.clone(),
        cwd: step.cwd.clone(),
        outputs: step.outputs.clone(),
        env: step.env.clone(),
        secrets: step.secrets.iter().map(|(name, _)| name.clone()).collect(),
        base: base.base.to_string(),
        os: base.os.to_string(),
        arch: base.arch.to_string(),
        layout: if base.canonical { "canonical" } else { "staged" }.to_string(),
        tools: tools.clone(),
        inputs,
        depends,
    }
}

/// Where an output of a repository appears in a workspace: `~/` outputs in the home, others in the repository.
fn output_target(layout: &Layout, mount: &str, output: &str) -> String {
    match output.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", layout.home()),
        None => format!("{}/{}", layout.repo(mount), output),
    }
}

/// The same output inside the staging area of a preparation (`root` for the repositories, `home` for `~/`).
fn staged_output(repo_stage: &Path, home: &Path, output: &str) -> PathBuf {
    match output.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => repo_stage.join(output),
    }
}

/// Create a directory and its parents.
fn ensure_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| format!("{}: {e}", path.display()))
}

/// Write a secret's value to a file only its owner can read.
fn write_secret(path: &Path, value: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    let _ = fs::remove_file(path);
    let mut file =
        fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(value.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}

/// Copy a snapshot's output to `target` as a writable tree of its own (a host workspace holds its outputs in place).
fn place_copy(runner: &dyn Runner, source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        runner.remove_tree(target)?;
    }
    if let Some(parent) = target.parent() {
        ensure_dir(parent)?;
    }
    copy_tree(source, target)?;
    store::make_writable(target)
}

/// `cp -a --reflink=auto source target`: a copy that keeps modes and links, sharing blocks where the filesystem can.
fn copy_tree(source: &Path, target: &Path) -> Result<(), String> {
    let status =
        Command::new("cp").args(["-a", "--reflink=auto"]).arg(source).arg(target).status().map_err(|e| format!("cannot run cp: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("copying {} to {} failed", source.display(), target.display())) }
}

/// The order processes start in: each after the processes it names. Services are not in it; they start first.
pub fn process_order(processes: &BTreeMap<String, Process>) -> Result<Vec<String>, String> {
    let mut order: Vec<String> = Vec::new();
    while order.len() < processes.len() {
        let before = order.len();
        for (name, p) in processes {
            if !order.contains(name) && p.after.iter().all(|dep| !processes.contains_key(dep) || order.contains(dep)) {
                order.push(name.clone());
            }
        }
        if order.len() == before {
            return Err("processes wait for each other in a cycle".into());
        }
    }
    Ok(order)
}

/// Poll a probe until it passes, at most `default` (or the probe's own timeout). A probe that cannot run fails at once.
fn wait_ready(runner: &dyn Runner, id: &str, what: &str, probe: &Probe, default: Duration) -> Result<(), String> {
    let timeout = probe.timeout_secs.map(Duration::from_secs).unwrap_or(default);
    let started = Instant::now();
    loop {
        if runner.probe(id, probe).map_err(|e| format!("{what}: its readiness check cannot run: {e}"))? {
            return Ok(());
        }
        if started.elapsed() >= timeout {
            return Err(format!("{what} is not ready after {}s", timeout.as_secs()));
        }
        std::thread::sleep(POLL);
    }
}

/// What a preparation needs from the controller.
struct Prep<'a> {
    app: &'a App,
    store: &'a Store,
    runner: &'a dyn Runner,
    canonical: bool,
    project: &'a str,
    owner: &'a str,
    prepare_timeout: Duration,
}

/// One step to prepare: its key, its component and what it starts from.
struct Job<'a> {
    key: &'a str,
    component: &'a ResolvedComponent,
    step: &'a ResolvedStep,
    /// The key and outputs of the component's earlier steps, which this step starts from.
    earlier: &'a [(String, Vec<String>)],
    repo_dir: &'a Path,
    sha: &'a str,
    base: &'a BaseRef,
    limits: &'a Limits,
}

/// Make the snapshot of a step exist: take its lease and prepare it, wait for another process that
/// prepares it, or take what is ready. A ready row whose directory is gone is prepared again once.
fn prepare_snapshot(p: &Prep, job: &Job) -> Result<(), String> {
    let scope = format!("project:{}", p.project);
    let label = &job.step.qualified;
    let started = Instant::now();
    let mut rebuilt = false;
    loop {
        let acquired =
            p.app.with_server(|db| db.env_snapshot_acquire(job.key, &scope, label, p.owner, LEASE.as_secs())).map_err(|e| e.to_string())?;
        match acquired {
            Acquire::Ready => {
                if p.store.verify_published(job.key).is_some() {
                    p.app.with_server(|db| db.env_snapshot_touch(job.key)).map_err(|e| e.to_string())?;
                    return Ok(());
                }
                if rebuilt {
                    return Err(format!("snapshot {} of {label} is recorded ready but its directory cannot be verified", job.key));
                }
                rebuilt = true;
                p.app.with_server(|db| db.env_snapshot_mark_missing(job.key)).map_err(|e| e.to_string())?;
            }
            Acquire::Acquired { attempt } => return run_leased(p, job, attempt),
            Acquire::Busy { owner, until } => {
                if started.elapsed() >= p.prepare_timeout {
                    return Err(format!(
                        "snapshot {label} is still being prepared by {owner} (its lease runs until {until}): gave up after {}s",
                        p.prepare_timeout.as_secs()
                    ));
                }
                std::thread::sleep(POLL);
            }
        }
    }
}

/// Prepare under the lease this process holds, renewing it meanwhile. A failure is recorded on the
/// snapshot; its staging directory stays for its log until garbage collection.
fn run_leased(p: &Prep, job: &Job, attempt: i64) -> Result<(), String> {
    let result = std::thread::scope(|scope| {
        let (done, stop) = mpsc::channel::<()>();
        scope.spawn(move || renew(p, job.key, stop));
        let result = build_and_publish(p, job, attempt);
        drop(done);
        result
    });
    if let Err(e) = &result
        && let Err(db) = p.app.with_server(|db| db.env_snapshot_fail(job.key, p.owner, e))
    {
        eprintln!("genie env: cannot record the failure of snapshot {}: {db}", job.key);
    }
    result
}

/// Renew a snapshot's lease every third of its length until `stop` is dropped.
fn renew(p: &Prep, key: &str, stop: mpsc::Receiver<()>) {
    let every = LEASE / 3;
    while let Err(RecvTimeoutError::Timeout) = stop.recv_timeout(every) {
        match p.app.with_server(|db| db.env_snapshot_renew(key, p.owner, LEASE.as_secs())) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!("genie env: the lease of snapshot {key} was taken over");
                return;
            }
            Err(e) => eprintln!("genie env: cannot renew the lease of snapshot {key}: {e}"),
        }
    }
}

/// The work of one attempt: stage the inputs, run the step and its verification, move the outputs
/// into the staging area and publish them.
fn build_and_publish(p: &Prep, job: &Job, attempt: i64) -> Result<(), String> {
    let remove = |path: &Path| p.runner.remove_tree(path);
    if let Some(m) = p.store.verify_published(job.key) {
        // Published by an attempt that died before it recorded the snapshot: the directory is complete.
        return p.app.with_server(|db| db.env_snapshot_publish(job.key, p.owner, m.size as i64, m.files as i64)).map_err(|e| e.to_string());
    }
    p.store.discard_staging(job.key, attempt, &remove)?;
    let staging = p.store.staging_dir(job.key, attempt);
    let root = staging.join("root");
    let home = staging.join("home");
    let secrets_dir = staging.join("secrets");
    let log = staging.join("step.log");
    for dir in [&root, &home, &secrets_dir] {
        ensure_dir(dir)?;
    }
    let step = job.step;
    let mount = job.component.mount.as_str();
    let repo_stage = if mount == "." { root.clone() } else { root.join(mount) };
    let layout = Layout::for_workspace(p.canonical, &root, &home)?;

    store_repo(job, &repo_stage)?;
    for (key, outputs) in job.earlier {
        for (i, out) in outputs.iter().enumerate() {
            let source = p.store.snapshot_dir(key).join("out").join(i.to_string());
            let target = staged_output(&repo_stage, &home, out);
            if target.exists() {
                fs::remove_dir_all(&target).map_err(|e| format!("{}: {e}", target.display()))?;
            }
            if let Some(parent) = target.parent() {
                ensure_dir(parent)?;
            }
            copy_tree(&source, &target)?;
            store::make_writable(&target)?;
        }
    }
    for out in &step.outputs {
        ensure_dir(&staged_output(&repo_stage, &home, out))?;
    }
    let cwd_stage = if step.cwd.is_empty() { repo_stage.clone() } else { repo_stage.join(&step.cwd) };
    ensure_dir(&cwd_stage)?;

    let mut mounts = vec![
        Mount { source: repo_stage.clone(), target: layout.repo(mount), mode: MountMode::ReadWrite },
        Mount { source: home.clone(), target: layout.home().into(), mode: MountMode::ReadWrite },
    ];
    for cache in &job.component.caches {
        let dir = p.store.cache_dir(p.project, &cache.name);
        ensure_dir(&dir)?;
        let target = layout.expand(&cache.path);
        if !p.canonical {
            ensure_dir(Path::new(&target))?;
        }
        mounts.push(Mount { source: dir, target, mode: MountMode::ReadWrite });
    }
    let mut secrets = Vec::new();
    for (name, file) in &step.secrets {
        let value = p
            .app
            .with_server(|db| db.env_secret(p.project, name))
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("step {} needs secret {name}, which project {} has not set", step.qualified, p.project))?;
        let path = secrets_dir.join(name);
        write_secret(&path, &value)?;
        secrets.push((path, file.clone()));
    }
    let mut env = step.env.clone();
    env.entry("HOME".into()).or_insert_with(|| layout.home().into());
    let labels =
        BTreeMap::from([("genie.workspace".to_string(), format!("prep-{}", job.key)), ("genie.kind".to_string(), "step".to_string())]);
    let run = StepRun {
        base: job.base.clone(),
        mounts,
        env,
        secrets,
        cwd: if step.cwd.is_empty() { layout.repo(mount) } else { format!("{}/{}", layout.repo(mount), step.cwd) },
        command: step.run.clone(),
        network: step.network,
        limits: job.limits.clone(),
        timeout: step.timeout_secs.map(Duration::from_secs).unwrap_or(STEP_TIMEOUT),
        log: log.clone(),
        labels,
    };
    let result = p.runner.run_step(&run).map_err(|e| format!("step {} could not run: {e}", step.qualified))?;
    if result.code != 0 {
        return Err(step_failure(&step.qualified, "failed", result.code, &log));
    }
    if let Some(verify) = &step.verify {
        let check = StepRun { command: verify.clone(), network: NetworkMode::None, ..run.clone() };
        let r: ExecResult = p.runner.run_step(&check).map_err(|e| format!("verification of {} could not run: {e}", step.qualified))?;
        if r.code != 0 {
            return Err(step_failure(&step.qualified, "failed its verification", r.code, &log));
        }
    }

    let out = staging.join("out");
    ensure_dir(&out)?;
    for (i, output) in step.outputs.iter().enumerate() {
        let produced = staged_output(&repo_stage, &home, output);
        if !produced.is_dir() {
            return Err(format!("step {} did not produce {output}: the output is not a directory", step.qualified));
        }
        fs::rename(&produced, out.join(i.to_string())).map_err(|e| format!("{}: {e}", produced.display()))?;
    }
    let (size, files) = p.store.publish(job.key, attempt, &step.outputs, &remove)?;
    p.app.with_server(|db| db.env_snapshot_publish(job.key, p.owner, size as i64, files as i64)).map_err(|e| e.to_string())?;
    if let Err(e) = p.store.discard_staging(job.key, attempt, &remove) {
        eprintln!("genie env: staging of {} left behind: {e}", job.key);
    }
    Ok(())
}

/// Stage a component's repository at its commit, with the inputs its steps name.
fn store_repo(job: &Job, repo_stage: &Path) -> Result<(), String> {
    ensure_dir(repo_stage)?;
    inputs::stage_context(job.repo_dir, job.sha, &job.step.inputs, repo_stage)
}

/// A step's failure as its error: the step, the exit code and the last lines of its log.
fn step_failure(qualified: &str, what: &str, code: i32, log: &Path) -> String {
    let text = fs::read_to_string(log).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    let tail = &lines[lines.len().saturating_sub(LOG_TAIL)..];
    let tail = if tail.is_empty() { "(no output)".to_string() } else { tail.join("\n") };
    format!("step {qualified} {what} with exit code {code}\n{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use genie_core::env::spec::{Inputs, Process};

    #[test]
    fn a_version_is_the_first_version_like_token_of_the_output() {
        assert_eq!(first_version("cargo 1.79.0 (3f1a3c0 2024-06-10)").as_deref(), Some("1.79.0"));
        assert_eq!(first_version("v22.3.0\n").as_deref(), Some("22.3.0"));
        assert_eq!(first_version("Python 3.12.4").as_deref(), Some("3.12.4"));
        assert_eq!(first_version("linux x86_64 build"), None, "x86_64 is not a version");
        assert_eq!(first_version(""), None);
    }

    #[test]
    fn a_wanted_version_matches_the_whole_components_it_names() {
        assert!(version_ok("", "anything 9"));
        assert!(version_ok("*", "anything"));
        assert!(version_ok("22", "v22.3.0"));
        assert!(version_ok("22.3", "v22.3.0"));
        assert!(!version_ok("22.3.0", "v22.3.10"), "22.3.10 is not 22.3.0");
        assert!(!version_ok("2", "v22.3.0"), "22 is not 2");
        assert!(!version_ok("3", "no version here"));
    }

    fn component() -> ResolvedComponent {
        ResolvedComponent {
            repo: "api".into(),
            mount: ".".into(),
            recipe: "custom".into(),
            revision: 1,
            recipe_hash: "hash-1".into(),
            tools: BTreeMap::new(),
            caches: Vec::new(),
            steps: Vec::new(),
            commands: BTreeMap::new(),
        }
    }

    fn step(run: &str) -> ResolvedStep {
        ResolvedStep {
            qualified: "api/deps".into(),
            id: "deps".into(),
            run: run.into(),
            cwd: String::new(),
            outputs: vec!["out".into()],
            inputs: Inputs::Files(vec!["deps.txt".into()]),
            env: BTreeMap::from([("MODE".to_string(), "dev".to_string())]),
            network: NetworkMode::Full,
            secrets: vec![("TOKEN".into(), "/run/token".into())],
            verify: None,
            timeout_secs: None,
        }
    }

    fn base<'a>(canonical: bool) -> KeyBase<'a> {
        KeyBase { project: "shop", base: "sha-base", os: "linux", arch: "x86_64", canonical }
    }

    fn key_of(
        comp: &ResolvedComponent,
        s: &ResolvedStep,
        tools: &BTreeMap<String, String>,
        inputs: &[(&str, &str)],
        depends: &[&str],
    ) -> String {
        let inputs = inputs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let depends = depends.iter().map(|d| d.to_string()).collect();
        key::key(&key_input(&base(false), comp, s, tools, inputs, depends))
    }

    #[test]
    fn the_key_input_names_what_the_step_runs_and_nothing_secret() {
        let comp = component();
        let s = step("cp deps.txt out/");
        let kin = key_input(&base(false), &comp, &s, &BTreeMap::new(), BTreeMap::from([("deps.txt".into(), "blob-1".into())]), vec![]);
        assert_eq!((kin.scope.as_str(), kin.step.as_str(), kin.layout.as_str()), ("project:shop", "api/deps", "staged"));
        assert_eq!(kin.env.get("MODE").map(String::as_str), Some("dev"));
        assert_eq!(kin.secrets, vec!["TOKEN".to_string()], "secret names only, never a value");
        assert_eq!(key_input(&base(true), &comp, &s, &BTreeMap::new(), BTreeMap::new(), vec![]).layout, "canonical");
    }

    #[test]
    fn every_part_of_the_key_input_changes_the_key_and_the_same_input_keeps_it() {
        let comp = component();
        let s = step("cp deps.txt out/");
        let tools = BTreeMap::from([("node".to_string(), "22.3.0".to_string())]);
        let same = key_of(&comp, &s, &tools, &[("deps.txt", "blob-1")], &[]);
        assert_eq!(same, key_of(&comp, &s, &tools, &[("deps.txt", "blob-1")], &[]));
        assert_ne!(same, key_of(&comp, &step("cp other.txt out/"), &tools, &[("deps.txt", "blob-1")], &[]), "the command");
        assert_ne!(same, key_of(&comp, &s, &tools, &[("deps.txt", "blob-2")], &[]), "an input's content");
        assert_ne!(same, key_of(&comp, &s, &BTreeMap::new(), &[("deps.txt", "blob-1")], &[]), "a tool version");
        assert_ne!(same, key_of(&comp, &s, &tools, &[("deps.txt", "blob-1")], &["earlier"]), "an earlier step");
        let mut other_env = s.clone();
        other_env.env.insert("MODE".into(), "prod".into());
        assert_ne!(same, key_of(&comp, &other_env, &tools, &[("deps.txt", "blob-1")], &[]), "an environment value");
        let mut other_recipe = comp.clone();
        other_recipe.recipe_hash = "hash-2".into();
        assert_ne!(same, key_of(&other_recipe, &s, &tools, &[("deps.txt", "blob-1")], &[]), "the recipe file");
    }

    #[test]
    fn an_output_lands_in_the_home_or_in_the_repository_and_the_staging_matches() {
        let layout = Layout::for_workspace(false, Path::new("/ws/shop/dev"), Path::new("/ws/home")).unwrap();
        assert_eq!(output_target(&layout, "web", "node_modules"), "/ws/shop/dev/web/node_modules");
        assert_eq!(output_target(&layout, ".", "out"), "/ws/shop/dev/out");
        assert_eq!(output_target(&layout, "web", "~/.cache/x"), "/ws/home/.cache/x");
        let root = Path::new("/stage/root");
        let home = Path::new("/stage/home");
        assert_eq!(staged_output(root, home, "out"), Path::new("/stage/root/out"));
        assert_eq!(staged_output(root, home, "~/.cache/x"), Path::new("/stage/home/.cache/x"));
    }

    #[test]
    fn processes_start_after_the_processes_they_name() {
        let proc = |after: &[&str]| Process {
            repo: None,
            command: "run".into(),
            after: after.iter().map(|s| s.to_string()).collect(),
            ready: None,
        };
        let processes =
            BTreeMap::from([("web".to_string(), proc(&["api"])), ("api".to_string(), proc(&["db"])), ("db".to_string(), proc(&[]))]);
        assert_eq!(process_order(&processes).unwrap(), vec!["db", "api", "web"]);
        let cyclic = BTreeMap::from([("a".to_string(), proc(&["b"])), ("b".to_string(), proc(&["a"]))]);
        assert!(process_order(&cyclic).unwrap_err().contains("cycle"));
    }

    #[test]
    fn a_failed_step_names_the_step_its_code_and_the_last_lines_of_its_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("step.log");
        let lines: Vec<String> = (1..=30).map(|i| format!("line {i}")).collect();
        fs::write(&log, lines.join("\n")).unwrap();
        let text = step_failure("api/deps", "failed", 2, &log);
        assert!(text.starts_with("step api/deps failed with exit code 2"), "{text}");
        assert!(text.contains("line 30") && text.contains("line 11") && !text.contains("line 10\n"), "the last 20 lines: {text}");
        assert!(step_failure("api/deps", "failed", 1, &dir.path().join("none.log")).contains("(no output)"));
    }
}
