//! The vault in sync with a git remote (`vault.remote`), so people can work on
//! it in Obsidian (with its git plugin, or plain git) while genie, the web and the
//! agents write it on the server.
//!
//! Every `vault.syncSecs` the server fetches the remote, merges it into the vault
//! and pushes its own commits. Edits of different lines merge by themselves. An
//! edit is never lost: where the same lines changed on both sides, the page keeps
//! the server's version and the remote's goes next to it as
//! `<page>.conflict-<time>.md`; a page deleted on one side and edited on the other
//! stays, edited. The section owners and the server's admins hear about it. The
//! per-user files of Obsidian (open tabs, cache, trash) stay out of the repository.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use genie_core::GenieError;
use serde::Serialize;

use crate::state::{App, AppResult};

/// Personal state of Obsidian that must not travel between people.
const IGNORED: &[&str] = &[
    ".obsidian/workspace.json",
    ".obsidian/workspace-mobile.json",
    ".obsidian/workspaces.json",
    ".obsidian/cache",
    ".trash/",
    ".DS_Store",
];

/// How the last sync went, for the web and `genie doctor`.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncState {
    pub remote: String,
    pub branch: String,
    /// When the last sync finished, successful or not.
    pub at: Option<String>,
    pub ok: bool,
    pub error: Option<String>,
    /// Commits brought in and sent out by the last sync.
    pub pulled: usize,
    pub pushed: usize,
    /// Pages changed on both sides since the previous sync.
    pub both: Vec<String>,
    /// Edits that overlapped, and where the other version went (for people).
    pub conflicts: Vec<String>,
    /// Pages the conflicts are about (their section owners are told).
    #[serde(skip)]
    pub conflict_pages: Vec<String>,
}

static STATE: Mutex<Option<SyncState>> = Mutex::new(None);

pub fn state() -> Option<SyncState> {
    STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let out =
        Command::new("git").arg("-C").arg(root).args(args).env("GIT_TERMINAL_PROMPT", "0").output().map_err(|e| format!("git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(format!("git {}: {}", args.first().copied().unwrap_or_default(), if err.is_empty() { "failed".into() } else { err }))
    }
}

/// The remote's name in the vault: a configured name, or `origin` pointed at a URL.
fn remote_name(root: &Path, remote: &str) -> Result<String, String> {
    let url_like = remote.contains("://") || remote.contains('@') || remote.starts_with('/') || remote.starts_with('.');
    if !url_like {
        git(root, &["remote", "get-url", remote]).map_err(|_| format!("the vault has no git remote named {remote}"))?;
        return Ok(remote.to_string());
    }
    match git(root, &["remote", "get-url", "origin"]) {
        Ok(url) if url == remote => {}
        Ok(_) => {
            git(root, &["remote", "set-url", "origin", remote])?;
        }
        Err(_) => {
            git(root, &["remote", "add", "origin", remote])?;
        }
    }
    Ok("origin".into())
}

/// Keep Obsidian's personal files out of the repository (`.gitignore`, committed).
fn ignore_personal_files(root: &Path) -> Result<(), String> {
    let file = root.join(".gitignore");
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    let have: BTreeSet<&str> = text.lines().map(str::trim).collect();
    let missing: Vec<&str> = IGNORED.iter().copied().filter(|p| !have.contains(p)).collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut out = text.clone();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    if text.is_empty() {
        out.push_str("# Personal state of Obsidian: open tabs, cache, trash\n");
    }
    for p in &missing {
        out.push_str(p);
        out.push('\n');
    }
    std::fs::write(&file, out).map_err(|e| format!(".gitignore: {e}"))?;
    // Files tracked before they were ignored leave the index (they stay on disk).
    for p in &missing {
        let _ = git(root, &["rm", "-r", "-q", "--cached", "--ignore-unmatch", "--", p.trim_end_matches('/')]);
    }
    git(root, &["add", ".gitignore"])?;
    git(
        root,
        &[
            "-c",
            "user.name=genie",
            "-c",
            "user.email=genie@genie.local",
            "commit",
            "-q",
            "-m",
            "genie: keep Obsidian's personal files out of the vault",
        ],
    )
    .map(|_| ())
}

fn changed_between(root: &Path, from: &str, to: &str) -> BTreeSet<String> {
    git(root, &["diff", "--name-only", &format!("{from}..{to}")]).map(|s| s.lines().map(str::to_string).collect()).unwrap_or_default()
}

/// One sync: fetch, merge, push. `Ok(None)` when no remote is configured.
pub fn sync(app: &App) -> AppResult<Option<SyncState>> {
    let Some(remote) = app.cfg.vault.remote.clone().filter(|r| !r.trim().is_empty()) else {
        return Ok(None);
    };
    let root = app.with_vault(|v| Ok(v.root().to_path_buf()))?;
    let mut st = SyncState { remote: remote.clone(), ..Default::default() };
    let result = run(app, &root, &remote, &mut st);
    st.at = Some(chrono::Utc::now().to_rfc3339());
    st.ok = result.is_ok();
    st.error = result.err();
    *STATE.lock().unwrap_or_else(|e| e.into_inner()) = Some(st.clone());
    tell(app, &st);
    Ok(Some(st))
}

fn run(app: &App, root: &Path, remote: &str, st: &mut SyncState) -> Result<(), String> {
    if !root.join(".git").exists() {
        return Err("the vault is not a git repository (vault.commit is off?)".into());
    }
    let name = remote_name(root, remote)?;
    let branch = match &app.cfg.vault.branch {
        Some(b) if !b.trim().is_empty() => b.clone(),
        _ => git(root, &["symbolic-ref", "--short", "HEAD"]).unwrap_or_else(|_| "main".into()),
    };
    st.branch = branch.clone();
    // Fetching only moves the remote's refs: no need to hold the vault meanwhile.
    let fetched = git(root, &["fetch", "-q", &name, &branch]);
    let theirs = format!("{name}/{branch}");
    let remote_has_branch = fetched.is_ok() && git(root, &["rev-parse", "--verify", "-q", &theirs]).is_ok();
    if let Err(e) = &fetched
        && !e.contains("couldn't find remote ref")
    {
        return Err(e.clone());
    }
    // Merging touches the files: writes of people and agents wait for it.
    app.with_vault(|_| {
        ignore_personal_files(root).map_err(GenieError::invalid)?;
        if !remote_has_branch {
            return Ok(());
        }
        let counts = git(root, &["rev-list", "--left-right", "--count", &format!("HEAD...{theirs}")]).map_err(GenieError::invalid)?;
        let behind: usize = counts.split_whitespace().nth(1).and_then(|n| n.parse().ok()).unwrap_or(0);
        if behind == 0 {
            return Ok(());
        }
        st.pulled = behind;
        if let Ok(base) = git(root, &["merge-base", "HEAD", &theirs]) {
            let ours = changed_between(root, &base, "HEAD");
            st.both = changed_between(root, &base, &theirs).intersection(&ours).cloned().collect();
        }
        let merged = git(
            root,
            &[
                "-c",
                "user.name=genie",
                "-c",
                "user.email=genie@genie.local",
                "merge",
                "-q",
                "--no-edit",
                "--allow-unrelated-histories",
                "-m",
                &format!("genie: merge {theirs} into the vault"),
                &theirs,
            ],
        );
        if let Err(e) = merged {
            let unmerged = git(root, &["diff", "--name-only", "--diff-filter=U"]).unwrap_or_default();
            if unmerged.is_empty() {
                let _ = git(root, &["merge", "--abort"]);
                return Err(GenieError::invalid(e));
            }
            if let Err(e) = keep_both(root, &unmerged, st) {
                let _ = git(root, &["merge", "--abort"]);
                return Err(GenieError::invalid(e));
            }
        }
        Ok(())
    })
    .map_err(|e| e.to_string())?;
    let ahead = if remote_has_branch {
        git(root, &["rev-list", "--count", &format!("{theirs}..HEAD")]).ok().and_then(|n| n.parse().ok()).unwrap_or(0)
    } else {
        git(root, &["rev-list", "--count", "HEAD"]).ok().and_then(|n| n.parse().ok()).unwrap_or(0)
    };
    if ahead > 0 {
        git(root, &["push", "-q", &name, &format!("HEAD:refs/heads/{branch}")])?;
        st.pushed = ahead;
    }
    Ok(())
}

/// A file of the merge's index stage (`2` ours, `3` theirs), if that side has one.
fn staged(root: &Path, stage: u8, path: &str) -> Option<Vec<u8>> {
    let out = Command::new("git").arg("-C").arg(root).args(["show", &format!(":{stage}:{path}")]).output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// `shop/notes.md` → `shop/notes.conflict-20260929-1530.md`.
fn conflict_copy(path: &str, stamp: &str) -> String {
    let (dir, name) = path.rsplit_once('/').map(|(d, n)| (format!("{d}/"), n)).unwrap_or((String::new(), path));
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{dir}{stem}.conflict-{stamp}.{ext}"),
        _ => format!("{dir}{name}.conflict-{stamp}"),
    }
}

/// Settle a merge git could not: nothing is lost. Both edited → the server's
/// version stays, the remote's goes next to it; deleted on one side, edited on
/// the other → the edited page stays. Then the merge is committed.
fn keep_both(root: &Path, unmerged: &str, st: &mut SyncState) -> Result<(), String> {
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M").to_string();
    let put = |rel: &str, bytes: &[u8]| -> Result<(), String> {
        let abs = root.join(rel);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&abs, bytes).map_err(|e| format!("{rel}: {e}"))?;
        git(root, &["add", "--", rel]).map(|_| ())
    };
    for path in unmerged.lines().filter(|l| !l.is_empty()) {
        match (staged(root, 2, path), staged(root, 3, path)) {
            (Some(ours), Some(theirs)) => {
                let copy = conflict_copy(path, &stamp);
                put(path, &ours)?;
                put(&copy, &theirs)?;
                st.conflicts
                    .push(format!("{path}: правили и там, и здесь — осталась версия сервера, версия из репозитория сохранена в {copy}"));
            }
            (Some(ours), None) => {
                put(path, &ours)?;
                st.conflicts.push(format!("{path}: удалена в репозитории, но изменена на сервере — страница оставлена"));
            }
            (None, Some(theirs)) => {
                put(path, &theirs)?;
                st.conflicts.push(format!("{path}: удалена на сервере, но изменена в репозитории — страница возвращена"));
            }
            (None, None) => {
                let _ = git(root, &["rm", "-q", "--cached", "--ignore-unmatch", "--", path]);
            }
        }
        st.conflict_pages.push(path.to_string());
    }
    git(root, &["-c", "user.name=genie", "-c", "user.email=genie@genie.local", "commit", "-q", "--no-edit"]).map(|_| ())
}

/// The server's admins, who hear about the vault.
fn admins(app: &App) -> Vec<i64> {
    app.with_server(|db| Ok(db.users()?.into_iter().filter(|u| u.is_admin && !u.disabled).map(|u| u.id).collect())).unwrap_or_default()
}

fn tell(app: &App, st: &SyncState) {
    let users = admins(app);
    let msg = |title: String, body: String| crate::notify::Message {
        kind: "vault".into(),
        title,
        body,
        link: Some("/settings/server/health".into()),
        ..Default::default()
    };
    if let Some(e) = &st.error
        && !users.is_empty()
    {
        // The same failure is told once a day, not every couple of minutes.
        let key = format!("vault-sync:{}:{:x}", chrono::Utc::now().format("%Y-%m-%d"), fingerprint(e));
        let _ = crate::notify::send(
            app,
            &users,
            &msg(
                "Синхронизация базы знаний не удалась".into(),
                format!(
                    "{e}\n\nРепозиторий: {} ({}). Сервер продолжает работать со своей копией и повторит попытку.",
                    st.remote, st.branch
                ),
            ),
            Some(&key),
        );
    } else if !st.conflicts.is_empty() {
        // The owners of the sections the pages are in, besides the admins.
        let mut to: BTreeSet<i64> = users.into_iter().collect();
        let owners: BTreeSet<String> =
            app.with_vault(|v| Ok(st.conflict_pages.iter().flat_map(|p| v.owners_for(p)).collect())).unwrap_or_default();
        for login in owners {
            if let Ok(Some(u)) = app.with_server(|db| db.user_by_login(&login)) {
                to.insert(u.id);
            }
        }
        let _ = crate::notify::send(
            app,
            &to.into_iter().collect::<Vec<_>>(),
            &msg(
                "Правки базы знаний пересеклись".into(),
                format!(
                    "Одни и те же страницы изменили и в репозитории ({}), и на сервере. Ничего не потеряно:\n\n{}\n\nСведите версии и удалите копию .conflict.",
                    st.remote,
                    st.conflicts.join("\n")
                ),
            ),
            None,
        );
    }
}

/// A short stable fingerprint of a text (FNV-1a, for dedupe keys).
fn fingerprint(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3))
}

/// Sync every `vault.syncSecs` (default 120) while the server runs.
pub fn start(app: &Arc<App>) {
    if app.cfg.vault.remote.as_deref().is_none_or(|r| r.trim().is_empty()) {
        return;
    }
    let every = Duration::from_secs(app.cfg.vault.sync_secs.unwrap_or(120).max(10));
    let app = app.clone();
    tokio::spawn(async move {
        loop {
            let res = app
                .blocking(|app| {
                    if let Some(e) = sync(app)?.and_then(|st| st.error) {
                        eprintln!("genie vault sync: {e}");
                    }
                    Ok(())
                })
                .await;
            if let Err(e) = res {
                eprintln!("genie vault sync: {e}");
            }
            tokio::time::sleep(every).await;
        }
    });
}
