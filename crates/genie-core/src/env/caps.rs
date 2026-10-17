//! What a runner can do, what the server allows, and what an environment needs: the three
//! sets intersected before any work starts.
//!
//! `check` returns every gap, not the first one, so an operator sees the whole list at once.
//! A requirement that cannot be enforced is a gap; it is never quietly weakened.

use std::fmt;

use serde::Serialize;

use super::spec::{NetworkMode, Resolved, parse_size};

/// How a snapshot is made visible in a workspace. The first one a runner supports is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    /// A copy-on-write overlay: writes go to the workspace's own upper layer.
    Overlay,
    /// A full copy: slower and larger, always available.
    Copy,
}

impl fmt::Display for ViewMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ViewMode::Overlay => "overlay",
            ViewMode::Copy => "copy",
        })
    }
}

/// A resource a runner can enforce for a workspace. Disk is measured, not enforced, so it is not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Limit {
    Cpu,
    Memory,
    Pids,
    Disk,
}

impl fmt::Display for Limit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Limit::Cpu => "cpu",
            Limit::Memory => "memory",
            Limit::Pids => "pids",
            Limit::Disk => "disk",
        })
    }
}

/// What a runner reports about itself: static and probed facts.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub runner: String,
    pub version: String,
    pub os: String,
    pub arch: String,
    pub base_images: bool,
    pub services: bool,
    /// Workspaces see the canonical paths (`/workspace`, `/home/genie`), not host paths.
    pub canonical_paths: bool,
    /// The agent harness can run inside an environment.
    pub harness: bool,
    pub views: Vec<ViewMode>,
    pub network: Vec<NetworkMode>,
    pub limits: Vec<Limit>,
    pub secrets: bool,
    pub capabilities: Vec<String>,
    /// Plain facts for people, such as what is not enforced here.
    pub notes: Vec<String>,
}

/// Resource amounts: sizes in bytes, cpus as a number, pids as a count.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub memory: u64,
    pub cpus: f64,
    pub pids: u64,
    pub disk: u64,
}

/// The server's rules for a project's environments (from `environments.policy`, sizes parsed).
#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    /// Network modes the server allows.
    pub network: Vec<NetworkMode>,
    /// A runner that cannot enforce cpu, memory and pids is a gap.
    pub require_limits: bool,
    /// Used for what a profile does not ask for.
    pub defaults: Limits,
    /// Nothing may ask for more than this.
    pub max: Limits,
    /// Capabilities the server grants on top of what a runner provides.
    pub capabilities: Vec<String>,
}

/// One unmet requirement. Shown as `item: reason`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gap {
    pub item: String,
    pub reason: String,
}

impl fmt::Display for Gap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.item, self.reason)
    }
}

fn gap(item: &str, reason: impl Into<String>) -> Gap {
    Gap { item: item.to_string(), reason: reason.into() }
}

/// Every requirement of `r` that the runner, the policy or the project cannot meet.
///
/// `secrets_defined` tells whether the project has a value for a secret name. `needs_harness`
/// is true when agents will run inside the environment.
pub fn check(r: &Resolved, policy: &Policy, caps: &Capabilities, secrets_defined: &dyn Fn(&str) -> bool, needs_harness: bool) -> Vec<Gap> {
    let mut gaps = Vec::new();
    let runner = caps.runner.as_str();

    if let Some(os) = &r.requires.os
        && *os != caps.os
    {
        gaps.push(gap("os", format!("the environment needs {os}, the {runner} runner runs on {}", caps.os)));
    }
    if !r.requires.arch.is_empty() && !r.requires.arch.contains(&caps.arch) {
        gaps.push(gap("arch", format!("the environment needs one of [{}], the {runner} runner is {}", r.requires.arch.join(", "), caps.arch)));
    }

    let base_set = r.base.image.is_some() || r.base.containerfile.is_some() || !r.base.packages.is_empty();
    if base_set && !caps.base_images {
        gaps.push(gap("base image", format!("the {runner} runner cannot build or pull base images")));
    }
    if !r.services.is_empty() && !caps.services {
        gaps.push(gap("services", format!("the {runner} runner cannot run services")));
    }

    // Every network mode a step or the work phase uses must be supported by the runner and allowed by policy.
    let mut used: Vec<NetworkMode> = r.components.iter().flat_map(|c| c.steps.iter().map(|s| s.network)).collect();
    used.push(r.network.work);
    used.sort();
    used.dedup();
    for mode in used {
        if mode == NetworkMode::Registries {
            gaps.push(gap("network mode registries", "not supported in this release"));
        } else if !caps.network.contains(&mode) {
            gaps.push(gap(&format!("network mode {mode}"), format!("the {runner} runner does not support it")));
        } else if !policy.network.contains(&mode) {
            gaps.push(gap(&format!("network mode {mode}"), "server policy does not allow it"));
        }
    }

    check_resources(r, policy, &mut gaps);

    if policy.require_limits {
        for (limit, item) in [(Limit::Cpu, "cpu limit"), (Limit::Memory, "memory limit"), (Limit::Pids, "pids limit")] {
            if !caps.limits.contains(&limit) {
                gaps.push(gap(item, format!("the {runner} runner cannot enforce it, and server policy requires limits (requireLimits)")));
            }
        }
    }

    // A capability must be provided by the runner and granted by the server: the intersection.
    for capability in &r.requires.capabilities {
        let in_runner = caps.capabilities.contains(capability);
        let in_policy = policy.capabilities.contains(capability);
        let reason = match (in_runner, in_policy) {
            (true, true) => continue,
            (false, false) => "neither the runner nor server policy provides it".to_string(),
            (false, true) => format!("the {runner} runner does not provide it"),
            (true, false) => "server policy does not grant it".to_string(),
        };
        gaps.push(gap(&format!("capability {capability}"), reason));
    }

    for name in r.secrets.keys() {
        if !secrets_defined(name) {
            gaps.push(gap(&format!("secret {name}"), "not set for this project (genie env secret set)"));
        }
    }
    if !r.secrets.is_empty() && !caps.secrets {
        gaps.push(gap("secrets", format!("the {runner} runner cannot deliver secret files")));
    }

    let snapshots_used = r.components.iter().any(|c| c.steps.iter().any(|s| !s.outputs.is_empty()));
    if snapshots_used && view_mode(caps).is_none() {
        gaps.push(gap("views", format!("the {runner} runner offers no way to view a snapshot (overlay or copy)")));
    }

    if needs_harness && !caps.harness {
        gaps.push(gap("agent harness", format!("the {runner} runner cannot run the agent harness inside the environment")));
    }
    gaps
}

/// Requests above the server's maximum, and sizes that cannot be read.
fn check_resources(r: &Resolved, policy: &Policy, gaps: &mut Vec<Gap>) {
    let sized = |item: &str, text: &Option<String>, max: u64, gaps: &mut Vec<Gap>| {
        let Some(text) = text else { return };
        match parse_size(text) {
            Ok(bytes) if bytes > max => {
                gaps.push(gap(item, format!("the environment asks for {}, server policy allows at most {}", size_text(bytes), size_text(max))))
            }
            Ok(_) => {}
            Err(e) => gaps.push(gap(item, e)),
        }
    };
    sized("memory", &r.requires.memory, policy.max.memory, gaps);
    sized("disk", &r.requires.disk, policy.max.disk, gaps);
    if let Some(cpus) = r.requires.cpus
        && cpus > policy.max.cpus
    {
        gaps.push(gap("cpus", format!("the environment asks for {cpus}, server policy allows at most {}", policy.max.cpus)));
    }
    if let Some(pids) = r.requires.pids
        && pids > policy.max.pids
    {
        gaps.push(gap("pids", format!("the environment asks for {pids}, server policy allows at most {}", policy.max.pids)));
    }
}

/// The limits a workspace runs with: what the profile asks for, else the policy default, never above the maximum.
/// A size that cannot be read falls back to the default; `check` reports it as a gap.
pub fn limits(r: &Resolved, policy: &Policy) -> Limits {
    let size = |text: &Option<String>, default: u64, max: u64| text.as_deref().and_then(|t| parse_size(t).ok()).unwrap_or(default).min(max);
    Limits {
        memory: size(&r.requires.memory, policy.defaults.memory, policy.max.memory),
        cpus: r.requires.cpus.unwrap_or(policy.defaults.cpus).min(policy.max.cpus),
        pids: r.requires.pids.unwrap_or(policy.defaults.pids).min(policy.max.pids),
        disk: size(&r.requires.disk, policy.defaults.disk, policy.max.disk),
    }
}

/// The view mode for snapshots on this runner: overlay when supported, else copy, else none.
pub fn view_mode(caps: &Capabilities) -> Option<ViewMode> {
    [ViewMode::Overlay, ViewMode::Copy].into_iter().find(|m| caps.views.contains(m))
}

/// A size for a message: whole GiB or MiB when it is one, else bytes.
fn size_text(bytes: u64) -> String {
    const GIB: u64 = 1 << 30;
    const MIB: u64 = 1 << 20;
    if bytes % GIB == 0 {
        format!("{}GiB", bytes / GIB)
    } else if bytes % MIB == 0 {
        format!("{}MiB", bytes / MIB)
    } else {
        format!("{bytes} bytes")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::env::spec::{Base, Cache, DataPolicy, Inputs, Requirements, ResolvedComponent, ResolvedStep, SecretGrant, Service};

    const GIB: u64 = 1 << 30;

    fn podman() -> Capabilities {
        Capabilities {
            runner: "podman".into(),
            version: "5.0.0".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            base_images: true,
            services: true,
            canonical_paths: true,
            harness: true,
            views: vec![ViewMode::Overlay, ViewMode::Copy],
            network: vec![NetworkMode::None, NetworkMode::Full],
            limits: vec![Limit::Cpu, Limit::Memory, Limit::Pids],
            secrets: true,
            capabilities: vec!["gpu".into()],
            notes: vec![],
        }
    }

    fn host() -> Capabilities {
        Capabilities {
            runner: "host".into(),
            version: "1".into(),
            os: "linux".into(),
            arch: "x86_64".into(),
            base_images: false,
            services: false,
            canonical_paths: false,
            harness: false,
            views: vec![ViewMode::Copy],
            network: vec![NetworkMode::None, NetworkMode::Full],
            limits: vec![],
            secrets: true,
            capabilities: vec![],
            notes: vec!["limits are not enforced".into()],
        }
    }

    fn limits_of(memory: u64, cpus: f64, pids: u64, disk: u64) -> Limits {
        Limits { memory, cpus, pids, disk }
    }

    /// The policy without limit enforcement: for tests of other gaps on the host runner.
    fn lenient() -> Policy {
        Policy { require_limits: false, ..policy() }
    }

    fn policy() -> Policy {
        Policy {
            network: vec![NetworkMode::None, NetworkMode::Full],
            require_limits: true,
            defaults: limits_of(8 * GIB, 4.0, 4096, 50 * GIB),
            max: limits_of(32 * GIB, 16.0, 16384, 200 * GIB),
            capabilities: vec!["gpu".into()],
        }
    }

    fn step(id: &str, network: NetworkMode, outputs: &[&str]) -> ResolvedStep {
        ResolvedStep {
            qualified: format!("api/{id}"),
            id: id.into(),
            run: "true".into(),
            cwd: String::new(),
            outputs: outputs.iter().map(|o| o.to_string()).collect(),
            inputs: Inputs::default(),
            env: BTreeMap::new(),
            network,
            secrets: vec![],
            verify: None,
            timeout_secs: None,
        }
    }

    /// An environment with one component whose single step has the given network and outputs.
    fn resolved(network: NetworkMode, outputs: &[&str]) -> Resolved {
        Resolved {
            components: vec![ResolvedComponent {
                repo: "api".into(),
                mount: "api".into(),
                recipe: "custom".into(),
                revision: 1,
                recipe_hash: "h".into(),
                tools: BTreeMap::new(),
                caches: Vec::<Cache>::new(),
                steps: vec![step("install", network, outputs)],
                commands: BTreeMap::new(),
            }],
            services: BTreeMap::new(),
            processes: BTreeMap::new(),
            checks: BTreeMap::new(),
            requires: Requirements::default(),
            network: crate::env::spec::NetworkPolicy { prepare: NetworkMode::Full, work: NetworkMode::None },
            env: BTreeMap::new(),
            secrets: BTreeMap::new(),
            base: Base::default(),
        }
    }

    fn items(gaps: &[Gap]) -> Vec<String> {
        gaps.iter().map(|g| g.item.clone()).collect()
    }

    fn check_with(r: &Resolved, p: &Policy, c: &Capabilities, harness: bool) -> Vec<Gap> {
        check(r, p, c, &|_| true, harness)
    }

    #[test]
    fn a_supported_environment_has_no_gaps() {
        let r = resolved(NetworkMode::Full, &["node_modules"]);
        assert!(check_with(&r, &policy(), &podman(), true).is_empty());
    }

    #[test]
    fn os_and_arch_mismatches_are_gaps() {
        let mut r = resolved(NetworkMode::Full, &[]);
        r.requires.os = Some("darwin".into());
        r.requires.arch = vec!["aarch64".into()];
        let gaps = check_with(&r, &policy(), &podman(), false);
        assert_eq!(items(&gaps), ["os", "arch"]);
        assert_eq!(gaps[0].to_string(), "os: the environment needs darwin, the podman runner runs on linux");
        assert!(gaps[1].reason.contains("one of [aarch64], the podman runner is x86_64"), "{}", gaps[1]);

        // Matching values pass.
        r.requires.os = Some("linux".into());
        r.requires.arch = vec!["x86_64".into(), "aarch64".into()];
        assert!(check_with(&r, &policy(), &podman(), false).is_empty());
    }

    #[test]
    fn base_images_need_a_runner_that_builds_them() {
        for base in [
            Base { image: Some("debian:12".into()), ..Default::default() },
            Base { containerfile: Some("api:Containerfile".into()), ..Default::default() },
            Base { packages: vec!["libpq-dev".into()], ..Default::default() },
        ] {
            let mut r = resolved(NetworkMode::Full, &[]);
            r.base = base;
            let gaps = check_with(&r, &lenient(), &host(), false);
            assert_eq!(items(&gaps), ["base image"]);
            assert!(gaps[0].reason.contains("cannot build or pull base images"));
            assert!(check_with(&r, &policy(), &podman(), false).is_empty());
        }
    }

    #[test]
    fn services_need_a_runner_that_runs_them() {
        let mut r = resolved(NetworkMode::Full, &[]);
        r.services.insert("db".into(), Service {
            image: "postgres:16".into(),
            env: BTreeMap::new(),
            command: vec![],
            ready: None,
            data: DataPolicy::Workspace,
            volume: None,
        });
        assert_eq!(items(&check_with(&r, &lenient(), &host(), false)), ["services"]);
        assert!(check_with(&r, &policy(), &podman(), false).is_empty());
    }

    #[test]
    fn processes_alone_are_fine_on_every_runner() {
        let mut r = resolved(NetworkMode::Full, &[]);
        r.processes.insert("api".into(), crate::env::spec::Process { repo: None, command: "run".into(), after: vec![], ready: None });
        assert!(check_with(&r, &lenient(), &host(), false).is_empty());
    }

    #[test]
    fn a_network_mode_the_runner_or_policy_lacks_is_a_gap() {
        // Bubblewrap without its network namespace cannot give `none`: a runner that lacks it is a gap.
        let r = resolved(NetworkMode::Full, &[]);
        let mut c = host();
        c.network = vec![NetworkMode::Full];
        let gaps = check_with(&r, &lenient(), &c, false);
        assert_eq!(items(&gaps), ["network mode none"], "{gaps:?}");
        assert_eq!(gaps[0].reason, "the host runner does not support it");

        // Policy forbids full network for steps.
        let mut r = resolved(NetworkMode::Full, &[]);
        r.network.work = NetworkMode::Full;
        let mut p = policy();
        p.network = vec![NetworkMode::None];
        let gaps = check_with(&r, &p, &podman(), false);
        assert_eq!(items(&gaps), ["network mode full"]);
        assert_eq!(gaps[0].reason, "server policy does not allow it");
    }

    #[test]
    fn registries_is_never_supported_in_this_release() {
        let r = resolved(NetworkMode::Registries, &[]);
        let gaps = check_with(&r, &policy(), &podman(), false);
        assert_eq!(gaps.len(), 1, "{gaps:?}");
        assert_eq!(gaps[0].to_string(), "network mode registries: not supported in this release");
    }

    #[test]
    fn a_request_above_the_policy_maximum_is_a_gap_and_a_bad_size_is_too() {
        let mut r = resolved(NetworkMode::Full, &[]);
        r.requires.memory = Some("40GiB".into());
        r.requires.cpus = Some(32.0);
        r.requires.pids = Some(20000);
        r.requires.disk = Some("500GiB".into());
        let gaps = check_with(&r, &policy(), &podman(), false);
        assert_eq!(items(&gaps), ["memory", "cpus", "pids", "disk"], "{gaps:#?}");
        assert_eq!(gaps[0].reason, "the environment asks for 40GiB, server policy allows at most 32GiB");
        assert_eq!(gaps[1].reason, "the environment asks for 32, server policy allows at most 16");

        // Exactly the maximum is allowed.
        r.requires = Requirements { memory: Some("32GiB".into()), cpus: Some(16.0), pids: Some(16384), disk: Some("200GiB".into()), ..Default::default() };
        assert!(check_with(&r, &policy(), &podman(), false).is_empty());

        r.requires.memory = Some("lots".into());
        let gaps = check_with(&r, &policy(), &podman(), false);
        assert_eq!(items(&gaps), ["memory"]);
        assert!(gaps[0].reason.contains("\"lots\" is not a size"), "{}", gaps[0]);
    }

    #[test]
    fn a_runner_that_cannot_enforce_limits_is_a_gap_unless_policy_allows_it() {
        let r = resolved(NetworkMode::Full, &[]);
        let gaps = check_with(&r, &policy(), &host(), false);
        assert_eq!(items(&gaps), ["cpu limit", "memory limit", "pids limit"]);
        assert!(gaps[1].reason.contains("cannot enforce it"), "{}", gaps[1]);

        let mut p = policy();
        p.require_limits = false;
        assert!(check_with(&r, &p, &host(), false).is_empty(), "requireLimits false accepts a runner without limits");

        // Disk is measured, not enforced: a runner without a disk limit is never a gap.
        let mut partial = podman();
        partial.limits = vec![Limit::Cpu, Limit::Memory, Limit::Pids];
        assert!(check_with(&r, &policy(), &partial, false).is_empty());
        partial.limits = vec![Limit::Cpu, Limit::Pids];
        assert_eq!(items(&check_with(&r, &policy(), &partial, false)), ["memory limit"]);
    }

    #[test]
    fn a_capability_must_be_provided_by_the_runner_and_granted_by_policy() {
        let mut r = resolved(NetworkMode::Full, &[]);
        r.requires.capabilities = vec!["gpu".into(), "kvm".into(), "fuse".into(), "tpu".into()];
        let mut p = policy();
        p.capabilities = vec!["kvm".into(), "gpu".into()];
        let mut c = podman();
        c.capabilities = vec!["gpu".into(), "fuse".into()];
        let gaps = check_with(&r, &p, &c, false);
        assert_eq!(items(&gaps), ["capability kvm", "capability fuse", "capability tpu"], "{gaps:#?}");
        assert_eq!(gaps[0].reason, "the podman runner does not provide it");
        assert_eq!(gaps[1].reason, "server policy does not grant it");
        assert_eq!(gaps[2].reason, "neither the runner nor server policy provides it");
    }

    #[test]
    fn a_secret_the_project_has_not_set_is_a_gap() {
        let mut r = resolved(NetworkMode::Full, &[]);
        r.secrets.insert("npm_token".into(), SecretGrant { steps: vec!["api/install".into()], file: "/run/secrets/npm".into() });
        let gaps = check(&r, &policy(), &podman(), &|name| name != "npm_token", false);
        assert_eq!(items(&gaps), ["secret npm_token"]);
        assert!(check(&r, &policy(), &podman(), &|_| true, false).is_empty());
    }

    #[test]
    fn secrets_need_a_runner_that_delivers_them() {
        let mut r = resolved(NetworkMode::Full, &[]);
        r.secrets.insert("token".into(), SecretGrant { steps: vec!["work".into()], file: "/run/secrets/t".into() });
        let mut c = host();
        c.secrets = false;
        assert_eq!(items(&check(&r, &lenient(), &c, &|_| true, false)), ["secrets"]);
    }

    #[test]
    fn snapshots_need_a_view_mode_only_when_a_step_has_outputs() {
        let mut c = podman();
        c.views = vec![];
        assert!(check_with(&resolved(NetworkMode::Full, &[]), &policy(), &c, false).is_empty(), "no outputs, no views needed");
        assert_eq!(items(&check_with(&resolved(NetworkMode::Full, &["node_modules"]), &policy(), &c, false)), ["views"]);
    }

    #[test]
    fn the_agent_harness_needs_a_runner_that_runs_it() {
        let r = resolved(NetworkMode::Full, &[]);
        let gaps = check_with(&r, &policy(), &host(), true);
        assert!(gaps.iter().any(|g| g.item == "agent harness"), "{gaps:?}");
        let gaps = check_with(&r, &lenient(), &host(), true);
        assert_eq!(items(&gaps), ["agent harness"]);
        assert!(check_with(&r, &policy(), &podman(), true).is_empty());
    }

    #[test]
    fn every_gap_is_reported_together() {
        let mut r = resolved(NetworkMode::Full, &["out"]);
        r.requires.memory = Some("64GiB".into());
        r.requires.capabilities = vec!["gpu".into()];
        r.secrets.insert("t".into(), SecretGrant { steps: vec![], file: "/t".into() });
        r.base.image = Some("debian".into());
        let mut p = policy();
        p.capabilities = vec![];
        let mut c = host();
        c.views = vec![];
        c.secrets = false;
        let gaps = check(&r, &p, &c, &|_| false, true);
        for needle in ["base image", "memory", "capability gpu", "secret t", "secrets", "views", "agent harness", "cpu limit"] {
            assert!(gaps.iter().any(|g| g.item == needle), "missing {needle} in {gaps:#?}");
        }
    }

    #[test]
    fn limits_take_the_request_then_the_default_and_never_exceed_the_maximum() {
        let mut r = resolved(NetworkMode::Full, &[]);
        assert_eq!(limits(&r, &policy()), limits_of(8 * GIB, 4.0, 4096, 50 * GIB), "defaults when nothing is asked");

        r.requires = Requirements { memory: Some("12GiB".into()), cpus: Some(2.0), pids: Some(100), disk: Some("80GiB".into()), ..Default::default() };
        assert_eq!(limits(&r, &policy()), limits_of(12 * GIB, 2.0, 100, 80 * GIB));

        r.requires = Requirements { memory: Some("64GiB".into()), cpus: Some(64.0), pids: Some(100000), disk: Some("900GiB".into()), ..Default::default() };
        assert_eq!(limits(&r, &policy()), limits_of(32 * GIB, 16.0, 16384, 200 * GIB), "clamped to the maximum; check reports the gap");

        r.requires.memory = Some("junk".into());
        assert_eq!(limits(&r, &policy()).memory, 8 * GIB, "an unreadable size falls back to the default");
    }

    #[test]
    fn the_first_supported_view_mode_is_used() {
        assert_eq!(view_mode(&podman()), Some(ViewMode::Overlay));
        assert_eq!(view_mode(&host()), Some(ViewMode::Copy));
        let mut none = podman();
        none.views = vec![];
        assert_eq!(view_mode(&none), None);
        assert_eq!(ViewMode::Overlay.to_string(), "overlay");
        assert_eq!(Limit::Pids.to_string(), "pids");
    }

    #[test]
    fn sizes_in_messages_read_naturally() {
        assert_eq!(size_text(8 * GIB), "8GiB");
        assert_eq!(size_text(512 << 20), "512MiB");
        assert_eq!(size_text(1000), "1000 bytes");
    }

    #[test]
    fn a_gap_serializes_for_the_web_and_the_cli() {
        let g = Gap { item: "os".into(), reason: "x".into() };
        assert_eq!(serde_json::to_string(&g).unwrap(), r#"{"item":"os","reason":"x"}"#);
    }
}
