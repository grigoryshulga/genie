//! The environment controller: what a task's workspace environment is built from and how it is
//! driven (`up`, `stop`, `delete`, `status`, `run`), and the server's housekeeping (`reconcile`,
//! `gc`). Snapshot preparation and the stages of `up` live in `up.rs`; housekeeping in `gc.rs`.
//!
//! Everything here blocks: callers run it on a blocking thread (`App::blocking`). Within this
//! process one workspace is driven at a time (its turn lock); snapshots are single-flight across
//! processes through their database lease.

mod gc;
mod up;

pub use gc::{GcReport, ReconcileReport, gc, reconcile};
pub use up::{UpRequest, up};

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use genie_core::GenieError;
use genie_core::env::caps::{self, Capabilities, Gap, Limits, Policy};
use genie_core::env::db::{EnvStage, EnvWorkspace, WsState, workspace_id};
use genie_core::env::spec::{RepoInfo, Resolved};
use genie_core::repos::ProjectRepo;

use super::catalog;
use super::layout::Layout;
use super::runner::{Exec, ExecResult, Runner, select};
use super::store::{self, Store};
use crate::config::EnvironmentsLimits;
use crate::state::{App, AppError, AppResult};

/// The profile an agent or `up` without a name uses.
pub const DEFAULT_PROFILE: &str = "default";

/// How long one command of `run` may take before it is killed.
const RUN_TIMEOUT: Duration = Duration::from_secs(3600);

/// The state of the controller in this process: the runner, the owner of the leases it takes, and
/// one turn lock per workspace.
pub struct Envs {
    owner: String,
    /// Selected on first use from `environments.runner`; a failure to select is kept too, so every call says why.
    runner: OnceLock<Result<Arc<dyn Runner>, String>>,
    turns: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl Default for Envs {
    fn default() -> Self {
        Envs::new()
    }
}

impl Envs {
    pub fn new() -> Envs {
        // The owner names this process's leases and stages: a restart has another one, which is how
        // reconciliation knows what the previous process left behind.
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
        Envs { owner: format!("{}-{nanos:x}", std::process::id()), runner: OnceLock::new(), turns: Mutex::new(HashMap::new()) }
    }

    /// The owner of this process's leases and stages.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The runner of this process, selected once from the configuration.
    pub fn runner(&self, app: &App) -> AppResult<Arc<dyn Runner>> {
        self.runner.get_or_init(|| select(&app.cfg.environments, &app.data, &app.cfg.runtime.sandbox)).clone().map_err(AppError::Internal)
    }

    /// The turn lock of a workspace: one `up`, `stop` or `delete` of it runs at a time in this process.
    fn turn(&self, id: &str) -> Arc<Mutex<()>> {
        let mut turns = self.turns.lock().unwrap_or_else(|e| e.into_inner());
        turns.entry(id.to_string()).or_default().clone()
    }
}

/// What `check` reports about a profile on this server's runner. Nothing is changed by it.
#[derive(Debug, Clone)]
pub struct CheckReport {
    pub profile: String,
    pub profile_hash: String,
    pub runner: Capabilities,
    pub components: Vec<String>,
    pub steps: usize,
    pub services: Vec<String>,
    pub processes: Vec<String>,
    pub checks: Vec<String>,
    pub limits: Limits,
    /// Everything that keeps the profile from running here, all at once. Empty when it can run.
    pub gaps: Vec<Gap>,
}

/// A command to run in a ready workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandRef {
    /// A command of the profile: `test` runs every repository's `test` in order, `web/test` only the
    /// repository `web`'s; a name that no repository defines is a profile check, run at the root.
    Named(String),
    /// A shell command line, run at the workspace root or in the directory given to `run`.
    Raw(String),
}

/// One command `run` executes: its label, the directory it runs in (`.` for the workspace root, else
/// a repository's mount) and the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Planned {
    pub label: String,
    pub dir: String,
    pub command: String,
}

/// How a workspace uses its disk: its own bytes, its bytes with the snapshots it uses, and the
/// profile's disk limit (`None` when the profile cannot be read).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    pub exclusive: u64,
    pub logical: u64,
    pub disk_limit: Option<u64>,
}

/// A workspace with its stages and disk use.
#[derive(Debug, Clone)]
pub struct WorkspaceStatus {
    pub workspace: EnvWorkspace,
    pub stages: Vec<EnvStage>,
    pub usage: Usage,
}

/// Check a profile against this server's runner and the policy, without side effects. Every gap is
/// reported: a profile that cannot run here says all the reasons at once.
pub fn check(app: &App, project: &str, profile: &str, for_agent: bool) -> AppResult<CheckReport> {
    let runner = app.envs.runner(app)?;
    let caps = runner.capabilities().map_err(AppError::Internal)?;
    let (resolved, profile_hash) = load_resolved(app, project, profile)?;
    let secrets = secret_names(app, project)?;
    let policy = policy(app)?;
    let gaps = caps::check(&resolved, &policy, &caps, &|name| secrets.contains(name), for_agent);
    Ok(CheckReport {
        profile: profile.into(),
        profile_hash,
        components: resolved.components.iter().map(|c| format!("{} ({} r{})", c.repo, c.recipe, c.revision)).collect(),
        steps: resolved.components.iter().map(|c| c.steps.len()).sum(),
        services: resolved.services.keys().cloned().collect(),
        processes: resolved.processes.keys().cloned().collect(),
        checks: resolved.checks.keys().cloned().collect(),
        limits: caps::limits(&resolved, &policy),
        runner: caps,
        gaps,
    })
}

/// The profile agents of a project run in: `default` when the project has that profile file, else none.
/// A file that cannot be read is logged and treated as none, so the agent runs without an environment.
pub fn profile_for(app: &App, project: &str) -> Option<String> {
    match catalog::load_profile(&app.data, project, DEFAULT_PROFILE) {
        Ok(Some(_)) => Some(DEFAULT_PROFILE.into()),
        Ok(None) => None,
        Err(e) => {
            eprintln!("genie env: {e}");
            None
        }
    }
}

/// Stop a workspace's processes and services; its data stays. Idempotent.
pub fn stop(app: &App, project: &str, name: &str) -> AppResult<EnvWorkspace> {
    let id = workspace_id(project, name);
    let turn = app.envs.turn(&id);
    let _turn = turn.lock().unwrap_or_else(|e| e.into_inner());
    workspace(app, project, name)?;
    let runner = app.envs.runner(app)?;
    runner.stop(&id).map_err(AppError::Internal)?;
    app.with_server(|db| db.env_set_state(&id, WsState::Stopped, None, None))
}

/// Delete a workspace: its runtime objects, its directory and its source clones. Work that is not
/// on the git host (uncommitted, or commits no remote branch has) refuses the deletion unless `force`.
pub fn delete(app: &App, project: &str, name: &str, force: bool) -> AppResult<EnvWorkspace> {
    let id = workspace_id(project, name);
    let turn = app.envs.turn(&id);
    let _turn = turn.lock().unwrap_or_else(|e| e.into_inner());
    let ws = workspace(app, project, name)?;
    if !force {
        let root = git_root(app, project, name);
        let unsaved = if root.exists() { crate::runtime::workspace_unsaved(&root) } else { Vec::new() };
        if !unsaved.is_empty() {
            return Err(AppError::Conflict(format!(
                "workspace {name} has work that is not saved: {}. Push or commit it, or delete with force to discard it",
                unsaved.join("; ")
            )));
        }
    }
    let runner = app.envs.runner(app)?;
    let store = store_of(app)?;
    app.with_server(|db| db.env_set_state(&id, WsState::Deleting, None, None))?;
    finish_delete(app, runner.as_ref(), &store, &ws)
}

/// Remove what a workspace owns and mark it deleted. Also finishes a deletion a restart interrupted.
fn finish_delete(app: &App, runner: &dyn Runner, store: &Store, ws: &EnvWorkspace) -> AppResult<EnvWorkspace> {
    let remove = |p: &Path| runner.remove_tree(p);
    runner.remove(&ws.id).map_err(AppError::Internal)?;
    store.remove_workspace(&ws.id, &remove).map_err(AppError::Internal)?;
    let root = git_root(app, &ws.project, &ws.name);
    if root.exists() {
        runner.remove_tree(&root).map_err(AppError::Internal)?;
    }
    app.with_server(|db| db.env_set_state(&ws.id, WsState::Deleted, None, None))
}

/// A workspace with its stages and disk use. Records the sizes it measured.
pub fn status(app: &App, project: &str, name: &str) -> AppResult<WorkspaceStatus> {
    let ws = workspace(app, project, name)?;
    let stages = app.with_server(|db| db.env_stages(&ws.id))?;
    let (exclusive, logical) = match &ws.lock {
        Some(lock) => {
            let keys: Vec<String> = lock.snapshots.values().cloned().collect::<BTreeSet<_>>().into_iter().collect();
            store_of(app)?.sizes(&ws.id, &keys).map_err(AppError::Internal)?
        }
        None => (0, 0),
    };
    app.with_server(|db| db.env_set_sizes(&ws.id, exclusive as i64, logical as i64))?;
    let disk_limit = limits_of(app, project, &ws.profile).ok().map(|l| l.disk);
    let workspace = workspace(app, project, name)?;
    Ok(WorkspaceStatus { workspace, stages, usage: Usage { exclusive, logical, disk_limit } })
}

/// Run a command of the profile, or a raw command line, in a ready workspace. A profile that has
/// changed since the workspace was prepared is refused: its commands would not be the ones it runs.
/// Stops at the first failing command and returns its exit code with the output so far.
pub fn run(app: &App, project: &str, name: &str, command: CommandRef, cwd: &str) -> AppResult<ExecResult> {
    let ws = workspace(app, project, name)?;
    if !matches!(ws.state, WsState::Ready | WsState::ServicesReady) {
        return Err(AppError::Conflict(format!(
            "workspace {name} is {}, not ready: run `genie env up --name {name}` first",
            ws.state.as_str()
        )));
    }
    let lock = ws.lock.clone().ok_or_else(|| AppError::Conflict(format!("workspace {name} has no environment yet")))?;
    if cwd.split('/').any(|part| part == "..") || cwd.starts_with('/') {
        return Err(AppError::Bad(format!("cwd {cwd:?} must be a path inside the workspace")));
    }
    let (resolved, hash) = load_resolved(app, project, &ws.profile)?;
    if hash != lock.profile_hash {
        return Err(AppError::Conflict(format!(
            "profile {} changed since workspace {name} was prepared: run `genie env up --name {name}` again",
            ws.profile
        )));
    }
    let plan = plan_commands(&resolved, &command).map_err(AppError::Bad)?;
    let runner = app.envs.runner(app)?;
    let caps = runner.capabilities().map_err(AppError::Internal)?;
    let store = store_of(app)?;
    let layout = Layout::for_workspace(caps.canonical_paths, &git_root(app, project, name), &store::workspace_home(&store.root, &ws.id))
        .map_err(AppError::Internal)?;
    let mut output = String::new();
    for p in plan {
        let dir = if cwd.is_empty() { layout.repo(&p.dir) } else { format!("{}/{}", layout.root(), cwd.trim_matches('/')) };
        let exec = Exec { cwd: dir, command: p.command, env: BTreeMap::new(), timeout: Some(RUN_TIMEOUT), log: None };
        let r = runner.exec(&ws.id, &exec).map_err(AppError::Internal)?;
        output.push_str(&format!("== {} (exit {}) ==\n{}", p.label, r.code, r.output));
        if !output.ends_with('\n') {
            output.push('\n');
        }
        if r.code != 0 {
            return Ok(ExecResult { code: r.code, output });
        }
    }
    Ok(ExecResult { code: 0, output })
}

/// The commands `run` executes for `command`, in order. Pure: the profile is all it reads.
pub fn plan_commands(r: &Resolved, command: &CommandRef) -> Result<Vec<Planned>, String> {
    match command {
        CommandRef::Raw(text) => Ok(vec![Planned { label: "command".into(), dir: ".".into(), command: text.clone() }]),
        CommandRef::Named(name) => {
            if let Some((repo, cmd)) = name.split_once('/') {
                let comp = r.components.iter().find(|c| c.repo == repo).ok_or_else(|| format!("the profile has no repository {repo:?}"))?;
                let text = comp.commands.get(cmd).ok_or_else(|| {
                    format!("repository {repo} has no command {cmd:?}; its commands are: {}", names(comp.commands.keys()))
                })?;
                return Ok(vec![Planned { label: name.clone(), dir: comp.mount.clone(), command: text.clone() }]);
            }
            let runs: Vec<Planned> = r
                .components
                .iter()
                .filter_map(|c| {
                    c.commands.get(name).map(|text| Planned {
                        label: format!("{}/{name}", c.repo),
                        dir: c.mount.clone(),
                        command: text.clone(),
                    })
                })
                .collect();
            if !runs.is_empty() {
                return Ok(runs);
            }
            match r.checks.get(name) {
                Some(text) => Ok(vec![Planned { label: name.clone(), dir: ".".into(), command: text.clone() }]),
                None => {
                    let mut known: Vec<&str> = r.checks.keys().map(String::as_str).collect();
                    known.extend(r.components.iter().flat_map(|c| c.commands.keys().map(String::as_str)));
                    known.sort_unstable();
                    known.dedup();
                    Err(format!(
                        "no command or check named {name:?}; the profile has: {}",
                        if known.is_empty() { "none".into() } else { known.join(", ") }
                    ))
                }
            }
        }
    }
}

fn names<'a>(keys: impl Iterator<Item = &'a String>) -> String {
    let v: Vec<&str> = keys.map(String::as_str).collect();
    if v.is_empty() { "none".into() } else { v.join(", ") }
}

/// The store of this server, opened (its directories created).
pub(crate) fn store_of(app: &App) -> AppResult<Store> {
    Store::open(&app.cfg.environments, &app.data).map_err(AppError::Internal)
}

/// The directory of a task workspace of a project: the source clones of an environment live there.
pub(crate) fn git_root(app: &App, project: &str, name: &str) -> PathBuf {
    crate::git::store::workspace_root(app, project, name)
}

/// The workspace called `name`, unless it does not exist or is deleted.
pub(crate) fn workspace(app: &App, project: &str, name: &str) -> AppResult<EnvWorkspace> {
    let id = workspace_id(project, name);
    app.with_server(|db| db.env_workspace(&id))?
        .filter(|ws| ws.state != WsState::Deleted)
        .ok_or_else(|| GenieError::not_found(format!("workspace {name} not found in project {project}: run `genie env up` first")).into())
}

/// The profile resolved against the project's repositories, with the hash of its file.
pub(crate) fn load_resolved(app: &App, project: &str, profile: &str) -> AppResult<(Resolved, String)> {
    let repos = project_repos(app, project)?;
    let infos: Vec<RepoInfo> = repos.iter().map(|r| RepoInfo { name: r.name.clone(), mount: r.mount.clone() }).collect();
    let resolved = catalog::resolve_profile(&app.data, project, profile, &infos).map_err(|problems| AppError::Bad(problems.join("\n")))?;
    let (_, hash) = catalog::load_profile(&app.data, project, profile)
        .map_err(AppError::Bad)?
        .ok_or_else(|| AppError::Bad(format!("no profile named {profile:?} in project {project}")))?;
    Ok((resolved, hash))
}

/// The repositories of a project as the profile sees them.
pub(crate) fn project_repos(app: &App, project: &str) -> AppResult<Vec<ProjectRepo>> {
    app.with_server(|db| db.repos(project))
}

/// The names of the project's environment secrets that have a value.
pub(crate) fn secret_names(app: &App, project: &str) -> AppResult<BTreeSet<String>> {
    Ok(app.with_server(|db| db.env_secret_names(project))?.into_iter().map(|s| s.name).collect())
}

/// The policy the configuration sets, with its sizes read.
pub(crate) fn policy(app: &App) -> AppResult<Policy> {
    let cfg = &app.cfg.environments;
    let limits = |l: &EnvironmentsLimits| -> AppResult<Limits> {
        Ok(Limits {
            memory: genie_core::env::spec::parse_size(&l.memory).map_err(AppError::Internal)?,
            cpus: l.cpus,
            pids: l.pids,
            disk: genie_core::env::spec::parse_size(&l.disk).map_err(AppError::Internal)?,
        })
    };
    Ok(Policy {
        network: cfg.policy.network.clone(),
        require_limits: cfg.policy.require_limits,
        defaults: limits(&cfg.policy.defaults)?,
        max: limits(&cfg.policy.max)?,
        capabilities: cfg.policy.capabilities.clone(),
    })
}

/// The limits a profile runs with on this server's policy.
pub(crate) fn limits_of(app: &App, project: &str, profile: &str) -> AppResult<Limits> {
    let (resolved, _) = load_resolved(app, project, profile)?;
    Ok(caps::limits(&resolved, &policy(app)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use genie_core::env::spec::{Cache, ResolvedComponent, ResolvedStep, Service};

    fn command_component(repo: &str, mount: &str, commands: &[(&str, &str)]) -> ResolvedComponent {
        ResolvedComponent {
            repo: repo.into(),
            mount: mount.into(),
            recipe: "custom".into(),
            revision: 1,
            recipe_hash: String::new(),
            tools: BTreeMap::new(),
            caches: Vec::<Cache>::new(),
            steps: Vec::<ResolvedStep>::new(),
            commands: commands.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    fn resolved() -> Resolved {
        Resolved {
            components: vec![
                command_component("api", ".", &[("test", "cargo test"), ("lint", "cargo clippy")]),
                command_component("web", "web", &[("test", "npm test")]),
            ],
            services: BTreeMap::<String, Service>::new(),
            processes: BTreeMap::new(),
            checks: BTreeMap::from([("smoke".to_string(), "curl -f localhost".to_string())]),
            requires: Default::default(),
            network: Default::default(),
            env: BTreeMap::new(),
            secrets: BTreeMap::new(),
            base: Default::default(),
        }
    }

    fn labels(plan: &[Planned]) -> Vec<(&str, &str, &str)> {
        plan.iter().map(|p| (p.label.as_str(), p.dir.as_str(), p.command.as_str())).collect()
    }

    #[test]
    fn a_bare_name_runs_every_repository_that_defines_it_in_order() {
        let plan = plan_commands(&resolved(), &CommandRef::Named("test".into())).unwrap();
        assert_eq!(labels(&plan), vec![("api/test", ".", "cargo test"), ("web/test", "web", "npm test")]);
    }

    #[test]
    fn a_qualified_name_runs_only_that_repository() {
        let plan = plan_commands(&resolved(), &CommandRef::Named("web/test".into())).unwrap();
        assert_eq!(labels(&plan), vec![("web/test", "web", "npm test")]);
        let err = plan_commands(&resolved(), &CommandRef::Named("web/lint".into())).unwrap_err();
        assert!(err.contains("no command \"lint\"") && err.contains("its commands are: test"), "{err}");
        assert!(plan_commands(&resolved(), &CommandRef::Named("nope/test".into())).unwrap_err().contains("no repository"));
    }

    #[test]
    fn a_name_no_repository_defines_is_a_check_at_the_root_and_an_unknown_one_lists_what_exists() {
        let plan = plan_commands(&resolved(), &CommandRef::Named("smoke".into())).unwrap();
        assert_eq!(labels(&plan), vec![("smoke", ".", "curl -f localhost")]);
        let err = plan_commands(&resolved(), &CommandRef::Named("deploy".into())).unwrap_err();
        assert!(err.contains("smoke") && err.contains("lint") && err.contains("test"), "{err}");
    }

    #[test]
    fn a_raw_command_runs_as_given_at_the_root() {
        let plan = plan_commands(&resolved(), &CommandRef::Raw("ls -la".into())).unwrap();
        assert_eq!(labels(&plan), vec![("command", ".", "ls -la")]);
    }

    #[test]
    fn a_workspace_id_joins_the_project_and_the_name() {
        assert_eq!(workspace_id("shop", "api"), "shop--api");
    }
}
