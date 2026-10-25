//! The server's copies of repositories and the agents' working copies.
//!
//! - A **mirror** (`<data>/repos/<host>/<remote>.git`, bare) is fetched from the
//!   host by the server only and shared by every project that uses the repository.
//! - A **workspace** (`<data>/workspaces/<project>/<name>/`) holds a clone of each
//!   repository at its mount path. Clones are made from the mirror (objects are
//!   hard-linked, so an agent cannot damage the mirror and does not need to read it),
//!   and their `origin` is the server's proxy, never the host.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use genie_core::repos::ProjectRepo;

use super::hosts::{self, Host};
use crate::state::App;

pub const ZERO_SHA: &str = "0000000000000000000000000000000000000000";

/// Run `git` and return its trimmed standard output; the error is git's own message.
pub fn run(dir: Option<&Path>, env: &[(String, String)], args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    if let Some(d) = dir {
        cmd.arg("-C").arg(d);
    }
    let out = cmd
        .args(args)
        .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!("git {}: {}", args.first().copied().unwrap_or_default(), if err.is_empty() { "failed".to_string() } else { err }))
    }
}

fn plain(dir: &Path, args: &[&str]) -> Result<String, String> {
    run(Some(dir), &[], args)
}

/// The host a repository lives on, from the current `git.json`, with the repository's own
/// token when it has one (else the host's).
pub fn host_of(app: &App, repo: &ProjectRepo) -> Result<Host, String> {
    let all = hosts::load(&app.data);
    let mut host = all.map.get(&repo.host).cloned().ok_or_else(|| {
        let why = all.errors.iter().find(|e| e.starts_with(&format!("host {}:", repo.host))).cloned();
        why.unwrap_or_else(|| format!("host {} is not configured in git.json (repository {})", repo.host, repo.name))
    })?;
    if repo.connection.is_some() {
        // A connected repository is reached with a token of its connection, never with a stored one.
        host.token = Some(super::tokens::resolve(app, repo, &host)?);
    } else if let Some(token) = app.with_server(|db| db.repo_token(&repo.project, &repo.name)).map_err(|e| e.to_string())? {
        host.token = Some(token);
    }
    Ok(host)
}

pub fn mirror_path(app: &App, host: &Host, remote: &str) -> PathBuf {
    app.data.join("repos").join(&host.id).join(format!("{remote}.git"))
}

/// Create the mirror when it is missing and bring its settings up to date with the host's.
pub fn ensure_mirror(app: &App, host: &Host, remote: &str) -> Result<PathBuf, String> {
    let dir = mirror_path(app, host, remote);
    let lock = app.git.lock(&dir.to_string_lossy());
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    if !dir.join("HEAD").exists() {
        if let Some(parent) = dir.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        run(None, &[], &["init", "--bare", "-q", &dir.to_string_lossy()])?;
        // Agents' pushes are checked by the proxy; git itself refuses the rest.
        plain(&dir, &["config", "receive.denyNonFastForwards", "true"])?;
        plain(&dir, &["config", "receive.denyDeletes", "true"])?;
    }
    plain(&dir, &["config", "remote.origin.url", &host.clone_url(remote)])?;
    plain(&dir, &["config", "--replace-all", "remote.origin.fetch", "+refs/heads/*:refs/heads/*"])?;
    plain(&dir, &["config", "--add", "remote.origin.fetch", "+refs/tags/*:refs/tags/*"])?;
    Ok(dir)
}

/// A mirror brought up to date, with a warning when the host could not be reached (the last state is used).
pub struct Refreshed {
    pub path: PathBuf,
    pub warning: Option<String>,
}

/// Fetch the mirror from the host unless it was fetched less than `max_age` ago.
/// Fails only when there is nothing to serve: no fetch has ever worked.
pub fn refresh(app: &App, host: &Host, remote: &str, max_age: Duration) -> Result<Refreshed, String> {
    let dir = ensure_mirror(app, host, remote)?;
    let key = dir.to_string_lossy().into_owned();
    if app.git.fetched_at(&key).is_some_and(|t| t.elapsed() < max_age) {
        return Ok(Refreshed { path: dir, warning: None });
    }
    let lock = app.git.lock(&key);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    // Somebody else fetched while we waited for the lock.
    if app.git.fetched_at(&key).is_some_and(|t| t.elapsed() < max_age) {
        return Ok(Refreshed { path: dir, warning: None });
    }
    let env = host.git_env();
    match run(Some(&dir), &env, &["fetch", "-q", "--prune", "origin"]) {
        Ok(_) => {
            app.git.mark_fetched(&key);
            // The host's default branch becomes the mirror's HEAD, and so what clones check out.
            if let Ok(out) = run(Some(&dir), &env, &["ls-remote", "--symref", "origin", "HEAD"])
                && let Some(line) = out.lines().find(|l| l.starts_with("ref: "))
                && let Some(target) = line.trim_start_matches("ref: ").split_whitespace().next()
                && target.starts_with("refs/heads/")
                && plain(&dir, &["rev-parse", "--verify", "-q", target]).is_ok()
            {
                let _ = plain(&dir, &["symbolic-ref", "HEAD", target]);
            }
            Ok(Refreshed { path: dir, warning: None })
        }
        Err(e) => {
            let has_refs = plain(&dir, &["for-each-ref", "--count=1"]).is_ok_and(|o| !o.is_empty());
            if has_refs { Ok(Refreshed { path: dir, warning: Some(e) }) } else { Err(e) }
        }
    }
}

/// The branch the mirror's HEAD points at, when it exists.
pub fn mirror_default_branch(mirror: &Path) -> Option<String> {
    let head = plain(mirror, &["symbolic-ref", "-q", "--short", "HEAD"]).ok()?;
    plain(mirror, &["rev-parse", "--verify", "-q", &format!("refs/heads/{head}")]).ok().map(|_| head)
}

/// The repository with its default branch filled in from the mirror when the record has none.
pub fn resolved(app: &App, repo: &ProjectRepo) -> ProjectRepo {
    let mut r = repo.clone();
    if r.default_branch.is_empty()
        && let Ok(host) = host_of(app, repo)
    {
        let m = mirror_path(app, &host, &repo.remote);
        if m.join("HEAD").exists()
            && let Some(b) = mirror_default_branch(&m)
        {
            r.default_branch = b;
        }
    }
    r
}

pub fn branch_exists(dir: &Path, branch: &str) -> bool {
    plain(dir, &["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")]).is_ok()
}

pub fn ref_sha(dir: &Path, refname: &str) -> Option<String> {
    plain(dir, &["rev-parse", "--verify", "-q", refname]).ok().filter(|s| !s.is_empty())
}

// --- the proxy's side of a push ----------------------------------------------------------

/// Send a branch of the mirror to the host: a fast-forward, or a forced update when `force`.
pub fn push_upstream(host: &Host, mirror: &Path, sha: &str, branch: &str, force: bool) -> Result<(), String> {
    let spec = format!("{}{sha}:refs/heads/{branch}", if force { "+" } else { "" });
    run(Some(mirror), &host.git_env(), &["push", "-q", "origin", &spec]).map(|_| ())
}

/// Delete a branch on the host.
pub fn delete_upstream(host: &Host, mirror: &Path, branch: &str) -> Result<(), String> {
    run(Some(mirror), &host.git_env(), &["push", "-q", "origin", &format!(":refs/heads/{branch}")]).map(|_| ())
}

/// Put a ref of the mirror back (`old` all zeros: delete it).
pub fn restore_ref(mirror: &Path, refname: &str, old: &str) {
    let _ = if old == ZERO_SHA || old.is_empty() {
        plain(mirror, &["update-ref", "-d", refname])
    } else {
        plain(mirror, &["update-ref", refname, old])
    };
}

// --- workspaces -------------------------------------------------------------------------

/// The address of a repository on the server's proxy.
pub fn proxy_url(app: &App, project: &str, repo: &str) -> String {
    format!("http://127.0.0.1:{}/git/{project}/{repo}.git", app.cfg.port)
}

pub fn workspace_root(app: &App, project: &str, name: &str) -> PathBuf {
    app.data.join("workspaces").join(project).join(name)
}

/// A repository placed in a workspace.
#[derive(Debug, Clone)]
pub struct Placed {
    pub repo: ProjectRepo,
    pub path: PathBuf,
    /// The task's branch checked out here (`None`: the default branch, read-only use).
    pub branch: Option<String>,
}

/// What to place: a repository and the branch to work on (`None`: its default branch).
pub struct Want {
    pub repo: ProjectRepo,
    pub branch: Option<String>,
}

/// Make (or reuse) a workspace holding the repositories at their mounts. Idempotent: an
/// existing clone is kept as it is (the agent's commits stay), its branch is switched to the wanted one.
/// With `view` the clones are read-only views that follow the default branch of the mirror.
pub fn assemble(app: &App, project: &str, name: &str, wants: Vec<Want>, view: bool) -> Result<(PathBuf, Vec<Placed>), String> {
    let root = workspace_root(app, project, name);
    std::fs::create_dir_all(&root).map_err(|e| format!("{}: {e}", root.display()))?;
    let mut wants = wants;
    wants.sort_by(|a, b| (a.repo.mount != ".").cmp(&(b.repo.mount != ".")).then(a.repo.mount.cmp(&b.repo.mount)));
    let mounts: Vec<String> = wants.iter().map(|w| w.repo.mount.clone()).filter(|m| m != ".").collect();
    let mut placed = Vec::new();
    let key = root.to_string_lossy().into_owned();
    let lock = app.git.lock(&key);
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    for w in wants {
        let host = host_of(app, &w.repo)?;
        let mirror = match refresh(app, &host, &w.repo.remote, Duration::from_secs(30)) {
            Ok(r) => {
                if let Some(warn) = r.warning {
                    eprintln!("genie git: {}: {}: using the last fetched state: {warn}", project, w.repo.name);
                }
                r.path
            }
            Err(e) => return Err(format!("repository {}: {e}", w.repo.name)),
        };
        let repo = resolved(app, &w.repo);
        let dir = if repo.mount == "." { root.clone() } else { root.join(&repo.mount) };
        if !dir.join(".git").exists() {
            if let Some(parent) = dir.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            let mut args = vec!["clone", "-q"];
            let default = mirror_default_branch(&mirror);
            if let Some(d) = default.as_deref() {
                args.extend(["-b", d]);
            }
            let (m, d) = (mirror.to_string_lossy().into_owned(), dir.to_string_lossy().into_owned());
            args.extend([m.as_str(), d.as_str()]);
            run(None, &[], &args).map_err(|e| format!("repository {}: {e}", repo.name))?;
        }
        configure(app, project, &host, &repo, &dir)?;
        if repo.mount == "." && !mounts.is_empty() {
            exclude(&dir, &mounts);
        }
        if view {
            sync_view(&mirror, &dir, &repo)?;
        } else if let Some(b) = &w.branch {
            switch_branch(&dir, b).map_err(|e| format!("repository {}: {e}", repo.name))?;
        }
        placed.push(Placed { repo, path: dir, branch: if view { None } else { w.branch } });
    }
    Ok((root, placed))
}

/// The settings of a clone: the proxy as `origin`, how to authenticate to it, the commit identity.
fn configure(app: &App, project: &str, host: &Host, repo: &ProjectRepo, dir: &Path) -> Result<(), String> {
    let url = proxy_url(app, project, &repo.name);
    plain(dir, &["remote", "set-url", "origin", &url])?;
    // Only the genie token of the agent goes to the proxy; the host's token never reaches a clone.
    let base = format!("http://127.0.0.1:{}", app.cfg.port);
    let helper = "!f() { test \"$1\" = get || exit 0; echo username=genie; echo \"password=$GENIE_TOKEN\"; }; f";
    plain(dir, &["config", &format!("credential.{base}.helper"), helper])?;
    plain(dir, &["config", "push.autoSetupRemote", "true"])?;
    plain(dir, &["config", "advice.detachedHead", "false"])?;
    if let Some(n) = &host.identity.name {
        plain(dir, &["config", "user.name", n])?;
    }
    if let Some(e) = &host.identity.email {
        plain(dir, &["config", "user.email", e])?;
    }
    Ok(())
}

/// Keep nested repositories out of the root repository's status.
fn exclude(dir: &Path, mounts: &[String]) {
    let file = dir.join(".git/info/exclude");
    let have = std::fs::read_to_string(&file).unwrap_or_default();
    let mut out = have.clone();
    for m in mounts {
        let line = format!("/{m}/");
        if !have.lines().any(|l| l == line) {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&line);
            out.push('\n');
        }
    }
    if out != have {
        let _ = std::fs::create_dir_all(dir.join(".git/info"));
        let _ = std::fs::write(&file, out);
    }
}

/// Check out the task's branch: the existing local one, the host's, or a new one from where the clone is.
fn switch_branch(dir: &Path, branch: &str) -> Result<(), String> {
    if branch_exists(dir, branch) {
        plain(dir, &["checkout", "-q", branch]).map(|_| ())
    } else if ref_sha(dir, &format!("refs/remotes/origin/{branch}")).is_some() {
        plain(dir, &["checkout", "-q", "-b", branch, "--track", &format!("origin/{branch}")]).map(|_| ())
    } else {
        plain(dir, &["checkout", "-q", "-b", branch]).map(|_| ())
    }
}

/// A read-only view follows the mirror's default branch (local edits are discarded: it is not a place to work).
fn sync_view(mirror: &Path, dir: &Path, repo: &ProjectRepo) -> Result<(), String> {
    let default = if repo.default_branch.is_empty() { mirror_default_branch(mirror) } else { Some(repo.default_branch.clone()) };
    let Some(default) = default else { return Ok(()) };
    let m = mirror.to_string_lossy().into_owned();
    plain(dir, &["fetch", "-q", &m, "+refs/heads/*:refs/remotes/origin/*"])?;
    if ref_sha(dir, &format!("refs/remotes/origin/{default}")).is_some() {
        plain(dir, &["checkout", "-q", "-f", "-B", &default, &format!("origin/{default}")])?;
        let _ = plain(dir, &["clean", "-fdq"]);
    }
    Ok(())
}

/// Bring a repository's mirror up to date now, record its default branch, and report how it stands.
pub fn sync(app: &App, repo: &ProjectRepo) -> Result<serde_json::Value, String> {
    let host = host_of(app, repo)?;
    let r = refresh(app, &host, &repo.remote, Duration::ZERO)?;
    let default = mirror_default_branch(&r.path);
    let branches = plain(&r.path, &["for-each-ref", "--format=%(refname:short)", "refs/heads"]).map(|s| s.lines().count()).unwrap_or(0);
    Ok(serde_json::json!({
        "repo": repo.name,
        "defaultBranch": default,
        "branches": branches,
        "warning": r.warning,
    }))
}
