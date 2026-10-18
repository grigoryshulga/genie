//! Server configuration.
//!
//! Defaults come from the repository's `config/default.json` (embedded at build
//! time, so the binary is self-contained) and are deep-merged with
//! `<data>/config.json`. Roles, team templates, skills and MCP connections live
//! next to it and are loaded by `agent_config`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use genie_core::env::spec::{NetworkMode, parse_size};
use serde::Deserialize;
use serde_json::Value;

const DEFAULT_JSON: &str = include_str!("../../../config/default.json");

/// `modelPrices`: what a model costs, in dollars per million tokens. A missing
/// cache price leaves that cache's tokens unpriced (never billed as input).
pub use genie_core::usage::ModelPrice;

/// `litellm`: where the server reads the models' tariffs from. Prices load at
/// start and by an administrator's «pull the config» action — never on a timer.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Litellm {
    /// The proxy's base URL, the same one pi's `litellm` provider uses
    /// (…/v1); `None` switches fetching off.
    pub base_url: Option<String>,
    /// The service token that may read `/model/info` and nothing else, when
    /// config.json should carry it; the server's `LITELLM_INFO_TOKEN`
    /// environment variable (how docker hands secrets over) is the other way.
    pub info_token: Option<String>,
}

/// The price of `model` (`provider/model`): its own entry, else an entry naming
/// the model without the provider, or the same model through another provider.
pub fn price_of<'a>(prices: &'a BTreeMap<String, ModelPrice>, model: &str) -> Option<&'a ModelPrice> {
    let tail = |m: &str| m.rsplit_once('/').map_or(m.to_string(), |(_, t)| t.to_string());
    prices.get(model).or_else(|| prices.get(&tail(model))).or_else(|| prices.iter().find(|(k, _)| tail(k) == tail(model)).map(|(_, p)| p))
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct MemberSpec {
    pub role: String,
    pub name: Option<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub instructions: Option<String>,
}

/// Where skills are found besides `<data>/skills` (e.g. a clone of a skills repository).
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SkillsConfig {
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Limits {
    pub max_members_per_team: usize,
    pub max_active_teams: usize,
    /// Active teams on the tasks of one epic at a time; 0 is no limit. The tasks
    /// above the limit wait in `ready` until a team of the epic stops.
    pub max_active_teams_per_epic: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_members_per_team: 6, max_active_teams: 4, max_active_teams_per_epic: 0 }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Worktrees {
    pub dir: String,
    pub branch: String,
}

impl Default for Worktrees {
    fn default() -> Self {
        Worktrees { dir: "{mainRoot}/../{repo}.worktrees/{team}".into(), branch: "genie/{team}".into() }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Language {
    pub internal: String,
    pub user: String,
}

impl Default for Language {
    fn default() -> Self {
        Language { internal: "English".into(), user: "Russian".into() }
    }
}

/// How agents run.
///
/// Team members and the orchestrator run as *live sessions* (`mode: "sessions"`):
/// one long-running `sessionCommand` process per agent (pi in RPC mode with the
/// genie-bus extension), mail delivered between its steps. Any other harness runs
/// in *turns* (`mode: "turns"`): `command` is launched for each batch of mail and
/// must finish. `"auto"` (default) uses sessions while `command` is pi's default.
/// One-shot jobs always run as turns.
///
/// Commands are lists of argument groups; a group is used only when every
/// placeholder in it resolved to a non-empty value, so optional flags
/// (`--model {model}`) disappear when unset. Placeholders: `{sessionDir}`,
/// `{sessionId}`, `{model}`, `{thinking}`, `{promptFile}`, `{message}` (turns),
/// `{extension}` (sessions), `{readonlyTools}` (read-only roles), `{cwd}`,
/// `{guard}` (the genie guard extension, which connects the role's MCP
/// connections: `GENIE_MCP_CONFIG`), `{mcpConfig}` (the file with them, in pi's
/// `mcp.json` format, for a harness that wants it), `{limitSkills}` (the role lists its skills)
/// and `{skill}` — a list: its group is repeated for each skill directory.
/// `{?name}` adds nothing but keeps its group only when `name` is set
/// (`["--no-skills", "{?limitSkills}"]`).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RuntimeConfig {
    pub mode: String,
    pub command: Vec<Vec<String>>,
    pub session_command: Vec<Vec<String>>,
    /// Concurrent one-shot jobs and turns.
    pub max_concurrent: usize,
    /// Live sessions at once; an idle one is stopped to make room.
    pub max_sessions: usize,
    /// A session idle this long is stopped (its conversation is kept and resumed).
    pub idle_stop_secs: u64,
    /// The V8 heap limit of a pi process, MB (`NODE_OPTIONS=--max-old-space-size`; 0: Node's own
    /// default, up to 4 GB). Keeps a runaway session from eating the machine; the cap is dropped
    /// again for the commands pi runs. Left alone when `NODE_OPTIONS` already sets one.
    pub node_heap_mb: u64,
    /// A turn, or a session step without any sign of life, is stopped after this long.
    pub turn_timeout_secs: u64,
    /// A team that neither works nor waits for anyone for this long is reported to the
    /// orchestrator. 0 turns the silent-team watchdog off.
    pub stall_secs: u64,
    /// When free space on the data or repository filesystem falls below this many
    /// gigabytes, the server says so at start and `genie doctor` warns.
    pub low_disk_gb: f64,
    /// Checks that stay `pending` for this long are reported to the team once and the watch
    /// stops taking them as running (`stalled`). 0 turns the timeout off.
    pub ci_pending_secs: u64,
    /// How many times failed checks may be rerun for one delivery (request or branch).
    /// 0 switches reruns off (per request and per task together).
    pub ci_reruns_per_request: u32,
    /// How many reruns a whole task may spend over all its repositories; 0 switches reruns off.
    pub ci_reruns_per_task: u32,
    pub max_attempts: u32,
    /// How long `genie mail ask` waits for the answer by default.
    pub ask_timeout_secs: u64,
    /// Characters of mail put into a session at one step boundary.
    pub delivery_budget: usize,
    /// Extra environment for agent processes.
    pub env: BTreeMap<String, String>,

    /// Agents reach MCP connections through the genie gateway (default): secrets
    /// stay on the server and every call is in the project's journal. `false`:
    /// the harness gets the connections themselves.
    pub mcp_gateway: bool,
    /// Disable to run the server without starting any agent (UI-only mode).
    pub enabled: bool,
    /// How agent processes are isolated from the server's data and the machine.
    pub sandbox: SandboxConfig,
}

/// `runtime.sandbox`: an object, or just its mode (`"sandbox": "off"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxConfig {
    /// `auto` — bubblewrap when it works on this machine, otherwise none (with a
    /// warning); `bwrap` — required: agents do not start without it; `off`.
    pub mode: String,
    /// More paths agents may write, besides their working directory, its git
    /// repository, their session files, pi's directory and the tool caches.
    pub writable: Vec<String>,
    /// More paths agents must not see. `~/` is the server user's home.
    pub hidden: Vec<String>,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        SandboxConfig { mode: "auto".into(), writable: Vec::new(), hidden: Vec::new() }
    }
}

impl<'de> Deserialize<'de> for SandboxConfig {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", default)]
        struct Full {
            mode: String,
            writable: Vec<String>,
            hidden: Vec<String>,
        }
        impl Default for Full {
            fn default() -> Self {
                Full { mode: "auto".into(), writable: Vec::new(), hidden: Vec::new() }
            }
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Setting {
            Mode(String),
            Full(Full),
        }
        Ok(match Setting::deserialize(d)? {
            Setting::Mode(mode) => SandboxConfig { mode, ..Default::default() },
            Setting::Full(f) => SandboxConfig { mode: f.mode, writable: f.writable, hidden: f.hidden },
        })
    }
}

impl RuntimeConfig {
    /// The pi agent directory set on purpose (runtime.env or the environment), `~/` expanded:
    /// `None` when the operator left it to genie, which then runs agents on its own profile.
    pub fn pi_dir_override(&self) -> Option<PathBuf> {
        let raw = self.env.get("PI_CODING_AGENT_DIR").cloned().or_else(|| std::env::var("PI_CODING_AGENT_DIR").ok())?;
        let home = std::env::var("HOME").unwrap_or_default();
        Some(match raw.strip_prefix("~/") {
            Some(rest) => PathBuf::from(&home).join(rest),
            None => PathBuf::from(raw),
        })
    }

    /// Whether members and the orchestrator run as live sessions.
    pub fn live_sessions(&self) -> bool {
        match self.mode.as_str() {
            "sessions" => true,
            "turns" => false,
            _ => self.command == RuntimeConfig::default().command,
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        let g = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        RuntimeConfig {
            mode: "auto".into(),
            command: vec![
                g(&["pi", "--print"]),
                g(&["--session-dir", "{sessionDir}"]),
                g(&["--session-id", "{sessionId}"]),
                g(&["--model", "{model}"]),
                g(&["--thinking", "{thinking}"]),
                g(&["--append-system-prompt", "{promptFile}"]),
                g(&["--exclude-tools", "{readonlyTools}"]),
                g(&["--no-skills", "{?limitSkills}"]),
                g(&["--skill", "{skill}"]),
                g(&["-e", "{guard}"]),
                g(&["--no-approve"]),
                g(&["{message}"]),
            ],
            session_command: vec![
                g(&["pi", "--mode", "rpc"]),
                g(&["--session-dir", "{sessionDir}"]),
                g(&["--session-id", "{sessionId}"]),
                g(&["--model", "{model}"]),
                g(&["--thinking", "{thinking}"]),
                g(&["--append-system-prompt", "{promptFile}"]),
                g(&["--exclude-tools", "{readonlyTools}"]),
                g(&["--no-skills", "{?limitSkills}"]),
                g(&["--skill", "{skill}"]),
                g(&["-e", "{extension}"]),
                g(&["-e", "{guard}"]),
                g(&["--no-approve"]),
            ],
            max_concurrent: 4,
            max_sessions: 12,
            idle_stop_secs: 300,
            node_heap_mb: 2048,
            turn_timeout_secs: 1800,
            stall_secs: 900,
            low_disk_gb: 5.0,
            ci_pending_secs: 1800,
            ci_reruns_per_request: 2,
            ci_reruns_per_task: 3,
            max_attempts: 3,
            ask_timeout_secs: 180,
            delivery_budget: genie_core::team::DELIVERY_BUDGET,
            env: BTreeMap::new(),
            mcp_gateway: true,
            enabled: true,
            sandbox: SandboxConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TelegramConfig {
    pub token: String,
    /// Bot API base, for tests and proxies.
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub from: String,
    /// "starttls" (default), "tls" or "none" (local test servers such as Mailpit).
    pub security: String,
}

impl Default for SmtpConfig {
    fn default() -> Self {
        SmtpConfig { host: String::new(), port: 587, username: None, password: None, from: String::new(), security: "starttls".into() }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct VaultConfig {
    /// Vault directory; defaults to `<data>/vault`.
    pub path: Option<PathBuf>,
    /// Commit writes when the vault is a git repository (default true).
    pub commit: Option<bool>,
    /// A git remote to keep the vault in sync with — a remote's name or a URL — so
    /// people can work on it in Obsidian: fetched, merged and pushed every `syncSecs`.
    pub remote: Option<String>,
    /// The branch to sync (default: the vault's current branch).
    pub branch: Option<String>,
    /// Seconds between syncs (default 120).
    pub sync_secs: Option<u64>,
}

/// `budgets`: dollars an agent's work may cost (models with a price in `modelPrices`);
/// 0 is no limit. Changes take effect when the server restarts.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Budgets {
    /// One task with its subtasks.
    pub per_task: f64,
    /// One epic with all its tasks.
    pub per_epic: f64,
    /// Everything agents spend in a UTC day, all projects' trackers apart (per project).
    pub per_day: f64,
}

/// `environments`: the workspace environments of tasks (`genie::env`). `policy.defaults` is what a
/// profile gets when it asks for nothing, `policy.max` the most it may ask for. Sizes are read by
/// `parse_size` (`8GiB`, `512MiB`, or plain bytes).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvironmentsConfig {
    /// The environment store. `null` is `<data>/env-store`; it must not be inside a Git working tree.
    pub root: Option<PathBuf>,
    /// `auto` (podman when rootless podman works, else host), `podman` or `host`.
    pub runner: String,
    pub podman: EnvironmentsPodman,
    pub policy: EnvironmentsPolicy,
    pub storage: EnvironmentsStorage,
    /// How long to wait for another process to finish preparing a snapshot this one needs.
    pub prepare_timeout_secs: u64,
    /// How long a service or a process may take to report ready.
    pub ready_timeout_secs: u64,
}

/// `environments.podman`: the rootless Podman program and the network containers join by default.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvironmentsPodman {
    pub binary: String,
    /// The network containers with full network join: `auto` (pasta on podman 5 and later, slirp4netns
    /// before), or an explicit `--network` value used as it is.
    pub network: String,
    /// The address containers use to reach the server's host (`GENIE_HOST_ADDRESS`).
    pub host_address: String,
}

/// `environments.policy`: which network modes and resources the server allows environments.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvironmentsPolicy {
    /// The network modes a profile may use (`registries` is never supported in this release).
    pub network: Vec<NetworkMode>,
    /// A runner that cannot enforce cpu, memory and pids limits fails the check unless this is off.
    pub require_limits: bool,
    pub defaults: EnvironmentsLimits,
    pub max: EnvironmentsLimits,
    /// Capabilities a profile may require beyond the runner's own.
    pub capabilities: Vec<String>,
}

/// Resource limits. `Config::load` deep-merges `<data>/config.json` over the repository's defaults, so
/// an override may name only the fields it changes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentsLimits {
    pub memory: String,
    pub cpus: f64,
    pub pids: u64,
    pub disk: String,
}

/// `environments.storage`: how much disk the snapshots and the download caches may take.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvironmentsStorage {
    pub snapshots_max: String,
    pub caches_max: String,
}

impl Default for EnvironmentsConfig {
    fn default() -> Self {
        EnvironmentsConfig {
            root: None,
            runner: "auto".into(),
            podman: EnvironmentsPodman::default(),
            policy: EnvironmentsPolicy::default(),
            storage: EnvironmentsStorage::default(),
            prepare_timeout_secs: 1800,
            ready_timeout_secs: 300,
        }
    }
}

impl Default for EnvironmentsPodman {
    fn default() -> Self {
        EnvironmentsPodman { binary: "podman".into(), network: "auto".into(), host_address: "10.0.2.2".into() }
    }
}

impl Default for EnvironmentsPolicy {
    fn default() -> Self {
        EnvironmentsPolicy {
            network: vec![NetworkMode::None, NetworkMode::Full],
            require_limits: true,
            defaults: EnvironmentsLimits { memory: "8GiB".into(), cpus: 4.0, pids: 4096, disk: "50GiB".into() },
            max: EnvironmentsLimits { memory: "32GiB".into(), cpus: 16.0, pids: 16384, disk: "200GiB".into() },
            capabilities: Vec::new(),
        }
    }
}

impl Default for EnvironmentsStorage {
    fn default() -> Self {
        EnvironmentsStorage { snapshots_max: "100GiB".into(), caches_max: "50GiB".into() }
    }
}

impl EnvironmentsConfig {
    /// Refuse settings no runner could honour, naming the key: an unknown runner, a size that does
    /// not parse, a default above its maximum, or a zero timeout.
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.runner.as_str(), "auto" | "podman" | "host") {
            return Err(format!("environments.runner: {} is not auto, podman or host", self.runner));
        }
        if self.podman.binary.trim().is_empty() {
            return Err("environments.podman.binary: the Podman program must not be empty".into());
        }
        if self.podman.network.trim().is_empty() {
            return Err("environments.podman.network: use auto or a Podman --network value, not an empty string".into());
        }
        let size = |key: &str, text: &str| parse_size(text).map_err(|e| format!("environments.{key}: {e}"));
        let (d, m) = (&self.policy.defaults, &self.policy.max);
        let (memory, memory_max) = (size("policy.defaults.memory", &d.memory)?, size("policy.max.memory", &m.memory)?);
        let (disk, disk_max) = (size("policy.defaults.disk", &d.disk)?, size("policy.max.disk", &m.disk)?);
        size("storage.snapshotsMax", &self.storage.snapshots_max)?;
        size("storage.cachesMax", &self.storage.caches_max)?;
        if memory > memory_max {
            return Err("environments.policy.defaults.memory is above environments.policy.max.memory".into());
        }
        if disk > disk_max {
            return Err("environments.policy.defaults.disk is above environments.policy.max.disk".into());
        }
        if !(d.cpus > 0.0 && d.cpus <= m.cpus) {
            return Err("environments.policy: defaults.cpus must be above 0 and at most max.cpus".into());
        }
        if d.pids == 0 || d.pids > m.pids {
            return Err("environments.policy: defaults.pids must be above 0 and at most max.pids".into());
        }
        if self.prepare_timeout_secs == 0 || self.ready_timeout_secs == 0 {
            return Err("environments: prepareTimeoutSecs and readyTimeoutSecs must be at least 1".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub decisions: crate::decisions::Config,
    pub port: u16,
    pub bind: String,
    /// Base URL used in links sent by mail and Telegram.
    pub public_url: Option<String>,
    pub allow_hosts: Vec<String>,
    /// Prices of models (`provider/model` or the model alone), for what agents' work cost.
    pub model_prices: BTreeMap<String, ModelPrice>,
    pub litellm: Litellm,
    pub budgets: Budgets,
    pub limits: Limits,
    pub worktrees: Worktrees,
    pub language: Language,
    pub runtime: RuntimeConfig,
    pub telegram: Option<TelegramConfig>,
    pub smtp: Option<SmtpConfig>,
    pub vault: VaultConfig,
    pub skills: SkillsConfig,
    pub environments: EnvironmentsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            decisions: crate::decisions::Config::default(),
            port: 7420,
            bind: "127.0.0.1".into(),
            public_url: None,
            allow_hosts: Vec::new(),
            model_prices: BTreeMap::new(),
            litellm: Litellm::default(),
            budgets: Budgets::default(),
            limits: Limits::default(),
            worktrees: Worktrees::default(),
            language: Language::default(),
            runtime: RuntimeConfig::default(),
            telegram: None,
            smtp: None,
            vault: VaultConfig::default(),
            skills: SkillsConfig::default(),
            environments: EnvironmentsConfig::default(),
        }
    }
}

fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                merge(b.entry(k).or_insert(Value::Null), v);
            }
        }
        (b, o) => *b = o,
    }
}

impl Config {
    /// Embedded defaults merged with `<data>/config.json` when present.
    pub fn load(data: &Path) -> Result<Config, String> {
        let mut value: Value = serde_json::from_str(DEFAULT_JSON).map_err(|e| format!("config/default.json: {e}"))?;
        let file = data.join("config.json");
        if file.exists() {
            let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            let over: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", file.display()))?;
            merge(&mut value, over);
        }
        let cfg: Config = serde_json::from_value(value).map_err(|e| format!("config: {e}"))?;
        cfg.environments.validate().map_err(|e| format!("config: {e}"))?;
        Ok(cfg)
    }

    pub fn public_url(&self) -> String {
        self.public_url.clone().unwrap_or_else(|| format!("http://127.0.0.1:{}", self.port)).trim_end_matches('/').to_string()
    }

    pub fn vault_path(&self, data: &Path) -> PathBuf {
        self.vault.path.clone().unwrap_or_else(|| data.join("vault"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_come_from_the_repository_config() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::load(dir.path()).unwrap();
        assert_eq!(cfg.limits.max_active_teams, 4);
        // No default role models (PD24): every role carries its own model, and the
        // configuration's problems name the roles that still lack one.
    }

    #[test]
    fn data_config_overrides_deeply() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.json"),
            r#"{"port": 9000, "runtime": {"maxConcurrent": 1}, "limits": {"maxActiveTeams": 2}}"#,
        )
        .unwrap();
        let cfg = Config::load(dir.path()).unwrap();
        assert_eq!(cfg.port, 9000);
        assert_eq!(cfg.runtime.max_concurrent, 1);
        assert_eq!(cfg.runtime.turn_timeout_secs, 1800, "unset runtime fields keep defaults");
        assert_eq!(cfg.runtime.stall_secs, 900, "unset runtime fields keep defaults");
        assert_eq!(cfg.runtime.ci_pending_secs, 1800, "unset runtime fields keep defaults");
        assert_eq!((cfg.runtime.ci_reruns_per_request, cfg.runtime.ci_reruns_per_task), (2, 3));
        assert_eq!(cfg.limits.max_members_per_team, 6);
        assert_eq!((cfg.runtime.idle_stop_secs, cfg.runtime.node_heap_mb), (300, 2048), "unset runtime fields keep defaults");
    }

    #[test]
    fn the_heap_cap_is_set_in_megabytes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), r#"{"runtime": {"nodeHeapMb": 0, "idleStopSecs": 60}}"#).unwrap();
        let cfg = Config::load(dir.path()).unwrap();
        assert_eq!((cfg.runtime.node_heap_mb, cfg.runtime.idle_stop_secs), (0, 60));
    }

    #[test]
    fn the_silent_team_watchdog_can_be_configured_and_switched_off() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), r#"{"runtime": {"stallSecs": 30}}"#).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap().runtime.stall_secs, 30);
        std::fs::write(dir.path().join("config.json"), r#"{"runtime": {"stallSecs": 0}}"#).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap().runtime.stall_secs, 0, "0 switches it off");
    }

    #[test]
    fn the_pending_checks_timeout_can_be_configured_and_switched_off() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), r#"{"runtime": {"ciPendingSecs": 60}}"#).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap().runtime.ci_pending_secs, 60);
        std::fs::write(dir.path().join("config.json"), r#"{"runtime": {"ciPendingSecs": 0}}"#).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap().runtime.ci_pending_secs, 0, "0 switches it off");
    }

    #[test]
    fn the_rerun_limits_can_be_configured_and_switched_off() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), r#"{"runtime": {"ciRerunsPerRequest": 5, "ciRerunsPerTask": 7}}"#).unwrap();
        let rt = Config::load(dir.path()).unwrap().runtime;
        assert_eq!((rt.ci_reruns_per_request, rt.ci_reruns_per_task), (5, 7));
        std::fs::write(dir.path().join("config.json"), r#"{"runtime": {"ciRerunsPerRequest": 0}}"#).unwrap();
        let rt = Config::load(dir.path()).unwrap().runtime;
        assert_eq!((rt.ci_reruns_per_request, rt.ci_reruns_per_task), (0, 3), "0 switches reruns off");
    }

    #[test]
    fn the_default_environments_section_is_the_one_the_code_defaults_to() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = Config::load(dir.path()).unwrap();
        let json: Value = serde_json::from_str(DEFAULT_JSON).unwrap();
        let from_json: EnvironmentsConfig = serde_json::from_value(json["environments"].clone()).unwrap();
        assert_eq!(from_json, EnvironmentsConfig::default(), "config/default.json and the defaults in code agree");
        assert_eq!(cfg.environments, EnvironmentsConfig::default());
        assert_eq!((cfg.environments.runner.as_str(), cfg.environments.root.is_none()), ("auto", true));
        assert_eq!(cfg.environments.policy.network, [NetworkMode::None, NetworkMode::Full]);
        assert!(cfg.environments.policy.require_limits, "a runner without limits is refused unless configured otherwise");
        assert_eq!((cfg.environments.policy.max.cpus, cfg.environments.policy.max.pids), (16.0, 16384));
        assert_eq!(cfg.environments.podman.host_address, "10.0.2.2");
        assert_eq!((cfg.environments.prepare_timeout_secs, cfg.environments.ready_timeout_secs), (1800, 300));
        cfg.environments.validate().unwrap();
    }

    #[test]
    fn a_partial_environments_section_fills_the_rest_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("config.json"),
            r#"{"environments": {"runner": "host", "policy": {"requireLimits": false, "network": ["none"]}, "podman": {"binary": "/opt/podman"}}}"#,
        )
        .unwrap();
        let env = Config::load(dir.path()).unwrap().environments;
        assert_eq!(env.runner, "host");
        assert!(!env.policy.require_limits);
        assert_eq!(env.policy.network, [NetworkMode::None]);
        assert_eq!(env.podman.binary, "/opt/podman");
        assert_eq!(env.podman.host_address, "10.0.2.2", "unset fields keep defaults");
        assert_eq!(env.policy.max, EnvironmentsConfig::default().policy.max, "an unset limits object keeps its default");
        assert_eq!(env.storage.snapshots_max, "100GiB");
        assert_eq!(env.ready_timeout_secs, 300);
    }

    #[test]
    fn a_partial_limit_override_changes_only_its_fields_and_a_bad_setting_is_refused_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("config.json");
        std::fs::write(&file, r#"{"environments": {"policy": {"max": {"memory": "64GiB"}}}}"#).unwrap();
        let max = Config::load(dir.path()).unwrap().environments.policy.max;
        assert_eq!((max.memory.as_str(), max.cpus, max.pids), ("64GiB", 16.0, 16384), "the other limits keep their defaults");

        for (text, key) in [
            (r#"{"environments": {"runner": "docker"}}"#, "environments.runner"),
            (r#"{"environments": {"storage": {"snapshotsMax": "lots"}}}"#, "environments.storage.snapshotsMax"),
            (
                r#"{"environments": {"policy": {"defaults": {"memory": "64GiB", "cpus": 4, "pids": 4096, "disk": "50GiB"}}}}"#,
                "above environments.policy.max.memory",
            ),
            (r#"{"environments": {"prepareTimeoutSecs": 0}}"#, "prepareTimeoutSecs"),
            (r#"{"environments": {"podman": {"network": "  "}}}"#, "environments.podman.network"),
        ] {
            std::fs::write(&file, text).unwrap();
            let err = Config::load(dir.path()).unwrap_err();
            assert!(err.contains(key), "{text}: {err}");
        }
    }

    #[test]
    fn the_podman_network_is_auto_by_default_or_an_explicit_value() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Config::load(dir.path()).unwrap().environments.podman.network, "auto");
        std::fs::write(
            dir.path().join("config.json"),
            r#"{"environments": {"podman": {"network": "pasta:--map-host-loopback,10.0.2.2"}}}"#,
        )
        .unwrap();
        assert_eq!(Config::load(dir.path()).unwrap().environments.podman.network, "pasta:--map-host-loopback,10.0.2.2");
    }

    #[test]
    fn the_sandbox_is_its_mode_or_an_object() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Config::load(dir.path()).unwrap().runtime.sandbox, SandboxConfig::default());
        std::fs::write(dir.path().join("config.json"), r#"{"runtime": {"sandbox": "off"}}"#).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap().runtime.sandbox.mode, "off");
        std::fs::write(dir.path().join("config.json"), r#"{"runtime": {"sandbox": {"writable": ["~/.m2"]}}}"#).unwrap();
        let s = Config::load(dir.path()).unwrap().runtime.sandbox;
        assert_eq!((s.mode.as_str(), s.writable), ("auto", vec!["~/.m2".to_string()]));
    }
}
