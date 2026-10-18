//! The runner contract: what the controller asks of an environment runtime (`podman` in a
//! container, `host` on the machine itself).
//!
//! The controller builds the specs below from a resolved profile and calls these methods; the
//! runners never read profiles or the database. Every method blocks, and the controller calls
//! them from blocking threads.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use genie_core::env::caps::{Capabilities, Limits};
use genie_core::env::spec::{NetworkMode, Probe};

use crate::config::EnvironmentsConfig;

/// How a mount is exposed to the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MountMode {
    ReadOnly,
    ReadWrite,
    /// A copy-on-write view: the source is the lower layer, writes go to `upper`, with `work` as
    /// the overlay's scratch directory.
    Overlay {
        upper: PathBuf,
        work: PathBuf,
    },
}

/// A host path made visible at `target` inside the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mount {
    pub source: PathBuf,
    pub target: String,
    pub mode: MountMode,
}

/// What a base image is built from: a registry image, or packages on top of one, or a Containerfile
/// of a project repository. `tag` is the name the built base gets (`genie-base:<hash16>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseSpec {
    pub image: Option<String>,
    pub packages: Vec<String>,
    /// The build context directory and the Containerfile inside it.
    pub containerfile: Option<(PathBuf, PathBuf)>,
    pub tag: String,
}

/// A resolved base: what it runs on, and the digest that identifies it in keys and locks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseRef {
    pub reference: String,
    pub digest: String,
}

/// One step run in its own short-lived workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRun {
    pub base: BaseRef,
    pub mounts: Vec<Mount>,
    pub env: BTreeMap<String, String>,
    /// Host files granted to the step, with the path they appear at.
    pub secrets: Vec<(PathBuf, String)>,
    pub cwd: String,
    /// Run with `sh -c`.
    pub command: String,
    pub network: NetworkMode,
    pub limits: Limits,
    pub timeout: Duration,
    /// Where stdout and stderr are appended.
    pub log: PathBuf,
    /// Labels that let the runtime find what it created (`genie.workspace`, `genie.kind`).
    pub labels: BTreeMap<String, String>,
}

/// A service that runs beside a workspace (a database, say), from an image.
#[derive(Debug, Clone, PartialEq)]
pub struct ServiceSpec {
    pub name: String,
    pub image: String,
    pub env: BTreeMap<String, String>,
    pub command: Vec<String>,
    /// Its data directory, when its policy keeps data.
    pub data: Option<Mount>,
}

/// A long-lived workspace: its repositories, views and home, and what runs beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkspaceSpec {
    pub id: String,
    pub base: BaseRef,
    pub mounts: Vec<Mount>,
    pub env: BTreeMap<String, String>,
    pub secrets: Vec<(PathBuf, String)>,
    pub network: NetworkMode,
    pub limits: Limits,
    pub services: Vec<ServiceSpec>,
    pub cwd: String,
    /// A host directory for pid files and state of processes started in the workspace.
    pub run_dir: PathBuf,
}

/// A command run inside a workspace.
#[derive(Debug, Clone, PartialEq)]
pub struct Exec {
    pub cwd: String,
    pub command: String,
    pub env: BTreeMap<String, String>,
    pub timeout: Option<Duration>,
    pub log: Option<PathBuf>,
}

/// The outcome of a command: its exit code and its combined output (the last 64 KiB).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecResult {
    pub code: i32,
    pub output: String,
}

/// A runtime object the runner created, found by `Runner::list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub workspace: String,
    pub kind: String,
    pub name: String,
    pub running: bool,
}

/// The operations of an environment runner. Implementations must be idempotent where the
/// signature says so: the controller retries after a crash and after a restart.
pub trait Runner: Send + Sync {
    /// The runner's name as written in config and locks: `podman` or `host`.
    fn kind(&self) -> &'static str;
    /// What this runner can do, probed on the machine it runs on.
    fn capabilities(&self) -> Result<Capabilities, String>;
    /// Build or find the base of a profile and report its reference and digest.
    fn ensure_base(&self, spec: &BaseSpec) -> Result<BaseRef, String>;
    /// The digest of a service's image, pulling it when missing.
    fn image_digest(&self, image: &str) -> Result<String, String>;
    /// Run a tool probe (such as `cargo --version`) in the base and return its output.
    fn probe_tool(&self, base: &BaseRef, command: &str) -> Result<String, String>;
    /// Run one step in an ephemeral workspace with its own network mode.
    fn run_step(&self, step: &StepRun) -> Result<ExecResult, String>;
    /// Create the workspace and start it. Idempotent: an existing one is started if stopped.
    fn create(&self, ws: &WorkspaceSpec) -> Result<(), String>;
    /// Run a command inside a started workspace.
    fn exec(&self, ws: &str, e: &Exec) -> Result<ExecResult, String>;
    /// Start a service of the workspace from its image.
    fn start_service(&self, ws: &WorkspaceSpec, name: &str) -> Result<(), String>;
    /// Start a process detached; its pid goes to a file in the workspace's run directory.
    fn start_process(&self, ws: &str, name: &str, e: &Exec) -> Result<(), String>;
    /// One readiness attempt. `Ok(false)` means not ready yet; `Err` means the check cannot run.
    fn probe(&self, ws: &str, p: &Probe) -> Result<bool, String>;
    /// Stop the workspace's processes and keep its data. Idempotent.
    fn stop(&self, ws: &str) -> Result<(), String>;
    /// Remove the workspace's runtime objects. Idempotent, and never touches source clones.
    fn remove(&self, ws: &str) -> Result<(), String>;
    /// The runtime objects this runner created, for reconciliation after a restart.
    fn list(&self) -> Result<Vec<Resource>, String>;
    /// Remove a directory tree that may hold files owned by a container user.
    fn remove_tree(&self, path: &Path) -> Result<(), String>;
    /// Remove the bases this runner built that are not in `keep`; returns what was removed.
    fn prune_bases(&self, keep: &BTreeSet<String>) -> Result<Vec<String>, String>;
    /// The program and arguments that run `argv` inside workspace `ws`. Only the env var names in
    /// `env_names` are forwarded from the calling process, so values never appear on the command line.
    fn harness(&self, ws: &str, cwd: &str, env_names: &[String], argv: &[String]) -> Result<(String, Vec<String>), String>;
}

/// The runner the server uses, from `environments.runner`. Only the runners of later phases can be
/// selected, so until they exist every choice is refused with its name.
pub fn select(cfg: &EnvironmentsConfig) -> Result<Arc<dyn Runner>, String> {
    match cfg.runner.as_str() {
        "podman" => Err("the podman runner is not available yet in this build; set environments.runner to auto or host once it is".into()),
        "host" => Err("the host runner is not available yet in this build".into()),
        _ => Err("neither the podman nor the host runner is available yet in this build; auto needs one of them".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_configured_runner_is_refused_until_its_phase_lands() {
        let mut cfg = EnvironmentsConfig::default();
        for runner in ["auto", "podman", "host"] {
            cfg.runner = runner.into();
            let err = select(&cfg).err().unwrap_or_else(|| panic!("{runner} unexpectedly available"));
            assert!(err.contains("available yet"), "{runner}: {err}");
        }
    }

    #[test]
    fn a_base_and_a_mount_compare_by_value() {
        let a = Mount { source: "/a".into(), target: "/workspace".into(), mode: MountMode::ReadOnly };
        let mut b = a.clone();
        assert_eq!(a, b);
        b.mode = MountMode::Overlay { upper: "/u".into(), work: "/w".into() };
        assert_ne!(a, b);
        assert_eq!(BaseRef { reference: "host".into(), digest: "host:linux-x86_64".into() }.reference, "host");
    }
}
