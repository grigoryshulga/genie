//! Knowledge vault in the server: one write path for people, agents and
//! automations (direct write, proposal for review, or refusal by the section's
//! policy), journal events for projects, and a watcher that picks up edits made
//! in Obsidian or by git pulls.

use std::sync::Arc;
use std::time::Duration;

use genie_core::vault::{AuthorKind, WriteOutcome};
use genie_core::work::Proposal;
use serde_json::{Value, json};

use crate::state::{App, AppResult};

pub enum DocWrite {
    Saved { created: bool, hash: String },
    Proposed(Box<Proposal>),
}

pub struct Author<'a> {
    /// Display name (git author) and login/agent name.
    pub name: &'a str,
    pub login: &'a str,
    pub kind: AuthorKind,
}

/// Append an event to the journal of the project that owns a page (if any).
pub fn doc_event(app: &App, path: &str, kind: &str, actor: &str, actor_role: &str, payload: Value) {
    let project = app.with_vault(|v| Ok(v.project_of(path))).ok().flatten();
    if let Some(p) = project {
        let _ = app.with_tracker(&p, |t| t.append_event(kind, None, actor, actor_role, payload.clone()).map(|_| ()));
        app.wake_engine.notify_one();
    }
}

/// Write a page on behalf of a person or an agent, honouring the section policy.
#[allow(clippy::too_many_arguments)]
pub fn write_doc(
    app: &App,
    path: &str,
    content: &str,
    author: Author<'_>,
    base_hash: Option<&str>,
    note: &str,
    task: Option<&str>,
) -> AppResult<DocWrite> {
    let message = if note.trim().is_empty() { format!("{}: update {path}", author.login) } else { note.trim().to_string() };
    let outcome = app.with_vault(|v| v.write(path, content, author.kind, (author.name, author.login), base_hash, &message))?;
    let role = match author.kind {
        AuthorKind::Human => "human",
        AuthorKind::Agent => "agent",
        AuthorKind::System => "system",
    };
    match outcome {
        WriteOutcome::Saved { created, hash } => {
            let rel = app.with_vault(|v| Ok(v.resolve(path)?.0))?;
            doc_event(app, &rel, "doc.changed", author.login, role, json!({ "path": rel, "created": created, "task": task }));
            Ok(DocWrite::Saved { created, hash })
        }
        WriteOutcome::NeedsReview => {
            let (rel, project, base) = app.with_vault(|v| {
                let rel = v.resolve(path)?.0;
                Ok((rel.clone(), v.project_of(&rel), v.current_hash(&rel)?))
            })?;
            let base = base_hash.map(str::to_string).or(base).or(Some(String::new()));
            let proposal = app.with_server(|db| {
                // A newer proposal from the same author for the same page replaces the open one.
                for old in db.proposals(Some("open"), 500)?.into_iter().filter(|p| p.path == rel && p.author == author.login) {
                    db.decide_proposal(old.id, "superseded", author.login, Some("replaced by a newer proposal"))?;
                }
                db.create_proposal(&rel, base.as_deref(), content, author.login, role, project.as_deref(), task, note)
            })?;
            doc_event(app, &rel, "doc.proposal", author.login, role, json!({ "path": rel, "proposal": proposal.id, "task": task }));
            Ok(DocWrite::Proposed(Box::new(proposal)))
        }
    }
}

/// A file name from a title: its letters and digits, a dash between words, at most 60 characters.
pub fn slug(title: &str) -> String {
    let mut out = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out: String = out.chars().take(60).collect();
    let out = out.trim_matches('-');
    if out.is_empty() { "note".into() } else { out.to_string() }
}

/// A draft note (`genie docs note`): frontmatter, then the body.
pub fn note_page(title: &str, body: &str, tags: &[String], related: &[String]) -> String {
    let quoted = |s: &str| serde_json::to_string(s).unwrap_or_default();
    let list = |items: &[String]| format!("[{}]", items.iter().map(|s| quoted(s)).collect::<Vec<_>>().join(", "));
    let mut lines = vec!["---".to_string(), format!("title: {}", quoted(title)), "type: note".into(), "status: draft".into()];
    if !tags.is_empty() {
        lines.push(format!("tags: {}", list(tags)));
    }
    if !related.is_empty() {
        lines.push(format!("related: {}", list(related)));
    }
    lines.push("---".into());
    let body = body.trim();
    format!("{}\n\n{}", lines.join("\n"), if body.is_empty() { String::new() } else { format!("{body}\n") })
}

/// Apply or reject a proposal. Applying writes as the proposal's author.
pub fn decide(app: &App, id: i64, approve: bool, by: &str, note: Option<&str>, force: bool) -> AppResult<Proposal> {
    let p = app.with_server(|db| db.proposal(id))?;
    if p.status != "open" {
        return Err(genie_core::GenieError::invalid(format!("proposal {id} is {}", p.status)).into());
    }
    if approve {
        let base = if force { None } else { p.base_hash.as_deref() };
        let msg =
            format!("{} (proposal #{id}, approved by {by})", if p.note.is_empty() { format!("update {}", p.path) } else { p.note.clone() });
        app.with_vault(|v| v.write(&p.path, &p.content, AuthorKind::System, (&p.author, &p.author), base, &msg))?;
        doc_event(app, &p.path, "doc.changed", by, "human", json!({ "path": p.path, "proposal": id, "task": p.task }));
    }
    let decided = app.with_server(|db| db.decide_proposal(id, if approve { "approved" } else { "rejected" }, by, note))?;
    doc_event(app, &p.path, "doc.proposal_decided", by, "human", json!({ "path": p.path, "proposal": id, "approved": approve }));
    Ok(decided)
}

const WATCH_MIN: Duration = Duration::from_secs(4);
const WATCH_MAX: Duration = Duration::from_secs(15);

/// Pick up edits made outside genie (Obsidian, git pull). Nothing tells the server about them, so
/// the vault is looked at: every few seconds while it changes, backing off to [`WATCH_MAX`] while
/// it is quiet (a walk over every page is not free). Edits through genie index themselves.
pub fn start_watcher(app: &Arc<App>) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut last = String::new();
        let mut wait = WATCH_MIN;
        loop {
            tokio::time::sleep(wait).await;
            let prev = last.clone();
            let res = app
                .blocking(move |app| {
                    let sig = app.with_vault(|v| Ok(v.signature()))?;
                    if sig == prev {
                        return Ok((sig, Vec::new()));
                    }
                    let changed = app.with_vault(|v| {
                        let _ = v.reload_config();
                        v.refresh()
                    })?;
                    Ok((sig, changed))
                })
                .await;
            match res {
                Ok((sig, changed)) => {
                    if !last.is_empty() {
                        let app2 = app.clone();
                        let _ = app
                            .blocking(move |_| {
                                for path in changed {
                                    doc_event(&app2, &path, "doc.changed", "vault", "system", json!({ "path": path, "external": true }));
                                }
                                Ok(())
                            })
                            .await;
                    }
                    wait = if sig == last { (wait * 2).min(WATCH_MAX) } else { WATCH_MIN };
                    last = sig;
                }
                Err(e) => eprintln!("genie vault: {e}"),
            }
        }
    });
}

/// Release a project's changelog: `Unreleased` becomes the version; the journal
/// gets a `release.published` event (notifications and automations react to it).
pub fn release(app: &App, project: &str, version: &str, by: &str) -> AppResult<String> {
    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let notes = app.with_vault(|v| {
        let space =
            v.spaces_of(project).into_iter().next().ok_or_else(|| genie_core::GenieError::not_found("the project has no vault space"))?;
        v.changelog_release(&space, version, &date)
    })?;
    app.with_tracker(project, |t| {
        t.append_event("release.published", None, by, "human", json!({ "version": version, "date": date, "notes": notes })).map(|_| ())
    })?;
    app.wake_engine.notify_one();
    Ok(notes)
}

/// Add a changelog entry to a project's space.
pub fn changelog_add(app: &App, project: &str, group: &str, text: &str, task: Option<&str>) -> AppResult<String> {
    let russian = app.cfg.language.user.to_lowercase().starts_with("rus");
    app.with_vault(|v| {
        let space =
            v.spaces_of(project).into_iter().next().ok_or_else(|| genie_core::GenieError::not_found("the project has no vault space"))?;
        v.changelog_add(&space, group, text, task, russian)?;
        Ok(format!("{space}/changelog.md"))
    })
}

// --- docs impact -----------------------------------------------------------------

/// Changed-path reasons kept per page, so a broad glob cannot flood the answer.
const MAX_REASONS_PER_PAGE: usize = 8;

/// A repository glob of a page's `paths`: `*` stays within a segment, `**`
/// crosses segments (`**/` also matches no segment at all), `?` is one character.
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    if pattern.is_empty() || pattern.starts_with('/') || path.starts_with('/') || pattern.split('/').any(|p| p == ".." || p == ".") {
        return false;
    }
    fn go(p: &[char], s: &[char]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some('*') if p.get(1) == Some(&'*') => {
                if p.get(2) == Some(&'/') {
                    let rest = &p[3..];
                    go(rest, s) || (0..s.len()).any(|i| s[i] == '/' && go(rest, &s[i + 1..]))
                } else {
                    (0..=s.len()).any(|i| go(&p[2..], &s[i..]))
                }
            }
            Some('*') => {
                let mut i = 0;
                loop {
                    if go(&p[1..], &s[i..]) {
                        return true;
                    }
                    if i == s.len() || s[i] == '/' {
                        return false;
                    }
                    i += 1;
                }
            }
            Some('?') => s.first().is_some_and(|c| *c != '/') && go(&p[1..], &s[1..]),
            Some(c) => s.first() == Some(c) && go(&p[1..], &s[1..]),
        }
    }
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = path.chars().collect();
    go(&p, &s)
}

/// What a team changed in its worktree: committed since its base, and not yet committed.
fn changed_paths(wt: &genie_core::team::TeamWorktree) -> (bool, Vec<String>, Vec<String>) {
    let root = std::path::Path::new(&wt.path);
    let git = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("git").arg("-C").arg(root).args(args).stderr(std::process::Stdio::null()).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    if !root.exists() {
        return (false, Vec::new(), vec![format!("worktree {} does not exist", wt.path)]);
    }
    let top = git(&["rev-parse", "--show-toplevel"]).map(|t| std::path::PathBuf::from(t.trim()));
    if top.and_then(|t| t.canonicalize().ok()) != root.canonicalize().ok() {
        return (false, Vec::new(), vec![format!("worktree {} is not a git working tree", wt.path)]);
    }
    let mut notes = Vec::new();
    let mut paths = std::collections::BTreeSet::new();
    match &wt.base {
        None => notes.push("no base commit recorded for the team".to_string()),
        Some(base) => {
            let short = &base[..base.len().min(10)];
            match git(&["diff", "--name-only", "--no-renames", &format!("{base}...HEAD")]) {
                None => notes.push(format!("base {short} is not reachable from the worktree")),
                Some(diff) => {
                    paths.extend(diff.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string));
                    if paths.is_empty() {
                        notes.push(format!("no changes found between {short} and HEAD"));
                    }
                }
            }
        }
    }
    if let Some(status) = git(&["status", "--porcelain=v1", "-z", "--untracked-files=all"]) {
        let mut chunks = status.split('\0').filter(|c| !c.is_empty());
        while let Some(chunk) = chunks.next() {
            let (code, file) = (chunk.get(..2).unwrap_or_default(), chunk.get(3..).unwrap_or_default());
            paths.insert(file.to_string());
            if code.contains(['R', 'C'])
                && let Some(from) = chunks.next()
            {
                paths.insert(from.to_string());
            }
        }
    }
    (true, paths.into_iter().collect(), notes)
}

/// Why a page is a candidate: a changed path matched its `paths`, or its `related` names the task or its epic.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "kebab-case")]
#[ts(export)]
pub enum DocsImpactReason {
    ChangedPath { path: String, pattern: String },
    Related { id: String },
}

#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocsImpactCandidate {
    pub path: String,
    pub title: String,
    #[serde(rename = "type")]
    #[ts(type = r#""guide" | "reference" | "decision" | "glossary" | "runbook" | "note" | null"#)]
    pub doc_type: Option<String>,
    #[ts(type = r#""draft" | "current" | "deprecated" | null"#)]
    pub status: Option<String>,
    pub stale: bool,
    pub stale_reasons: Vec<String>,
    pub diagnostics: Vec<String>,
    pub reasons: Vec<DocsImpactReason>,
    /// English one-liner for the CLI and logs; the web phrases the reasons itself.
    pub summary: String,
}

/// The docs-impact hint of a task (`GET /api/tasks/<id>/docs-impact`).
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocsImpact {
    pub task_id: String,
    pub status: genie_core::Status,
    /// The task is in review or done: the statuses the hint is meant for.
    pub applicable: bool,
    pub changed_paths_available: bool,
    pub changed_paths: Vec<String>,
    /// Stable English notes on what could not be read; the web translates them.
    pub notes: Vec<String>,
    pub candidates: Vec<DocsImpactCandidate>,
}

/// Pages of the project's knowledge a task may have made stale: a path its team
/// changed matches a page's `paths`, or the page names the task (or its epic) in
/// `related`. A hint for the reviewer and the documenter, never a gate: whatever
/// cannot be read becomes a note.
pub fn docs_impact(app: &App, project: &str, id: &str) -> AppResult<DocsImpact> {
    use genie_core::Status;
    let task = app.with_tracker(project, |t| t.get(id))?;
    let mut notes: Vec<String> = Vec::new();
    let (mut available, mut changed) = (false, Vec::new());
    let worktree = task.team.as_ref().and_then(|team| app.with_tracker(project, |t| t.bus().get(team)).ok()).and_then(|t| t.worktree);
    match worktree {
        None => notes.push("no team worktree for this task".into()),
        Some(wt) => {
            let (ok, paths, n) = changed_paths(&wt);
            (available, changed) = (ok, paths);
            notes.extend(n);
        }
    }
    let related: std::collections::BTreeSet<String> =
        std::iter::once(task.id.clone()).chain(task.parent.clone()).map(|s| s.to_uppercase()).collect();
    let pages = match app.with_vault(|v| v.tree()) {
        Ok(p) => p,
        Err(e) => {
            notes.push(format!("docs index unavailable: {e}"));
            Vec::new()
        }
    };
    let mut found: Vec<(usize, DocsImpactCandidate)> = Vec::new();
    for page in pages.into_iter().filter(|p| p.project.as_deref() == Some(project)) {
        let mut reasons = Vec::new();
        let mut matched = 0;
        for pattern in page.paths.iter().flatten() {
            for path in &changed {
                if glob_matches(pattern, path) {
                    matched += 1;
                    if reasons.len() < MAX_REASONS_PER_PAGE {
                        reasons.push(DocsImpactReason::ChangedPath { path: path.clone(), pattern: pattern.clone() });
                    }
                }
            }
        }
        let named: Vec<&String> = page.related.iter().filter(|r| related.contains(&r.trim().to_uppercase())).collect();
        for r in &named {
            reasons.push(DocsImpactReason::Related { id: (*r).clone() });
        }
        if reasons.is_empty() || (page.status.as_deref() == Some("deprecated") && named.is_empty()) {
            continue;
        }
        let summary = reasons
            .iter()
            .map(|r| match r {
                DocsImpactReason::ChangedPath { path, pattern } => format!("changes {path} (page paths {pattern})"),
                DocsImpactReason::Related { id } => format!("related to {id}"),
            })
            .collect::<Vec<_>>()
            .join("; ");
        found.push((
            matched,
            DocsImpactCandidate {
                path: page.path,
                title: page.title,
                doc_type: page.doc_type,
                status: page.status,
                stale: page.stale,
                stale_reasons: page.stale_reasons,
                diagnostics: page.diagnostics,
                reasons,
                summary,
            },
        ));
    }
    // Pages matched by changed paths first (most matches first), stale ones before fresh, then by path.
    found.sort_by(|a, b| (a.0 == 0).cmp(&(b.0 == 0)).then(b.0.cmp(&a.0)).then(b.1.stale.cmp(&a.1.stale)).then(a.1.path.cmp(&b.1.path)));
    Ok(DocsImpact {
        task_id: task.id,
        status: task.status,
        applicable: matches!(task.status, Status::Review | Status::Done),
        changed_paths_available: available,
        changed_paths: changed,
        notes,
        candidates: found.into_iter().map(|f| f.1).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::{glob_matches, note_page, slug};

    #[test]
    fn notes_get_readable_file_names_and_a_draft_frontmatter() {
        assert_eq!(slug("Как считать возвраты?"), "как-считать-возвраты");
        assert_eq!(slug("  --  "), "note");
        assert_eq!(slug(&"a".repeat(80)).len(), 60);
        assert_eq!(
            note_page("Returns \"v2\"", "  body\n", &["billing".into()], &[]),
            "---\ntitle: \"Returns \\\"v2\\\"\"\ntype: note\nstatus: draft\ntags: [\"billing\"]\n---\n\nbody\n"
        );
    }

    #[test]
    fn globs_follow_the_docs_paths_contract() {
        assert!(glob_matches("src/**", "src/a/b.rs"));
        assert!(glob_matches("src/**/*.abap", "src/pricing.abap"), "**/ also matches no segment");
        assert!(glob_matches("src/**/*.abap", "src/a/b/pricing.abap"));
        assert!(glob_matches("src/*.rs", "src/lib.rs"));
        assert!(!glob_matches("src/*.rs", "src/a/lib.rs"), "* stays within a segment");
        assert!(glob_matches("docs/?.md", "docs/a.md"));
        assert!(!glob_matches("docs/?.md", "docs/ab.md"));
        assert!(!glob_matches("../x", "../x"));
        assert!(!glob_matches("src/**", "/etc/passwd"));
    }
}
