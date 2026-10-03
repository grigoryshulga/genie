//! Command line: the operations of the catalog (`genie task …`, `genie team …`,
//! see [`crate::ops`]), server administration (works on the data directory
//! directly, with or without a running server) and `genie serve`.

use std::path::{Path, PathBuf};

use clap::{ArgMatches, CommandFactory, FromArgMatches, Parser, Subcommand};
use genie_core::Tracker;
use genie_core::server_db::ServerDb;

use crate::config::Config;
use crate::ops::{self, Auth, Cx, InProcess, Remote};
use crate::state::App;

#[derive(Parser)]
#[command(name = "genie", version, about = "Genie: tasks, knowledge and agent teams for small teams")]
pub struct Cli {
    /// Data directory (default: $GENIE_DATA or ~/.local/share/genie).
    #[arg(long, global = true)]
    data: Option<PathBuf>,
    /// The project to act in (default: $GENIE_PROJECT, else your first project).
    #[arg(long, global = true)]
    project: Option<String>,
    /// Print the answer as JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server: web UI, API, agents, automations and channels.
    Serve {
        #[arg(long)]
        port: Option<u16>,
        /// Serve the web UI built in this directory (`npm run build:web` → web/dist) instead of the one built into genie.
        #[arg(long)]
        web: Option<PathBuf>,
        /// Do not start agents (UI and automations only).
        #[arg(long)]
        no_agents: bool,
    },
    /// Your pi session becomes the orchestrator of the project (--project) on the running server; the server's orchestrator waits until pi ends.
    Orchestrate {
        /// Take the console from whoever holds it.
        #[arg(long)]
        force: bool,
        /// The pi command (default: $GENIE_PI, else pi).
        #[arg(long)]
        pi: Option<String>,
        /// More arguments for pi, after `--`.
        #[arg(last = true)]
        pi_args: Vec<String>,
    },
    /// Give a user a role in a project (viewer, member, admin, owner).
    Member { project: String, login: String, role: String },
    /// Print an invitation link for a project.
    Invite {
        project: String,
        #[arg(long, default_value = "member")]
        role: String,
        #[arg(long)]
        email: Option<String>,
    },
    /// Consistent snapshot of every database, the vault and the server's configuration into a directory.
    Backup {
        dir: PathBuf,
        /// Keep only this many most recent backups in the directory (older `genie-*` are removed).
        #[arg(long)]
        keep: Option<usize>,
        /// Also copy `secrets.key`, the key of the stored tokens (repositories, LiteLLM): without it a restored server asks for them again.
        #[arg(long)]
        with_secrets: bool,
    },
    /// Put a backup back: the databases, the vault and the configuration into the data directory (stop the server first).
    Restore {
        /// A backup directory made by `genie backup` (`genie-<time>`).
        backup: PathBuf,
        /// Replace what the data directory already holds.
        #[arg(long)]
        force: bool,
    },
    /// What happened over the last days, for reviewing a pilot: tasks, decisions, reviews, agent runs, knowledge (--project: one project only).
    Stats {
        #[arg(long, default_value_t = 7)]
        days: i64,
    },
    /// Check that the server is ready: data, web UI, people, projects, pi and the models, sandbox, git, channels, network.
    Doctor {
        /// The web UI directory the server is started with (`serve --web`), if any.
        #[arg(long)]
        web: Option<PathBuf>,
    },
    /// Knowledge vault maintenance.
    #[command(subcommand)]
    Vault(VaultCmd),
    /// Create a standalone tracker directory (legacy layout; --project names it).
    Init {
        #[arg(long, default_value = ".genie")]
        dir: PathBuf,
        #[arg(long)]
        prefix: Option<String>,
    },
}

#[derive(Subcommand)]
enum VaultCmd {
    /// Copy Markdown pages (e.g. a repository's docs/) into a vault space, keeping folders.
    Import {
        dir: PathBuf,
        #[arg(long)]
        space: String,
    },
    /// Rebuild the search index from the files.
    Reindex,
    /// Sync the vault with its git remote (vault.remote) once: fetch, merge, push. The running server does it by itself.
    Sync,
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

/// Where the operations of the command line go: with `GENIE_TOKEN`, to the
/// server at `GENIE_URL` as that token's person or agent; without, straight to
/// the data directory as its operator (a server admin), with or without a
/// running server.
pub fn op_context(data: &Path, project: Option<String>, setup: bool) -> Result<Cx, String> {
    let (task, team) = (env("GENIE_TASK"), env("GENIE_TEAM"));
    if let Some(token) = env("GENIE_TOKEN") {
        let base = env("GENIE_URL").unwrap_or_else(|| format!("http://127.0.0.1:{}", Config::load(data).map(|c| c.port).unwrap_or(7420)));
        return Ok(Cx { api: Box::new(Remote::new(&base, &token, project.clone())), project, task, team, local: true });
    }
    // Setting up (people, projects) may start a data directory; anything else needs one.
    if !setup && !data.join("server.db").exists() {
        return Err(format!(
            "no genie server data in {}: pass --data, or reach a server with GENIE_URL and GENIE_TOKEN (genie user token <login>)",
            data.display()
        ));
    }
    let cfg = Config::load(data)?;
    let port = cfg.port;
    let app = App::open(data, cfg, None).map_err(|e| e.to_string())?;
    Ok(Cx {
        api: Box::new(InProcess::new(crate::http::router(app), port, Auth::Operator, project.clone())),
        project,
        task,
        team,
        local: true,
    })
}

async fn run_op(entry: &ops::Entry, args: &ArgMatches, data: &Path, project: Option<String>, json: bool) -> Result<(), String> {
    let cx = op_context(data, project, matches!(entry.group, "user" | "project"))?;
    let out = entry.run_cli(args, &cx).await?;
    if json {
        println!("{}", serde_json::to_string_pretty(&out.data).map_err(|e| e.to_string())?);
    } else if !out.text.is_empty() {
        println!("{}", out.text);
    }
    out.failed.map_or(Ok(()), Err)
}

/// A command kept from before the catalog, done by its operation.
async fn legacy_op(group: &str, name: &str, args: serde_json::Value, data: &Path) -> Result<String, String> {
    let entry = ops::find(group, name).ok_or(format!("no operation {group} {name}"))?;
    Ok(entry.run_json(args, &op_context(data, None, true)?).await?.text)
}

/// The whole command line: the commands below and the catalog's.
pub fn command() -> clap::Command {
    ops::commands(Cli::command())
}

pub async fn run() -> Result<(), String> {
    let matches = command().get_matches();
    if let Some((entry, args)) = ops::chosen(&matches) {
        let data = matches.get_one::<PathBuf>("data").cloned().unwrap_or_else(crate::default_data_dir);
        let project = matches.get_one::<String>("project").cloned().or_else(|| env("GENIE_PROJECT"));
        return run_op(entry, args, &data, project, matches.get_flag("json")).await;
    }
    let cli = Cli::from_arg_matches(&matches).map_err(|e| e.to_string())?;
    let data = cli.data.unwrap_or_else(crate::default_data_dir);
    let (project, json) = (cli.project, cli.json);
    match cli.command {
        Command::Serve { port, web, no_agents } => {
            let mut cfg = Config::load(&data)?;
            if let Some(p) = port {
                cfg.port = p;
            }
            if no_agents {
                cfg.runtime.enabled = false;
            }
            let app = App::open(&data, cfg, web).map_err(|e| e.to_string())?;
            match (crate::http::web::resolve(app.web_root.as_deref()), &app.web_root) {
                (crate::http::web::WebUi::BuiltIn, Some(dir)) => {
                    eprintln!("genie: no web UI in {}: serving the one built into genie", dir.display())
                }
                (crate::http::web::WebUi::Missing(why), _) => eprintln!("genie: {why}"),
                _ => {}
            }
            app.print_agent_errors();
            crate::runtime::start(&app);
            crate::git::delivery::spawn_poller(app.clone());
            crate::serve(app).await?;
        }
        Command::Orchestrate { force, pi, pi_args } => {
            let cx = op_context(&data, project.clone(), false)?;
            let url =
                env("GENIE_URL").unwrap_or_else(|| format!("http://127.0.0.1:{}", Config::load(&data).map(|c| c.port).unwrap_or(7420)));
            let pi = pi.or_else(|| env("GENIE_PI")).unwrap_or_else(|| "pi".into());
            let run = crate::orchestrate::Orchestrate { api: cx.api, url: url.trim_end_matches('/').to_string(), force, pi, pi_args };
            let code = crate::orchestrate::run(run).await?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Command::Member { project: slug, login, role } => {
            let args = serde_json::json!({ "project": slug, "login": login, "role": role });
            println!("{}", legacy_op("project", "member", args, &data).await?);
        }
        Command::Invite { project: slug, role, email } => {
            let args = serde_json::json!({ "project": slug, "role": role, "email": email });
            println!("{}", legacy_op("project", "invite", args, &data).await?);
        }
        Command::Restore { backup: dir, force } => {
            println!("{}", restore(&data, &dir, force)?);
        }
        Command::Backup { dir, keep, with_secrets } => {
            let cfg = Config::load(&data)?;
            let report = backup_with(&data, &cfg, &dir, with_secrets)?;
            println!("{report}");
            if let Some(keep) = keep {
                for old in prune_backups(&dir, keep)? {
                    println!("removed {}", old.display());
                }
            }
        }
        Command::Stats { days } => {
            let cfg = Config::load(&data)?;
            let stats = crate::stats::collect(&data, days, project.as_deref(), &cfg.model_prices)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&stats).map_err(|e| e.to_string())?);
            } else {
                println!("{}", crate::stats::render(&stats));
            }
        }
        Command::Doctor { web } => {
            let cfg = Config::load(&data)?;
            let agents = crate::agent_config::AgentConfig::load(&data, &cfg, None);
            let (report, failed) = crate::doctor::print(&crate::doctor::run(&data, &cfg, &agents, web.as_deref()));
            println!("{report}");
            if failed > 0 {
                return Err(format!("{failed} check(s) failed"));
            }
        }
        Command::Vault(cmd) => {
            let cfg = Config::load(&data)?;
            let vault_dir = cfg.vault_path(&data);
            let index = data.join("vault-index.db");
            match cmd {
                VaultCmd::Import { dir, space } => {
                    let mut copied = 0;
                    let mut stack = vec![dir.clone()];
                    while let Some(d) = stack.pop() {
                        for e in std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?.flatten() {
                            let path = e.path();
                            let name = e.file_name().to_string_lossy().into_owned();
                            if name.starts_with('.') {
                                continue;
                            }
                            if path.is_dir() {
                                stack.push(path);
                            } else if name.ends_with(".md") {
                                let rel = path.strip_prefix(&dir).map_err(|e| e.to_string())?;
                                let target = vault_dir.join(&space).join(rel);
                                if target.exists() {
                                    println!("skip {} (exists)", target.display());
                                    continue;
                                }
                                std::fs::create_dir_all(target.parent().unwrap_or(&vault_dir)).map_err(|e| e.to_string())?;
                                std::fs::copy(&path, &target).map_err(|e| e.to_string())?;
                                copied += 1;
                            }
                        }
                    }
                    let mut v =
                        genie_core::vault::Vault::open(&vault_dir, &index, cfg.vault.commit.unwrap_or(true)).map_err(|e| e.to_string())?;
                    let _ = std::process::Command::new("git").arg("-C").arg(&vault_dir).args(["add", "-A", &space]).output();
                    let _ = std::process::Command::new("git")
                        .arg("-C")
                        .arg(&vault_dir)
                        .args([
                            "-c",
                            "user.name=genie",
                            "-c",
                            "user.email=genie@genie.local",
                            "commit",
                            "-q",
                            "-m",
                            &format!("import {} into {space}", dir.display()),
                        ])
                        .output();
                    v.refresh().map_err(|e| e.to_string())?;
                    println!("{copied} page(s) imported into {}/{space}", vault_dir.display());
                }
                VaultCmd::Sync => {
                    let app = App::open(&data, cfg.clone(), None).map_err(|e| e.to_string())?;
                    match crate::vault_sync::sync(&app).map_err(|e| e.to_string())? {
                        None => return Err("vault.remote is not set in config.json".into()),
                        Some(st) if !st.ok => return Err(st.error.unwrap_or_default()),
                        Some(st) => {
                            println!("vault synced with {} ({}): {} commit(s) in, {} out", st.remote, st.branch, st.pulled, st.pushed);
                            if !st.both.is_empty() {
                                println!("changed on both sides (the server's lines kept where they overlap): {}", st.both.join(", "));
                            }
                        }
                    }
                }
                VaultCmd::Reindex => {
                    let _ = std::fs::remove_file(&index);
                    let v =
                        genie_core::vault::Vault::open(&vault_dir, &index, cfg.vault.commit.unwrap_or(true)).map_err(|e| e.to_string())?;
                    println!("index rebuilt for {}", v.root().display());
                }
            }
        }
        Command::Init { dir, prefix } => {
            let t = Tracker::init(&dir, prefix.as_deref(), project.as_deref()).map_err(|e| e.to_string())?;
            let m = t.meta().map_err(|e| e.to_string())?;
            println!("genie tracker in {} (project {}, prefix {})", dir.display(), m.project, m.prefix);
        }
    }
    Ok(())
}

/// `VACUUM INTO` gives a consistent copy of a live SQLite database (WAL included)
/// without stopping the server; the vault is bundled with git. The server's
/// configuration (`config.json` with the channel secrets, roles, templates,
/// skills, `mcp.json`) is copied too: the backup directory is private (0700).
pub fn backup(data: &std::path::Path, cfg: &Config, dir: &std::path::Path) -> Result<String, String> {
    backup_with(data, cfg, dir, false)
}

/// [`backup`], optionally with `secrets.key` (the key of the tokens stored in `server.db`).
pub fn backup_with(data: &std::path::Path, cfg: &Config, dir: &std::path::Path, with_secrets: bool) -> Result<String, String> {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
    let out = dir.join(format!("genie-{stamp}"));
    std::fs::create_dir_all(out.join("projects")).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    let snapshot = |src: &std::path::Path, dst: &std::path::Path| -> Result<(), String> {
        let conn = rusqlite::Connection::open(src).map_err(|e| format!("{}: {e}", src.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(30)).map_err(|e| e.to_string())?;
        conn.execute("VACUUM INTO ?1", [dst.to_string_lossy()]).map_err(|e| format!("{}: {e}", src.display()))?;
        Ok(())
    };
    let mut lines = Vec::new();
    snapshot(&data.join("server.db"), &out.join("server.db"))?;
    lines.push("server.db".to_string());
    let db = ServerDb::open(&data.join("server.db")).map_err(|e| e.to_string())?;
    for p in db.projects().map_err(|e| e.to_string())? {
        let src = std::path::Path::new(&p.tracker_dir).join("genie.db");
        snapshot(&src, &out.join("projects").join(format!("{}.db", p.slug)))?;
        lines.push(format!("projects/{}.db ({})", p.slug, src.display()));
    }
    let vault = cfg.vault_path(data);
    if vault.join(".git").exists() {
        let bundle = out.join("vault.bundle");
        let st = std::process::Command::new("git")
            .arg("-C")
            .arg(&vault)
            .args(["bundle", "create"])
            .arg(&bundle)
            .arg("--all")
            .output()
            .map_err(|e| e.to_string())?;
        if !st.status.success() {
            return Err(format!("git bundle: {}", String::from_utf8_lossy(&st.stderr)));
        }
        lines.push("vault.bundle (restore: git clone vault.bundle vault)".into());
    }
    let mut config = Vec::new();
    for item in ["config.json", "git.json", "mcp.json", "agents", "teams", "skills"] {
        let src = data.join(item);
        if src.exists() {
            copy_tree(&src, &out.join("config").join(item))?;
            config.push(item);
        }
    }
    if !config.is_empty() {
        lines.push(format!("config/: {}", config.join(", ")));
    }
    // Where the server lived: a restore elsewhere moves the trackers that were under this directory.
    let manifest = serde_json::json!({ "data": data.to_string_lossy(), "version": env!("CARGO_PKG_VERSION"), "made": chrono::Utc::now().to_rfc3339() });
    std::fs::write(out.join("MANIFEST.json"), serde_json::to_string_pretty(&manifest).unwrap_or_default()).map_err(|e| e.to_string())?;
    let key = data.join("secrets.key");
    if with_secrets && key.exists() {
        copy_tree(&key, &out.join("secrets.key"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(out.join("secrets.key"), std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        }
        lines.push("secrets.key (the key of the stored tokens: keep this backup private)".into());
    } else if key.exists() {
        lines.push("secrets.key is NOT included: tokens of repositories and LiteLLM keys cannot be read after a restore (--with-secrets, or keep the file apart)".into());
    }
    Ok(format!("backup in {}:\n  {}", out.display(), lines.join("\n  ")))
}

/// Put a backup (made by [`backup`]) into `data`: every database is checked first, then
/// `server.db`, the trackers (to the paths the restored server records for them), the vault
/// and the configuration go back. Nothing is overwritten without `force`; stop the server first.
pub fn restore(data: &std::path::Path, backup: &std::path::Path, force: bool) -> Result<String, String> {
    let server_db = backup.join("server.db");
    if !server_db.is_file() {
        return Err(format!("{} is not a genie backup: no server.db in it", backup.display()));
    }
    let integrity = |path: &std::path::Path| -> Result<(), String> {
        let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let verdict: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).map_err(|e| format!("{}: {e}", path.display()))?;
        if verdict == "ok" { Ok(()) } else { Err(format!("{} is damaged: {verdict}", path.display())) }
    };
    integrity(&server_db)?;
    let mut trackers: Vec<(String, PathBuf)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(backup.join("projects")) {
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_some_and(|x| x == "db") {
                integrity(&path)?;
                if let Some(slug) = path.file_stem().and_then(|s| s.to_str()) {
                    trackers.push((slug.to_string(), path));
                }
            }
        }
    }
    let target_db = data.join("server.db");
    if target_db.exists() && !force {
        return Err(format!(
            "{} already holds a server (server.db): restore into an empty directory, or pass --force to replace it",
            data.display()
        ));
    }
    std::fs::create_dir_all(data).map_err(|e| format!("{}: {e}", data.display()))?;
    let put = |from: &std::path::Path, to: &std::path::Path| -> Result<(), String> {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        // The databases of a live server carry -wal/-shm files that must not outlive the copy they belonged to.
        for suffix in ["-wal", "-shm"] {
            let mut stale = to.as_os_str().to_owned();
            stale.push(suffix);
            let _ = std::fs::remove_file(stale);
        }
        std::fs::copy(from, to).map(|_| ()).map_err(|e| format!("{}: {e}", to.display()))
    };
    let mut lines = Vec::new();
    put(&server_db, &target_db)?;
    lines.push(format!("server.db → {}", target_db.display()));

    let db = ServerDb::open(&target_db).map_err(|e| e.to_string())?;
    let mut missing = Vec::new();
    let old_data: Option<String> = std::fs::read_to_string(backup.join("MANIFEST.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|m| m["data"].as_str().map(str::to_string));
    for p in db.projects().map_err(|e| e.to_string())? {
        match trackers.iter().find(|(slug, _)| *slug == p.slug) {
            Some((_, from)) => {
                // A tracker that lived under the old data directory moves with it (projects without code).
                let mut tracker_dir = p.tracker_dir.clone();
                if let Some(old) = &old_data
                    && let Ok(rest) = std::path::Path::new(&tracker_dir).strip_prefix(old)
                    && std::path::Path::new(old) != data
                {
                    tracker_dir = data.join(rest).to_string_lossy().into_owned();
                    db.conn()
                        .execute("UPDATE projects SET tracker_dir = ?1 WHERE slug = ?2", rusqlite::params![tracker_dir, p.slug])
                        .map_err(|e| e.to_string())?;
                }
                let to = std::path::Path::new(&tracker_dir).join("genie.db");
                put(from, &to)?;
                lines.push(format!("projects/{}.db → {}", p.slug, to.display()));
                if let Some(repo) = &p.repo
                    && !std::path::Path::new(repo).is_dir()
                {
                    lines.push(format!("  note: the repository of {} ({repo}) is not on this machine: clone it there, or `genie project update` to move it", p.slug));
                }
            }
            None => missing.push(p.slug),
        }
    }
    if !missing.is_empty() {
        lines.push(format!("WARNING: the backup has no tracker for: {}", missing.join(", ")));
    }

    let cfg = Config::load(&backup.join("config")).unwrap_or_default();
    let vault = cfg.vault_path(data);
    let bundle = backup.join("vault.bundle");
    if bundle.is_file() {
        if vault.join(".git").exists() && !force {
            lines.push(format!("vault: {} exists, left as it is (--force replaces it)", vault.display()));
        } else {
            if vault.exists() {
                std::fs::remove_dir_all(&vault).map_err(|e| format!("{}: {e}", vault.display()))?;
            }
            let out =
                std::process::Command::new("git").arg("clone").arg("-q").arg(&bundle).arg(&vault).output().map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(format!("git clone of the vault bundle: {}", String::from_utf8_lossy(&out.stderr).trim()));
            }
            lines.push(format!("vault.bundle → {}", vault.display()));
        }
    }

    let mut restored = Vec::new();
    if let Ok(entries) = std::fs::read_dir(backup.join("config")) {
        for e in entries.flatten() {
            let to = data.join(e.file_name());
            if to.exists() && !force {
                lines.push(format!("config/{} exists, left as it is (--force replaces it)", e.file_name().to_string_lossy()));
                continue;
            }
            if to.is_dir() {
                std::fs::remove_dir_all(&to).map_err(|e| format!("{}: {e}", to.display()))?;
            }
            copy_tree(&e.path(), &to)?;
            restored.push(e.file_name().to_string_lossy().into_owned());
        }
    }
    if !restored.is_empty() {
        lines.push(format!("config: {}", restored.join(", ")));
    }

    let key = backup.join("secrets.key");
    if key.is_file() {
        put(&key, &data.join("secrets.key"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(data.join("secrets.key"), std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
        }
        lines.push("secrets.key → the stored tokens are readable".into());
    } else if db.has_sealed_secrets().unwrap_or(false) && !data.join("secrets.key").exists() {
        lines.push("note: the backup has no secrets.key: tokens of repositories and LiteLLM keys must be entered again".into());
    }
    lines.push("not in a backup: agents' conversations (sessions/), workspaces, mirrors of repositories (made again on the first sync). Start the server, then `genie doctor`.".into());
    Ok(format!("restored from {}:\n  {}", backup.display(), lines.join("\n  ")))
}

fn copy_tree(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    if src.is_dir() {
        std::fs::create_dir_all(dst).map_err(|e| format!("{}: {e}", dst.display()))?;
        for e in std::fs::read_dir(src).map_err(|e| format!("{}: {e}", src.display()))?.flatten() {
            copy_tree(&e.path(), &dst.join(e.file_name()))?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::copy(src, dst).map(|_| ()).map_err(|e| format!("{}: {e}", src.display()))
    }
}

/// Remove all but the `keep` most recent backups (`genie-<stamp>` directories) in `dir`.
pub fn prune_backups(dir: &std::path::Path, keep: usize) -> Result<Vec<PathBuf>, String> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("genie-") && n[6..].chars().all(|c| c.is_ascii_digit() || c == '-'))
        })
        .collect();
    // The stamps sort by time.
    found.sort();
    let old = found.len().saturating_sub(keep.max(1));
    let removed: Vec<PathBuf> = found.into_iter().take(old).collect();
    for p in &removed {
        std::fs::remove_dir_all(p).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_command_line_is_consistent() {
        super::command().debug_assert();
    }
}
