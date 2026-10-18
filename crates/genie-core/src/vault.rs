//! Knowledge vault: an Obsidian-compatible folder of Markdown pages (ideally its
//! own git repository), indexed for search and written through the server.
//!
//! - Files are the source of truth; the index (`vault-index.db`) is a cache that
//!   is refreshed from file signatures and can be deleted at any time.
//! - Frontmatter follows the genie docs contract (`docs/reference/genie-docs-system.md`);
//!   unknown properties written by Obsidian are kept and ignored.
//! - `[[wiki-links]]` and `![[embeds]]` are extracted outside code spans and
//!   fenced blocks; links resolve by path, then by unique file name or alias.
//! - Writes are atomic, checked against the base content hash (no lost updates
//!   between the web editor, agents and Obsidian), committed to git when the
//!   vault is a repository, and subject to the section's publishing policy:
//!   `direct`, `review` (a proposal for the section owners) or `locked`.
//! - Each project has a space (top-level folder) with a `changelog.md`.

use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::db::Db;
use crate::error::{GenieError, Result};

pub const DOC_TYPES: &[&str] = &["guide", "reference", "decision", "glossary", "runbook", "note"];
pub const DOC_STATUSES: &[&str] = &["draft", "current", "deprecated"];

const INDEX_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS pages (
  path TEXT PRIMARY KEY,
  signature TEXT NOT NULL,
  title TEXT NOT NULL,
  type TEXT,
  status TEXT,
  summary TEXT,
  tags TEXT NOT NULL,
  aliases TEXT NOT NULL,
  paths TEXT,
  related TEXT NOT NULL,
  verified TEXT,
  project TEXT,
  content_hash TEXT NOT NULL,
  headings TEXT NOT NULL,
  diagnostics TEXT NOT NULL,
  links TEXT NOT NULL,
  updated TEXT
);
CREATE VIRTUAL TABLE IF NOT EXISTS pages_fts USING fts5(
  path UNINDEXED, title, summary, headings, body, tags,
  tokenize = "unicode61 remove_diacritics 2"
);
"#;

// --- configuration -----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "lowercase")]
#[ts(export)]
pub enum Publish {
    Direct,
    Review,
    Locked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct Policy {
    pub humans: Publish,
    pub agents: Publish,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { humans: Publish::Direct, agents: Publish::Review }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Section {
    pub policy: Option<Policy>,
    pub owners: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Space {
    /// Project (slug) whose code the `paths` of pages refer to.
    pub project: Option<String>,
    pub owners: Vec<String>,
    pub policy: Option<Policy>,
    pub sections: BTreeMap<String, Section>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VaultConfig {
    pub spaces: BTreeMap<String, Space>,
    /// Policy outside any space (shared pages, `inbox/`).
    pub policy: Option<Policy>,
}

/// Who writes: people and agents may have different publishing rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorKind {
    Human,
    Agent,
    /// The server itself (changelog, approved proposals): always direct.
    System,
}

// --- parsing -----------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct ParsedDoc {
    pub title: String,
    #[serde(rename = "type")]
    pub doc_type: Option<String>,
    pub status: Option<String>,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub paths: Option<Vec<String>>,
    pub related: Vec<String>,
    pub verified: Option<String>,
    pub project: Option<String>,
    pub body: String,
    pub headings: Vec<String>,
    pub diagnostics: Vec<String>,
    /// Wiki-link and embed targets outside code, in order.
    pub links: Vec<String>,
}

fn parse_scalar(v: &str) -> std::result::Result<String, String> {
    let v = v.trim();
    if v.is_empty() {
        return Ok(String::new());
    }
    if v.starts_with('"') || v.ends_with('"') {
        if !(v.len() >= 2 && v.starts_with('"') && v.ends_with('"')) {
            return Err("unclosed double-quoted string".into());
        }
        return serde_json::from_str::<String>(v).map_err(|e| format!("invalid double-quoted string ({e})"));
    }
    if v.starts_with('\'') || v.ends_with('\'') {
        if !(v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'')) {
            return Err("unclosed single-quoted string".into());
        }
        return Ok(v[1..v.len() - 1].replace("''", "'"));
    }
    // Comments after a value: `value # note`.
    Ok(match v.find(" #") {
        Some(i) => v[..i].trim().to_string(),
        None => v.to_string(),
    })
}

fn parse_inline_list(v: &str) -> std::result::Result<Vec<String>, String> {
    let v = v.trim();
    if !(v.starts_with('[') && v.ends_with(']')) {
        return Err("expected a YAML string list".into());
    }
    let inside = v[1..v.len() - 1].trim();
    if inside.is_empty() {
        return Ok(Vec::new());
    }
    let mut parts = Vec::new();
    let (mut start, mut quote, mut chars) = (0usize, None::<char>, inside.char_indices().peekable());
    while let Some((i, c)) = chars.next() {
        match (quote, c) {
            (Some('"'), '\\') => {
                chars.next();
            }
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (None, ',') => {
                parts.push(inside[start..i].trim().to_string());
                start = i + 1;
            }
            _ => {}
        }
    }
    if quote.is_some() {
        return Err("unclosed quoted list value".into());
    }
    parts.push(inside[start..].trim().to_string());
    if parts.iter().any(|p| p.is_empty()) {
        return Err("empty item in YAML string list".into());
    }
    parts.iter().map(|p| parse_scalar(p)).collect()
}

enum Raw {
    Scalar(String),
    List(Vec<String>),
}

fn valid_date(s: &str) -> bool {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok() && s.len() == 10
}

/// Split frontmatter and body; parse the narrow YAML subset of the contract.
pub fn parse_doc(markdown: &str, path: &str) -> ParsedDoc {
    let mut doc = ParsedDoc::default();
    let text = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    let mut body = text;
    let mut raw: BTreeMap<String, Raw> = BTreeMap::new();
    if let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) {
        let end = rest.find("\n---\n").or_else(|| rest.find("\n---\r\n")).or_else(|| rest.strip_suffix("\n---").map(|r| r.len()));
        match end {
            Some(end) => {
                let fm = &rest[..end];
                body = rest[end..].trim_start_matches('\n').trim_start_matches("---").trim_start_matches(['\r', '\n']);
                let lines: Vec<&str> = fm.lines().collect();
                let mut i = 0;
                while i < lines.len() {
                    let line = lines[i];
                    if line.trim().is_empty() || line.trim_start().starts_with('#') {
                        i += 1;
                        continue;
                    }
                    if line.starts_with([' ', '\t']) {
                        // Nested values of unknown (Obsidian) properties.
                        i += 1;
                        continue;
                    }
                    let Some((key, value)) = line.split_once(':') else {
                        doc.diagnostics.push(format!("Line {}: expected a frontmatter key and value", i + 2));
                        i += 1;
                        continue;
                    };
                    let key = key.trim().to_string();
                    let value = value.trim();
                    if raw.contains_key(&key) {
                        doc.diagnostics.push(format!("Duplicate frontmatter field \"{key}\""));
                        i += 1;
                        continue;
                    }
                    if value.is_empty() {
                        let mut items = Vec::new();
                        let mut j = i + 1;
                        while j < lines.len() && lines[j].trim_start().starts_with("- ") && lines[j].starts_with([' ', '\t', '-']) {
                            match parse_scalar(lines[j].trim_start().trim_start_matches("- ")) {
                                Ok(v) => items.push(v),
                                Err(e) => doc.diagnostics.push(format!("Line {}: {e}", j + 2)),
                            }
                            j += 1;
                        }
                        raw.insert(key, if j > i + 1 { Raw::List(items) } else { Raw::Scalar(String::new()) });
                        i = j;
                        continue;
                    }
                    if value.starts_with('[') {
                        match parse_inline_list(value) {
                            Ok(l) => {
                                raw.insert(key, Raw::List(l));
                            }
                            Err(e) => doc.diagnostics.push(format!("Field \"{key}\": {e}")),
                        }
                    } else {
                        match parse_scalar(value) {
                            Ok(v) => {
                                raw.insert(key, Raw::Scalar(v));
                            }
                            Err(e) => doc.diagnostics.push(format!("Field \"{key}\": {e}")),
                        }
                    }
                    i += 1;
                }
            }
            None => doc.diagnostics.push("Frontmatter opening delimiter has no closing ---; the file was read as Markdown".into()),
        }
    }
    let scalar = |doc: &mut ParsedDoc, k: &str| -> Option<String> {
        match raw.get(k) {
            Some(Raw::Scalar(s)) if !s.trim().is_empty() && !s.contains('\n') => Some(s.trim().to_string()),
            Some(_) => {
                doc.diagnostics.push(format!("Field \"{k}\" must be a non-empty string"));
                None
            }
            None => None,
        }
    };
    let list = |doc: &mut ParsedDoc, k: &str| -> Option<Vec<String>> {
        match raw.get(k) {
            Some(Raw::List(l)) if l.iter().all(|x| !x.trim().is_empty()) => Some(l.iter().map(|s| s.trim().to_string()).collect()),
            Some(Raw::Scalar(s)) if k == "tags" && !s.trim().is_empty() => {
                Some(s.split(',').map(|x| x.trim().trim_start_matches('#').to_string()).filter(|x| !x.is_empty()).collect())
            }
            Some(_) => {
                doc.diagnostics.push(format!("Field \"{k}\" must be a list of non-empty strings"));
                None
            }
            None => None,
        }
    };
    let title = scalar(&mut doc, "title");
    doc.doc_type = scalar(&mut doc, "type");
    if let Some(t) = &doc.doc_type
        && !DOC_TYPES.contains(&t.as_str())
    {
        doc.diagnostics.push(format!("Field \"type\" must be one of: {}", DOC_TYPES.join(", ")));
        doc.doc_type = None;
    }
    doc.status = scalar(&mut doc, "status");
    if let Some(s) = &doc.status
        && !DOC_STATUSES.contains(&s.as_str())
    {
        doc.diagnostics.push(format!("Field \"status\" must be one of: {}", DOC_STATUSES.join(", ")));
        doc.status = None;
    }
    doc.summary = scalar(&mut doc, "summary");
    doc.tags = list(&mut doc, "tags").unwrap_or_default();
    doc.aliases = list(&mut doc, "aliases").unwrap_or_default();
    doc.paths = list(&mut doc, "paths");
    doc.related = list(&mut doc, "related").unwrap_or_default().into_iter().map(|r| r.to_uppercase()).collect();
    doc.verified = scalar(&mut doc, "verified");
    if let Some(v) = &doc.verified
        && !valid_date(v)
    {
        doc.diagnostics.push("Field \"verified\" must be an ISO date in YYYY-MM-DD form".into());
        doc.verified = None;
    }
    doc.project = scalar(&mut doc, "project");

    let mut in_fence: Option<String> = None;
    let mut first_h1 = None;
    let mut first_para = None;
    let mut prose = String::new();
    for line in body.lines() {
        let t = line.trim_start();
        if let Some(f) = &in_fence {
            if t.starts_with(f.as_str()) {
                in_fence = None;
            }
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = Some(t[..3].to_string());
            continue;
        }
        if let Some(h) = t.strip_prefix('#') {
            let level = 1 + h.chars().take_while(|c| *c == '#').count();
            let text = h.trim_start_matches('#');
            if level <= 6 && text.starts_with(' ') {
                let text = text.trim().trim_end_matches('#').trim().to_string();
                if level == 1 && first_h1.is_none() {
                    first_h1 = Some(text.clone());
                }
                doc.headings.push(text);
                prose.push('\n');
                continue;
            }
        }
        if first_para.is_none() && !t.is_empty() && !t.starts_with(['|', '>', '-', '*', '!', '<']) {
            first_para = Some(t.to_string());
        }
        prose.push_str(line);
        prose.push('\n');
        // Inline tags (#tag) count as tags.
        for word in t.split_whitespace() {
            if let Some(tag) = word.strip_prefix('#')
                && !tag.is_empty()
                && tag.chars().next().is_some_and(|c| c.is_alphabetic())
                && tag.chars().all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '/'))
                && !doc.tags.iter().any(|x| x == tag)
            {
                doc.tags.push(tag.to_string());
            }
        }
    }
    doc.links = wiki_targets(&prose);
    doc.title =
        title.or(first_h1).unwrap_or_else(|| Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    if doc.summary.is_none() {
        doc.summary = first_para.map(|p| p.chars().take(240).collect());
    }
    doc.body = body.to_string();
    doc
}

/// `[[target]]`, `[[target#heading|text]]` and `![[embed]]` targets outside inline code.
pub fn wiki_targets(prose_without_fences: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in prose_without_fences.lines() {
        // Blank out inline code spans (any backtick run length).
        let mut clean = String::with_capacity(line.len());
        let bytes: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == '`' {
                let run = bytes[i..].iter().take_while(|c| **c == '`').count();
                let rest: String = bytes[i + run..].iter().collect();
                let fence = "`".repeat(run);
                if let Some(end) = rest.find(&fence) {
                    let skip = run + rest[..end].chars().count() + run;
                    clean.extend(std::iter::repeat_n(' ', skip));
                    i += skip;
                    continue;
                }
            }
            clean.push(bytes[i]);
            i += 1;
        }
        let mut rest = clean.as_str();
        while let Some(start) = rest.find("[[") {
            let after = &rest[start + 2..];
            let Some(end) = after.find("]]") else { break };
            let inner = &after[..end];
            let target = inner.split('|').next().unwrap_or_default().split('#').next().unwrap_or_default().trim();
            if !target.is_empty() && !inner.contains('\n') {
                out.push(target.to_string());
            }
            rest = &after[end + 2..];
        }
    }
    out
}

/// Case-folded, `ё` → `е`, so both spellings match each other.
fn normalize(s: &str) -> String {
    s.to_lowercase().replace('ё', "е")
}

fn sha(s: &[u8]) -> String {
    hex::encode(Sha256::digest(s))
}

// --- index -------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocPage {
    pub root_id: String,
    pub path: String,
    pub title: String,
    /// One of `DOC_TYPES`.
    #[serde(rename = "type")]
    #[ts(type = r#""guide" | "reference" | "decision" | "glossary" | "runbook" | "note" | null"#)]
    pub doc_type: Option<String>,
    /// One of `DOC_STATUSES`.
    #[ts(type = r#""draft" | "current" | "deprecated" | null"#)]
    pub status: Option<String>,
    pub summary: Option<String>,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub paths: Option<Vec<String>>,
    pub related: Vec<String>,
    pub verified: Option<String>,
    pub updated: Option<String>,
    pub content_hash: String,
    pub headings: Vec<String>,
    pub diagnostics: Vec<String>,
    pub stale: bool,
    pub stale_reasons: Vec<String>,
    /// Space (top-level folder) and its project.
    pub space: Option<String>,
    pub project: Option<String>,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocLink {
    pub target: String,
    pub target_path: Option<String>,
    pub resolution: String,
    pub matches: Vec<String>,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocRead {
    #[serde(flatten)]
    pub page: DocPage,
    pub content: String,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub heading: Option<String>,
    pub links: Vec<DocLink>,
    pub backlinks: Vec<String>,
    /// Policy that applies to people and agents writing this page.
    pub policy: Policy,
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DocSearchResult {
    #[serde(flatten)]
    pub page: DocPage,
    pub score: f64,
    pub snippet: String,
}

#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Restrict results to roots owned by this project, excluding shared pages.
    pub project: Option<String>,
    pub limit: usize,
    pub doc_type: Option<String>,
    pub status: Option<String>,
    pub spaces: Vec<String>,
    pub related: Vec<String>,
}

/// Result of a write: saved directly, or turned into a proposal by the section's policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteOutcome {
    Saved { created: bool, hash: String },
    NeedsReview,
}

pub struct Vault {
    root: PathBuf,
    index: Db,
    pub config: VaultConfig,
    /// Code repositories of projects (slug → path), for staleness.
    repos: HashMap<String, PathBuf>,
    stale_cache: HashMap<(String, String), (Instant, Vec<String>)>,
    commit: bool,
}

impl Vault {
    /// Open (and create if needed) a vault directory with its index file.
    pub fn open(root: &Path, index_file: &Path, commit: bool) -> Result<Vault> {
        std::fs::create_dir_all(root)?;
        let fresh = !root.join(".genie").exists();
        std::fs::create_dir_all(root.join(".genie"))?;
        if fresh && commit && !root.join(".git").exists() {
            let _ = std::process::Command::new("git").args(["init", "-q"]).current_dir(root).status();
        }
        let config = Self::load_config(root)?;
        let index = Db::open_with_schema(index_file, INDEX_SCHEMA)?;
        let mut v = Vault { root: root.canonicalize()?, index, config, repos: HashMap::new(), stale_cache: HashMap::new(), commit };
        if fresh {
            v.save_config()?;
        }
        v.refresh()?;
        Ok(v)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn load_config(root: &Path) -> Result<VaultConfig> {
        let file = root.join(".genie/vault.json");
        if !file.exists() {
            return Ok(VaultConfig::default());
        }
        let text = std::fs::read_to_string(&file)?;
        serde_json::from_str(&text).map_err(|e| GenieError::invalid(format!(".genie/vault.json: {e}")))
    }

    pub fn reload_config(&mut self) -> Result<()> {
        self.config = Self::load_config(&self.root)?;
        Ok(())
    }

    pub fn save_config(&self) -> Result<()> {
        let text = serde_json::to_string_pretty(&self.config)? + "\n";
        atomic_write(&self.root.join(".genie/vault.json"), text.as_bytes())?;
        self.git_commit(&[".genie/vault.json"], "genie: vault settings", "genie", "genie")
    }

    /// Make sure a project has a space (folder with a changelog) bound to it.
    pub fn ensure_space(&mut self, project: &str, repo: Option<&Path>) -> Result<String> {
        if let Some(r) = repo {
            self.repos.insert(project.to_string(), r.to_path_buf());
        }
        let existing = self.config.spaces.iter().find(|(_, s)| s.project.as_deref() == Some(project)).map(|(k, _)| k.clone());
        let name = match existing {
            Some(n) => n,
            None => {
                self.config.spaces.insert(project.to_string(), Space { project: Some(project.to_string()), ..Default::default() });
                self.save_config()?;
                project.to_string()
            }
        };
        let changelog = self.root.join(&name).join("changelog.md");
        if !changelog.exists() {
            std::fs::create_dir_all(self.root.join(&name))?;
            let text = format!(
                "---\ntitle: Changelog — {project}\ntype: reference\nstatus: current\ntags: [changelog]\n---\n\n# Changelog\n\n## Unreleased\n"
            );
            atomic_write(&changelog, text.as_bytes())?;
            self.git_commit(&[&format!("{name}/changelog.md")], &format!("genie: changelog for {project}"), "genie", "genie")?;
            self.refresh()?;
        }
        Ok(name)
    }

    /// Normalise a vault-relative page path; refuse anything outside the vault.
    pub fn resolve(&self, path: &str) -> Result<(String, PathBuf)> {
        let p = path.trim().trim_start_matches('/').replace('\\', "/");
        let p = if p.ends_with(".md") { p } else { format!("{p}.md") };
        let rel = Path::new(&p);
        if p.len() <= 3 || rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(GenieError::invalid(format!("invalid page path {path}")));
        }
        if rel.components().any(|c| c.as_os_str().to_string_lossy().starts_with('.')) {
            return Err(GenieError::invalid("pages cannot live in hidden folders"));
        }
        let abs = self.root.join(rel);
        // Refuse writing through symlinks that leave the vault.
        let mut cur = self.root.clone();
        for c in rel.components() {
            cur.push(c);
            if let Ok(meta) = std::fs::symlink_metadata(&cur)
                && meta.file_type().is_symlink()
            {
                return Err(GenieError::invalid("refusing to use a symlinked vault path"));
            }
        }
        Ok((p, abs))
    }

    fn space_of(&self, path: &str) -> Option<(String, &Space)> {
        let first = path.split('/').next()?;
        self.config.spaces.get(first).map(|s| (first.to_string(), s))
    }

    /// Effective policy for a page: its section, else its space, else the vault default.
    pub fn policy_for(&self, path: &str) -> Policy {
        if let Some((name, space)) = self.space_of(path) {
            let inner = path.strip_prefix(&name).unwrap_or_default().trim_start_matches('/');
            let section = space
                .sections
                .iter()
                .filter(|(k, _)| inner == k.as_str() || inner.starts_with(&format!("{k}/")))
                .max_by_key(|(k, _)| k.len());
            if let Some((_, s)) = section
                && let Some(p) = s.policy
            {
                return p;
            }
            if let Some(p) = space.policy {
                return p;
            }
        }
        self.config.policy.unwrap_or_default()
    }

    /// Logins that may approve proposals for a page (section, then space owners).
    pub fn owners_for(&self, path: &str) -> Vec<String> {
        let Some((name, space)) = self.space_of(path) else { return Vec::new() };
        let inner = path.strip_prefix(&name).unwrap_or_default().trim_start_matches('/');
        let mut owners: Vec<String> = space
            .sections
            .iter()
            .filter(|(k, s)| !s.owners.is_empty() && (inner == k.as_str() || inner.starts_with(&format!("{k}/"))))
            .flat_map(|(_, s)| s.owners.clone())
            .collect();
        if owners.is_empty() {
            owners = space.owners.clone();
        }
        owners
    }

    pub fn project_of(&self, path: &str) -> Option<String> {
        self.space_of(path).and_then(|(_, s)| s.project.clone())
    }

    /// Spaces bound to a project.
    pub fn spaces_of(&self, project: &str) -> Vec<String> {
        self.config.spaces.iter().filter(|(_, s)| s.project.as_deref() == Some(project)).map(|(k, _)| k.clone()).collect()
    }

    /// Resolve a page inside a project's configured documentation roots.
    /// Ownership comes from vault configuration, never page frontmatter.
    pub fn resolve_for_project(&self, project: &str, path: &str) -> Result<(String, PathBuf)> {
        let (rel, abs) = self.resolve(path)?;
        if self.project_of(&rel).as_deref() != Some(project) {
            return Err(GenieError::not_found("documentation page not found in this project"));
        }
        Ok((rel, abs))
    }

    pub fn project_signature(&self, project: &str) -> String {
        let mut h = Sha256::new();
        // Root bindings affect visibility even when no file changes.
        for root in self.spaces_of(project) {
            h.update(root.as_bytes());
        }
        for (rel, _, sig) in self.scan().into_iter().filter(|(rel, ..)| self.project_of(rel).as_deref() == Some(project)) {
            h.update(rel.as_bytes());
            h.update(sig.as_bytes());
        }
        hex::encode(h.finalize())
    }

    pub fn project_tree(&mut self, project: &str) -> Result<Vec<DocPage>> {
        self.tree_scoped(Some(project))
    }

    pub fn project_read(&mut self, project: &str, path: &str, heading: Option<&str>, max_chars: Option<usize>) -> Result<DocRead> {
        self.resolve_for_project(project, path)?;
        self.read_scoped(path, heading, max_chars, Some(project))
    }

    fn scan(&self) -> Vec<(String, PathBuf, String)> {
        let mut out = Vec::new();
        let mut stack = vec![self.root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') || name == "node_modules" {
                    continue;
                }
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_symlink() {
                    continue;
                }
                let path = e.path();
                if ft.is_dir() {
                    stack.push(path);
                } else if name.ends_with(".md")
                    && let Ok(meta) = e.metadata()
                {
                    let mtime = meta.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
                    let rel = path.strip_prefix(&self.root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                    out.push((rel, path, format!("{mtime}:{}", meta.len())));
                }
            }
        }
        out.sort();
        out
    }

    /// Signature of the vault's files: changes whenever a page is added, edited or removed.
    pub fn signature(&self) -> String {
        let mut h = Sha256::new();
        for (rel, _, sig) in self.scan() {
            h.update(rel.as_bytes());
            h.update(sig.as_bytes());
        }
        hex::encode(h.finalize())
    }

    /// Re-index changed pages; returns the paths that changed or disappeared.
    pub fn refresh(&mut self) -> Result<Vec<String>> {
        let files = self.scan();
        let known: HashMap<String, String> = {
            let mut stmt = self.index.conn().prepare("SELECT path, signature FROM pages")?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
        };
        let mut changed = Vec::new();
        let conn = self.index.conn();
        self.index.tx(|| {
            for (rel, abs, sig) in &files {
                if known.get(rel) == Some(sig) {
                    continue;
                }
                let Ok(bytes) = std::fs::read(abs) else { continue };
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let doc = parse_doc(&text, rel);
                let updated = std::fs::metadata(abs).ok().and_then(|m| m.modified().ok()).map(|t| chrono::DateTime::<chrono::Utc>::from(t).format("%Y-%m-%d").to_string());
                conn.execute("DELETE FROM pages_fts WHERE path = ?1", [rel])?;
                conn.execute(
                    "INSERT INTO pages(path, signature, title, type, status, summary, tags, aliases, paths, related, verified, project, content_hash, headings, diagnostics, links, updated)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                     ON CONFLICT(path) DO UPDATE SET signature=excluded.signature, title=excluded.title, type=excluded.type, status=excluded.status,
                       summary=excluded.summary, tags=excluded.tags, aliases=excluded.aliases, paths=excluded.paths, related=excluded.related,
                       verified=excluded.verified, project=excluded.project, content_hash=excluded.content_hash, headings=excluded.headings,
                       diagnostics=excluded.diagnostics, links=excluded.links, updated=excluded.updated",
                    params![
                        rel,
                        sig,
                        doc.title,
                        doc.doc_type,
                        doc.status,
                        doc.summary,
                        serde_json::to_string(&doc.tags)?,
                        serde_json::to_string(&doc.aliases)?,
                        doc.paths.as_ref().map(serde_json::to_string).transpose()?,
                        serde_json::to_string(&doc.related)?,
                        doc.verified,
                        doc.project,
                        sha(&bytes),
                        serde_json::to_string(&doc.headings)?,
                        serde_json::to_string(&doc.diagnostics)?,
                        serde_json::to_string(&doc.links)?,
                        updated,
                    ],
                )?;
                conn.execute(
                    "INSERT INTO pages_fts(path, title, summary, headings, body, tags) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        rel,
                        normalize(&doc.title),
                        normalize(doc.summary.as_deref().unwrap_or_default()),
                        normalize(&doc.headings.join("\n")),
                        normalize(&doc.body),
                        normalize(&[doc.tags.clone(), doc.aliases.clone()].concat().join(" ")),
                    ],
                )?;
                changed.push(rel.clone());
            }
            let present: std::collections::HashSet<&String> = files.iter().map(|f| &f.0).collect();
            for path in known.keys().filter(|p| !present.contains(p)) {
                conn.execute("DELETE FROM pages WHERE path = ?1", [path])?;
                conn.execute("DELETE FROM pages_fts WHERE path = ?1", [path])?;
                changed.push(path.clone());
            }
            Ok(())
        })?;
        Ok(changed)
    }

    fn page_row(&self, r: &rusqlite::Row<'_>) -> rusqlite::Result<DocPage> {
        let json = |s: String| serde_json::from_str::<Vec<String>>(&s).unwrap_or_default();
        let path: String = r.get("path")?;
        let space = self.space_of(&path).map(|(n, _)| n);
        let project_fm: Option<String> = r.get("project")?;
        Ok(DocPage {
            root_id: "vault".into(),
            project: project_fm.or_else(|| self.project_of(&path)),
            space,
            title: r.get("title")?,
            doc_type: r.get("type")?,
            status: r.get("status")?,
            summary: r.get("summary")?,
            tags: json(r.get("tags")?),
            aliases: json(r.get("aliases")?),
            paths: r.get::<_, Option<String>>("paths")?.map(json),
            related: json(r.get("related")?),
            verified: r.get("verified")?,
            updated: r.get("updated")?,
            content_hash: r.get("content_hash")?,
            headings: json(r.get("headings")?),
            diagnostics: json(r.get("diagnostics")?),
            stale: false,
            stale_reasons: Vec::new(),
            path,
        })
    }

    /// Code changes under the page's `paths` since `verified` (in the project's repository).
    fn staleness(&mut self, page: &mut DocPage) {
        let (Some(paths), Some(verified), Some(project)) = (page.paths.clone(), page.verified.clone(), page.project.clone()) else {
            return;
        };
        let Some(repo) = self.repos.get(&project).cloned() else { return };
        let key = (page.path.clone(), verified.clone());
        if let Some((at, reasons)) = self.stale_cache.get(&key)
            && at.elapsed() < Duration::from_secs(120)
        {
            page.stale_reasons = reasons.clone();
            page.stale = !reasons.is_empty();
            return;
        }
        let mut reasons = Vec::new();
        for pattern in &paths {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["log", "-1", "--format=%h", &format!("--since={verified} 23:59:59"), "--", &format!(":(glob){pattern}")])
                .output();
            if let Ok(o) = out
                && o.status.success()
            {
                let h = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !h.is_empty() {
                    reasons.push(format!("Commit changed {h}, matching paths pattern {pattern}"));
                }
            }
        }
        self.stale_cache.insert(key, (Instant::now(), reasons.clone()));
        page.stale = !reasons.is_empty();
        page.stale_reasons = reasons;
    }

    pub fn tree(&mut self) -> Result<Vec<DocPage>> {
        self.tree_scoped(None)
    }

    fn tree_scoped(&mut self, project: Option<&str>) -> Result<Vec<DocPage>> {
        let mut pages = {
            let mut stmt = self.index.conn().prepare("SELECT * FROM pages ORDER BY path")?;
            stmt.query_map([], |r| self.page_row(r))?.collect::<rusqlite::Result<Vec<_>>>()?
        };
        pages.retain(|p| project.is_none_or(|project| self.project_of(&p.path).as_deref() == Some(project)));
        for p in &mut pages {
            self.staleness(p);
        }
        Ok(pages)
    }

    fn page(&self, path: &str) -> Result<Option<DocPage>> {
        Ok(self.index.conn().query_row("SELECT * FROM pages WHERE path = ?1", [path], |r| self.page_row(r)).optional()?)
    }

    fn all_link_targets(&self) -> Result<Vec<(String, String, Vec<String>)>> {
        let mut stmt = self.index.conn().prepare("SELECT path, links, aliases FROM pages")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
        rows.map(|r| {
            let (p, l, a) = r?;
            Ok((p, l, serde_json::from_str(&a).unwrap_or_default()))
        })
        .collect()
    }

    /// Resolve a wiki-link target: canonical path, then unique file name or alias.
    fn resolve_link(target: &str, pages: &[(String, String, Vec<String>)]) -> DocLink {
        let t = target.trim().trim_start_matches("./").trim_end_matches(".md");
        let exact = format!("{t}.md");
        if pages.iter().any(|(p, ..)| *p == exact || (t.contains('.') && p == t)) {
            let path = if pages.iter().any(|(p, ..)| *p == exact) { exact } else { t.to_string() };
            return DocLink { target: target.into(), target_path: Some(path.clone()), resolution: "resolved".into(), matches: vec![path] };
        }
        let base = normalize(t.rsplit('/').next().unwrap_or(t));
        let matches: Vec<String> = pages
            .iter()
            .filter(|(p, _, aliases)| {
                let stem = normalize(Path::new(p).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default().as_str());
                stem == base || aliases.iter().any(|a| normalize(a) == normalize(t))
            })
            .map(|(p, ..)| p.clone())
            .collect();
        match matches.len() {
            1 => DocLink { target: target.into(), target_path: Some(matches[0].clone()), resolution: "resolved".into(), matches },
            0 => DocLink { target: target.into(), target_path: None, resolution: "unresolved".into(), matches },
            _ => DocLink { target: target.into(), target_path: None, resolution: "ambiguous".into(), matches },
        }
    }

    /// Read a page (or the section under `heading`), with links and backlinks.
    pub fn read(&mut self, path: &str, heading: Option<&str>, max_chars: Option<usize>) -> Result<DocRead> {
        self.read_scoped(path, heading, max_chars, None)
    }

    fn read_scoped(&mut self, path: &str, heading: Option<&str>, max_chars: Option<usize>, project: Option<&str>) -> Result<DocRead> {
        let (rel, abs) = self.resolve(path)?;
        let mut page = self.page(&rel)?.ok_or_else(|| GenieError::not_found(format!("documentation page {rel} not found")))?;
        self.staleness(&mut page);
        let file = std::fs::read_to_string(&abs).map_err(|_| GenieError::not_found(format!("documentation page {rel} not found")))?;
        // The page's fields travel in `page`; `content` is the Markdown body, as in the TS contract
        // (the web editor saves frontmatter + body, so a body with frontmatter would duplicate it).
        let text = parse_doc(&file, &rel).body;
        let mut content = text.clone();
        if let Some(h) = heading {
            let lines: Vec<&str> = text.lines().collect();
            let level_of = |l: &str| {
                let t = l.trim_start();
                let n = t.chars().take_while(|c| *c == '#').count();
                (n > 0 && n <= 6 && t[n..].starts_with(' ')).then_some(n)
            };
            let start =
                lines.iter().position(|l| level_of(l).is_some() && normalize(l.trim_start_matches('#').trim()) == normalize(h.trim()));
            let Some(start) = start else { return Err(GenieError::not_found(format!("heading {h} not found in {rel}"))) };
            let level = level_of(lines[start]).unwrap_or(1);
            let end = lines
                .iter()
                .skip(start + 1)
                .position(|l| level_of(l).is_some_and(|n| n <= level))
                .map(|i| start + 1 + i)
                .unwrap_or(lines.len());
            content = lines[start..end].join("\n");
        }
        let mut truncated = false;
        if let Some(max) = max_chars
            && content.chars().count() > max
        {
            content = content.chars().take(max).collect();
            truncated = true;
        }
        let all: Vec<_> = self
            .all_link_targets()?
            .into_iter()
            .filter(|(path, ..)| project.is_none_or(|project| self.project_of(path).as_deref() == Some(project)))
            .collect();
        let own: Vec<String> =
            all.iter().find(|(p, ..)| *p == rel).map(|(_, l, _)| serde_json::from_str(l).unwrap_or_default()).unwrap_or_default();
        let links: Vec<DocLink> = own.iter().map(|t| Self::resolve_link(t, &all)).collect();
        let backlinks: Vec<String> = all
            .iter()
            .filter(|(p, ..)| *p != rel)
            .filter(|(_, l, _)| {
                let targets: Vec<String> = serde_json::from_str(l).unwrap_or_default();
                targets.iter().any(|t| Self::resolve_link(t, &all).target_path.as_deref() == Some(rel.as_str()))
            })
            .map(|(p, ..)| p.clone())
            .collect();
        Ok(DocRead { policy: self.policy_for(&rel), page, content, truncated, heading: heading.map(str::to_string), links, backlinks })
    }

    /// Full-text search with BM25 ranking and snippets marked `[…]`.
    pub fn search(&mut self, query: &str, opts: &SearchOptions) -> Result<Vec<DocSearchResult>> {
        let terms: Vec<String> = normalize(query)
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(|t| format!("\"{}\"*", t.replace('"', "")))
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let fts = terms.join(" OR ");
        let limit = if opts.limit == 0 { 20 } else { opts.limit.min(100) };
        let roots = opts.project.as_ref().map(|project| self.spaces_of(project));
        let roots_json = serde_json::to_string(&roots.clone().unwrap_or_default())?;
        let mut stmt = self.index.conn().prepare(
            "SELECT p.*, bm25(pages_fts, 0.0, 8.0, 4.0, 3.0, 1.0, 5.0) AS score, snippet(pages_fts, 4, '[', ']', '…', 12) AS snip
             FROM pages_fts JOIN pages p ON p.path = pages_fts.path WHERE pages_fts MATCH ?1
             AND (?2 = 0 OR substr(p.path, 1, instr(p.path, '/') - 1) IN (SELECT value FROM json_each(?3)))
             ORDER BY score LIMIT 400",
        )?;
        let rows = stmt.query_map(params![&fts, roots.is_some(), roots_json], |r| {
            Ok((self.page_row(r)?, r.get::<_, f64>("score")?, r.get::<_, String>("snip")?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (page, score, snippet) = row?;
            let deprecated = page.status.as_deref() == Some("deprecated");
            let related = page.related.iter().any(|r| opts.related.contains(r));
            if deprecated && opts.status.as_deref() != Some("deprecated") && !related {
                continue;
            }
            if opts.doc_type.as_ref().is_some_and(|t| page.doc_type.as_ref() != Some(t))
                || opts.status.as_ref().is_some_and(|s| page.status.as_ref() != Some(s))
            {
                continue;
            }
            if !opts.spaces.is_empty() && !page.space.as_ref().is_some_and(|s| opts.spaces.contains(s)) && page.space.is_some() {
                continue;
            }
            out.push(DocSearchResult { page, score: -score, snippet });
            if out.len() >= limit {
                break;
            }
        }
        Ok(out)
    }

    /// Content hash of a page on disk (`None` when it does not exist).
    pub fn current_hash(&self, path: &str) -> Result<Option<String>> {
        let (_, abs) = self.resolve(path)?;
        Ok(std::fs::read(&abs).ok().map(|b| sha(&b)))
    }

    /// Write a page. `base_hash`: the content hash the author started from
    /// (`Some("")` for a new page); a different current hash is a conflict.
    pub fn write(
        &mut self,
        path: &str,
        content: &str,
        kind: AuthorKind,
        author: (&str, &str),
        base_hash: Option<&str>,
        message: &str,
    ) -> Result<WriteOutcome> {
        let (rel, abs) = self.resolve(path)?;
        if content.trim().is_empty() {
            return Err(GenieError::invalid("page content must not be empty"));
        }
        let parsed = parse_doc(content, &rel);
        if !parsed.diagnostics.is_empty() {
            return Err(GenieError::invalid(format!("invalid documentation metadata: {}", parsed.diagnostics.join("; "))));
        }
        let policy = self.policy_for(&rel);
        let rule = match kind {
            AuthorKind::Human => policy.humans,
            AuthorKind::Agent => policy.agents,
            AuthorKind::System => Publish::Direct,
        };
        match rule {
            Publish::Locked => {
                return Err(GenieError::Denied(format!(
                    "{rel} is locked for {}",
                    if kind == AuthorKind::Agent { "agents" } else { "you" }
                )));
            }
            Publish::Review => return Ok(WriteOutcome::NeedsReview),
            Publish::Direct => {}
        }
        let current = std::fs::read(&abs).ok().map(|b| sha(&b));
        if let Some(base) = base_hash {
            let matches = match (&current, base) {
                (None, "") => true,
                (Some(c), b) => c == b,
                (None, _) => false,
            };
            if !matches {
                return Err(GenieError::invalid(format!("conflict: {rel} changed since you opened it; reload and apply your edit again")));
            }
        }
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        atomic_write(&abs, content.as_bytes())?;
        self.git_commit(&[&rel], message, author.0, author.1)?;
        self.refresh()?;
        Ok(WriteOutcome::Saved { created: current.is_none(), hash: sha(content.as_bytes()) })
    }

    fn git_commit(&self, paths: &[&str], message: &str, name: &str, login: &str) -> Result<()> {
        if !self.commit || !self.root.join(".git").exists() {
            return Ok(());
        }
        let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&self.root).args(args).output();
        let _ = git(&[&["add", "--"], paths].concat());
        let (user, email, author) =
            (format!("user.name={name}"), format!("user.email={login}@genie.local"), format!("{name} <{login}@genie.local>"));
        // "Nothing to commit" is fine, and a failed commit never loses the write itself.
        let _ = git(&[&["-c", &user, "-c", &email, "commit", "-q", "--author", &author, "-m", message, "--"], paths].concat());
        Ok(())
    }

    // --- changelog ---------------------------------------------------------------

    /// Add an entry to `## Unreleased` of a space's changelog under its group.
    pub fn changelog_add(&mut self, space: &str, group: &str, text: &str, task: Option<&str>, russian: bool) -> Result<()> {
        let group = changelog_group(group, russian);
        let rel = format!("{space}/changelog.md");
        let (_, abs) = self.resolve(&rel)?;
        let current = std::fs::read_to_string(&abs).unwrap_or_else(|_| "# Changelog\n\n## Unreleased\n".into());
        let entry = format!("- {}{}", text.trim().trim_start_matches("- "), task.map(|t| format!(" ({t})")).unwrap_or_default());
        let updated = insert_changelog_entry(&current, &group, &entry);
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent)?;
        }
        atomic_write(&abs, updated.as_bytes())?;
        self.git_commit(&[&rel], &format!("changelog: {}", text.chars().take(60).collect::<String>()), "genie", "genie")?;
        self.refresh()?;
        Ok(())
    }

    /// Turn `## Unreleased` into `## [version] — date`; returns the released notes.
    pub fn changelog_release(&mut self, space: &str, version: &str, date: &str) -> Result<String> {
        let rel = format!("{space}/changelog.md");
        let (_, abs) = self.resolve(&rel)?;
        let text = std::fs::read_to_string(&abs).map_err(|_| GenieError::not_found(format!("{rel} not found")))?;
        let Some(start) = text.find("## Unreleased") else { return Err(GenieError::invalid("the changelog has no Unreleased section")) };
        let body_start = start + "## Unreleased".len();
        let end = text[body_start..].find("\n## ").map(|i| body_start + i + 1).unwrap_or(text.len());
        let notes = text[body_start..end].trim().to_string();
        if notes.is_empty() {
            return Err(GenieError::invalid("nothing to release: Unreleased is empty"));
        }
        let released = format!("{}## Unreleased\n\n## [{version}] — {date}\n\n{notes}\n\n{}", &text[..start], text[end..].trim_start());
        atomic_write(&abs, released.trim_end().as_bytes())?;
        self.git_commit(&[&rel], &format!("release {version}"), "genie", "genie")?;
        self.refresh()?;
        Ok(notes)
    }
}

fn changelog_group(group: &str, russian: bool) -> String {
    let g = group.trim().to_lowercase();
    let (en, ru) = match g.as_str() {
        "added" | "add" | "добавлено" => ("Added", "Добавлено"),
        "fixed" | "fix" | "исправлено" => ("Fixed", "Исправлено"),
        "removed" | "удалено" => ("Removed", "Удалено"),
        "security" | "безопасность" => ("Security", "Безопасность"),
        "deprecated" | "устарело" => ("Deprecated", "Устарело"),
        _ => ("Changed", "Изменено"),
    };
    (if russian { ru } else { en }).to_string()
}

/// Insert `entry` under `### group` inside `## Unreleased` (both created if missing).
pub fn insert_changelog_entry(text: &str, group: &str, entry: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let unreleased = match lines.iter().position(|l| l.trim() == "## Unreleased") {
        Some(i) => i,
        None => {
            let at = lines.iter().position(|l| l.starts_with("## ")).unwrap_or(lines.len());
            lines.splice(at..at, ["## Unreleased".to_string(), String::new()]);
            at
        }
    };
    let section_end =
        lines.iter().skip(unreleased + 1).position(|l| l.starts_with("## ")).map(|i| unreleased + 1 + i).unwrap_or(lines.len());
    let heading = format!("### {group}");
    match lines[unreleased + 1..section_end].iter().position(|l| l.trim() == heading) {
        Some(rel) => {
            let h = unreleased + 1 + rel;
            let group_end = lines[h + 1..section_end].iter().position(|l| l.starts_with("### ")).map(|i| h + 1 + i).unwrap_or(section_end);
            let mut at = group_end;
            while at > h + 1 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            lines.insert(at, entry.to_string());
        }
        None => {
            let mut at = section_end;
            while at > unreleased + 1 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            let block = vec![String::new(), heading, String::new(), entry.to_string()];
            lines.splice(at..at, block);
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().ok_or_else(|| GenieError::invalid("no parent directory"))?;
    let mut rnd = [0u8; 6];
    getrandom::fill(&mut rnd).expect("OS random source");
    let tmp =
        dir.join(format!(".{}.{}.tmp", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), hex::encode(rnd)));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let v = Vault::open(&dir.path().join("vault"), &dir.path().join("index.db"), true).unwrap();
        (dir, v)
    }

    const PAGE: &str = "---\ntitle: Авторизация\ntype: guide\nstatus: current\ntags: [auth]\naliases: [вход]\ncssclasses: wide\nlinks:\n  - obsidian-only\n---\n\n# Авторизация\n\nКак выдаются ёлочные сессии. См. [[shop/glossary|глоссарий]] и ![[shot.png]].\n\n`[[not-a-link]]`\n\n```\n[[also-not]]\n```\n\n## Сессии\n\nТокены живут 30 дней #security\n";

    #[test]
    fn parses_obsidian_pages_and_links_outside_code() {
        let d = parse_doc(PAGE, "shop/auth.md");
        assert!(d.diagnostics.is_empty(), "unknown Obsidian properties are ignored: {:?}", d.diagnostics);
        assert_eq!(d.title, "Авторизация");
        assert_eq!(d.links, vec!["shop/glossary", "shot.png"]);
        assert_eq!(d.headings, vec!["Авторизация", "Сессии"]);
        assert!(d.tags.contains(&"security".to_string()) && d.tags.contains(&"auth".to_string()));
        let bad = parse_doc("---\ntype: poem\nverified: 2026-13-40\n---\nx", "a.md");
        assert_eq!(bad.diagnostics.len(), 2);
    }

    #[test]
    fn search_read_links_and_backlinks() {
        let (_d, mut v) = vault();
        v.ensure_space("shop", None).unwrap();
        v.write("shop/auth", PAGE, AuthorKind::Human, ("Анна", "anna"), None, "auth").unwrap();
        v.write("shop/glossary.md", "# Глоссарий\n\nСессия — период входа.", AuthorKind::Human, ("Анна", "anna"), Some(""), "glossary")
            .unwrap();
        let hits = v.search("елочные", &SearchOptions::default()).unwrap();
        assert_eq!(hits[0].page.path, "shop/auth.md", "ё/е match both ways");
        assert!(hits[0].snippet.contains('['));
        let r = v.read("shop/auth.md", None, None).unwrap();
        assert!(!r.content.starts_with("---") && !r.content.contains("paths:"), "content is the body: {}", r.content);
        assert_eq!(r.links[0].resolution, "resolved");
        assert_eq!(r.links[1].resolution, "unresolved", "missing attachment");
        let g = v.read("shop/glossary", None, None).unwrap();
        assert_eq!(g.backlinks, vec!["shop/auth.md"]);
        let s = v.read("shop/auth.md", Some("Сессии"), None).unwrap();
        assert!(s.content.starts_with("## Сессии") && !s.content.contains("Как выдаются"));
        assert!(v.read("../etc/passwd", None, None).is_err());
        assert!(v.resolve(".genie/vault.json").is_err());
    }

    #[test]
    fn policies_conflicts_and_obsidian_edits() {
        let (_d, mut v) = vault();
        v.ensure_space("shop", None).unwrap();
        v.config.spaces.get_mut("shop").unwrap().sections.insert(
            "decisions".into(),
            Section { policy: Some(Policy { humans: Publish::Review, agents: Publish::Locked }), owners: vec!["lead".into()] },
        );
        assert_eq!(v.write("shop/notes.md", "# N\n\nx", AuthorKind::Agent, ("bot", "bot"), None, "m").unwrap(), WriteOutcome::NeedsReview);
        assert!(matches!(
            v.write("shop/notes.md", "# N\n\nx", AuthorKind::Human, ("a", "a"), None, "m").unwrap(),
            WriteOutcome::Saved { created: true, .. }
        ));
        assert!(matches!(
            v.write("shop/decisions/adr-1.md", "# D\n\nx", AuthorKind::Agent, ("bot", "bot"), None, "m"),
            Err(GenieError::Denied(_))
        ));
        assert_eq!(
            v.write("shop/decisions/adr-1.md", "# D\n\nx", AuthorKind::Human, ("a", "a"), None, "m").unwrap(),
            WriteOutcome::NeedsReview
        );
        assert_eq!(v.owners_for("shop/decisions/adr-1.md"), vec!["lead"]);
        let base = v.current_hash("shop/notes.md").unwrap().unwrap();
        // Someone edits the file in Obsidian.
        std::fs::write(v.root().join("shop/notes.md"), "# N\n\nedited in Obsidian").unwrap();
        let err = v.write("shop/notes.md", "# N\n\nmine", AuthorKind::Human, ("a", "a"), Some(&base), "m").unwrap_err();
        assert!(err.to_string().contains("conflict"));
        assert!(v.refresh().unwrap().contains(&"shop/notes.md".to_string()));
        assert!(v.search("obsidian", &SearchOptions::default()).unwrap().iter().any(|h| h.page.path == "shop/notes.md"));
        let log = std::process::Command::new("git").arg("-C").arg(v.root()).args(["log", "--format=%an|%s"]).output().unwrap();
        let log = String::from_utf8_lossy(&log.stdout);
        assert!(log.contains("a|m"), "writes are committed with the author:\n{log}");
    }

    #[test]
    fn changelog_entries_and_release() {
        let (_d, mut v) = vault();
        v.ensure_space("shop", None).unwrap();
        v.changelog_add("shop", "fixed", "Экспорт больше не теряет строки", Some("SHOP-7"), true).unwrap();
        v.changelog_add("shop", "added", "Экспорт в CSV", Some("SHOP-3"), true).unwrap();
        v.changelog_add("shop", "fixed", "Верная кодировка", None, true).unwrap();
        let text = std::fs::read_to_string(v.root().join("shop/changelog.md")).unwrap();
        let fixed = text.find("### Исправлено").unwrap();
        assert!(text[fixed..].contains("- Экспорт больше не теряет строки (SHOP-7)\n- Верная кодировка"), "{text}");
        let notes = v.changelog_release("shop", "1.0.0", "2026-09-28").unwrap();
        assert!(notes.contains("Экспорт в CSV"));
        let text = std::fs::read_to_string(v.root().join("shop/changelog.md")).unwrap();
        let unreleased = text.find("## Unreleased").unwrap();
        let release = text.find("## [1.0.0] — 2026-09-28").unwrap();
        assert!(unreleased < release);
        assert!(v.changelog_release("shop", "1.0.1", "2026-09-29").is_err(), "nothing to release");
    }

    #[test]
    fn stale_pages_follow_code_changes_in_the_project_repo() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(&repo).args(args).output().unwrap();
        git(&["init", "-q"]);
        std::fs::write(repo.join("src/auth.rs"), "fn a() {}").unwrap();
        git(&["add", "."]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "-m", "auth"]);
        let mut v = Vault::open(&dir.path().join("vault"), &dir.path().join("i.db"), true).unwrap();
        v.ensure_space("shop", Some(&repo)).unwrap();
        v.write("shop/auth.md", "---\npaths: [src/**]\nverified: 2020-01-01\n---\n# A\n\nx", AuthorKind::Human, ("a", "a"), None, "m")
            .unwrap();
        v.write("shop/other.md", "---\npaths: [lib/**]\nverified: 2020-01-01\n---\n# O\n\nx", AuthorKind::Human, ("a", "a"), None, "m")
            .unwrap();
        let tree = v.tree().unwrap();
        let get = |p: &str| tree.iter().find(|x| x.path == p).unwrap().clone();
        assert!(get("shop/auth.md").stale);
        assert!(!get("shop/other.md").stale);
    }
}
