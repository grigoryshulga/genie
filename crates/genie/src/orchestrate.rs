//! `genie orchestrate`: your own pi session becomes the orchestrator of a
//! project. It takes the project's orchestrator console on the running server
//! (the server's orchestrator waits meanwhile), starts pi in this terminal with
//! the orchestrator's prompt, its token and the genie-bus extension — team mail
//! arrives in the conversation between steps and wakes the idle session — keeps
//! the console while pi runs and gives it back when pi ends.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::ops::{Api, Remote};

/// The console's card of teams and its `/genie` boards (see the file).
pub const CONSOLE_EXTENSION: &str = include_str!("../pi/genie-console.ts");

/// How often the console is renewed (it lapses after `CONSOLE_TTL_SECS` without).
const RENEW_EVERY: Duration = Duration::from_secs(crate::http::console::CONSOLE_TTL_SECS as u64 / 4);

pub struct Orchestrate {
    /// The API as the person who takes the console (their token, or the operator).
    pub api: Box<dyn Api>,
    /// The running server the session talks to.
    pub url: String,
    pub force: bool,
    /// The pi command.
    pub pi: String,
    /// More arguments for pi (after `--`).
    pub pi_args: Vec<String>,
}

/// Files the session reads, removed when it ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(dir: &Path, name: &str, text: &str) -> Result<PathBuf, String> {
    let path = dir.join(name);
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

pub async fn run(o: Orchestrate) -> Result<i32, String> {
    let health = reqwest::Client::new().get(format!("{}/api/health", o.url)).timeout(Duration::from_secs(5)).send().await;
    if !health.is_ok_and(|r| r.status().is_success()) {
        return Err(format!("no genie server answers at {}: start `genie serve` (or set GENIE_URL to the server)", o.url));
    }
    let taken = o.api.call("POST", "/orchestrator/console", Some(json!({ "force": o.force }))).await?;
    let token = taken["token"].as_str().ok_or("the server gave no console token")?.to_string();
    let project = taken["console"]["project"].as_str().unwrap_or_default().to_string();
    let console: Box<dyn Api> = Box::new(Remote::new(&o.url, &token, Some(project.clone())));

    let dir = std::env::temp_dir().join(format!("genie-console-{project}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let scratch = Scratch(dir);
    let prompt = write(&scratch.0, "prompt.md", taken["prompt"].as_str().unwrap_or_default())?;
    let bus = write(&scratch.0, "genie-bus.ts", crate::sessions::EXTENSION)?;
    let card = write(&scratch.0, "genie-console.ts", CONSOLE_EXTENSION)?;

    let mut args: Vec<String> = Vec::new();
    let own = |flag: &str| o.pi_args.iter().any(|a| a == flag || a.starts_with(&format!("{flag}=")));
    for (flag, key) in [("--model", "model"), ("--thinking", "thinking")] {
        if let Some(v) = taken[key].as_str().filter(|v| !v.is_empty() && !own(flag)) {
            args.extend([flag.to_string(), v.to_string()]);
        }
    }
    args.extend(["--append-system-prompt".into(), prompt.to_string_lossy().into_owned()]);
    for ext in [&bus, &card] {
        args.extend(["-e".into(), ext.to_string_lossy().into_owned()]);
    }
    args.extend(o.pi_args.iter().cloned());

    eprintln!("genie: you hold the orchestrator console of {project} (the server's orchestrator waits); starting {} …", o.pi);
    let spawned = tokio::process::Command::new(&o.pi)
        .args(&args)
        .env("GENIE_URL", &o.url)
        .env("GENIE_TOKEN", &token)
        .env("GENIE_PROJECT", &project)
        .env("GENIE_AGENT_ROLE", "orchestrator")
        .env("GENIE_AGENT_NAME", "orchestrator")
        .env("GENIE_CONSOLE", "1")
        .env("GENIE_BIN", std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "genie".into()))
        .env_remove("GENIE_TASK")
        .env_remove("GENIE_TEAM")
        // The server's LiteLLM service token prices models; the console needs none of it.
        .env_remove(crate::model_prices::INFO_TOKEN)
        .env_remove(format!("{}_FILE", crate::model_prices::INFO_TOKEN))
        // Keep the TypeScript pi extension of the older, serverless genie out.
        .env("GENIE_ROLE", "off")
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            let _ = console.call("DELETE", "/orchestrator/console", None).await;
            return Err(format!("cannot start {}: {e} (install pi, or pass --pi <command>)", o.pi));
        }
    };

    // Ctrl-C belongs to pi; the console stays until pi ends.
    let status = loop {
        tokio::select! {
            s = child.wait() => break s,
            _ = tokio::signal::ctrl_c() => {}
            _ = tokio::time::sleep(RENEW_EVERY) => {
                if let Err(e) = console.call("POST", "/orchestrator/console/renew", Some(json!({}))).await {
                    eprintln!("genie: the console of {project} could not be renewed: {e}");
                }
            }
        }
    };
    let released: Value = console.call("DELETE", "/orchestrator/console", None).await.unwrap_or(Value::Null);
    if released["released"] == json!(true) {
        eprintln!("genie: the orchestrator console of {project} is given back; the server's orchestrator carries on");
    }
    drop(scratch);
    let status = status.map_err(|e| e.to_string())?;
    Ok(status.code().unwrap_or(1))
}
