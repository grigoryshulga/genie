//! Server configuration.
//!
//! Defaults come from the repository's `config/default.json` (embedded at build
//! time, so the binary is self-contained) and are deep-merged with
//! `<data>/config.json`. Roles, team templates, skills and MCP connections live
//! next to it and are loaded by `agent_config`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

const DEFAULT_JSON: &str = include_str!("../../../config/default.json");

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct RoleModel {
    pub model: Option<String>,
    pub thinking: Option<String>,
}

/// `modelPrices`: what a model costs, in dollars per million tokens. Cache prices
/// default to the input price.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ModelPrice {
    pub input: f64,
    pub output: f64,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
}

impl ModelPrice {
    /// What `t` cost, in dollars.
    pub fn cost(&self, t: &genie_core::usage::Tokens) -> f64 {
        (t.input as f64 * self.input
            + t.output as f64 * self.output
            + t.cache_read as f64 * self.cache_read.unwrap_or(self.input)
            + t.cache_write as f64 * self.cache_write.unwrap_or(self.input))
            / 1_000_000.0
    }
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
/// `{guard}` (the genie guard extension), `{mcpConfig}` (the role's MCP
/// connections for pi-mcp-adapter), `{limitSkills}` (the role lists its skills)
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
    /// A turn, or a session step without any sign of life, is stopped after this long.
    pub turn_timeout_secs: u64,
    /// A team that neither works nor waits for anyone for this long is reported to the
    /// orchestrator. 0 turns the silent-team watchdog off.
    pub stall_secs: u64,
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
    /// Whether pi loads pi-mcp-adapter, so agents get their MCP config (`{mcpConfig}`):
    /// unset — found in pi's settings (`pi install npm:pi-mcp-adapter`); `true`/`false` — say so.
    pub mcp_adapter: Option<bool>,
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
    /// Whether pi loads pi-mcp-adapter (which reads `--mcp-config`; pi refuses the flag
    /// without it): `mcpAdapter`, else a command loading it, else pi's settings — its
    /// packages and extensions — or its extensions directory.
    pub fn mcp_adapter(&self) -> bool {
        const NAME: &str = "pi-mcp-adapter";
        if let Some(v) = self.mcp_adapter {
            return v;
        }
        if self.command.iter().chain(&self.session_command).flatten().any(|a| a.contains(NAME)) {
            return true;
        }
        let home = std::env::var("HOME").unwrap_or_default();
        let dir = match self.env.get("PI_CODING_AGENT_DIR").cloned().or_else(|| std::env::var("PI_CODING_AGENT_DIR").ok()) {
            Some(d) => match d.strip_prefix("~/") {
                Some(rest) => PathBuf::from(&home).join(rest),
                None => PathBuf::from(d),
            },
            None => PathBuf::from(&home).join(".pi").join("agent"),
        };
        let settings: serde_json::Value =
            std::fs::read_to_string(dir.join("settings.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        let listed = ["packages", "extensions"]
            .iter()
            .filter_map(|k| settings[*k].as_array())
            .flatten()
            .any(|e| e.as_str().or_else(|| e["source"].as_str()).is_some_and(|s| s.contains(NAME)));
        listed
            || std::fs::read_dir(dir.join("extensions"))
                .is_ok_and(|rd| rd.flatten().any(|e| e.file_name().to_string_lossy().contains(NAME)))
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
                g(&["--mcp-config", "{mcpConfig}"]),
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
                g(&["--mcp-config", "{mcpConfig}"]),
            ],
            max_concurrent: 4,
            max_sessions: 12,
            idle_stop_secs: 900,
            turn_timeout_secs: 1800,
            stall_secs: 900,
            ci_pending_secs: 1800,
            ci_reruns_per_request: 2,
            ci_reruns_per_task: 3,
            max_attempts: 3,
            ask_timeout_secs: 180,
            delivery_budget: genie_core::team::DELIVERY_BUDGET,
            env: BTreeMap::new(),
            mcp_adapter: None,
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

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub port: u16,
    pub bind: String,
    /// Base URL used in links sent by mail and Telegram.
    pub public_url: Option<String>,
    pub allow_hosts: Vec<String>,
    pub role_models: BTreeMap<String, RoleModel>,
    /// Prices of models (`provider/model` or the model alone), for what agents' work cost.
    pub model_prices: BTreeMap<String, ModelPrice>,
    pub budgets: Budgets,
    pub limits: Limits,
    pub worktrees: Worktrees,
    pub language: Language,
    pub runtime: RuntimeConfig,
    pub telegram: Option<TelegramConfig>,
    pub smtp: Option<SmtpConfig>,
    pub vault: VaultConfig,
    pub skills: SkillsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            port: 7420,
            bind: "127.0.0.1".into(),
            public_url: None,
            allow_hosts: Vec::new(),
            role_models: BTreeMap::new(),
            model_prices: BTreeMap::new(),
            budgets: Budgets::default(),
            limits: Limits::default(),
            worktrees: Worktrees::default(),
            language: Language::default(),
            runtime: RuntimeConfig::default(),
            telegram: None,
            smtp: None,
            vault: VaultConfig::default(),
            skills: SkillsConfig::default(),
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
        serde_json::from_value(value).map_err(|e| format!("config: {e}"))
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
        assert!(cfg.role_models.contains_key("executor"));
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
    fn the_sandbox_is_its_mode_or_an_object() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(Config::load(dir.path()).unwrap().runtime.sandbox, SandboxConfig::default());
        std::fs::write(dir.path().join("config.json"), r#"{"runtime": {"sandbox": "off"}}"#).unwrap();
        assert_eq!(Config::load(dir.path()).unwrap().runtime.sandbox.mode, "off");
        std::fs::write(dir.path().join("config.json"), r#"{"runtime": {"sandbox": {"writable": ["~/.m2"]}}}"#).unwrap();
        let s = Config::load(dir.path()).unwrap().runtime.sandbox;
        assert_eq!((s.mode.as_str(), s.writable), ("auto", vec!["~/.m2".to_string()]));
    }

    #[test]
    fn pi_mcp_adapter_is_found_in_pis_settings() {
        let dir = tempfile::tempdir().unwrap();
        let mut rt = RuntimeConfig::default();
        rt.env.insert("PI_CODING_AGENT_DIR".into(), dir.path().to_string_lossy().into_owned());
        assert!(!rt.mcp_adapter(), "no settings");
        let settings = |v: &str| std::fs::write(dir.path().join("settings.json"), v).unwrap();
        settings(r#"{"packages": ["npm:pi-web-access"]}"#);
        assert!(!rt.mcp_adapter());
        settings(r#"{"packages": ["npm:pi-web-access", "npm:pi-mcp-adapter@3.1.0"]}"#);
        assert!(rt.mcp_adapter(), "pi install npm:pi-mcp-adapter");
        settings(r#"{"packages": [{"source": "git:github.com/nicobailon/pi-mcp-adapter", "extensions": ["index.ts"]}]}"#);
        assert!(rt.mcp_adapter(), "a filtered package entry");
        settings("{}");
        std::fs::create_dir_all(dir.path().join("extensions/pi-mcp-adapter")).unwrap();
        assert!(rt.mcp_adapter(), "the extensions directory");
        rt.mcp_adapter = Some(false);
        assert!(!rt.mcp_adapter(), "the setting wins");
    }
}
