//! Documentation changesets in the server: edit, publish, discard, review, revert, restore,
//! history, read-your-writes and the doc-merger hand-off.
//!
//! Every change to the vault's `main` goes through [`publish`] under the vault lock: the
//! changeset branch is merged into `main` (a merge commit), validated like an edit, and only
//! then moved onto the checked-out branch and the index. Textual conflicts become a pending
//! merge and a `doc-merger` job instead of an error; the author keeps their changeset.

use std::collections::BTreeMap;

use genie_core::GenieError;
use genie_core::changesets::{
    Changeset, ChangesetFilter, ChangesetOrigin, ChangesetStatus, DocMerge, MergeStatus, NewChangeset, ReviewDecision,
};
use genie_core::doc_ops::{DocOp, apply_ops};
use genie_core::vault::{AuthorKind, DocPage, DocRead, Publish, Vault, parse_doc};
use genie_core::vault_git::{CommitInfo, CommitMeta, FileMerge, Git};
use genie_core::work::NewJob;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::state::{App, AppError, AppResult};

/// Who edits: a person, an agent or the server itself.
#[derive(Debug, Clone)]
pub struct Editor {
    pub name: String,
    pub login: String,
    pub kind: AuthorKind,
    pub task: Option<String>,
    pub project: String,
    pub admin: bool,
    /// The configured role of an agent caller (`doc-merger` may resolve merges).
    pub role: Option<String>,
}

impl Editor {
    fn system(project: &str, task: Option<String>) -> Editor {
        Editor {
            name: "genie".into(),
            login: "genie".into(),
            kind: AuthorKind::System,
            task,
            project: project.into(),
            admin: true,
            role: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Edited {
    pub changeset: Changeset,
    pub commit: String,
    pub content: String,
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum Published {
    Published { changeset: Box<Changeset>, commit: String },
    Merging { changeset: Box<Changeset>, merge: Box<DocMerge> },
}

/// A journal event collected inside a vault closure and emitted once the lock is released.
struct Event {
    path: String,
    kind: &'static str,
    payload: Value,
}

fn kind_str(kind: AuthorKind) -> &'static str {
    match kind {
        AuthorKind::Human => "human",
        AuthorKind::Agent => "agent",
        AuthorKind::System => "system",
    }
}

fn kind_of(text: &str) -> AuthorKind {
    match text {
        "agent" => AuthorKind::Agent,
        "system" => AuthorKind::System,
        _ => AuthorKind::Human,
    }
}

fn git_of(v: &Vault) -> genie_core::Result<Git> {
    Git::open(v.root()).map_err(|_| GenieError::invalid("documentation changesets need the vault to be a git repository (vault.commit)"))
}

fn head(git: &Git) -> genie_core::Result<String> {
    git.resolve("HEAD")?.ok_or_else(|| GenieError::invalid("the vault has no commits yet"))
}

fn branch_ref(cs: &Changeset) -> String {
    format!("refs/heads/{}", cs.branch)
}

fn tip_of(git: &Git, cs: &Changeset) -> genie_core::Result<String> {
    git.resolve(&branch_ref(cs))?.ok_or_else(|| GenieError::not_found(format!("changeset {} has no branch {}", cs.id, cs.branch)))
}

fn utf8(bytes: Vec<u8>) -> genie_core::Result<String> {
    String::from_utf8(bytes).map_err(|_| GenieError::invalid("the page is not valid UTF-8"))
}

fn has_markers(content: &str) -> bool {
    content.lines().any(|line| {
        let line = line.trim_end_matches('\r');
        line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> ") || line == "======="
    })
}

fn locked(v: &Vault, path: &str, kind: AuthorKind) -> genie_core::Result<()> {
    let rule = match kind {
        AuthorKind::Human => v.policy_for(path).humans,
        AuthorKind::Agent => v.policy_for(path).agents,
        AuthorKind::System => Publish::Direct,
    };
    if rule == Publish::Locked {
        return Err(GenieError::Denied(format!("{path} is locked for {}", if kind == AuthorKind::Agent { "agents" } else { "you" })));
    }
    Ok(())
}

fn validate_content(v: &Vault, path: &str, content: &str, kind: AuthorKind) -> genie_core::Result<()> {
    locked(v, path, kind)?;
    let parsed = parse_doc(content, path);
    if !parsed.diagnostics.is_empty() {
        return Err(GenieError::invalid(format!("invalid documentation metadata: {}", parsed.diagnostics.join("; "))));
    }
    if has_markers(content) {
        return Err(GenieError::invalid("the page still has conflict markers (<<<<<<<, ======= or >>>>>>>)"));
    }
    Ok(())
}

fn meta(ed: &Editor, message: &str, cs: &Changeset, extra: &[(&str, String)]) -> CommitMeta {
    let mut trailers = vec![("Genie-Changeset".to_string(), cs.id.to_string())];
    if let Some(task) = &cs.task {
        trailers.push(("Genie-Task".to_string(), task.clone()));
    }
    trailers.push(("Genie-Author-Kind".to_string(), kind_str(ed.kind).to_string()));
    trailers.extend(extra.iter().map(|(k, v)| ((*k).to_string(), v.clone())));
    CommitMeta {
        message: message.trim().to_string(),
        author_name: if ed.name.trim().is_empty() { ed.login.clone() } else { ed.name.clone() },
        author_email: format!("{}@genie.local", ed.login),
        trailers,
    }
}

fn message_or(default: String, note: &str) -> String {
    let note = note.trim();
    if note.is_empty() { default } else { note.to_string() }
}

/// A path in the editor's project roots, as the vault stores it.
fn in_project(v: &Vault, project: &str, path: &str) -> genie_core::Result<String> {
    Ok(v.resolve_for_project(project, path)?.0)
}

fn changeset_in_project(v: &Vault, cs: &Changeset, project: &str) -> genie_core::Result<()> {
    if let Some(p) = &cs.project {
        if p != project {
            return Err(GenieError::not_found(format!("changeset {} not found in this project", cs.id)));
        }
        return Ok(());
    }
    let spaces = v.spaces_of(project);
    if cs.paths.iter().any(|p| spaces.iter().any(|s| p.starts_with(&format!("{s}/")) || p == s)) {
        return Ok(());
    }
    Err(GenieError::not_found(format!("changeset {} not found in this project", cs.id)))
}

fn may_write(cs: &Changeset, ed: &Editor) -> bool {
    cs.author == ed.login || ed.kind == AuthorKind::System || (ed.kind == AuthorKind::Agent && cs.task.is_some() && cs.task == ed.task)
}

fn may_close(cs: &Changeset, ed: &Editor) -> bool {
    may_write(cs, ed) || ed.admin || ed.kind == AuthorKind::System
}

/// The changeset an edit joins, opening one on first use.
fn changeset_for(v: &Vault, app: &App, git: &Git, ed: &Editor, path: &str, note: &str, explicit: Option<i64>) -> AppResult<Changeset> {
    if let Some(id) = explicit {
        let cs = app.with_server(|db| db.changeset(id))?;
        changeset_in_project(v, &cs, &ed.project)?;
        if cs.status != ChangesetStatus::Open {
            return Err(
                GenieError::invalid(format!("changeset #{id} is {}; only an open changeset takes edits", cs.status.as_str())).into()
            );
        }
        if !may_write(&cs, ed) {
            return Err(GenieError::Denied(format!("changeset #{id} belongs to {}", cs.author)).into());
        }
        return Ok(cs);
    }
    let task = ed.task.as_deref();
    let filter_path = task.is_none().then_some(path);
    if let Some(cs) = app
        .with_server(|db| db.open_changeset_for(&ed.login, task, filter_path))?
        .filter(|cs| changeset_in_project(v, cs, &ed.project).is_ok())
    {
        return Ok(cs);
    }
    let main = head(git)?;
    let title = message_or(format!("update {path}"), note);
    let cs = app.with_server(|db| {
        db.create_changeset(&NewChangeset {
            branch: "",
            project: Some(&ed.project),
            task: ed.task.as_deref(),
            author: &ed.login,
            author_name: if ed.name.trim().is_empty() { &ed.login } else { &ed.name },
            author_kind: kind_str(ed.kind),
            title: &title,
            base: &main,
            origin: ChangesetOrigin::Genie,
            reverts: None,
        })
    })?;
    if let Err(e) = git.update_ref(&branch_ref(&cs), &main, None) {
        let _ = app.with_server(|db| db.set_changeset_status(cs.id, ChangesetStatus::Discarded, Some(&ed.login)));
        return Err(e.into());
    }
    Ok(cs)
}

/// Commit `changes` on the changeset branch and record the touched paths.
#[allow(clippy::too_many_arguments)]
fn commit_changes(
    app: &App,
    git: &Git,
    ed: &Editor,
    cs: &Changeset,
    tip: &str,
    changes: Vec<(String, Option<Vec<u8>>)>,
    message: &str,
    paths: &[String],
) -> AppResult<String> {
    let commit = git.commit_tree(tip, &[tip], &changes, &meta(ed, message, cs, &[]))?;
    git.update_ref(&branch_ref(cs), &commit, Some(tip))?;
    app.with_server(|db| db.touch_changeset(cs.id, paths))?;
    Ok(commit)
}

fn emit(app: &App, events: Vec<Event>, ed: &Editor) {
    for e in events {
        crate::knowledge::doc_event(app, &e.path, e.kind, &ed.login, kind_str(ed.kind), e.payload);
    }
}

pub fn edit(app: &App, ed: &Editor, path: &str, ops: &[DocOp], note: &str, changeset: Option<i64>) -> AppResult<Edited> {
    let (edited, events) = app.with_vault(|v| {
        let rel = in_project(v, &ed.project, path)?;
        let git = git_of(v)?;
        locked(v, &rel, ed.kind)?;
        let cs = changeset_for(v, app, &git, ed, &rel, note, changeset)?;
        let tip = tip_of(&git, &cs)?;
        let page = git.show(&tip, &rel)?.map(utf8).transpose()?;
        let content = apply_ops(page.as_deref(), ops)?;
        validate_content(v, &rel, &content, ed.kind)?;
        let message = message_or(format!("update {rel}"), note);
        let commit = commit_changes(
            app,
            &git,
            ed,
            &cs,
            &tip,
            vec![(rel.clone(), Some(content.clone().into_bytes()))],
            &message,
            std::slice::from_ref(&rel),
        )?;
        let cs = app.with_server(|db| db.changeset(cs.id))?;
        let events = vec![Event {
            path: rel.clone(),
            kind: "doc.changeset_edited",
            payload: json!({ "path": rel, "changeset": cs.id, "task": cs.task }),
        }];
        Ok((Edited { changeset: cs, commit, content }, events))
    })?;
    emit(app, events, ed);
    Ok(edited)
}

pub fn remove_page(app: &App, ed: &Editor, path: &str, note: &str, changeset: Option<i64>) -> AppResult<Edited> {
    let (edited, events) = app.with_vault(|v| {
        let rel = in_project(v, &ed.project, path)?;
        let git = git_of(v)?;
        let cs = changeset_for(v, app, &git, ed, &rel, note, changeset)?;
        let tip = tip_of(&git, &cs)?;
        if git.show(&tip, &rel)?.is_none() {
            return Err(GenieError::not_found(format!("documentation page {rel} not found")));
        }
        locked(v, &rel, ed.kind)?;
        let message = message_or(format!("remove {rel}"), note);
        let commit = commit_changes(app, &git, ed, &cs, &tip, vec![(rel.clone(), None)], &message, std::slice::from_ref(&rel))?;
        let cs = app.with_server(|db| db.changeset(cs.id))?;
        let events = vec![Event {
            path: rel.clone(),
            kind: "doc.changeset_edited",
            payload: json!({ "path": rel, "changeset": cs.id, "task": cs.task, "removed": true }),
        }];
        Ok((Edited { changeset: cs, commit, content: String::new() }, events))
    })?;
    emit(app, events, ed);
    Ok(edited)
}

pub fn move_page(app: &App, ed: &Editor, from: &str, to: &str, note: &str, changeset: Option<i64>) -> AppResult<Edited> {
    let (edited, events) = app.with_vault(|v| {
        let from = in_project(v, &ed.project, from)?;
        let to = in_project(v, &ed.project, to)?;
        if from == to {
            return Err(GenieError::invalid("a page cannot be moved onto itself"));
        }
        let git = git_of(v)?;
        let cs = changeset_for(v, app, &git, ed, &from, note, changeset)?;
        let tip = tip_of(&git, &cs)?;
        let content = git.show(&tip, &from)?.ok_or_else(|| GenieError::not_found(format!("documentation page {from} not found")))?;
        if git.show(&tip, &to)?.is_some() {
            return Err(GenieError::invalid(format!("documentation page {to} already exists")));
        }
        locked(v, &from, ed.kind)?;
        locked(v, &to, ed.kind)?;
        let text = utf8(content.clone())?;
        validate_content(v, &to, &text, ed.kind)?;
        let message = message_or(format!("move {from} to {to}"), note);
        let commit = commit_changes(
            app,
            &git,
            ed,
            &cs,
            &tip,
            vec![(from.clone(), None), (to.clone(), Some(content))],
            &message,
            &[from.clone(), to.clone()],
        )?;
        let cs = app.with_server(|db| db.changeset(cs.id))?;
        let events = vec![
            Event {
                path: from.clone(),
                kind: "doc.changeset_edited",
                payload: json!({ "path": from, "moveTo": to, "changeset": cs.id, "task": cs.task }),
            },
            Event {
                path: to.clone(),
                kind: "doc.changeset_edited",
                payload: json!({ "path": to, "moveFrom": from, "changeset": cs.id, "task": cs.task }),
            },
        ];
        Ok((Edited { changeset: cs, commit, content: text }, events))
    })?;
    emit(app, events, ed);
    Ok(edited)
}

/// Record uncommitted external edits on `main` as a published changeset, so history is complete.
fn absorb_external(v: &mut Vault, app: &App, git: &Git) -> genie_core::Result<()> {
    let meta = CommitMeta {
        message: "external edits on the server".into(),
        author_name: "genie".into(),
        author_email: "genie@genie.local".into(),
        trailers: Vec::new(),
    };
    let Some(commit) = git.absorb_worktree(&meta)? else { return Ok(()) };
    let _ = v.refresh();
    let info = git.log(&commit, None, 1)?;
    let parent = info.first().and_then(|i| i.parents.first().cloned()).unwrap_or_else(|| commit.clone());
    let paths = git.changed_paths(&parent, &commit)?;
    if paths.is_empty() {
        return Ok(());
    }
    let short: String = commit.chars().take(12).collect();
    let branch = format!("external/{short}");
    let cs = app.with_server(|db| {
        db.create_changeset(&NewChangeset {
            branch: &branch,
            project: None,
            task: None,
            author: "external",
            author_name: "external",
            author_kind: "system",
            title: "External edits on the server",
            base: &parent,
            origin: ChangesetOrigin::Remote,
            reverts: None,
        })
    })?;
    app.with_server(|db| db.touch_changeset(cs.id, &paths))?;
    app.with_server(|db| db.mark_published(cs.id, &commit, false, "external"))?;
    Ok(())
}

fn review_needed(v: &Vault, app: &App, cs: &Changeset, paths: &[String]) -> AppResult<bool> {
    let kind = kind_of(&cs.author_kind);
    let by_policy = paths.iter().any(|p| match kind {
        AuthorKind::Human => v.policy_for(p).humans == Publish::Review,
        AuthorKind::Agent => v.policy_for(p).agents == Publish::Review,
        AuthorKind::System => false,
    });
    if by_policy {
        return Ok(true);
    }
    let resolved = app.with_server(|db| db.merges(&[MergeStatus::Resolved], 1000))?;
    Ok(resolved.into_iter().find(|m| m.changeset == cs.id).is_some_and(|m| m.contradiction))
}

fn dispatch_merger(app: &App, project: &str, cs: &Changeset, merge: &DocMerge) -> AppResult<DocMerge> {
    let paths = merge.conflicts.join(", ");
    let goal = format!(
        "Resolve the pending documentation merge {} (changeset #{}, pages: {paths}). Follow your role instructions: `genie docs merge-show {}` then `genie docs merge-resolve {} ...`. When one side deleted a page and the other edited it, keep the edited page unless the notes say otherwise.",
        merge.id, cs.id, merge.id, merge.id
    );
    let initiator = crate::llm_key::orchestrator_initiator(app, project);
    // A fresh install has no model for the new built-in role yet: the documenter's model does the job.
    let agents = app.agents();
    let model = agents
        .role_for(project, "doc-merger")
        .ok()
        .and_then(|r| r.model.clone())
        .or_else(|| agents.role_for(project, "documenter").ok().and_then(|r| r.model.clone()));
    let job = app.with_server(|db| {
        db.create_job(NewJob {
            project: project.to_string(),
            task: cs.task.clone(),
            run_step: None,
            role: "doc-merger".into(),
            model,
            goal,
            inputs: json!({ "merge": merge.id, "changeset": cs.id }),
            output_schema: Some(json!({ "note": "one line" })),
            workspace: "none".into(),
            initiator,
        })
    })?;
    app.with_server(|db| db.set_merge_job(merge.id, job.id))?;
    let merge = app.with_server(|db| db.update_merge(merge.id, MergeStatus::Running, None, true))?;
    app.wake_runtime.notify_one();
    Ok(merge)
}

pub fn publish(app: &App, id: i64, by: &Editor) -> AppResult<Published> {
    let (published, events) = app.with_vault(|v| {
        let git = git_of(v)?;
        absorb_external(v, app, &git)?;
        let cs = app.with_server(|db| db.changeset(id))?;
        // The server (sync, migration) publishes across the whole vault; everyone else inside a project.
        let vault_wide = by.kind == AuthorKind::System;
        if !vault_wide {
            changeset_in_project(v, &cs, &by.project)?;
        }
        if !may_close(&cs, by) {
            return Err(GenieError::Denied(format!(
                "only the author, an agent of the same task, an admin or the server publishes changeset #{id}"
            )));
        }
        if !matches!(cs.status, ChangesetStatus::Open | ChangesetStatus::Merging) {
            return Err(GenieError::invalid(format!("changeset #{id} is {}; only an open one is published", cs.status.as_str())));
        }
        let main = head(&git)?;
        let tip = tip_of(&git, &cs)?;
        let base =
            git.merge_base(&main, &tip)?.ok_or_else(|| GenieError::invalid(format!("changeset #{id} and main have no common ancestor")))?;
        let paths = git.changed_paths(&base, &tip)?;
        for path in paths.iter().filter(|_| !vault_wide) {
            if v.project_of(path).as_deref() != Some(by.project.as_str()) {
                return Err(GenieError::not_found(format!("documentation page {path} not found in this project")));
            }
        }
        if paths.is_empty() {
            let cs = app.with_server(|db| db.mark_published(id, &main, false, &by.login))?;
            let events = vec![Event {
                path: cs.paths.first().cloned().unwrap_or_else(|| format!("{}/changelog.md", by.project)),
                kind: "doc.changeset_published",
                payload: json!({ "changeset": id, "task": cs.task, "commit": main }),
            }];
            return Ok((Published::Published { changeset: Box::new(cs), commit: main }, events));
        }
        let merged = git.merge3(&base, &main, &tip, &paths)?;
        let conflicts: Vec<String> =
            merged.iter().filter(|(_, m)| matches!(m, FileMerge::Conflict { .. })).map(|(p, _)| p.clone()).collect();
        if !conflicts.is_empty() {
            app.with_server(|db| db.touch_changeset(id, &paths))?;
            let merge = app.with_server(|db| db.create_merge(id, &main, &tip, &conflicts))?;
            app.with_server(|db| db.set_changeset_status(id, ChangesetStatus::Merging, Some(&by.login)))?;
            // A server publication (sync) has no project of its own: the merger works in the page's.
            let project =
                if by.project.is_empty() { conflicts.iter().find_map(|p| v.project_of(p)).unwrap_or_default() } else { by.project.clone() };
            let merge = dispatch_merger(app, &project, &cs, &merge)?;
            let cs = app.with_server(|db| db.changeset(id))?;
            return Ok((Published::Merging { changeset: Box::new(cs), merge: Box::new(merge) }, Vec::new()));
        }
        let kind = kind_of(&cs.author_kind);
        let mut changes: Vec<(String, Option<Vec<u8>>)> = Vec::with_capacity(merged.len());
        for (path, result) in merged {
            match result {
                FileMerge::Clean(Some(bytes)) => {
                    let content = utf8(bytes)?;
                    validate_content(v, &path, &content, kind)?;
                    changes.push((path, Some(content.into_bytes())));
                }
                FileMerge::Clean(None) => {
                    locked(v, &path, kind)?;
                    changes.push((path, None));
                }
                FileMerge::Conflict { .. } => {
                    return Err(GenieError::invalid(format!("{path} still conflicts")));
                }
            }
        }
        let needed = review_needed(v, app, &cs, &paths)?;
        let title = if cs.title.trim().is_empty() {
            cs.paths.first().map(|p| format!("update {p}")).unwrap_or_else(|| format!("changeset #{id}"))
        } else {
            cs.title.clone()
        };
        let mut extra: Vec<(&str, String)> = Vec::new();
        if let Some(merge) = app.with_server(|db| db.merges(&[MergeStatus::Resolved], 1000))?.into_iter().find(|m| m.changeset == id) {
            let by_merger = merge
                .resolution
                .as_deref()
                .and_then(|sha| git.log(sha, None, 1).ok())
                .and_then(|log| log.into_iter().next())
                .is_some_and(|info| info.trailers.iter().any(|(k, v)| k == "Genie-Merged-By" && v == "doc-merger"));
            if by_merger {
                extra.push(("Genie-Merged-By", "doc-merger".to_string()));
                app.with_server(|db| db.mark_merged_by_agent(id))?;
            }
        }
        let commit =
            git.commit_tree(&main, &[&main, &tip], &changes, &meta(by, &format!("Publish changeset #{id}: {title}"), &cs, &extra))?;
        git.fast_forward(&commit)?;
        v.refresh()?;
        // Changesets made outside the edit path (sync, migration) learn their pages here.
        app.with_server(|db| db.touch_changeset(id, &paths))?;
        let cs = app.with_server(|db| db.mark_published(id, &commit, needed, &by.login))?;
        let mut events: Vec<Event> = paths
            .iter()
            .map(|path| Event {
                path: path.clone(),
                kind: "doc.changed",
                payload: json!({ "path": path, "changeset": id, "task": cs.task }),
            })
            .collect();
        events.push(Event {
            path: paths.first().cloned().unwrap_or_else(|| format!("{}/changelog.md", by.project)),
            kind: "doc.changeset_published",
            payload: json!({ "changeset": id, "task": cs.task, "commit": commit, "reviewNeeded": needed }),
        });
        Ok((Published::Published { changeset: Box::new(cs), commit }, events))
    })?;
    emit(app, events, by);
    if let Published::Published { changeset, commit } = &published
        && let Some(before) = app
            .with_vault(|v| git_of(v)?.log(commit, None, 1))
            .ok()
            .and_then(|l| l.into_iter().next())
            .and_then(|c| c.parents.first().cloned())
    {
        for path in &changeset.paths {
            if crate::context::roles_requiring(app, &by.project, path).is_empty() {
                continue;
            }
            let diff = app.with_vault(|v| git_of(v)?.diff(&before, commit, std::slice::from_ref(path))).unwrap_or_default();
            if !diff.trim().is_empty() {
                crate::context::notify_required_changed(app, &by.project, path, changeset.id, &diff);
            }
        }
    }
    Ok(published)
}

pub fn discard(app: &App, id: i64, by: &Editor) -> AppResult<Changeset> {
    let cs = app.with_server(|db| db.changeset(id))?;
    app.with_vault(|v| changeset_in_project(v, &cs, &by.project))?;
    if !may_close(&cs, by) {
        return Err(GenieError::Denied(format!(
            "only the author, an agent of the same task, an admin or the server discards changeset #{id}"
        ))
        .into());
    }
    if !matches!(cs.status, ChangesetStatus::Open | ChangesetStatus::Merging) {
        return Err(GenieError::invalid(format!("changeset #{id} is {}; only an open one is discarded", cs.status.as_str())).into());
    }
    app.with_server(|db| db.set_changeset_status(id, ChangesetStatus::Discarded, Some(&by.login)))
}

pub fn review(app: &App, id: i64, by: &Editor) -> AppResult<Changeset> {
    let cs = app.with_server(|db| db.changeset(id))?;
    app.with_vault(|v| changeset_in_project(v, &cs, &by.project))?;
    if cs.status != ChangesetStatus::Published {
        return Err(GenieError::invalid(format!("changeset #{id} is {}; only a published one is reviewed", cs.status.as_str())).into());
    }
    if !cs.review_needed {
        return Err(GenieError::invalid(format!("changeset #{id} does not await a review")).into());
    }
    let owners: Vec<String> = app.with_vault(|v| Ok(cs.paths.iter().flat_map(|p| v.owners_for(p)).collect()))?;
    if !by.admin && !owners.contains(&by.login) {
        return Err(GenieError::Denied("only the section owners or admins review a changeset".into()).into());
    }
    app.with_server(|db| db.mark_reviewed(id, &by.login, ReviewDecision::Reviewed))
}

fn is_owner(v: &Vault, cs: &Changeset, by: &Editor) -> bool {
    by.admin || cs.paths.iter().any(|p| v.owners_for(p).contains(&by.login))
}

pub fn revert(app: &App, id: i64, by: &Editor) -> AppResult<Published> {
    let (changeset, reviewer) = app.with_vault(|v| {
        let cs = app.with_server(|db| db.changeset(id))?;
        changeset_in_project(v, &cs, &by.project)?;
        if cs.status != ChangesetStatus::Published {
            return Err(GenieError::invalid(format!("changeset #{id} is {}; only a published one is reverted", cs.status.as_str())));
        }
        let commit = cs.published_commit.clone().ok_or_else(|| GenieError::invalid(format!("changeset #{id} has no published commit")))?;
        let git = git_of(v)?;
        let info = git.log(&commit, None, 1)?;
        let parent = info
            .first()
            .and_then(|i| i.parents.first().cloned())
            .ok_or_else(|| GenieError::invalid(format!("published commit {commit} has no parent")))?;
        let paths = git.changed_paths(&parent, &commit)?;
        for path in &paths {
            if v.project_of(path).as_deref() != Some(by.project.as_str()) {
                return Err(GenieError::not_found(format!("documentation page {path} not found in this project")));
            }
        }
        let title = format!("Revert #{id}");
        let new = app.with_server(|db| {
            db.create_changeset(&NewChangeset {
                branch: "",
                project: cs.project.as_deref().or(Some(by.project.as_str())),
                task: by.task.as_deref(),
                author: &by.login,
                author_name: if by.name.trim().is_empty() { &by.login } else { &by.name },
                author_kind: kind_str(by.kind),
                title: &title,
                base: &commit,
                origin: ChangesetOrigin::Revert,
                reverts: Some(id),
            })
        })?;
        git.update_ref(&branch_ref(&new), &commit, None)?;
        let fork = commit.clone();
        let mut changes: Vec<(String, Option<Vec<u8>>)> = Vec::with_capacity(paths.len());
        for path in &paths {
            changes.push((path.clone(), git.show(&parent, path)?));
        }
        if !changes.is_empty() {
            let message =
                format!("Revert changeset #{id}: {}", if cs.title.trim().is_empty() { "undo the published change" } else { &cs.title });
            let commit = git.commit_tree(&fork, &[&fork], &changes, &meta(by, &message, &new, &[]))?;
            git.update_ref(&branch_ref(&new), &commit, Some(&fork))?;
        }
        let new = app.with_server(|db| db.changeset(new.id))?;
        let reviewer = is_owner(v, &cs, by);
        Ok((new, reviewer))
    })?;
    let published = publish(app, changeset.id, by)?;
    if reviewer && matches!(published, Published::Published { .. }) {
        let _ = app.with_server(|db| db.mark_reviewed(id, &by.login, ReviewDecision::Reverted));
    }
    Ok(published)
}

pub fn restore(app: &App, ed: &Editor, path: &str, at: &str) -> AppResult<Published> {
    let changeset = app.with_vault(|v| {
        let rel = in_project(v, &ed.project, path)?;
        let git = git_of(v)?;
        let main = head(&git)?;
        let commit = git.resolve(at)?.ok_or_else(|| GenieError::not_found(format!("version {at} not found")))?;
        if git.merge_base(&main, &commit)? != Some(commit.clone()) {
            return Err(GenieError::invalid(format!("version {commit} is not on main")));
        }
        let content =
            git.show(&commit, &rel)?.ok_or_else(|| GenieError::not_found(format!("documentation page {rel} not found at {commit}")))?;
        validate_content(v, &rel, &utf8(content.clone())?, ed.kind)?;
        let short: String = commit.chars().take(10).collect();
        let title = format!("Restore {rel} at {short}");
        let new = app.with_server(|db| {
            db.create_changeset(&NewChangeset {
                branch: "",
                project: Some(&ed.project),
                task: ed.task.as_deref(),
                author: &ed.login,
                author_name: if ed.name.trim().is_empty() { &ed.login } else { &ed.name },
                author_kind: kind_str(ed.kind),
                title: &title,
                base: &main,
                origin: ChangesetOrigin::Restore,
                reverts: None,
            })
        })?;
        git.update_ref(&branch_ref(&new), &main, None)?;
        let message = message_or(title.clone(), "");
        let commit = git.commit_tree(&main, &[&main], &[(rel.clone(), Some(content))], &meta(ed, &message, &new, &[]))?;
        git.update_ref(&branch_ref(&new), &commit, Some(&main))?;
        Ok(app.with_server(|db| db.changeset(new.id))?)
    })?;
    publish(app, changeset.id, ed)
}

pub fn history(app: &App, project: &str, path: &str, limit: usize) -> AppResult<Vec<CommitInfo>> {
    app.with_vault(|v| {
        let rel = in_project(v, project, path)?;
        let git = git_of(v)?;
        git.log_full("HEAD", Some(&rel), limit)
    })
}

pub fn diff(app: &App, project: &str, path: &str, from: &str, to: &str) -> AppResult<String> {
    app.with_vault(|v| {
        let rel = in_project(v, project, path)?;
        let git = git_of(v)?;
        git.diff(from, to, &[rel])
    })
}

pub fn changeset_detail(app: &App, project: &str, id: i64) -> AppResult<Value> {
    app.with_vault(|v| {
        let cs = app.with_server(|db| db.changeset(id))?;
        changeset_in_project(v, &cs, project)?;
        let git = git_of(v)?;
        let tip = match tip_of(&git, &cs) {
            Ok(tip) => tip,
            Err(_) => cs.published_commit.clone().ok_or_else(|| GenieError::not_found(format!("changeset {id} has no branch")))?,
        };
        let commits = git.commits_between(&cs.base, &tip)?;
        let commits: Vec<CommitInfo> = commits.iter().filter_map(|sha| git.log(sha, None, 1).ok().and_then(|mut l| l.pop())).collect();
        let diff = git.diff(&cs.base, &tip, &[])?;
        let merge = app.with_server(|db| db.active_merge(cs.id))?;
        Ok(json!({ "changeset": cs, "commits": commits, "diff": diff, "merge": merge }))
    })
}

fn sha(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn fold(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase().replace('ё', "е")
}

/// The section under `heading`, or the whole body when it is None; an unknown heading is not_found.
fn section(text: &str, heading: &str) -> genie_core::Result<String> {
    let lines: Vec<&str> = text.lines().collect();
    let level_of = |l: &str| {
        let t = l.trim_start();
        let n = t.chars().take_while(|c| *c == '#').count();
        (n > 0 && n <= 6 && t[n..].starts_with(' ')).then_some(n)
    };
    let start = lines.iter().position(|l| level_of(l).is_some() && fold(l.trim_start_matches('#').trim()) == fold(heading));
    let Some(start) = start else { return Err(GenieError::not_found(format!("heading {heading} not found"))) };
    let level = level_of(lines[start]).unwrap_or(1);
    let end =
        lines.iter().skip(start + 1).position(|l| level_of(l).is_some_and(|n| n <= level)).map(|i| start + 1 + i).unwrap_or(lines.len());
    Ok(lines[start..end].join("\n"))
}

fn page_value(v: &Vault, project: &str, path: &str, rev: &str, content: &str) -> Value {
    let doc = parse_doc(content, path);
    let body = doc.body.clone();
    let (links, backlinks) = v.links_of(project, path, &doc.links).unwrap_or_default();
    let page = DocPage {
        root_id: "vault".into(),
        path: path.to_string(),
        title: doc.title,
        doc_type: doc.doc_type,
        status: doc.status,
        summary: doc.summary,
        tags: doc.tags,
        aliases: doc.aliases,
        paths: doc.paths,
        related: doc.related,
        verified: doc.verified,
        updated: None,
        content_hash: sha(content),
        headings: doc.headings,
        diagnostics: doc.diagnostics,
        stale: false,
        stale_reasons: Vec::new(),
        space: path.split('/').next().map(str::to_string),
        project: Some(project.to_string()),
    };
    let read = DocRead { page, content: body, truncated: false, heading: None, links, backlinks, policy: v.policy_for(path) };
    let mut value = json!(read);
    if let Some(object) = value.as_object_mut() {
        object.insert("version".into(), json!(rev));
    }
    value
}

#[allow(clippy::too_many_arguments)]
pub fn read_for(
    app: &App,
    ed: &Editor,
    path: &str,
    changeset: Option<&str>,
    at: Option<&str>,
    heading: Option<&str>,
    max_chars: Option<usize>,
) -> AppResult<Value> {
    app.with_vault(|v| {
        let rel = in_project(v, &ed.project, path)?;
        let git = git_of(v)?;
        let main = head(&git)?;
        let mut selected: Option<i64> = None;
        let (rev, indexed) = if let Some(sel) = changeset {
            if sel == "main" {
                (main.clone(), true)
            } else {
                let id: i64 = sel.parse().map_err(|_| GenieError::invalid(format!("changeset must be an id or \"main\", got {sel:?}")))?;
                let cs = app.with_server(|db| db.changeset(id))?;
                changeset_in_project(v, &cs, &ed.project)?;
                selected = Some(id);
                (tip_of(&git, &cs)?, false)
            }
        } else if let Some(at) = at {
            let rev = git.resolve(at)?.ok_or_else(|| GenieError::not_found(format!("version {at} not found")))?;
            (rev.clone(), rev == main)
        } else {
            // Read-your-writes: the reader's changeset for its task, else any open one of theirs touching the page
            // (a reader does not always name its task: a person in the web, an agent outside its team).
            let task = ed.task.as_deref();
            let for_task = app.with_server(|db| db.open_changeset_for(&ed.login, task, task.is_none().then_some(rel.as_str())))?;
            let touches = |cs: &Changeset| cs.paths.iter().any(|p| p == &rel) && changeset_in_project(v, cs, &ed.project).is_ok();
            let found = match for_task.filter(|cs| touches(cs)) {
                Some(cs) => Some(cs),
                None => app
                    .with_server(|db| {
                        db.changesets(&ChangesetFilter {
                            statuses: vec![ChangesetStatus::Open, ChangesetStatus::Merging],
                            author: Some(ed.login.clone()),
                            path: Some(rel.clone()),
                            limit: 10,
                            ..Default::default()
                        })
                    })?
                    .into_iter()
                    .find(|cs| touches(cs)),
            };
            match found {
                Some(cs) => {
                    selected = Some(cs.id);
                    (tip_of(&git, &cs)?, false)
                }
                None => (main.clone(), true),
            }
        };
        let mut value = if indexed && rev == main {
            let read = v.project_read(&ed.project, &rel, heading, max_chars)?;
            json!(read)
        } else {
            let content = git
                .show(&rev, &rel)?
                .map(utf8)
                .transpose()?
                .ok_or_else(|| GenieError::not_found(format!("documentation page {rel} not found at {rev}")))?;
            let doc = parse_doc(&content, &rel);
            let body = match heading {
                Some(h) => section(&doc.body, h)?,
                None => doc.body.clone(),
            };
            let mut body = body;
            let mut truncated = false;
            if let Some(max) = max_chars
                && body.chars().count() > max
            {
                body = body.chars().take(max).collect();
                truncated = true;
            }
            let mut value = page_value(v, &ed.project, &rel, &rev, &content);
            if let Some(object) = value.as_object_mut() {
                object.insert("content".into(), json!(body));
                object.insert("truncated".into(), json!(truncated));
                if let Some(h) = heading {
                    object.insert("heading".into(), json!(h));
                }
            }
            value
        };
        let roots = v.spaces_of(&ed.project);
        let open = app.with_server(|db| {
            db.changesets(&ChangesetFilter {
                project_roots: Some(roots.clone()),
                statuses: vec![ChangesetStatus::Open, ChangesetStatus::Merging],
                path: Some(rel.clone()),
                ..Default::default()
            })
        })?;
        let unreviewed = app.with_server(|db| {
            db.changesets(&ChangesetFilter { project_roots: Some(roots), unreviewed: true, path: Some(rel.clone()), ..Default::default() })
        })?;
        if let Some(object) = value.as_object_mut() {
            object.insert("version".into(), json!(rev));
            object.insert("changeset".into(), json!(selected));
            object.insert(
                "openChangesets".into(),
                json!(
                    open.into_iter()
                        .filter(|cs| cs.author != ed.login)
                        .map(|cs| json!({ "id": cs.id, "author": cs.author, "task": cs.task }))
                        .collect::<Vec<_>>()
                ),
            );
            object.insert(
                "unreviewed".into(),
                json!(unreviewed.into_iter().map(|cs| json!({ "id": cs.id, "author": cs.author })).collect::<Vec<_>>()),
            );
        }
        Ok(value)
    })
}

fn side_notes(git: &Git, from: &str, to: &str, path: &str) -> genie_core::Result<Vec<Value>> {
    let shas = git.commits_between(from, to)?;
    let mut notes = Vec::new();
    for sha in shas {
        let Some(info) = git.log(&sha, None, 1)?.into_iter().next() else { continue };
        if git.diff(from, &sha, &[path.to_string()]).map(|d| d.is_empty()).unwrap_or(true) {
            continue;
        }
        let task = info.trailers.iter().find(|(k, _)| k == "Genie-Task").map(|(_, v)| v.clone());
        notes.push(json!({
            "sha": info.sha,
            "author": info.author_name,
            "email": info.author_email,
            "message": info.subject,
            "body": info.body,
            "task": task,
        }));
    }
    Ok(notes)
}

pub fn merge_detail(app: &App, project: &str, id: i64) -> AppResult<Value> {
    app.with_vault(|v| {
        let merge = app.with_server(|db| db.doc_merge(id))?;
        let cs = app.with_server(|db| db.changeset(merge.changeset))?;
        changeset_in_project(v, &cs, project)?;
        let git = git_of(v)?;
        let base =
            git.merge_base(&merge.main, &merge.tip)?.ok_or_else(|| GenieError::invalid(format!("merge {id} has no common ancestor")))?;
        let paths = git.changed_paths(&base, &merge.tip)?;
        let merged = git.merge3(&base, &merge.main, &merge.tip, &paths)?;
        let mut pages = Vec::new();
        for (path, result) in merged {
            if !merge.conflicts.contains(&path) && !matches!(result, FileMerge::Conflict { .. }) {
                continue;
            }
            let (b, o, t, marked) = match result {
                FileMerge::Clean(_) => (None, None, None, String::new()),
                FileMerge::Conflict { base, ours, theirs, marked } => (
                    base.map(|x| String::from_utf8_lossy(&x).into_owned()),
                    ours.map(|x| String::from_utf8_lossy(&x).into_owned()),
                    theirs.map(|x| String::from_utf8_lossy(&x).into_owned()),
                    String::from_utf8_lossy(&marked).into_owned(),
                ),
            };
            pages.push(json!({
                "path": path,
                "base": b,
                "ours": o,
                "theirs": t,
                "marked": marked,
                "oursNotes": side_notes(&git, &base, &merge.main, &path)?,
                "theirsNotes": side_notes(&git, &base, &merge.tip, &path)?,
            }));
        }
        Ok(json!({ "merge": merge, "changeset": cs, "pages": pages }))
    })
}

fn may_resolve(v: &Vault, cs: &Changeset, merge: &DocMerge, by: &Editor) -> bool {
    if by.role.as_deref() == Some("doc-merger")
        || by.admin
        || by.kind == AuthorKind::System
        || cs.author == by.login
        || (by.kind == AuthorKind::Agent && cs.task.is_some() && cs.task == by.task)
    {
        return true;
    }
    merge.conflicts.iter().any(|p| v.owners_for(p).contains(&by.login))
}

fn notify_users(app: &App, users: Vec<i64>, title: String, body: String, link: Option<String>) {
    if users.is_empty() {
        return;
    }
    let mut users = users;
    users.sort_unstable();
    users.dedup();
    let msg = crate::notify::Message { kind: "vault".into(), title, body, link, ..Default::default() };
    let _ = crate::notify::send(app, &users, &msg, None);
}

fn admin_ids(app: &App) -> Vec<i64> {
    app.with_server(|db| Ok(db.users()?.into_iter().filter(|u| u.is_admin && !u.disabled).map(|u| u.id).collect())).unwrap_or_default()
}

fn owner_ids(app: &App, paths: &[String]) -> Vec<i64> {
    let logins: Vec<String> = app.with_vault(|v| Ok(paths.iter().flat_map(|p| v.owners_for(p)).collect())).unwrap_or_default();
    let mut ids = Vec::new();
    for login in logins {
        if let Ok(Some(user)) = app.with_server(|db| db.user_by_login(&login)) {
            ids.push(user.id);
        }
    }
    ids
}

fn fail_merge(app: &App, merge: &DocMerge, note: &str) -> AppResult<DocMerge> {
    let updated = app.with_server(|db| db.update_merge(merge.id, MergeStatus::Failed, Some(note), false))?;
    let mut to = admin_ids(app);
    to.extend(owner_ids(app, &merge.conflicts));
    notify_users(
        app,
        to,
        "Сведение правок базы знаний не удалось".into(),
        format!("Merge {} (changeset #{}) needs a person: {note}", merge.id, merge.changeset),
        Some("/settings/server/health".into()),
    );
    Ok(updated)
}

#[allow(clippy::too_many_arguments)]
pub fn resolve_merge(
    app: &App,
    id: i64,
    by: &Editor,
    pages: &[(String, Option<String>)],
    done: bool,
    fail: bool,
    contradiction: bool,
    note: &str,
) -> AppResult<Value> {
    let merge = app.with_server(|db| db.doc_merge(id))?;
    let cs = app.with_server(|db| db.changeset(merge.changeset))?;
    app.with_vault(|v| changeset_in_project(v, &cs, &by.project))?;
    if !matches!(merge.status, MergeStatus::Pending | MergeStatus::Running | MergeStatus::Failed) {
        return Err(GenieError::invalid(format!("merge {id} is {}; there is nothing to resolve", merge.status.as_str())).into());
    }
    let allowed = app.with_vault(|v| Ok(may_resolve(v, &cs, &merge, by)))?;
    if !allowed {
        return Err(GenieError::Denied("only the doc-merger, the author, the section owners or admins resolve this merge".into()).into());
    }
    if fail && done {
        return Err(GenieError::invalid("a merge is either resolved or failed, not both").into());
    }
    if fail {
        let merge = fail_merge(app, &merge, note)?;
        return Ok(json!({ "status": "failed", "merge": merge }));
    }
    if !done {
        return Ok(json!({ "status": merge.status.as_str(), "merge": merge }));
    }
    let by_path: BTreeMap<&str, Option<&str>> = pages.iter().map(|(p, c)| (p.as_str(), c.as_deref())).collect();
    for conflict in &merge.conflicts {
        if !by_path.contains_key(conflict.as_str()) {
            return Err(GenieError::invalid(format!("the merge needs a resolution for {conflict}")).into());
        }
    }
    let kind = kind_of(&cs.author_kind);
    let changeset = app.with_vault(|v| {
        let git = git_of(v)?;
        let base =
            git.merge_base(&merge.main, &merge.tip)?.ok_or_else(|| GenieError::invalid(format!("merge {id} has no common ancestor")))?;
        let paths = git.changed_paths(&base, &merge.tip)?;
        let merged = git.merge3(&base, &merge.main, &merge.tip, &paths)?;
        let mut changes: Vec<(String, Option<Vec<u8>>)> = Vec::with_capacity(merged.len());
        for (path, result) in merged {
            let change = match by_path.get(path.as_str()) {
                Some(Some(content)) => {
                    validate_content(v, &path, content, kind)?;
                    Some(content.as_bytes().to_vec())
                }
                Some(None) => {
                    locked(v, &path, kind)?;
                    None
                }
                None => match result {
                    FileMerge::Clean(change) => {
                        if let Some(bytes) = &change {
                            let content = utf8(bytes.clone())?;
                            validate_content(v, &path, &content, kind)?;
                        } else {
                            locked(v, &path, kind)?;
                        }
                        change
                    }
                    FileMerge::Conflict { .. } => {
                        return Err(GenieError::invalid(format!("{path} still conflicts")));
                    }
                },
            };
            changes.push((path, change));
        }
        let message = message_or(format!("Resolve merge #{id} for changeset #{}", cs.id), note);
        let mut extra: Vec<(&str, String)> = Vec::new();
        if by.role.as_deref() == Some("doc-merger") {
            extra.push(("Genie-Merged-By", "doc-merger".to_string()));
        }
        let commit = git.commit_tree(&merge.main, &[&merge.tip, &merge.main], &changes, &meta(by, &message, &cs, &extra))?;
        git.update_ref(&branch_ref(&cs), &commit, Some(&merge.tip))?;
        app.with_server(|db| db.resolve_merge(id, &by.login, &commit, contradiction, note))?;
        app.with_server(|db| db.set_changeset_status(cs.id, ChangesetStatus::Open, None))?;
        Ok(app.with_server(|db| db.changeset(cs.id))?)
    })?;
    let system = Editor::system(&by.project, changeset.task.clone());
    match publish(app, changeset.id, &system)? {
        Published::Published { changeset, commit } => Ok(json!({ "status": "published", "changeset": changeset, "commit": commit })),
        Published::Merging { changeset, merge } => Ok(json!({ "status": "merging", "changeset": changeset, "merge": merge })),
    }
}

/// Retry or fail merges whose doc-merger job ended without resolving, and remind idle authors.
pub fn sweep(app: &App) -> AppResult<()> {
    let running = app.with_server(|db| db.merges(&[MergeStatus::Running], 0))?;
    for merge in running {
        let Some(job_id) = merge.job else { continue };
        let job = match app.with_server(|db| db.job(job_id)) {
            Ok(job) => job,
            Err(_) => continue,
        };
        if !matches!(job.status.as_str(), "succeeded" | "failed" | "cancelled") {
            continue;
        }
        if merge.attempts < 2 {
            let cs = app.with_server(|db| db.changeset(merge.changeset))?;
            let project = match cs.project.clone() {
                Some(p) => p,
                None => app
                    .with_vault(|v| Ok(cs.paths.first().and_then(|p| v.project_of(p))))?
                    .ok_or_else(|| GenieError::not_found("the changeset has no project"))?,
            };
            dispatch_merger(app, &project, &cs, &merge)?;
        } else {
            fail_merge(app, &merge, "the doc-merger job ended without resolving the merge")?;
        }
    }
    let before = (chrono::Utc::now() - chrono::Duration::days(3)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let idle = app.with_server(|db| db.idle_changesets(&before))?;
    for cs in idle {
        let project = match cs.project.clone() {
            Some(p) => Some(p),
            None => app.with_vault(|v| Ok(cs.paths.first().and_then(|p| v.project_of(p))))?,
        };
        let mut to = Vec::new();
        if cs.author_kind == "human" {
            if let Some(user) = app.with_server(|db| db.user_by_login(&cs.author))? {
                to.push(user.id);
            }
        } else if let (Some(project), Some(task)) = (project.as_deref(), cs.task.as_deref())
            && let Ok(t) = app.with_tracker(project, |t| t.get(task))
            && let Some(assignee) = t.assignee
            && let Some(user) = app.with_server(|db| db.user_by_login(&assignee))?
        {
            to.push(user.id);
        }
        if to.is_empty() {
            to = admin_ids(app);
        }
        notify_users(
            app,
            to,
            "Незавершённый набор правок документации".into(),
            format!("Changeset #{} by {} has been idle for three days; publish or discard it.", cs.id, cs.author),
            Some("/docs".into()),
        );
        app.with_server(|db| db.mark_reminded(cs.id))?;
    }
    Ok(())
}

impl From<AppError> for GenieError {
    fn from(e: AppError) -> Self {
        match e {
            AppError::Genie(e) => e,
            AppError::Bad(m) | AppError::Forbidden(m) | AppError::Conflict(m) | AppError::Internal(m) => GenieError::invalid(m),
        }
    }
}

/// One-time migration of open proposals and `.conflict` copies (sync/migration task).
pub fn migrate_legacy(app: &App) -> AppResult<()> {
    crate::changesets_migrate::migrate_legacy(app)
}
