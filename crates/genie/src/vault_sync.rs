//! The vault in sync with a git remote (`vault.remote`), so people can work on
//! it in Obsidian (with its git plugin, or plain git) while genie, the web and the
//! agents write it on the server.
//!
//! Every `vault.syncSecs` the server fetches the remote and turns what it finds
//! into changesets. Commits pushed to main become one changeset per person and
//! sync run, published as the system: it merges by itself when the pages do not
//! overlap, otherwise the merge agent settles the overlap like for any other
//! changeset (no `.conflict` copies are written). Branches under `changes/` that
//! people push are registered as open changesets; the branches of changesets the
//! server closed are removed from the remote. The per-user files of Obsidian
//! (open tabs, cache, trash) stay out of the repository.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use genie_core::GenieError;
use genie_core::changesets::{Changeset, ChangesetFilter, ChangesetOrigin, ChangesetStatus, NewChangeset};
use genie_core::vault::AuthorKind;
use genie_core::vault_git::{CommitMeta, Git};
use serde::Serialize;

use crate::changesets::{Editor, Published};
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

/// The author of commits the server makes by itself.
const SYSTEM_NAME: &str = "genie";
const SYSTEM_EMAIL: &str = "genie@genie.local";

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
    /// Overlaps the merge agent is resolving, and pushes the remote refused (for people).
    pub conflicts: Vec<String>,
    /// Pages the conflicts are about.
    #[serde(skip)]
    pub conflict_pages: Vec<String>,
    /// Merges the vault is waiting for; main is not pushed while one is open.
    pub merging: Vec<i64>,
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

/// One sync: fetch, import, publish, push. `Ok(None)` when no remote is configured.
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

/// What the vault locked its worktree for: the remote's tip to import.
struct Incoming {
    /// The remote's commit the changeset's branch stands on.
    theirs: String,
    base: String,
    paths: Vec<String>,
    both: Vec<String>,
    commits: usize,
    author_login: String,
    author_name: String,
}

fn commit_meta(author: &str, email: &str, message: &str, trailers: Vec<(String, String)>) -> CommitMeta {
    CommitMeta { message: message.into(), author_name: author.into(), author_email: email.into(), trailers }
}

/// `<email>@...` → the login; the author's name when the email has no local part.
fn login_of(name: &str, email: &str) -> String {
    let login = email.split('@').next().unwrap_or_default().trim();
    if login.is_empty() { name.trim().to_string() } else { login.to_string() }
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
    let theirs = format!("{name}/{branch}");
    let changes = format!("refs/remotes/{name}/changes/");

    // Fetching only moves the remote's refs: no need to hold the vault meanwhile.
    let missing_branch = |e: &str| e.contains("couldn't find remote ref");
    let mut fetch_error = None;
    match git(root, &["fetch", "-q", &name, &branch]) {
        Ok(_) => {}
        Err(e) if missing_branch(&e) => {}
        Err(e) => fetch_error = Some(e),
    }
    if let Err(e) = git(root, &["fetch", "-q", &name, &format!("+refs/heads/changes/*:{changes}*")])
        && !missing_branch(&e)
    {
        fetch_error.get_or_insert(e);
    }
    if let Some(e) = fetch_error {
        return Err(e);
    }
    let remote_has_branch = git(root, &["rev-parse", "--verify", "-q", &theirs]).is_ok();

    // Absorbing touches the files: writes of people and agents wait for it.
    let incoming = app
        .with_vault(|_| {
            ignore_personal_files(root).map_err(GenieError::invalid)?;
            let vgit = Git::open(root)?;
            vgit.absorb_worktree(&commit_meta(SYSTEM_NAME, SYSTEM_EMAIL, "genie: save the vault's uncommitted work", Vec::new()))?;
            if !remote_has_branch {
                return Ok(None);
            }
            let main = vgit.resolve("HEAD")?.ok_or_else(|| GenieError::invalid("the vault has no commits"))?;
            let Some(their_tip) = vgit.resolve(&theirs)? else { return Ok(None) };
            let commits = vgit.commits_between(&main, &their_tip)?;
            if commits.is_empty() {
                return Ok(None);
            }
            let base = match vgit.merge_base(&main, &their_tip)? {
                Some(base) => base,
                None => root_commit(root, &their_tip).map_err(GenieError::invalid)?,
            };
            let paths = vgit.changed_paths(&main, &their_tip)?;
            let mut both = Vec::new();
            if base != main {
                let ours = vgit.changed_paths(&base, &main)?;
                let remote_paths = vgit.changed_paths(&base, &their_tip)?;
                both = ours.into_iter().filter(|p| remote_paths.contains(p)).collect();
            }
            let tip = vgit.log(&their_tip, None, 1)?.into_iter().next();
            let (author_name, author_email) = tip.map(|c| (c.author_name, c.author_email)).unwrap_or_default();
            Ok(Some(Incoming {
                theirs: their_tip,
                base,
                paths,
                both,
                commits: commits.len(),
                author_login: login_of(&author_name, &author_email),
                author_name,
            }))
        })
        .map_err(|e| e.to_string())?;

    if let Some(incoming) = incoming {
        st.pulled = incoming.commits;
        st.both = incoming.both.clone();
        let title = sync_title(remote);
        match sync_changeset(app, &title)? {
            Some(c) if c.status == ChangesetStatus::Merging => {
                // Already handed to the merge agent: no second import while it works.
                st.conflicts.push(format!("{}: the sync changeset #{} waits for its merge", c.branch, c.id));
                wait_for_merge(app, c.id, st)?;
            }
            Some(c) => {
                move_branch(root, &c.branch, &incoming.theirs)?;
                publish_sync(app, c.id, st)?;
            }
            None => {
                let id = open_sync_changeset(app, root, remote, &incoming)?;
                publish_sync(app, id, st)?;
            }
        }
    }

    register_remote_changesets(app, root, &name)?;
    push(app, root, &name, &branch, &theirs, st)?;
    delete_closed_branches(app, root, &name, &changes)?;
    Ok(())
}

fn sync_title(remote: &str) -> String {
    format!("Sync from {remote}")
}

/// The open or merging changeset the last sync opened for this remote.
fn sync_changeset(app: &App, title: &str) -> Result<Option<Changeset>, String> {
    let list = app
        .with_server(|db| {
            db.changesets(&ChangesetFilter {
                statuses: vec![ChangesetStatus::Open, ChangesetStatus::Merging],
                limit: 500,
                ..Default::default()
            })
        })
        .map_err(|e| e.to_string())?;
    Ok(list.into_iter().find(|c| c.origin == ChangesetOrigin::Remote && c.title == title))
}

/// The first parent of every history: git's `rev-list --max-parents=0`.
fn root_commit(root: &Path, rev: &str) -> Result<String, String> {
    let out =
        Command::new("git").arg("-C").arg(root).args(["rev-list", "--max-parents=0", rev]).output().map_err(|e| format!("git: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_string();
    if !out.status.success() || first.is_empty() {
        return Err(format!("git rev-list --max-parents=0 {rev}: no root commit"));
    }
    Ok(first)
}

/// Open the changeset that imports the remote's main commits; its branch stands on their tip.
fn open_sync_changeset(app: &App, root: &Path, remote: &str, incoming: &Incoming) -> Result<i64, String> {
    let title = sync_title(remote);
    let cs = app
        .with_server(|db| {
            db.create_changeset(&NewChangeset {
                branch: "",
                project: None,
                task: None,
                author: &incoming.author_login,
                author_name: &incoming.author_name,
                author_kind: "human",
                title: &title,
                base: &incoming.base,
                origin: ChangesetOrigin::Remote,
                reverts: None,
            })
        })
        .map_err(|e| e.to_string())?;
    move_branch(root, &cs.branch, &incoming.theirs)?;
    if !incoming.paths.is_empty() {
        app.with_server(|db| db.touch_changeset(cs.id, &incoming.paths)).map_err(|e| e.to_string())?;
    }
    Ok(cs.id)
}

/// Point a changeset branch at `sha`, whatever it held before (the sync owns it).
fn move_branch(root: &Path, branch: &str, sha: &str) -> Result<(), String> {
    let vgit = Git::open(root).map_err(|e| e.to_string())?;
    let refname = format!("refs/heads/{branch}");
    let current = vgit.resolve(&refname).map_err(|e| e.to_string())?;
    if current.as_deref() == Some(sha) {
        return Ok(());
    }
    vgit.update_ref(&refname, sha, current.as_deref()).map_err(|e| e.to_string())
}

/// The system publishes the sync changeset; a textual overlap becomes a pending merge.
fn publish_sync(app: &App, id: i64, st: &mut SyncState) -> Result<(), String> {
    let cs = app.with_server(|db| db.changeset(id)).map_err(|e| e.to_string())?;
    let by = system_editor(cs.project.as_deref());
    match crate::changesets::publish(app, id, &by).map_err(|e| e.to_string())? {
        Published::Published { .. } => Ok(()),
        Published::Merging { merge, .. } => {
            st.merging.push(merge.id);
            for page in &merge.conflicts {
                st.conflicts.push(format!("{page}: overlapping edits; the merge agent is resolving merge {}", merge.id));
                st.conflict_pages.push(page.clone());
            }
            Ok(())
        }
    }
}

/// Remember the merge a changeset already waits for, so main is not pushed meanwhile.
fn wait_for_merge(app: &App, changeset: i64, st: &mut SyncState) -> Result<(), String> {
    let merge = app.with_server(|db| db.active_merge(changeset)).map_err(|e| e.to_string())?;
    let Some(merge) = merge else { return Ok(()) };
    st.merging.push(merge.id);
    for page in merge.conflicts {
        st.conflicts.push(format!("{page}: overlapping edits; the merge agent is resolving merge {}", merge.id));
        st.conflict_pages.push(page);
    }
    Ok(())
}

fn system_editor(project: Option<&str>) -> Editor {
    Editor {
        name: SYSTEM_NAME.into(),
        login: SYSTEM_NAME.into(),
        kind: AuthorKind::System,
        task: None,
        project: project.unwrap_or_default().to_string(),
        admin: true,
        role: None,
    }
}

/// Branches under `changes/` on the remote that no changeset of this server owns become
/// open changesets; branches of known changesets fast-forward to their remote tip.
fn register_remote_changesets(app: &App, root: &Path, name: &str) -> Result<(), String> {
    let vgit = Git::open(root).map_err(|e| e.to_string())?;
    let prefix = format!("refs/remotes/{name}/changes/");
    let refs = vgit.list_refs(&prefix).map_err(|e| e.to_string())?;
    if refs.is_empty() {
        return Ok(());
    }
    let main = vgit.resolve("HEAD").map_err(|e| e.to_string())?.ok_or("the vault has no commits")?;
    for (refname, sha) in refs {
        let Some(short) = refname.strip_prefix(&prefix) else { continue };
        if short.trim().is_empty() {
            continue;
        }
        let branch = format!("changes/{short}");
        let local = format!("refs/heads/{branch}");
        let known = app.with_server(|db| db.changeset_by_branch(&branch)).map_err(|e| e.to_string())?;
        match known {
            Some(c) => {
                if matches!(c.status, ChangesetStatus::Open | ChangesetStatus::Merging) {
                    fast_forward(&vgit, &local, &sha)?;
                }
            }
            None => {
                let (base, paths) = match vgit.merge_base(&main, &sha).map_err(|e| e.to_string())? {
                    Some(base) => (base.clone(), vgit.changed_paths(&base, &sha).map_err(|e| e.to_string())?),
                    // No common history: the branch brings its whole tree.
                    None => (root_commit(root, &sha)?, vgit.changed_paths(&main, &sha).map_err(|e| e.to_string())?),
                };
                let tip = vgit.log(&sha, None, 1).map_err(|e| e.to_string())?.into_iter().next();
                let (author_name, author_email) = tip.map(|c| (c.author_name, c.author_email)).unwrap_or_default();
                let login = login_of(&author_name, &author_email);
                let title = format!("Changes from {login}");
                let cs = app
                    .with_server(|db| {
                        db.create_changeset(&NewChangeset {
                            branch: &branch,
                            project: None,
                            task: None,
                            author: &login,
                            author_name: &author_name,
                            author_kind: "human",
                            title: &title,
                            base: &base,
                            origin: ChangesetOrigin::Remote,
                            reverts: None,
                        })
                    })
                    .map_err(|e| e.to_string())?;
                if !paths.is_empty() {
                    app.with_server(|db| db.touch_changeset(cs.id, &paths)).map_err(|e| e.to_string())?;
                }
                fast_forward(&vgit, &local, &sha)?;
            }
        }
    }
    Ok(())
}

/// Move a changeset branch forward to the remote's tip; a diverged branch stays as it is.
fn fast_forward(vgit: &Git, refname: &str, sha: &str) -> Result<(), String> {
    let current = vgit.resolve(refname).map_err(|e| e.to_string())?;
    if current.as_deref() == Some(sha) {
        return Ok(());
    }
    if let Some(old) = &current {
        let ancestor = vgit.merge_base(old, sha).map_err(|e| e.to_string())?;
        if ancestor.as_deref() != Some(old.as_str()) {
            return Ok(());
        }
    }
    vgit.update_ref(refname, sha, current.as_deref()).map_err(|e| e.to_string())
}

/// Push main when it is ahead, then the open branches this server owns, then remove the
/// branches of changesets it closed. A refused branch is reported, not fatal.
fn push(app: &App, root: &Path, name: &str, branch: &str, theirs: &str, st: &mut SyncState) -> Result<(), String> {
    if st.merging.is_empty() {
        let ahead = if git(root, &["rev-parse", "--verify", "-q", theirs]).is_ok() {
            git(root, &["rev-list", "--count", &format!("{theirs}..HEAD")]).ok().and_then(|n| n.parse().ok()).unwrap_or(0)
        } else {
            git(root, &["rev-list", "--count", "HEAD"]).ok().and_then(|n| n.parse().ok()).unwrap_or(0)
        };
        if ahead > 0 {
            git(root, &["push", "-q", name, &format!("HEAD:refs/heads/{branch}")])?;
            st.pushed = ahead;
        }
    }
    let open = app
        .with_server(|db| db.changesets(&ChangesetFilter { statuses: vec![ChangesetStatus::Open], limit: 500, ..Default::default() }))
        .map_err(|e| e.to_string())?;
    for c in open.iter().filter(|c| c.branch == format!("changes/{}", c.id)) {
        let refspec = format!("refs/heads/{0}:refs/heads/{0}", c.branch);
        if let Err(e) = git(root, &["push", "-q", name, &refspec]) {
            st.conflicts.push(format!("{}: the branch was not pushed ({})", c.branch, e));
        }
    }
    Ok(())
}

/// Branches of changesets this server published or discarded leave the remote.
fn delete_closed_branches(app: &App, root: &Path, name: &str, prefix: &str) -> Result<(), String> {
    let vgit = Git::open(root).map_err(|e| e.to_string())?;
    for (refname, _) in vgit.list_refs(prefix).map_err(|e| e.to_string())? {
        let Some(short) = refname.strip_prefix(prefix) else { continue };
        let branch = format!("changes/{short}");
        let closed = app
            .with_server(|db| {
                Ok(db
                    .changeset_by_branch(&branch)?
                    .is_some_and(|c| matches!(c.status, ChangesetStatus::Published | ChangesetStatus::Discarded)))
            })
            .map_err(|e| e.to_string())?;
        if closed {
            let _ = git(root, &["push", "-q", name, "--delete", &branch]);
        }
    }
    Ok(())
}

/// The server's admins, who hear about the vault.
fn admins(app: &App) -> Vec<i64> {
    app.with_server(|db| Ok(db.users()?.into_iter().filter(|u| u.is_admin && !u.disabled).map(|u| u.id).collect())).unwrap_or_default()
}

/// Only failures are told: an overlap is the merge agent's work, not a person's.
fn tell(app: &App, st: &SyncState) {
    let users = admins(app);
    if let Some(e) = &st.error
        && !users.is_empty()
    {
        // The same failure is told once a day, not every couple of minutes.
        let key = format!("vault-sync:{}:{:x}", chrono::Utc::now().format("%Y-%m-%d"), fingerprint(e));
        let _ = crate::notify::send(
            app,
            &users,
            &crate::notify::Message {
                kind: "vault".into(),
                title: "Синхронизация базы знаний не удалась".into(),
                body: format!(
                    "{e}\n\nРепозиторий: {} ({}). Сервер продолжает работать со своей копией и повторит попытку.",
                    st.remote, st.branch
                ),
                link: Some("/settings/server/health".into()),
                ..Default::default()
            },
            Some(&key),
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
