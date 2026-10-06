//! Preflight of a genie server: what has to work before people come — the data
//! directory, the web UI, people and projects, pi and the models of the roles,
//! the agent sandbox, git, the channels and the network settings. `genie doctor`
//! prints it (and fails when something is broken), `GET /api/doctor` shows it to
//! the server's admins.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use genie_core::db::SCHEMA_VERSION;
use genie_core::server_db::{SERVER_SCHEMA_VERSION, ServerDb};
use serde::Serialize;

use crate::agent_config::AgentConfig;
use crate::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    /// What the check is about: data, web, people, projects, pi, models, sandbox, git, channels, network.
    pub area: &'static str,
    pub level: Level,
    pub text: String,
    /// What to do about it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

struct Out(Vec<Check>);

impl Out {
    fn push(&mut self, area: &'static str, level: Level, text: impl Into<String>, hint: Option<&str>) {
        self.0.push(Check { area, level, text: text.into(), hint: hint.map(str::to_string) });
    }
    fn ok(&mut self, area: &'static str, text: impl Into<String>) {
        self.push(area, Level::Ok, text, None);
    }
    fn warn(&mut self, area: &'static str, text: impl Into<String>, hint: &str) {
        self.push(area, Level::Warn, text, Some(hint));
    }
    fn fail(&mut self, area: &'static str, text: impl Into<String>, hint: &str) {
        self.push(area, Level::Fail, text, Some(hint));
    }
}

/// Run every check. `web`: the web UI directory the server is started with (`serve --web`).
pub fn run(data: &Path, cfg: &Config, agents: &AgentConfig, web: Option<&Path>) -> Vec<Check> {
    let mut out = Out(Vec::new());
    storage(&mut out, data);
    schemas(&mut out, data);
    match (crate::http::web::resolve(web), web) {
        (crate::http::web::WebUi::BuiltIn, Some(dir)) => out.warn(
            "web",
            format!("no web UI in {}: the one built into genie is served", dir.display()),
            "drop --web from the command line (the web UI is inside genie now)",
        ),
        (crate::http::web::WebUi::BuiltIn, None) => out.ok("web", "web UI built into genie"),
        (crate::http::web::WebUi::Dir(dir), _) => out.ok("web", format!("web UI in {}", dir.display())),
        (crate::http::web::WebUi::Missing(why), _) => {
            let (what, hint) = why.split_once(": ").unwrap_or((why.as_str(), ""));
            out.fail("web", what.to_string(), hint);
        }
    }
    let repos = people_and_projects(&mut out, data);
    agents_and_models(&mut out, cfg, agents);
    match crate::sandbox::status(&cfg.runtime.sandbox) {
        (true, note) => out.ok("sandbox", note),
        (false, note) if cfg.runtime.sandbox.mode == "off" => out.warn("sandbox", note, "set runtime.sandbox to \"auto\" once bubblewrap works here"),
        (false, note) => out.fail(
            "sandbox",
            note,
            "apt install bubblewrap; on Ubuntu 24.04+: sysctl kernel.apparmor_restrict_unprivileged_userns=0 (or an AppArmor profile for bwrap)",
        ),
    }
    git(&mut out, &repos);
    git_hosts(&mut out, data);
    vault(&mut out, data, cfg);
    channels(&mut out, cfg);
    network(&mut out, cfg);
    out.0
}

/// The checks as lines for the terminal, and how many failed.
pub fn print(checks: &[Check]) -> (String, usize) {
    let mut lines = Vec::new();
    for c in checks {
        let mark = match c.level {
            Level::Ok => "ok  ",
            Level::Warn => "warn",
            Level::Fail => "FAIL",
        };
        lines.push(format!("{mark} {:<9} {}", c.area, c.text));
        if let Some(h) = &c.hint {
            lines.push(format!("               → {h}"));
        }
    }
    let failed = checks.iter().filter(|c| c.level == Level::Fail).count();
    let warned = checks.iter().filter(|c| c.level == Level::Warn).count();
    lines.push(String::new());
    lines.push(match (failed, warned) {
        (0, 0) => "everything is ready".to_string(),
        (0, w) => format!("ready, {w} warning(s)"),
        (f, _) => format!("{f} problem(s) to fix before people and agents can work"),
    });
    (lines.join("\n"), failed)
}

fn storage(out: &mut Out, data: &Path) {
    let probe = data.join(format!(".doctor-{}", std::process::id()));
    match std::fs::create_dir_all(data).and_then(|_| std::fs::write(&probe, b"x")) {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            out.ok("data", format!("data directory {}", data.display()));
        }
        Err(e) => {
            out.fail(
                "data",
                format!("cannot write the data directory {}: {e}", data.display()),
                "run the server as the user who owns it (--data or $GENIE_DATA)",
            );
            return;
        }
    }
    if let Some(free) = free_bytes(data) {
        let gb = free as f64 / 1e9;
        if free < 200_000_000 {
            out.fail("data", format!("{gb:.1} GB free on the data disk"), "free some space: databases, worktrees and agent logs grow");
        } else if free < 2_000_000_000 {
            out.warn("data", format!("{gb:.1} GB free on the data disk"), "worktrees and agent logs grow; keep a few GB free");
        } else {
            out.ok("data", format!("{gb:.0} GB free on the data disk"));
        }
    }
}

/// The schema versions the data directory holds: what the last update migrated, and whether
/// the data is newer than this binary (a binary rolled back without a restore).
fn schemas(out: &mut Out, data: &Path) {
    let server = data.join("server.db");
    match genie_core::migrate::version(&server) {
        Ok(None) => {}
        Ok(Some(v)) if v > SERVER_SCHEMA_VERSION => out.fail(
            "data",
            format!("server.db records schema {v}; this genie understands {SERVER_SCHEMA_VERSION}"),
            "restore a backup made before the update, or run the newer genie",
        ),
        Ok(Some(v)) if v < SERVER_SCHEMA_VERSION => out.warn(
            "data",
            format!("server.db schema {v} (this genie: {SERVER_SCHEMA_VERSION})"),
            "opening the server migrates it; `genie migrate --check` shows what will change",
        ),
        Ok(Some(v)) => out.ok("data", format!("server.db schema {v} (this genie: {SERVER_SCHEMA_VERSION})")),
        Err(e) => out.fail("data", format!("server.db: {e}"), "restore server.db from a backup if it is damaged"),
    }
    let mut current = 0;
    let mut pending = Vec::new();
    for (slug, tracker) in genie_core::migrate::trackers(data) {
        match genie_core::migrate::version(&tracker) {
            Ok(None) => {}
            Ok(Some(v)) if v > SCHEMA_VERSION => out.fail(
                "data",
                format!("{slug}: genie.db records schema {v}; this genie understands {SCHEMA_VERSION}"),
                "restore the project's genie.db from a backup, or run the newer genie",
            ),
            Ok(Some(v)) if v < SCHEMA_VERSION => pending.push(format!("{slug} (schema {v})")),
            Ok(Some(_)) => current += 1,
            Err(e) => out.fail(
                "data",
                format!("{slug}: tracker {}: {e}", tracker.display()),
                "restore the project's genie.db from a backup if it is damaged",
            ),
        }
    }
    if current > 0 {
        out.ok("data", format!("{current} tracker(s) at schema {SCHEMA_VERSION}"));
    }
    if !pending.is_empty() {
        out.warn(
            "data",
            format!("tracker(s) behind schema {SCHEMA_VERSION}: {}", pending.join(", ")),
            "opening a project migrates it; `genie migrate` applies every one of them",
        );
    }
}

/// Free space on the file system of `dir` (`df`, POSIX output).
fn free_bytes(dir: &Path) -> Option<u64> {
    let o = Command::new("df").arg("-Pk").arg(dir).output().ok()?;
    let text = String::from_utf8_lossy(&o.stdout);
    let kb: u64 = text.lines().nth(1)?.split_whitespace().nth(3)?.parse().ok()?;
    Some(kb * 1024)
}

/// People and projects; returns the projects' repositories.
fn people_and_projects(out: &mut Out, data: &Path) -> Vec<(String, PathBuf)> {
    let db = match ServerDb::open(&data.join("server.db")) {
        Ok(db) => db,
        Err(e) => {
            out.fail("people", format!("server.db: {e}"), "check the data directory; restore server.db from a backup if it is damaged");
            return Vec::new();
        }
    };
    let users = db.users().unwrap_or_default();
    let active: Vec<_> = users.iter().filter(|u| !u.disabled).collect();
    if active.is_empty() {
        out.warn(
            "people",
            "no users: everything on this machine, the agents the server runs included, acts as the owner without a login",
            "create an admin: genie user add <login> --admin --password-stdin (or in the web: Project and people); from then on agents need their tokens",
        );
    } else if !active.iter().any(|u| u.is_admin) {
        out.warn(
            "people",
            format!("{} user(s), none of them a server admin", active.len()),
            "genie user add <login> --admin --password-stdin",
        );
    } else {
        out.ok("people", format!("{} user(s), {} admin(s)", active.len(), active.iter().filter(|u| u.is_admin).count()));
    }
    let projects = db.projects().unwrap_or_default();
    if projects.is_empty() {
        out.warn("projects", "no projects yet", "genie project add <slug> --name \"…\" [--repo <path>] (or in the web)");
    }
    let mut repos = Vec::new();
    for p in projects {
        if let Err(e) = genie_core::Tracker::open(Path::new(&p.tracker_dir)) {
            out.fail("projects", format!("{}: tracker {}: {e}", p.slug, p.tracker_dir), "restore the project's genie.db from a backup");
            continue;
        }
        let members = db.members_of(&p.slug).map(|m| m.len()).unwrap_or(0);
        match &p.repo {
            Some(r) if !is_git_repo(Path::new(r)) => {
                out.fail(
                    "projects",
                    format!("{}: repository {r} is missing or not a git repository", p.slug),
                    "restore the repository or register the project again",
                );
            }
            Some(r) => {
                out.ok("projects", format!("{} ({}): repository {r}, {} people, orchestrator {}", p.slug, p.name, members, p.autonomy));
                repos.push((p.slug.clone(), PathBuf::from(r)));
            }
            None => out.ok("projects", format!("{} ({}): without code, {} people, orchestrator {}", p.slug, p.name, members, p.autonomy)),
        }
    }
    repos
}

fn is_git_repo(dir: &Path) -> bool {
    dir.is_dir() && Command::new("git").arg("-C").arg(dir).args(["rev-parse", "--git-dir"]).output().is_ok_and(|o| o.status.success())
}

/// The program of a command line, found the way the agents' PATH finds it.
fn find_program(program: &str, cfg: &Config) -> Option<PathBuf> {
    if program.contains('/') {
        let p = PathBuf::from(program);
        return p.is_file().then_some(p);
    }
    let path = cfg.runtime.env.get("PATH").cloned().or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
    std::env::split_paths(&path).map(|d| d.join(program)).find(|p| p.is_file())
}

/// The models pi can use (`provider/model`), from `pi --list-models`.
fn list_models(pi: &Path, cfg: &Config) -> Option<BTreeSet<String>> {
    let list = output(pi, &["--list-models"], cfg)?;
    Some(
        list.lines()
            .skip(1)
            .filter_map(|l| {
                let mut w = l.split_whitespace();
                Some(format!("{}/{}", w.next()?, w.next()?))
            })
            .collect(),
    )
}

/// The models agents can be given: pi's catalogue when pi answers (`None` when
/// it is not on the PATH or does not answer).
pub fn pi_models(cfg: &Config) -> Option<BTreeSet<String>> {
    let pi = [&cfg.runtime.session_command, &cfg.runtime.command]
        .iter()
        .filter_map(|c| c.first().and_then(|g| g.first()))
        .filter_map(|program| find_program(program, cfg))
        .find(|p| p.file_name().is_some_and(|n| n == "pi"))?;
    list_models(&pi, cfg)
}

/// A command's output within a time limit (pi reading its catalogue, git).
fn output(program: &Path, args: &[&str], cfg: &Config) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(std::process::Stdio::null());
    for (k, v) in &cfg.runtime.env {
        cmd.env(k, v);
    }
    let mut child = cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() > Duration::from_secs(20) => {
                let _ = child.kill();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    }
    let out = child.wait_with_output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn agents_and_models(out: &mut Out, cfg: &Config, agents: &AgentConfig) {
    let errors = agents.errors().count();
    if errors > 0 {
        out.fail(
            "agents",
            format!("{errors} error(s) in the agent configuration"),
            "genie agents check shows them; the web page Agents too",
        );
    } else {
        out.ok(
            "agents",
            format!(
                "{} roles, {} team templates, {} skills, {} MCP connections",
                agents.roles.len(),
                agents.teams.len(),
                agents.skills.len(),
                agents.mcp.len()
            ),
        );
    }
    if !cfg.runtime.enabled {
        out.warn("pi", "agents are off (runtime.enabled: false)", "remove runtime.enabled or start genie serve without --no-agents");
        return;
    }
    let programs: BTreeSet<&str> = [&cfg.runtime.session_command, &cfg.runtime.command]
        .iter()
        .filter_map(|c| c.first().and_then(|g| g.first()))
        .map(String::as_str)
        .collect();
    let mut pi = None;
    for program in programs {
        match find_program(program, cfg) {
            Some(p) => {
                let version = output(&p, &["--version"], cfg).map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
                out.ok("pi", format!("{program}: {}{}", p.display(), version.map(|v| format!(", version {v}")).unwrap_or_default()));
                if program == "pi" || p.file_name().is_some_and(|n| n == "pi") {
                    pi = Some(p);
                }
            }
            None => out.fail(
                "pi",
                format!("{program} is not on the PATH of the server"),
                "npm install -g @earendil-works/pi-coding-agent (or set runtime.env.PATH)",
            ),
        }
    }
    if cfg.runtime.mcp_adapter_loaded() {
        out.warn(
            "pi",
            "pi loads pi-mcp-adapter, which replaces pi's native MCP support (pi 1.0): agents may see MCP servers outside their role, their tools are blocked",
            "pi remove npm:pi-mcp-adapter (the Docker image does it at start)",
        );
    }
    if cfg.runtime.mcp_adapter.is_some() {
        out.warn(
            "pi",
            "runtime.mcpAdapter is obsolete: agents get their MCP connections through pi's built-in MCP support",
            "remove runtime.mcpAdapter from config.json",
        );
    }
    if cfg.runtime.session_command.iter().chain(&cfg.runtime.command).flatten().any(|a| a == "--mcp-config") {
        out.warn(
            "pi",
            "the agent command passes --mcp-config, which pi's built-in MCP support does not know",
            "remove `--mcp-config {mcpConfig}` from runtime.command and runtime.sessionCommand",
        );
    }
    // The models agents will ask pi for: roles (with roleModels) and team members.
    let mut wanted: BTreeSet<(String, String)> = BTreeSet::new();
    for r in agents.roles.values() {
        let model = r
            .model
            .clone()
            .or_else(|| cfg.role_models.get(&r.id).and_then(|m| m.model.clone()))
            .or_else(|| cfg.role_models.get(r.class.as_str()).and_then(|m| m.model.clone()));
        match model {
            Some(m) => wanted.insert((m, format!("role {}", r.id))),
            None => wanted.insert((String::new(), format!("role {}", r.id))),
        };
    }
    for t in agents.teams.values() {
        for m in &t.members {
            if let Some(model) = &m.model {
                wanted.insert((model.clone(), format!("template {} ({})", t.id, m.key)));
            }
        }
    }
    let unset: Vec<&str> = wanted.iter().filter(|(m, _)| m.is_empty()).map(|(_, who)| who.as_str()).collect();
    if !unset.is_empty() {
        out.warn(
            "models",
            format!("no model for {}: pi uses its default model", unset.join(", ")),
            "set roleModels in config.json (see config/default.json)",
        );
    }
    let b = cfg.budgets;
    if b.per_task > 0.0 || b.per_epic > 0.0 || b.per_day > 0.0 {
        let unpriced: BTreeSet<&str> = wanted
            .iter()
            .map(|(m, _)| m.as_str())
            .filter(|m| !m.is_empty() && crate::config::price_of(&cfg.model_prices, m).is_none())
            .collect();
        if !unpriced.is_empty() {
            out.warn(
                "budgets",
                format!(
                    "budgets count dollars, but these models have no price and are not counted: {}",
                    unpriced.iter().copied().collect::<Vec<_>>().join(", ")
                ),
                "add them to modelPrices in config.json (dollars per million tokens)",
            );
        }
    }
    let Some(pi) = pi else { return };
    let Some(available) = list_models(&pi, cfg) else {
        out.warn("models", "pi --list-models did not answer", "run it as the server user to see why");
        return;
    };
    let mut by_model: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
    for (model, who) in &wanted {
        if !model.is_empty() {
            by_model.entry(model.as_str()).or_default().push(who.as_str());
        }
    }
    for (model, who) in by_model {
        let id = model.split(':').next().unwrap_or(model);
        let found = if id.contains('/') {
            available.contains(id)
        } else {
            available.iter().any(|a| a.ends_with(&format!("/{id}")) || a.contains(id))
        };
        if found {
            out.ok("models", format!("{model}: available ({})", who.join(", ")));
        } else {
            out.fail(
                "models",
                format!("{model} is not available to pi ({})", who.join(", ")),
                "log in to the provider as the server user (pi, then /login), or fix the model id; pi --list-models shows what is available",
            );
        }
    }
}

fn git(out: &mut Out, repos: &[(String, PathBuf)]) {
    match Command::new("git").arg("--version").output() {
        Ok(o) if o.status.success() => out.ok("git", String::from_utf8_lossy(&o.stdout).trim().to_string()),
        _ => {
            out.fail("git", "git is not installed", "apt install git");
            return;
        }
    }
    for (slug, repo) in repos {
        let email = Command::new("git").arg("-C").arg(repo).args(["config", "user.email"]).output().ok().filter(|o| o.status.success());
        if email.is_none() {
            out.warn(
                "git",
                format!("{slug}: no git identity (user.email) for commits"),
                "agents commit as <name>@genie.local; set git config --global user.name/user.email for the server user to commit as the team",
            );
        }
    }
}

/// The git hosts of `git.json` and the projects' repositories on them. Reachability and
/// token rights are checked by `genie repos check` / the web (they need the network).
fn git_hosts(out: &mut Out, data: &Path) {
    use crate::git::hosts;
    let h = hosts::load(data);
    for e in &h.errors {
        out.fail("git", format!("git.json: {e}"), "fix the entry in <data>/git.json; the host is ignored until then");
    }
    for host in h.map.values() {
        for (what, path) in [("ssh_key", &host.ssh_key), ("ca_cert", &host.ca_cert), ("known_hosts", &host.known_hosts)] {
            if let Some(p) = path
                && !Path::new(p).is_file()
            {
                out.fail("git", format!("host {}: {what} {p} is not a file", host.id), "fix the path in git.json");
            }
        }
        if host.insecure_skip_verify {
            out.warn("git", format!("host {}: certificates are not verified (insecure_skip_verify)", host.id), "give ca_cert instead");
        }
        out.ok("git", format!("host {} ({}): {} over {}", host.id, host.kind.as_str(), host.url, if host.ssh { "ssh" } else { "https" }));
    }
    let Ok(db) = ServerDb::open(&data.join("server.db")) else { return };
    let repos = db.all_repos().unwrap_or_default();
    for r in &repos {
        if !h.map.contains_key(&r.host) {
            out.fail(
                "git",
                format!("{}: repository {} lives on host {}, which git.json does not define", r.project, r.name, r.host),
                "add the host to <data>/git.json or move the repository",
            );
        } else if let Err(e) = crate::git::policy::Policy::parse(&r.policy) {
            out.fail(
                "git",
                format!("{}: repository {}: {e}", r.project, r.name),
                "PATCH /api/repos/<name> with a valid policy; until then agents get no access",
            );
        }
        if let Some(host) = h.map.get(&r.host)
            && host.kind != hosts::Kind::Plain
            && db.repo_token_info(&r.project, &r.name).ok().flatten().is_none()
        {
            out.warn(
                "git",
                format!("{}: repository {} has no access token", r.project, r.name),
                "set it on the repository: the project page, or `genie repos set --token-stdin`",
            );
        }
    }
    if !repos.is_empty() {
        out.ok(
            "git",
            format!(
                "{} repositor{} in {} project(s)",
                repos.len(),
                if repos.len() == 1 { "y" } else { "ies" },
                repos.iter().map(|r| &r.project).collect::<BTreeSet<_>>().len()
            ),
        );
    }
}

fn vault(out: &mut Out, data: &Path, cfg: &Config) {
    let root = cfg.vault_path(data);
    if !root.join(".git").exists() {
        out.warn(
            "vault",
            format!("the vault {} is not a git repository: no history, no sync", root.display()),
            "leave vault.commit on (the default) and restart the server",
        );
        return;
    }
    let Some(remote) = cfg.vault.remote.as_deref().filter(|r| !r.trim().is_empty()) else {
        out.warn(
            "vault",
            format!("the vault lives only on this server ({})", root.display()),
            "set vault.remote in config.json to a git repository: people open it in Obsidian, and it is one more copy",
        );
        return;
    };
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(&root).args(["ls-remote", "--heads", remote]).env("GIT_TERMINAL_PROMPT", "0");
    let Ok(mut child) = cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::piped()).spawn() else {
        return;
    };
    let start = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if start.elapsed() > Duration::from_secs(20) => {
                let _ = child.kill();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => break None,
        }
    };
    let mut err = String::new();
    if let Some(mut e) = child.stderr.take() {
        let _ = std::io::Read::read_to_string(&mut e, &mut err);
    }
    match status {
        Some(s) if s.success() => {
            let last = crate::vault_sync::state().and_then(|st| st.error);
            match last {
                Some(e) => out.warn(
                    "vault",
                    format!("syncs with {remote}, the last sync failed: {e}"),
                    "the server retries; the admins got a notification",
                ),
                None => out.ok("vault", format!("syncs with {remote} every {} s", cfg.vault.sync_secs.unwrap_or(120))),
            }
        }
        Some(_) => out.fail(
            "vault",
            format!("the vault's remote {remote} does not answer: {}", err.lines().last().unwrap_or("git ls-remote failed")),
            "check the address and the access of the server user (an SSH key in ~/.ssh or a token in the URL)",
        ),
        None => out.fail("vault", format!("the vault's remote {remote} did not answer in 20 s"), "check the network and the address"),
    }
}

fn channels(out: &mut Out, cfg: &Config) {
    match &cfg.telegram {
        Some(t) if !t.token.trim().is_empty() => out.ok("channels", "Telegram bot configured"),
        _ => out.warn(
            "channels",
            "Telegram is not configured",
            "telegram.token in config.json: notifications and agents' questions reach people in Telegram",
        ),
    }
    match &cfg.smtp {
        Some(s) if !s.host.trim().is_empty() => {
            let addr = format!("{}:{}", s.host, s.port);
            let reachable = std::net::ToSocketAddrs::to_socket_addrs(&addr)
                .ok()
                .and_then(|mut a| a.next())
                .is_some_and(|a| std::net::TcpStream::connect_timeout(&a, Duration::from_secs(3)).is_ok());
            if reachable {
                out.ok("channels", format!("mail through {addr}"));
            } else {
                out.warn(
                    "channels",
                    format!("the mail server {addr} does not answer"),
                    "check smtp.host and smtp.port in config.json and the firewall",
                );
            }
        }
        _ => out.warn("channels", "mail is not configured", "smtp in config.json: notifications by e-mail"),
    }
}

fn network(out: &mut Out, cfg: &Config) {
    let loopback = ["127.0.0.1", "localhost", "::1"].contains(&cfg.bind.as_str());
    if loopback {
        out.ok("network", format!("listens on {}:{} (this machine only)", cfg.bind, cfg.port));
    } else if cfg.allow_hosts.is_empty() {
        out.fail(
            "network",
            format!("listens on {}:{}, but allowHosts is empty: other machines get 421", cfg.bind, cfg.port),
            "add the names people use to open genie to allowHosts in config.json",
        );
    } else {
        out.ok("network", format!("listens on {}:{} for {}", cfg.bind, cfg.port, cfg.allow_hosts.join(", ")));
    }
    let channels =
        cfg.telegram.as_ref().is_some_and(|t| !t.token.trim().is_empty()) || cfg.smtp.as_ref().is_some_and(|s| !s.host.trim().is_empty());
    match &cfg.public_url {
        None if channels || !loopback => out.warn(
            "network",
            format!("publicUrl is not set: links in messages point to {}", cfg.public_url()),
            "set publicUrl in config.json to the address people open",
        ),
        None => {}
        Some(url) => {
            let host = url.split("://").nth(1).unwrap_or(url).split('/').next().unwrap_or_default();
            let bare = host.rsplit_once(':').map(|(h, _)| h).unwrap_or(host);
            let allowed = ["127.0.0.1", "localhost", "[::1]"].contains(&bare) || cfg.allow_hosts.iter().any(|h| h == host || h == bare);
            if allowed {
                out.ok("network", format!("links in messages: {url}"));
            } else {
                out.fail(
                    "network",
                    format!("publicUrl {url}: {host} is not in allowHosts, its links get 421"),
                    "add it to allowHosts in config.json",
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(json: serde_json::Value) -> Config {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), json.to_string()).unwrap();
        Config::load(dir.path()).unwrap()
    }

    fn levels(checks: &[Check], area: &str) -> Vec<Level> {
        checks.iter().filter(|c| c.area == area).map(|c| c.level).collect()
    }

    #[test]
    fn network_settings_that_lock_people_out_fail() {
        let mut out = Out(Vec::new());
        network(&mut out, &cfg(serde_json::json!({ "bind": "0.0.0.0" })));
        assert_eq!(levels(&out.0, "network"), [Level::Fail, Level::Warn], "{:?}", out.0);
        let mut out = Out(Vec::new());
        network(
            &mut out,
            &cfg(serde_json::json!({ "bind": "0.0.0.0", "allowHosts": ["genie.lan"], "publicUrl": "http://genie.lan:7420" })),
        );
        assert_eq!(levels(&out.0, "network"), [Level::Ok, Level::Ok], "{:?}", out.0);
        let mut out = Out(Vec::new());
        network(&mut out, &cfg(serde_json::json!({ "publicUrl": "https://genie.example.com" })));
        assert_eq!(levels(&out.0, "network"), [Level::Ok, Level::Fail], "a public address the Host check refuses: {:?}", out.0);
    }

    #[test]
    fn a_fresh_data_directory_is_ready_but_for_people_and_projects() {
        let data = tempfile::tempdir().unwrap();
        let c = cfg(serde_json::json!({}));
        let web = tempfile::tempdir().unwrap();
        std::fs::write(web.path().join("index.html"), "<html>").unwrap();
        let mut out = Out(Vec::new());
        storage(&mut out, data.path());
        people_and_projects(&mut out, data.path());
        assert_eq!(levels(&out.0, "data")[0], Level::Ok);
        assert_eq!(levels(&out.0, "people"), [Level::Warn]);
        assert_eq!(levels(&out.0, "projects"), [Level::Warn]);
        let (text, _) = print(&run(data.path(), &c, &AgentConfig::load(data.path(), &c, None), Some(web.path())));
        assert!(text.contains("warn people"), "{text}");
        assert!(text.contains(&format!("ok   web       web UI in {}", web.path().display())), "{text}");
    }
}
