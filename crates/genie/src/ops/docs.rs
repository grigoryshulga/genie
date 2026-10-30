//! The project's knowledge base.

use std::path::PathBuf;

use genie_core::Capability;
use genie_core::doc_ops::DocOp;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Cx, Entry, Listed, Need, Op, Out, enc, register, render};

pub fn register(all: &mut Vec<Entry>) {
    register!(
        all,
        Search,
        Read,
        Tree,
        Required,
        Edit,
        Write,
        Publish,
        Discard,
        Changes,
        Changeset,
        History,
        Diff,
        Revert,
        Restore,
        Review,
        MergeShow,
        MergeResolve,
        Note,
        Impact,
        Spaces,
        Space,
        Changelog,
        Release
    );
}

/// Full-text search over the project's pages; deprecated pages only when --status asks for them.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Search {
    pub query: String,
    /// guide, reference, decision, glossary, runbook or note.
    #[arg(long = "type")]
    #[serde(rename = "type")]
    pub doc_type: Option<String>,
    /// draft, current or deprecated.
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
}

impl Op for Search {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "search";
    const LEGACY: Option<&'static str> = Some("docs search");
    const CAPS: &'static [Capability] = &[Capability::DocsRead];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let mut q = format!("/docs/search?q={}", enc(&self.query));
        for (k, v) in [("type", self.doc_type), ("status", self.status), ("limit", self.limit.map(|n| n.to_string()))] {
            if let Some(v) = v {
                q.push_str(&format!("&{k}={}", enc(&v)));
            }
        }
        let v = cx.call("GET", &q, None).await?;
        let rows = v["results"].as_array().cloned().unwrap_or_default();
        let text = if rows.is_empty() {
            format!("no documentation matches \"{}\"", self.query)
        } else {
            rows.iter()
                .map(|r| {
                    let snippet = r["snippet"].as_str().unwrap_or_default().split_whitespace().collect::<Vec<_>>().join(" ");
                    format!("{}\n    {snippet}", render::doc_line(r))
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        };
        Ok(Out::new(text, v["results"].clone()))
    }
}

/// Read a page, or one of its sections with --heading.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Read {
    /// Path in the vault, e.g. billing/returns.md.
    pub path: String,
    /// Only the section under this heading.
    #[arg(long, conflicts_with = "whole")]
    pub heading: Option<String>,
    /// The whole page (the default without --heading).
    #[arg(long)]
    #[serde(default)]
    pub whole: bool,
    /// Cut the text after this many characters.
    #[arg(long)]
    pub max_chars: Option<usize>,
    /// Read a changeset's version (default: your own open changeset touching the page).
    #[arg(long)]
    pub changeset: Option<String>,
    /// Read the page at this commit on main.
    #[arg(long)]
    pub at: Option<String>,
}

impl Op for Read {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "read";
    const LEGACY: Option<&'static str> = Some("docs read");
    const CAPS: &'static [Capability] = &[Capability::DocsRead];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let mut q = format!("/docs/page?path={}", enc(&self.path));
        if let Some(h) = &self.heading {
            q.push_str(&format!("&heading={}", enc(h)));
        }
        if let Some(n) = self.max_chars {
            q.push_str(&format!("&maxChars={n}"));
        }
        if let Some(c) = &self.changeset {
            q.push_str(&format!("&changeset={}", enc(c)));
        }
        if let Some(a) = &self.at {
            q.push_str(&format!("&at={}", enc(a)));
        }
        let v = cx.call("GET", &q, None).await?;
        Ok(Out::new(render::doc(&v), v))
    }
}

/// Every page in the selected project's documentation roots, with draft, stale and diagnostics markers.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Tree {}

impl Op for Tree {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "tree";
    const LEGACY: Option<&'static str> = Some("docs tree");
    const CAPS: &'static [Capability] = &[Capability::DocsRead];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/docs/tree", None).await?;
        let pages = v["pages"].as_array().cloned().unwrap_or_default();
        let text = if pages.is_empty() {
            "no documentation pages".to_string()
        } else {
            pages.iter().map(render::doc_line).collect::<Vec<_>>().join("\n")
        };
        Ok(Out::new(text, v["pages"].clone()))
    }
}

/// The pages the project requires a role to read, whole, as an agent receives them.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Required {
    /// The role (executor, reviewer, documenter, ...); default: the caller's own role for an agent.
    #[arg(long)]
    pub role: Option<String>,
}

impl Op for Required {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "required";
    const LEGACY: Option<&'static str> = Some("docs required");
    const CAPS: &'static [Capability] = &[Capability::DocsRead];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let mut q = String::from("/docs/required");
        if let Some(role) = self.role.filter(|r| !r.trim().is_empty()) {
            q.push_str(&format!("?role={}", enc(&role)));
        }
        let v = cx.call("GET", &q, None).await?;
        Ok(Out::new(render::required(&v), v))
    }
}

/// Change a page section by section in your changeset; publish it with `genie docs publish`.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Edit {
    /// Path in the vault, e.g. billing/returns.md.
    pub path: String,
    /// The operations as JSON: one DocOp object or an array of them (the MCP form of the flags).
    #[arg(long, allow_hyphen_values = true)]
    pub ops: Option<String>,
    /// Replace a section's body; pass the new text with --body-file.
    #[arg(long, value_name = "HEADING")]
    pub replace_section: Option<String>,
    /// Append to the page, or to --heading's section; pass the text with --text-file.
    #[arg(long)]
    pub append: bool,
    /// The section --append writes into (default: the end of the page).
    #[arg(long)]
    pub heading: Option<String>,
    /// The text to append (command line only).
    #[arg(long)]
    #[serde(skip)]
    #[schemars(skip)]
    pub text_file: Option<PathBuf>,
    /// Insert a new section; pass its body with --body-file.
    #[arg(long, value_name = "HEADING")]
    pub insert_section: Option<String>,
    /// Insert after this section (default: the end of the page).
    #[arg(long, value_name = "HEADING")]
    pub after: Option<String>,
    /// The new section's heading level, 1-6 (default: 2).
    #[arg(long)]
    pub level: Option<u8>,
    /// Read a section body from this file (command line only).
    #[arg(long)]
    #[serde(skip)]
    #[schemars(skip)]
    pub body_file: Option<PathBuf>,
    /// Remove a section and its subsections.
    #[arg(long, value_name = "HEADING")]
    pub remove_section: Option<String>,
    /// Set a frontmatter field (`field=value`; the value is JSON or a plain string); repeat for more.
    #[arg(long = "set-meta", value_name = "FIELD=VALUE")]
    #[serde(default)]
    pub set_meta: Vec<String>,
    /// Replace the whole page with this file's text (command line only).
    #[arg(long)]
    #[serde(skip)]
    #[schemars(skip)]
    pub file: Option<PathBuf>,
    /// What changed and why: the commit message.
    #[arg(long, allow_hyphen_values = true)]
    pub note: Option<String>,
    /// The changeset to edit (default: your own for your task, opened on the first edit).
    #[arg(long)]
    pub changeset: Option<i64>,
    /// The task this change belongs to (default: yours).
    #[arg(long)]
    pub task: Option<String>,
}

impl Edit {
    /// The operations of this call: the JSON form and the flags, in the order they were given.
    fn operations(&self, cx: &Cx) -> Result<Vec<DocOp>, String> {
        let mut ops: Vec<DocOp> = Vec::new();
        if let Some(raw) = &self.ops {
            let value: Value = serde_json::from_str(raw).map_err(|e| format!("--ops: {e}"))?;
            match value {
                Value::Array(items) => {
                    ops = serde_json::from_value(Value::Array(items)).map_err(|e| format!("--ops: {e}"))?;
                }
                one => ops.push(serde_json::from_value(one).map_err(|e| format!("--ops: {e}"))?),
            }
        }
        if let Some(heading) = &self.replace_section {
            let body = cx.text(None, self.body_file.clone())?.ok_or("pass the new section body with --body-file")?;
            ops.push(DocOp::ReplaceSection { heading: heading.clone(), body });
        }
        if self.append {
            let text = cx.text(None, self.text_file.clone())?.ok_or("pass the text to append with --text-file")?;
            ops.push(DocOp::Append { heading: self.heading.clone(), text });
        }
        if let Some(heading) = &self.insert_section {
            let body = cx.text(None, self.body_file.clone())?.ok_or("pass the new section body with --body-file")?;
            ops.push(DocOp::InsertSection { after: self.after.clone(), heading: heading.clone(), level: self.level, body });
        }
        if let Some(heading) = &self.remove_section {
            ops.push(DocOp::RemoveSection { heading: heading.clone() });
        }
        for kv in &self.set_meta {
            let (field, value) = kv.split_once('=').ok_or_else(|| format!("--set-meta {kv}: expected field=value"))?;
            let raw = value.trim();
            let value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()));
            ops.push(DocOp::SetMeta { field: field.trim().to_string(), value });
        }
        if let Some(file) = &self.file {
            let content = cx.text(None, Some(file.clone()))?.ok_or("the file is empty")?;
            ops.push(DocOp::WritePage { content });
        }
        if ops.is_empty() {
            return Err(
                "pass --ops or an editing flag (--replace-section, --append, --insert-section, --remove-section, --set-meta, --file)"
                    .into(),
            );
        }
        Ok(ops)
    }
}

impl Op for Edit {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "edit";
    const LEGACY: Option<&'static str> = Some("docs edit");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::DocsWrite];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let ops = self.operations(cx)?;
        let task = self.task.or_else(|| cx.task.clone());
        let path = self.path.clone();
        let body =
            json!({ "path": self.path, "ops": ops, "note": self.note.unwrap_or_default(), "task": task, "changeset": self.changeset });
        let v = cx.call("POST", "/docs/edit", Some(body)).await?;
        let id = v["changeset"]["id"].as_i64().unwrap_or_default();
        let page = v["page"]["path"].as_str().unwrap_or(&path);
        let sha = v["commit"].as_str().unwrap_or_default();
        Ok(Out::new(format!("{page} changed in changeset #{id} (commit {})", short(sha)), v))
    }
}

/// Write a page in one go; kept for people and imports, prefer `genie docs edit`.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Write {
    pub path: String,
    /// The page's full Markdown; `-` reads it from stdin.
    #[arg(long, allow_hyphen_values = true)]
    pub text: Option<String>,
    /// Read the page from a file (command line only).
    #[arg(long)]
    #[serde(skip)]
    #[schemars(skip)]
    pub file: Option<PathBuf>,
    /// What changed and why: the commit message.
    #[arg(long, allow_hyphen_values = true)]
    pub note: Option<String>,
    /// create (the page must not exist), update (it must) or upsert.
    #[arg(long, default_value = "upsert", value_parser = ["create", "update", "upsert"])]
    #[serde(default = "upsert")]
    pub mode: String,
    /// The task this change belongs to (default: yours).
    #[arg(long)]
    pub task: Option<String>,
}

fn upsert() -> String {
    "upsert".into()
}

impl Op for Write {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "write";
    const LEGACY: Option<&'static str> = Some("docs write");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::DocsWrite];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let content = cx.text(self.text, self.file)?.ok_or("pass the page as --text (or --file on the command line)")?;
        let task = self.task.or_else(|| cx.task.clone());
        let path = self.path.clone();
        let body = json!({ "path": self.path, "content": content, "note": self.note.unwrap_or_default(), "mode": self.mode, "task": task });
        let v = cx.call("POST", "/docs/page", Some(body)).await?;
        let id = v["changeset"]["id"].as_i64().unwrap_or_default();
        let page = v["page"]["path"].as_str().unwrap_or(&path);
        Ok(Out::new(format!("{page} saved in changeset #{id}; publish it with `genie docs publish`"), v))
    }
}

fn short(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// Publish a changeset into main (default: your open changeset for your task).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Publish {
    /// The changeset; default: the one you have open for your task.
    #[arg(long)]
    pub changeset: Option<i64>,
}

impl Op for Publish {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "publish";
    const LEGACY: Option<&'static str> = Some("docs publish");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::DocsWrite];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = open_changeset(cx, self.changeset).await?;
        let v = cx.call("POST", &format!("/changesets/{id}/publish"), None).await?;
        Ok(Out::new(publish_text(&v), v))
    }
}

/// Close a changeset without publishing it (default: your open changeset for your task).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Discard {
    /// The changeset; default: the one you have open for your task.
    #[arg(long)]
    pub changeset: Option<i64>,
}

impl Op for Discard {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "discard";
    const LEGACY: Option<&'static str> = Some("docs discard");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::DocsWrite];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = open_changeset(cx, self.changeset).await?;
        let v = cx.call("POST", &format!("/changesets/{id}/discard"), None).await?;
        Ok(Out::new(format!("discarded changeset #{id}"), v))
    }
}

async fn open_changeset(cx: &Cx, given: Option<i64>) -> Result<i64, String> {
    if let Some(id) = given {
        return Ok(id);
    }
    let mut q = String::from("/changesets?status=open&mine=1");
    if let Some(task) = cx.task.clone().filter(|t| !t.is_empty()) {
        q.push_str(&format!("&task={}", enc(&task)));
    }
    let v = cx.call("GET", &q, None).await?;
    let ids: Vec<i64> = v.as_array().map(|a| a.iter().filter_map(|c| c["id"].as_i64()).collect()).unwrap_or_default();
    match ids.as_slice() {
        [] => Err("no open changeset: edit a page first".into()),
        [id] => Ok(*id),
        many => Err(format!("{} open changesets: pass --changeset <id>", many.len())),
    }
}

fn publish_text(v: &Value) -> String {
    match v["status"].as_str() {
        Some("merging") => format!("merging: merge {}, the merge agent is on it", v["merge"]["id"]),
        _ => format!("published {}", v["commit"].as_str().unwrap_or_default()),
    }
}

/// The Changes feed: open, published and reviewed changesets (default: your own open ones).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    /// open, merging, published, discarded; several separated by commas.
    #[arg(long)]
    pub status: Option<String>,
    #[arg(long)]
    pub task: Option<String>,
    /// A login.
    #[arg(long)]
    pub author: Option<String>,
    /// Published changesets whose section owners have not reviewed them yet.
    #[arg(long)]
    pub unreviewed: bool,
    #[arg(long)]
    pub path: Option<String>,
    #[arg(long)]
    pub limit: Option<usize>,
}

impl Op for Changes {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "changes";
    const LEGACY: Option<&'static str> = Some("docs changes");
    const CAPS: &'static [Capability] = &[Capability::DocsRead];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let filters = self.status.is_some() || self.task.is_some() || self.author.is_some() || self.unreviewed || self.path.is_some();
        let mut q = String::from(if filters { "/changesets?" } else { "/changesets?status=open&mine=1&" });
        for (k, v) in [
            ("status", self.status),
            ("task", self.task),
            ("author", self.author),
            ("path", self.path),
            ("limit", self.limit.map(|n| n.to_string())),
        ] {
            if let Some(v) = v {
                q.push_str(&format!("{k}={}&", enc(&v)));
            }
        }
        if self.unreviewed {
            q.push_str("unreviewed=1&");
        }
        let v = cx.call("GET", q.trim_end_matches(['?', '&']), None).await?;
        let rows = v.as_array().cloned().unwrap_or_default();
        let text = if rows.is_empty() {
            "no changesets".to_string()
        } else {
            rows.iter().map(render::changeset_line).collect::<Vec<_>>().join("\n")
        };
        Ok(Out::new(text, v))
    }
}

/// A changeset with its commits and full diff.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Changeset {
    pub id: i64,
}

impl Op for Changeset {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "changeset";
    const LEGACY: Option<&'static str> = Some("docs changeset");
    const CAPS: &'static [Capability] = &[Capability::DocsRead];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", &format!("/changesets/{}", self.id), None).await?;
        Ok(Out::new(render::changeset_detail(&v), v))
    }
}

/// Commits on main that touched a page, following moves.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct History {
    pub path: String,
    #[arg(long)]
    pub limit: Option<usize>,
}

impl Op for History {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "history";
    const LEGACY: Option<&'static str> = Some("docs history");
    const CAPS: &'static [Capability] = &[Capability::DocsRead];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let mut q = format!("/history?path={}", enc(&self.path));
        if let Some(n) = self.limit {
            q.push_str(&format!("&limit={n}"));
        }
        let v = cx.call("GET", &q, None).await?;
        let rows = v.as_array().cloned().unwrap_or_default();
        let text = if rows.is_empty() {
            format!("no history for {}", self.path)
        } else {
            rows.iter().map(render::commit_line).collect::<Vec<_>>().join("\n")
        };
        Ok(Out::new(text, v))
    }
}

/// A unified diff of a page between two versions.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Diff {
    pub path: String,
    /// The older version: a commit SHA or a changeset id.
    #[arg(long)]
    pub from: String,
    /// The newer version.
    #[arg(long)]
    pub to: String,
}

impl Op for Diff {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "diff";
    const LEGACY: Option<&'static str> = Some("docs diff");
    const CAPS: &'static [Capability] = &[Capability::DocsRead];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let q = format!("/diff?path={}&from={}&to={}", enc(&self.path), enc(&self.from), enc(&self.to));
        let v = cx.call("GET", &q, None).await?;
        let diff = v["diff"].as_str().unwrap_or_default();
        let text = if diff.trim().is_empty() {
            format!("{}: no differences between {} and {}", self.path, self.from, self.to)
        } else {
            format!("{} {} → {}\n{diff}", self.path, self.from, self.to)
        };
        Ok(Out::new(text, v))
    }
}

/// Revert a published changeset with a new one that undoes it.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Revert {
    pub id: i64,
}

impl Op for Revert {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "revert";
    const LEGACY: Option<&'static str> = Some("docs revert");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::DocsWrite];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", &format!("/changesets/{}/revert", self.id), None).await?;
        Ok(Out::new(format!("reverted #{}: {}", self.id, publish_text(&v)), v))
    }
}

/// Publish a page's content at a version as a new changeset.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Restore {
    pub path: String,
    /// The commit SHA to restore the page to.
    #[arg(long)]
    pub at: String,
}

impl Op for Restore {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "restore";
    const LEGACY: Option<&'static str> = Some("docs restore");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::DocsWrite];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", "/restore", Some(json!({ "path": self.path, "at": self.at }))).await?;
        Ok(Out::new(format!("restored {} at {}: {}", self.path, self.at, publish_text(&v)), v))
    }
}

/// Mark a published changeset reviewed (the touched sections' owners only).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Review {
    pub id: i64,
}

impl Op for Review {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "review";
    const LEGACY: Option<&'static str> = Some("docs review");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::DocsWrite];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", &format!("/changesets/{}/review", self.id), Some(json!({ "decision": "reviewed" }))).await?;
        let by = v["changeset"]["reviewedBy"].as_str().filter(|b| !b.is_empty()).unwrap_or("you");
        Ok(Out::new(format!("changeset #{} reviewed by {by}", self.id), v))
    }
}

/// A pending merge: the conflicted pages with base, ours, theirs and the marked text.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct MergeShow {
    pub id: i64,
}

impl Op for MergeShow {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "merge-show";
    const LEGACY: Option<&'static str> = Some("docs merge-show");
    const CAPS: &'static [Capability] = &[Capability::DocsRead];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", &format!("/merges/{}", self.id), None).await?;
        Ok(Out::new(render::merge(&v), v))
    }
}

/// Resolve a pending merge: stage merged pages and publish, or hand it to the owners.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MergeResolve {
    pub id: i64,
    /// A conflicted page; repeat with --file in the same order.
    #[arg(long = "page")]
    #[serde(default)]
    pub page: Vec<String>,
    /// The whole merged page for the --page before it (command line only).
    #[arg(long = "file")]
    #[serde(skip)]
    #[schemars(skip)]
    pub file: Vec<PathBuf>,
    /// Every conflicted page is staged: validate and publish.
    #[arg(long)]
    pub done: bool,
    /// Hand the merge to the section owners.
    #[arg(long)]
    pub fail: bool,
    /// The two sides contradict each other in meaning: list it unreviewed.
    #[arg(long)]
    pub contradiction: bool,
    /// One line: what you combined and why, or why you stopped.
    #[arg(long, allow_hyphen_values = true)]
    pub note: Option<String>,
}

impl MergeResolve {
    fn pages(&self, cx: &Cx) -> Result<Vec<Value>, String> {
        if self.page.len() != self.file.len() {
            return Err("give each --page one --file with its merged text".into());
        }
        let mut pages = Vec::new();
        for (path, file) in self.page.iter().zip(self.file.iter()) {
            let content = cx.text(None, Some(file.clone()))?.ok_or_else(|| format!("{}: the file is empty", file.display()))?;
            pages.push(json!({ "path": path, "content": content }));
        }
        Ok(pages)
    }
}

impl Op for MergeResolve {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "merge-resolve";
    const LEGACY: Option<&'static str> = Some("docs merge-resolve");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::DocsWrite];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let pages = self.pages(cx)?;
        let body = json!({
            "pages": pages,
            "done": self.done,
            "fail": self.fail,
            "contradiction": self.contradiction,
            "note": self.note.unwrap_or_default(),
        });
        let v = cx.call("POST", &format!("/merges/{}/resolve", self.id), Some(body)).await?;
        let status = v["merge"]["status"].as_str().or_else(|| v["status"].as_str()).unwrap_or("resolved");
        Ok(Out::new(format!("merge #{} {status}", self.id), v))
    }
}

/// Capture a draft note in the inbox of the project's space.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub title: String,
    /// The note's text.
    #[arg(short = 'd', long = "description", allow_hyphen_values = true)]
    #[serde(default)]
    pub body: String,
    /// A tag; repeat or separate with commas.
    #[arg(long = "tag")]
    #[serde(default)]
    pub tags: Vec<String>,
    /// A related page or task; repeat or separate with commas.
    #[arg(long)]
    #[serde(default)]
    pub related: Vec<String>,
}

fn split(items: Vec<String>) -> Vec<String> {
    items.iter().flat_map(|x| x.split(',')).map(str::trim).filter(|x| !x.is_empty()).map(str::to_string).collect()
}

impl Op for Note {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "note";
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::DocsWrite];
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let body =
            json!({ "title": self.title, "body": self.body, "tags": split(self.tags), "related": split(self.related), "task": cx.task });
        let v = cx.call("POST", "/docs/note", Some(body)).await?;
        let path = v["path"].as_str().unwrap_or_default();
        let text = match v["proposal"].as_i64() {
            Some(id) => format!("proposal #{id} created for {path}: the section needs a review, the owner decides"),
            None => format!("created {path}"),
        };
        Ok(Out::new(text, v))
    }
}

/// Pages a task's changes may have made stale; never blocks the task (default: your task).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Impact {
    pub task: Option<String>,
}

impl Op for Impact {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "impact";
    const CAPS: &'static [Capability] = &[Capability::DocsRead];
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v: Value = cx.call("GET", &format!("/tasks/{}/docs-impact", enc(&cx.task(self.task)?)), None).await?;
        Ok(Out::new(render::impact(&v), v))
    }
}

fn st<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default()
}

fn policy(p: &Value) -> String {
    if p.is_null() { "default".into() } else { format!("people {}, agents {}", st(p, "humans"), st(p, "agents")) }
}

fn space_line(name: &str, sp: &Value) -> String {
    let owners: Vec<&str> = sp["owners"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    format!(
        "{name}/  project {} · owners {} · {}",
        sp["project"].as_str().unwrap_or("—"),
        if owners.is_empty() { "—".into() } else { owners.join(", ") },
        policy(&sp["policy"])
    )
}

/// Documentation roots of the selected project: their project, owners and who publishes directly.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Spaces {}

impl Op for Spaces {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "spaces";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/docs/spaces", None).await?;
        let mut out = Vec::new();
        for (name, sp) in v["spaces"].as_object().cloned().unwrap_or_default() {
            out.push(space_line(&name, &sp));
            for (sec, cfg) in sp["sections"].as_object().cloned().unwrap_or_default() {
                out.push(format!("  {name}/{sec}/  {}", policy(&cfg["policy"])));
            }
        }
        out.push(format!("outside the spaces: {}", policy(&v["policy"])));
        Ok(Out::new(out.join("\n"), v))
    }
}

/// Set up a space: its project, owners and who publishes directly (direct, review or locked); sections stay (server admins and the project's admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub name: String,
    /// The project whose code its pages describe.
    #[arg(long = "of")]
    pub of: Option<String>,
    /// An owner's login; repeat for more. Owners review what their sections publish.
    #[arg(long = "owner")]
    pub owners: Option<Vec<String>>,
    /// How people publish: direct, review or locked.
    #[arg(long)]
    pub humans: Option<String>,
    /// How agents publish: direct, review or locked.
    #[arg(long)]
    pub agents: Option<String>,
}

impl Op for Space {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "space";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let all = cx.call("GET", "/docs/spaces", None).await?;
        let mut sp = all["spaces"][&self.name].clone();
        if sp.is_null() {
            sp = json!({ "project": null, "owners": [], "policy": null, "sections": {} });
        }
        if let Some(p) = self.of {
            sp["project"] = if p.is_empty() { Value::Null } else { json!(p) };
        }
        if let Some(o) = self.owners {
            sp["owners"] = json!(o);
        }
        if self.humans.is_some() || self.agents.is_some() {
            let cur = sp["policy"].clone();
            let pick =
                |given: Option<String>, k: &str, default: &str| given.unwrap_or_else(|| cur[k].as_str().unwrap_or(default).to_string());
            sp["policy"] = json!({ "humans": pick(self.humans, "humans", "direct"), "agents": pick(self.agents, "agents", "review") });
        }
        let v = cx.call("PUT", &format!("/docs/spaces/{}", enc(&self.name)), Some(sp)).await?;
        Ok(Out::new(space_line(&self.name, &v["spaces"][&self.name]), v))
    }
}

/// The project's changelog page.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Changelog {}

impl Op for Changelog {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "changelog";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/docs/changelog", None).await?;
        Ok(Out::new(format!("{}\n\n{}", st(&v, "path"), st(&v, "content")), v))
    }
}

/// Release the changelog's unreleased entries as a version (project admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Release {
    /// A single word such as 1.2.0.
    pub version: String,
}

impl Op for Release {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "release";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", "/docs/changelog/release", Some(json!({ "version": self.version }))).await?;
        let notes = v["notes"].as_str().map(str::to_string).unwrap_or_else(|| v["notes"].to_string());
        Ok(Out::new(format!("released {}\n\n{notes}", self.version), v))
    }
}
