//! One-time migration of the retired knowledge workflows onto documentation changesets.
//!
//! Open knowledge proposals become open changesets forked from the newest `main` commit whose
//! page content still has the proposal's `base_hash`, so their authors can publish or discard
//! them in the Changes feed. Leftover `<page>.conflict-<stamp>.md` copies become changesets
//! that put their text back on the page (through the merge agent when it overlaps) and then
//! leave `main`. Both halves are guarded by what they consume: a migrated proposal is no longer
//! open, and a migrated copy is gone from `main`, so a second start does nothing.

use std::path::Path;
use std::process::Command;

use genie_core::GenieError;
use genie_core::changesets::{ChangesetFilter, ChangesetOrigin, ChangesetStatus, NewChangeset};
use genie_core::vault::AuthorKind;
use genie_core::vault_git::{CommitMeta, Git};
use genie_core::work::Proposal;
use sha2::{Digest, Sha256};

use crate::changesets::{Editor, Published};
use crate::state::{App, AppError, AppResult};

const SYSTEM_NAME: &str = "genie";
const SYSTEM_EMAIL: &str = "genie@genie.local";

/// Move open proposals and conflict copies onto changesets. Called once at server start;
/// idempotent, and errors leave the rest for the next start.
pub fn migrate_legacy(app: &App) -> AppResult<()> {
    let root = app.with_vault(|v| Ok(v.root().to_path_buf()))?;
    if !root.join(".git").exists() {
        return Ok(());
    }
    let proposals = app.with_server(|db| db.proposals(Some("open"), 1000))?;
    let copies = conflict_copies(&root).map_err(GenieError::invalid)?;
    if proposals.is_empty() && copies.is_empty() {
        return Ok(());
    }

    let mut errors: Vec<AppError> = Vec::new();
    for proposal in proposals {
        if let Err(e) = migrate_proposal(app, &root, &proposal) {
            eprintln!("genie docs migration: proposal #{}: {e}", proposal.id);
            errors.push(e);
        }
    }

    // A copy whose page could not be handed to a changeset stays where it is.
    let mut removable: Vec<String> = Vec::new();
    for (copy, original) in copies {
        match migrate_copy(app, &root, &copy, &original) {
            Ok(()) => removable.push(copy),
            Err(e) => {
                eprintln!("genie docs migration: {copy}: {e}");
                errors.push(e);
            }
        }
    }
    if !removable.is_empty()
        && let Err(e) = remove_copies(app, &root, &removable)
    {
        eprintln!("genie docs migration: removing the migrated conflict copies: {e}");
        errors.push(e);
    }

    match errors.into_iter().next() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

fn commit_meta(name: &str, email: &str, message: &str, trailers: Vec<(String, String)>) -> CommitMeta {
    CommitMeta { message: message.into(), author_name: name.into(), author_email: email.into(), trailers }
}

fn sha_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The git author of a login: the person's name and email when the account is known.
fn author_of(app: &App, login: &str) -> (String, String) {
    match app.with_server(|db| db.user_by_login(login)) {
        Ok(Some(user)) => {
            let email = user.email.filter(|e| !e.trim().is_empty()).unwrap_or_else(|| format!("{}@genie.local", user.login));
            (user.name, email)
        }
        _ => (login.to_string(), format!("{}@genie.local", login.trim())),
    }
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

fn git_out(root: &Path, args: &[&str]) -> Result<String, String> {
    let out =
        Command::new("git").arg("-C").arg(root).args(args).env("GIT_TERMINAL_PROMPT", "0").output().map_err(|e| format!("git: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!("git {}: {}", args.first().copied().unwrap_or_default(), if err.is_empty() { "failed".into() } else { err }));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The newest `main` commit whose version of `path` has the proposal's `base_hash`; `main` itself
/// when none does (also when the proposal carries no hash).
fn base_for_hash(vgit: &Git, main: &str, path: &str, hash: Option<&str>) -> Result<String, String> {
    let Some(hash) = hash.map(str::trim).filter(|h| !h.is_empty()) else { return Ok(main.to_string()) };
    for commit in vgit.log(main, Some(path), 200).map_err(|e| e.to_string())? {
        if let Some(bytes) = vgit.show(&commit.sha, path).map_err(|e| e.to_string())?
            && sha_hex(&bytes) == hash
        {
            return Ok(commit.sha);
        }
    }
    Ok(main.to_string())
}

/// Fork an open proposal's content onto a changeset branch and close the proposal. A changeset
/// left over from an earlier, interrupted run is reused instead of opened twice.
fn migrate_proposal(app: &App, root: &Path, proposal: &Proposal) -> AppResult<()> {
    let vgit = Git::open(root)?;
    let main = vgit.resolve("HEAD")?.ok_or_else(|| GenieError::invalid("the vault has no commits"))?;
    let existing = app.with_server(|db| {
        db.changesets(&ChangesetFilter {
            statuses: vec![ChangesetStatus::Open],
            path: Some(proposal.path.clone()),
            limit: 0,
            ..Default::default()
        })
    })?;
    if let Some(cs) = existing.into_iter().find(|c| c.origin == ChangesetOrigin::Migration && c.author == proposal.author) {
        close_proposal(app, proposal.id, cs.id)?;
        return Ok(());
    }

    let base = base_for_hash(&vgit, &main, &proposal.path, proposal.base_hash.as_deref()).map_err(GenieError::invalid)?;
    let (author_name, author_email) = author_of(app, &proposal.author);
    let title = format!("Migrate proposal #{} to {}", proposal.id, proposal.path);
    let cs = app.with_server(|db| {
        db.create_changeset(&NewChangeset {
            branch: "",
            project: proposal.project.as_deref(),
            task: proposal.task.as_deref(),
            author: &proposal.author,
            author_name: &author_name,
            author_kind: &proposal.author_kind,
            title: &title,
            base: &base,
            origin: ChangesetOrigin::Migration,
            reverts: None,
        })
    })?;

    let mut trailers =
        vec![("Genie-Changeset".to_string(), cs.id.to_string()), ("Genie-Author-Kind".to_string(), proposal.author_kind.clone())];
    if let Some(task) = &proposal.task {
        trailers.push(("Genie-Task".to_string(), task.clone()));
    }
    let message = if proposal.note.trim().is_empty() {
        format!("Migrate proposal #{} for {}", proposal.id, proposal.path)
    } else {
        proposal.note.clone()
    };
    let changes = vec![(proposal.path.clone(), Some(proposal.content.clone().into_bytes()))];
    let commit = vgit.commit_tree(&base, &[&base], &changes, &commit_meta(&author_name, &author_email, &message, trailers))?;
    vgit.update_ref(&format!("refs/heads/{}", cs.branch), &commit, None)?;
    app.with_server(|db| db.touch_changeset(cs.id, std::slice::from_ref(&proposal.path)))?;
    close_proposal(app, proposal.id, cs.id)?;
    eprintln!("genie docs migration: proposal #{} → changeset #{} ({})", proposal.id, cs.id, proposal.path);
    Ok(())
}

fn close_proposal(app: &App, id: i64, changeset: i64) -> AppResult<()> {
    app.with_server(|db| db.decide_proposal(id, "migrated", "genie", Some(&format!("changeset #{changeset}"))))?;
    Ok(())
}

/// `shop/notes.md` → `shop/notes.conflict-20260929-1530.md`, and back.
fn original_of(copy: &str) -> Option<String> {
    let (dir, name) = copy.rsplit_once('/').map(|(d, n)| (format!("{d}/"), n)).unwrap_or((String::new(), copy));
    if let Some((stem, ext)) = name.rsplit_once('.')
        && let Some((original, stamp)) = stem.rsplit_once(".conflict-")
        && !original.is_empty()
        && is_stamp(stamp)
    {
        return Some(format!("{dir}{original}.{ext}"));
    }
    let (original, stamp) = name.rsplit_once(".conflict-")?;
    (!original.is_empty() && is_stamp(stamp)).then(|| format!("{dir}{original}"))
}

/// `YYYYMMDD-HHMM`, the stamp the old sync put on conflict copies.
fn is_stamp(stamp: &str) -> bool {
    let b = stamp.as_bytes();
    b.len() == 13 && b[8] == b'-' && b[..8].iter().all(u8::is_ascii_digit) && b[9..].iter().all(u8::is_ascii_digit)
}

/// Conflict copies on `main`, with the page each one belongs to.
fn conflict_copies(root: &Path) -> Result<Vec<(String, String)>, String> {
    let listing = git_out(root, &["ls-tree", "-r", "-z", "--name-only", "HEAD"])?;
    Ok(listing.split('\0').filter(|copy| !copy.is_empty()).filter_map(|copy| original_of(copy).map(|o| (copy.to_string(), o))).collect())
}

/// Publish a copy's text as the page. `Ok` means the copy may leave `main`: its content is on
/// `main` or in a merge the merge agent is settling.
fn migrate_copy(app: &App, root: &Path, copy: &str, original: &str) -> AppResult<()> {
    let vgit = Git::open(root)?;
    let added = git_out(root, &["log", "--diff-filter=A", "--format=%H", "--", copy]).map_err(GenieError::invalid)?;
    let added = added
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .ok_or_else(|| GenieError::invalid(format!("{copy}: no commit added it to the vault")))?;
    let parent = git_out(root, &["rev-parse", &format!("{added}^")])
        .map_err(|_| GenieError::invalid(format!("{copy}: the commit that added it has no parent")))?;
    let parent = parent.trim().to_string();
    let Some(content) = vgit.show("HEAD", copy)? else {
        return Err(GenieError::invalid(format!("{copy}: not in the vault's history")).into());
    };
    let project = app.with_vault(|v| Ok(v.project_of(original)))?;
    let title = format!("Restore {original} from its conflict copy");
    let cs = app.with_server(|db| {
        db.create_changeset(&NewChangeset {
            branch: "",
            project: project.as_deref(),
            task: None,
            author: SYSTEM_NAME,
            author_name: SYSTEM_NAME,
            author_kind: "system",
            title: &title,
            base: &parent,
            origin: ChangesetOrigin::Migration,
            reverts: None,
        })
    })?;
    let trailers = vec![("Genie-Changeset".to_string(), cs.id.to_string()), ("Genie-Author-Kind".to_string(), "system".to_string())];
    let changes = vec![(original.to_string(), Some(content))];
    let commit = vgit.commit_tree(
        &parent,
        &[&parent],
        &changes,
        &commit_meta(SYSTEM_NAME, SYSTEM_EMAIL, &format!("Restore {original} from {copy}"), trailers),
    )?;
    vgit.update_ref(&format!("refs/heads/{}", cs.branch), &commit, None)?;
    app.with_server(|db| db.touch_changeset(cs.id, &[original.to_string()]))?;

    let by = system_editor(project.as_deref());
    match crate::changesets::publish(app, cs.id, &by)? {
        Published::Published { .. } => {}
        Published::Merging { merge, .. } => {
            eprintln!("genie docs migration: {copy}: merge #{} is settling the overlap", merge.id);
        }
    }
    eprintln!("genie docs migration: {copy} → changeset #{} ({original})", cs.id);
    Ok(())
}

/// Remove the copies whose content now lives on `main`, as one system changeset.
fn remove_copies(app: &App, root: &Path, copies: &[String]) -> AppResult<()> {
    let vgit = Git::open(root)?;
    let main = vgit.resolve("HEAD")?.ok_or_else(|| GenieError::invalid("the vault has no commits"))?;
    let title = "Remove migrated conflict copies";
    let cs = app.with_server(|db| {
        db.create_changeset(&NewChangeset {
            branch: "",
            project: None,
            task: None,
            author: SYSTEM_NAME,
            author_name: SYSTEM_NAME,
            author_kind: "system",
            title,
            base: &main,
            origin: ChangesetOrigin::Migration,
            reverts: None,
        })
    })?;
    let trailers = vec![("Genie-Changeset".to_string(), cs.id.to_string()), ("Genie-Author-Kind".to_string(), "system".to_string())];
    let changes: Vec<(String, Option<Vec<u8>>)> = copies.iter().map(|copy| (copy.clone(), None)).collect();
    let commit = vgit.commit_tree(&main, &[&main], &changes, &commit_meta(SYSTEM_NAME, SYSTEM_EMAIL, title, trailers))?;
    vgit.update_ref(&format!("refs/heads/{}", cs.branch), &commit, None)?;
    app.with_server(|db| db.touch_changeset(cs.id, copies))?;
    let by = system_editor(None);
    crate::changesets::publish(app, cs.id, &by)?;
    eprintln!("genie docs migration: removed {} conflict copies from main", copies.len());
    Ok(())
}
