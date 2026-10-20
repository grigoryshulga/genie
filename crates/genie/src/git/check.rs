//! Checks of a host and of a project's repository against the real thing: what the token
//! can do, whether the default branch is protected, whether the repository is reachable.
//! They need the network, so they run when an administrator asks (`genie repos check`, the
//! web), not at every `genie doctor`.

use std::sync::Arc;

use serde::Serialize;

use super::hosts::{self, Kind};
use super::policy::Policy;
use super::provider::{Api, ApiError};
use super::store;
use crate::state::App;

#[derive(Debug, Clone, Serialize)]
pub struct Line {
    /// `ok`, `warn` or `fail`.
    pub level: &'static str,
    pub text: String,
}

fn line(level: &'static str, text: impl Into<String>) -> Line {
    Line { level, text: text.into() }
}

/// Is anything in the report a failure?
pub fn failed(lines: &[Line]) -> bool {
    lines.iter().any(|l| l.level == "fail")
}

/// A host: its configuration (the tokens are the repositories', see [`repo`]).
pub async fn host(app: &Arc<App>, id: &str) -> Vec<Line> {
    let all = match app.blocking(|app| Ok(hosts::load(&app.data))).await {
        Ok(h) => h,
        Err(e) => return vec![line("fail", e.to_string())],
    };
    let mut out = Vec::new();
    for e in all.errors.iter().filter(|e| e.starts_with(&format!("host {id}:"))) {
        out.push(line("fail", format!("git.json: {e}")));
    }
    let Some(h) = all.map.get(id) else {
        if out.is_empty() {
            out.push(line("fail", format!("host {id} is not configured in git.json")));
        }
        return out;
    };
    out.push(line("ok", format!("{} at {} over {}", h.kind.as_str(), h.url, if h.ssh { "ssh" } else { "https" })));
    if h.kind == Kind::Plain {
        out.push(line("ok", "a plain git server: transport only (no pull/merge requests, no checks)"));
        return out;
    }
    out.push(line("ok", "a host holds no token: each repository of a project has its own (check the repository)"));
    out
}

/// A repository of a project: reachability, the token's rights, protection of the default branch; with `probe`, a real push of a throw-away branch.
pub async fn repo(app: &Arc<App>, project: &str, name: &str, probe: bool) -> Vec<Line> {
    let (p, n) = (project.to_string(), name.to_string());
    let loaded = app
        .blocking(move |app| {
            let record = app.with_server(|db| db.repo(&p, &n))?;
            let host = store::host_of(app, &record);
            Ok((record, host))
        })
        .await;
    let (record, host) = match loaded {
        Ok(x) => x,
        Err(e) => return vec![line("fail", e.to_string())],
    };
    let mut out = Vec::new();
    match Policy::parse(&record.policy) {
        Ok(_) => out.push(line("ok", "the policy is valid")),
        Err(e) => out.push(line("fail", format!("{e}; until it is fixed agents get no access"))),
    }
    let host = match host {
        Ok(h) => h,
        Err(e) => {
            out.push(line("fail", e));
            return out;
        }
    };
    let own = app.with_server(|db| db.repo_token_info(&record.project, &record.name)).ok().flatten();
    match own {
        Some(i) if i.unreadable => {
            out.push(line("fail", "the repository's own token cannot be read (the server's key changed): enter it again"))
        }
        Some(i) => out.push(line(
            "ok",
            format!("works with the repository's own token{}", if i.hint.is_empty() { String::new() } else { format!(" ({})", i.hint) }),
        )),
        None if host.kind != Kind::Plain => {
            out.push(line("fail", "the repository has no token: set one (the web, or `genie repos set --token-stdin`)"))
        }
        None => {}
    }
    // The server's own copy: can it be fetched from the host?
    let rec = record.clone();
    let synced = app.blocking(move |app| Ok(store::sync(app, &rec))).await;
    let mirror_default = match synced {
        Ok(Ok(info)) => {
            let default = info["defaultBranch"].as_str().map(str::to_string);
            out.push(line(
                "ok",
                format!(
                    "fetched over {}: {} branch(es), default {}",
                    if host.ssh { "ssh" } else { "https" },
                    info["branches"],
                    default.as_deref().unwrap_or("(none yet)")
                ),
            ));
            if let Some(w) = info["warning"].as_str() {
                out.push(line("warn", w));
            }
            default
        }
        Ok(Err(e)) | Err(crate::state::AppError::Internal(e)) => {
            out.push(line("fail", format!("cannot fetch {}: {e}", record.remote)));
            None
        }
        Err(e) => {
            out.push(line("fail", e.to_string()));
            None
        }
    };
    if !record.default_branch.is_empty() && mirror_default.as_deref().is_some_and(|d| d != record.default_branch) {
        out.push(line(
            "warn",
            format!(
                "genie has default branch {}, the host's HEAD is {}",
                record.default_branch,
                mirror_default.clone().unwrap_or_default()
            ),
        ));
    }
    // The host's API: rights and protection.
    if host.kind != Kind::Plain && host.token.is_some() {
        match Api::with_clock(&host, app.clock.clone()) {
            Err(e) => out.push(line("fail", e.to_string())),
            Ok(api) => match api.repo(&record.remote).await {
                Ok(info) => {
                    out.push(line("ok", format!("the host knows {} (default branch {})", record.remote, info.default_branch)));
                    match info.can_push {
                        Some(true) => out.push(line("ok", "the token can push")),
                        Some(false) => out.push(line("warn", "the token cannot push: agents' pushes will be refused by the host")),
                        None => out.push(line("warn", "the host did not say whether the token can push")),
                    }
                    match &info.protected_branches {
                        Some(list) if list.contains(&info.default_branch) => out.push(line("ok", format!("{} is protected on the host", info.default_branch))),
                        Some(_) => out.push(line("warn", format!("{} is not protected on the host: protect it there too (genie's own rules are the first line, the host's the second)", info.default_branch))),
                        None => out.push(line("warn", "the token cannot see branch protection: check it on the host")),
                    }
                }
                Err(ApiError::NotFound(_)) => {
                    out.push(line("fail", format!("the host does not know {} (or the token cannot see it)", record.remote)))
                }
                Err(e) => out.push(line("fail", e.to_string())),
            },
        }
    }
    if probe {
        out.extend(probe_push(app, &record, &host).await);
    }
    out
}

/// Push a throw-away branch to the host and delete it again.
async fn probe_push(app: &Arc<App>, record: &genie_core::repos::ProjectRepo, host: &hosts::Host) -> Vec<Line> {
    let (record, host) = (record.clone(), host.clone());
    let result = app
        .blocking(move |app| {
            let mirror = store::mirror_path(app, &host, &record.remote);
            let Some(default) = store::mirror_default_branch(&mirror) else {
                return Ok(vec![line("warn", "push probe skipped: the repository has no commits yet")]);
            };
            let Some(sha) = store::ref_sha(&mirror, &format!("refs/heads/{default}")) else {
                return Ok(vec![line("warn", "push probe skipped: no default branch in the mirror")]);
            };
            let nonce = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
            let branch = format!("genie-check/{nonce}");
            let mut out = Vec::new();
            match store::push_upstream(&host, &mirror, &sha, &branch, false) {
                Ok(()) => {
                    out.push(line("ok", "the token can push a branch (a throw-away branch was pushed)"));
                    match store::delete_upstream(&host, &mirror, &branch) {
                        Ok(()) => out.push(line("ok", "…and deleted again")),
                        Err(e) => {
                            out.push(line("warn", format!("the probe branch {branch} could not be deleted: remove it by hand ({e})")))
                        }
                    }
                }
                Err(e) => out.push(line("fail", format!("the token cannot push: {e}"))),
            }
            Ok(out)
        })
        .await;
    result.unwrap_or_else(|e| vec![line("fail", e.to_string())])
}
