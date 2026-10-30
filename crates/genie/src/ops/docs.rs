//! The project's knowledge base.

use std::path::PathBuf;

use genie_core::Capability;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Cx, Entry, Listed, Need, Op, Out, enc, register, render};

pub fn register(all: &mut Vec<Entry>) {
    register!(
        all, Search, Read, Tree, Required, Write, Note, Impact, Proposals, Proposal, Approve, Reject, Spaces, Space, Changelog, Release
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

/// Write a page; where the section asks for a review it becomes a proposal the owner decides on.
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
        let body = json!({ "path": self.path, "content": content, "note": self.note.unwrap_or_default(), "mode": self.mode, "task": task });
        let v = cx.call("POST", "/docs/page", Some(body)).await?;
        let text = match v["proposal"].as_i64() {
            Some(id) => format!("proposal #{id} created for {}: the section needs a review, the owner decides", self.path),
            None => format!("{} saved", v["page"]["path"].as_str().unwrap_or(&self.path)),
        };
        Ok(Out::new(text, v))
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

fn proposal_line(p: &Value) -> String {
    format!(
        "#{:<4} {:<9} {}  by {} ({}){}{}",
        p["id"].to_string(),
        st(p, "status"),
        st(p, "path"),
        st(p, "author"),
        st(p, "authorKind"),
        p["task"].as_str().map(|t| format!(" for {t}")).unwrap_or_default(),
        if st(p, "note").is_empty() { String::new() } else { format!(" — {}", st(p, "note")) }
    )
}

/// Proposed changes to pages waiting for their owners (--status open, approved, rejected, superseded or all).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Proposals {
    #[arg(long, default_value = "open")]
    #[serde(default = "open")]
    pub status: String,
}

fn open() -> String {
    "open".into()
}

impl Op for Proposals {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "proposals";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", &format!("/docs/proposals?status={}", enc(&self.status)), None).await?;
        let rows = v.as_array().cloned().unwrap_or_default();
        let text = if rows.is_empty() {
            format!("no {} proposals", self.status)
        } else {
            rows.iter().map(proposal_line).collect::<Vec<_>>().join("\n")
        };
        Ok(Out::new(text, v))
    }
}

/// A proposal: the proposed page next to the current one, and who decides.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Proposal {
    pub id: i64,
}

impl Op for Proposal {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "proposal";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", &format!("/docs/proposals/{}", self.id), None).await?;
        let p = &v["proposal"];
        let owners: Vec<&str> = v["owners"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let mut out = vec![proposal_line(p)];
        out.push(format!("decides: {}", if owners.is_empty() { "the project's admins".to_string() } else { owners.join(", ") }));
        if let Some(by) = p["decidedBy"].as_str() {
            out.push(format!("decided by {by}{}", p["decisionNote"].as_str().map(|n| format!(": {n}")).unwrap_or_default()));
        }
        out.push(String::new());
        out.push("## Proposed".into());
        out.push(st(p, "content").to_string());
        out.push(String::new());
        out.push("## Current".into());
        out.push(v["current"].as_str().unwrap_or("(the page does not exist yet)").to_string());
        Ok(Out::new(out.join("\n"), v))
    }
}

/// Apply a proposal to its page (the section's owners and the project's admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Approve {
    pub id: i64,
    #[arg(long, allow_hyphen_values = true)]
    pub note: Option<String>,
    /// Apply it even though the page changed since it was proposed.
    #[arg(long)]
    #[serde(default)]
    pub force: bool,
}

impl Op for Approve {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "approve";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx
            .call("POST", &format!("/docs/proposals/{}/approve", self.id), Some(json!({ "note": self.note, "force": self.force })))
            .await?;
        Ok(Out::new(format!("proposal #{} approved: {} changed", self.id, st(&v, "path")), v))
    }
}

/// Turn a proposal down (the section's owners and the project's admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Reject {
    pub id: i64,
    /// Why, for its author.
    #[arg(long, allow_hyphen_values = true)]
    pub note: Option<String>,
}

impl Op for Reject {
    const GROUP: &'static str = "docs";
    const NAME: &'static str = "reject";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", &format!("/docs/proposals/{}/reject", self.id), Some(json!({ "note": self.note }))).await?;
        Ok(Out::new(format!("proposal #{} rejected", self.id), v))
    }
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
    /// An owner's login; repeat for more. Owners decide on proposals.
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
