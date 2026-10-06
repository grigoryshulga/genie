//! Agent configuration: roles, team templates, skills and MCP connections
//! (docs/platform/agent-roles-and-teams.md, decisions PD13–PD16).
//!
//! genie ships built-in roles (`agents/*.md` of the repository) and team presets
//! (`config/teams/*.json`), compiled into the binary. The server's data directory
//! adds to them or overrides them — the administrator's configuration:
//!
//! - `<data>/agents/<id>.md` — a role: settings in a flat frontmatter, the prompt
//!   in the body. A file with a built-in id overrides that role field by field
//!   (an empty body keeps the built-in prompt); a new id adds a role, based on a
//!   class (`base`) or on another role (`extends`);
//! - `<data>/teams/<id>.json` — a team template; the legacy `teams` key of
//!   `<data>/config.json` is read too (a file with the same id wins);
//! - `<data>/skills/<name>/SKILL.md` and the directories in `skills.paths` — skills;
//! - `<data>/mcp.json` — MCP connections in the `mcpServers` format.
//!
//! Loading never fails as a whole. A broken file is reported in `problems`; the
//! item keeps its previous valid version (or the built-in one), so a typo in a
//! role does not stop the teams that use it.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use genie_core::{Capability, GenieError, MEMBER_ROLES, Role, Status, adjust_capabilities, class_capabilities};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::config::Config;
use crate::state::App;

const BUILTIN_ROLES: &[(&str, &str)] = &[
    ("orchestrator", include_str!("../../../agents/orchestrator.md")),
    ("analyst", include_str!("../../../agents/analyst.md")),
    ("executor", include_str!("../../../agents/executor.md")),
    ("reviewer", include_str!("../../../agents/reviewer.md")),
    ("tester", include_str!("../../../agents/tester.md")),
    ("documenter", include_str!("../../../agents/documenter.md")),
    ("researcher", include_str!("../../../agents/researcher.md")),
    ("planner", include_str!("../../../agents/planner.md")),
];

const BUILTIN_TEAMS: &[(&str, &str)] = &[
    ("standard", include_str!("../../../config/teams/standard.json")),
    ("pair", include_str!("../../../config/teams/pair.json")),
    ("full", include_str!("../../../config/teams/full.json")),
    ("abap", include_str!("../../../config/teams/abap.json")),
    ("spike", include_str!("../../../config/teams/spike.json")),
    ("research", include_str!("../../../config/teams/research.json")),
    ("idea", include_str!("../../../config/teams/idea.json")),
];

/// Who the reserved relation endpoint is.
pub const ORCHESTRATOR: &str = "orchestrator";

/// The shipped file of a built-in role (what an override starts from).
pub fn builtin_role_text(id: &str) -> Option<&'static str> {
    BUILTIN_ROLES.iter().find(|(b, _)| *b == id).map(|(_, t)| *t)
}

/// The shipped file of a built-in team preset.
pub fn builtin_team_text(id: &str) -> Option<&'static str> {
    BUILTIN_TEAMS.iter().find(|(b, _)| *b == id).map(|(_, t)| *t)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileAccess {
    Write,
    Read,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Refinement,
    Delivery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Workspace {
    Worktree,
    Repo,
    Scratch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MailMode {
    Open,
    Flow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RelKind {
    Handoff,
    Returns,
    Reports,
    Consults,
}

/// Where an item comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// Compiled into genie.
    Builtin,
    /// A data-directory file overriding a built-in item.
    Override,
    /// A data-directory file with a new id.
    Custom,
    /// The `teams` key of `<data>/config.json`.
    Legacy,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDef {
    pub id: String,
    pub title: String,
    pub description: String,
    /// The process class: permissions by default, the name pool, the history label.
    pub class: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    pub names: Vec<String>,
    pub allow: Vec<Capability>,
    pub deny: Vec<Capability>,
    /// The effective permissions: the class's set with `allow` and `deny` applied.
    pub capabilities: Vec<Capability>,
    pub files: FileAccess,
    /// What the role may do in the project's repositories (`none`, `read`, `write`) at most;
    /// `None`: what `files` allows (write → write, read → read, none → none).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    pub deny_commands: Vec<String>,
    /// `None`: every skill installed on the machine (the harness default).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    /// MCP grants: `server`, `server:tool-pattern` or `*`.
    pub mcp: Vec<String>,
    pub stages: Vec<Stage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projects: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    pub prompt: String,
    pub origin: Origin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The file is broken; this is its last valid version.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
}

impl RoleDef {
    pub fn available_in(&self, project: &str) -> bool {
        self.projects.as_ref().is_none_or(|p| p.iter().any(|x| x == project))
    }

    pub fn can(&self, cap: Capability) -> bool {
        self.class == Role::Orchestrator || self.capabilities.contains(&cap)
    }

    /// The prompt with the role's extra instructions.
    pub fn full_prompt(&self) -> String {
        match self.instructions.as_deref().map(str::trim).filter(|i| !i.is_empty()) {
            Some(i) => format!("{}\n\n## Specifics of this role\n\n{i}\n", self.prompt.trim_end()),
            None => self.prompt.clone(),
        }
    }

    /// `--exclude-tools` for the harness: read-only roles lose edit and write.
    pub fn excluded_tools(&self) -> Option<String> {
        (self.files != FileAccess::Write).then(|| "edit,write".to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberDef {
    /// How relations name this member: the role id, unless the template sets another key.
    pub key: String,
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    pub from: String,
    pub to: Vec<String>,
    #[serde(rename = "type")]
    pub kind: RelKind,
    /// genie itself hands over when the task enters this status.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamDef {
    pub id: String,
    pub title: String,
    pub description: String,
    pub stage: Stage,
    pub workspace: Workspace,
    pub mail: MailMode,
    pub members: Vec<MemberDef>,
    pub relations: Vec<Relation>,
    /// The template has no `relations`: they were derived from the members' classes.
    pub relations_derived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projects: Option<Vec<String>>,
    pub origin: Origin,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
}

impl TeamDef {
    pub fn available_in(&self, project: &str) -> bool {
        self.projects.as_ref().is_none_or(|p| p.iter().any(|x| x == project))
    }
}

/// How a running team works, fixed when it is assembled and stored with the team.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TeamSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub stage: Stage,
    pub workspace: Workspace,
    pub mail: MailMode,
    pub members: Vec<SpecMember>,
    pub relations: Vec<Relation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub charter: Option<String>,
    /// The template as the team took it (`template_hash`): a later edit of the
    /// template shows on the team, which keeps working by its snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_hash: Option<String>,
    /// The person the team works on behalf of (a login): whose LiteLLM key its agents use.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initiator: Option<String>,
}

/// What a team takes from a template, as a hash (`TeamSpec::template_hash`).
pub fn template_hash(t: &TeamDef) -> String {
    let v = json!({ "stage": t.stage, "workspace": t.workspace, "mail": t.mail, "members": t.members, "relations": t.relations, "charter": t.charter });
    genie_core::server_db::hash_secret(&v.to_string())
}

/// A member of a running team: its key in the relations, its name, its role id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpecMember {
    pub key: String,
    pub name: String,
    pub role: String,
}

/// A key no member in `members` has: the role id, else `role-2`, `role-3`…
pub fn free_key(members: &[SpecMember], role: &str) -> String {
    let taken = |k: &str| members.iter().any(|m| m.key == k);
    if !taken(role) {
        return role.to_string();
    }
    (2..).map(|i| format!("{role}-{i}")).find(|k| !taken(k)).unwrap_or_default()
}

impl TeamSpec {
    pub fn from_value(v: &Value) -> Option<TeamSpec> {
        serde_json::from_value(v.clone()).ok()
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    pub fn by_key(&self, key: &str) -> Option<&SpecMember> {
        self.members.iter().find(|m| m.key == key)
    }

    pub fn by_name(&self, name: &str) -> Option<&SpecMember> {
        self.members.iter().find(|m| m.name == name)
    }

    /// A free key for another member with role `role` (`reviewer`, `reviewer-2`…).
    pub fn free_key(&self, role: &str) -> String {
        free_key(&self.members, role)
    }

    /// Teammates the member `key` may write to in a `flow` team: along its
    /// handoff, returns and consults relations.
    pub fn flow_targets(&self, key: &str) -> Vec<&SpecMember> {
        let mut out: Vec<&SpecMember> = Vec::new();
        for r in self.relations.iter().filter(|r| r.from == key && r.kind != RelKind::Reports) {
            for t in &r.to {
                if let Some(m) = self.by_key(t)
                    && !out.iter().any(|x| x.key == m.key)
                {
                    out.push(m);
                }
            }
        }
        out
    }

    /// Whether the member `key` is a voice of the team to the orchestrator
    /// (`reports`); in a team where nobody reports, everyone is.
    pub fn is_voice(&self, key: &str) -> bool {
        let mut reporters = self.relations.iter().filter(|r| r.kind == RelKind::Reports).peekable();
        reporters.peek().is_none() || reporters.any(|r| r.from == key)
    }

    /// Why mail from the member named `from` to `to` (a member name,
    /// `orchestrator` or `all`) leaves the template's route in a `flow` team.
    /// `None`: it follows the route, the team's mail is open, or the sender is
    /// not a member (the orchestrator, a person). Answers (`genie mail reply`)
    /// are not checked: they always go back to whoever asked.
    pub fn flow_refusal(&self, from: &str, to: &str, intent: Option<&str>) -> Option<String> {
        if self.mail != MailMode::Flow {
            return None;
        }
        let me = self.by_name(from)?;
        let targets = self.flow_targets(&me.key);
        let voice = self.is_voice(&me.key);
        let route = format!(
            "You may write to {}{}; answer questions with `genie mail reply <id>`",
            if targets.is_empty() {
                "no teammate".to_string()
            } else {
                targets.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", ")
            },
            if voice {
                " and the orchestrator"
            } else {
                ", and send the orchestrator only questions and blockers (--intent question or blocker)"
            }
        );
        if to == ORCHESTRATOR {
            if voice || matches!(intent, Some("question" | "blocker")) {
                return None;
            }
            let voices: Vec<&str> = self
                .relations
                .iter()
                .filter(|r| r.kind == RelKind::Reports)
                .filter_map(|r| self.by_key(&r.from).map(|m| m.name.as_str()))
                .collect();
            return Some(format!(
                "this team's mail follows its template (mail: flow): {} report to the orchestrator. {route}.",
                voices.join(", ")
            ));
        }
        if to == genie_core::team::BROADCAST {
            return Some(format!("this team's mail follows its template (mail: flow): no mail to everyone. {route}."));
        }
        if targets.iter().any(|m| m.name == to) {
            return None;
        }
        Some(format!("{to} is not on your route in this team (mail: flow). {route}."))
    }

    /// Relations derived from the members' classes (teams without a template's relations).
    pub fn derived(members: Vec<SpecMember>, classes: &[Role], refinement: bool) -> TeamSpec {
        let keyed: Vec<(String, Role)> = members.iter().zip(classes).map(|(m, c)| (m.key.clone(), *c)).collect();
        TeamSpec {
            template: None,
            title: None,
            stage: if refinement { Stage::Refinement } else { Stage::Delivery },
            workspace: Workspace::Worktree,
            mail: MailMode::Open,
            relations: default_relations(&keyed, refinement),
            members,
            charter: None,
            template_hash: None,
            initiator: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDef {
    pub name: String,
    pub description: String,
    /// The skill's directory (holds `SKILL.md`).
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub id: String,
    pub description: String,
    /// `stdio` (a command) or `http` (a URL).
    pub transport: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projects: Option<Vec<String>>,
    /// Agents reach it through the genie gateway (default), which keeps its
    /// secrets and records the calls; `"gateway": false` hands the harness the
    /// entry itself.
    pub gateway: bool,
    /// The entry as the harness gets it (without genie's own fields); may hold
    /// `${env:NAME}` references, so it is never serialised for the web.
    #[serde(skip)]
    pub config: Value,
}

impl McpServer {
    pub fn available_in(&self, project: &str) -> bool {
        self.projects.as_ref().is_none_or(|p| p.iter().any(|x| x == project))
    }

    /// The environment variables the entry refers to (`${env:NAME}`).
    pub fn env_refs(&self) -> Vec<String> {
        fn walk(v: &Value, out: &mut Vec<String>) {
            match v {
                Value::String(s) => {
                    let mut rest = s.as_str();
                    while let Some(start) = rest.find("${env:") {
                        let after = &rest[start + 6..];
                        let Some(end) = after.find('}') else { break };
                        out.push(after[..end].to_string());
                        rest = &after[end + 1..];
                    }
                }
                Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
                Value::Object(o) => o.values().for_each(|x| walk(x, out)),
                _ => {}
            }
        }
        let mut out = Vec::new();
        walk(&self.config, &mut out);
        out.sort();
        out.dedup();
        out
    }

    /// The entry with `${env:NAME}` replaced from the server's environment.
    pub fn resolved(&self) -> Value {
        fn walk(v: &Value) -> Value {
            match v {
                Value::String(s) => Value::String(expand_env(s)),
                Value::Array(a) => Value::Array(a.iter().map(walk).collect()),
                Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), walk(v))).collect()),
                other => other.clone(),
            }
        }
        walk(&self.config)
    }
}

fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find("${env:") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 6..];
        match after.find('}') {
            Some(end) => {
                out.push_str(&std::env::var(&after[..end]).unwrap_or_default());
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warning,
}

/// Something wrong in the configuration, shown in the web and by `genie agents check`.
#[derive(Debug, Clone, Serialize)]
pub struct Problem {
    pub level: Level,
    /// `role:<id>`, `team:<id>`, `skill:<name>`, `mcp:<id>` or `mcp.json`.
    pub item: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentConfig {
    pub roles: BTreeMap<String, RoleDef>,
    pub teams: BTreeMap<String, TeamDef>,
    pub skills: BTreeMap<String, SkillDef>,
    pub mcp: BTreeMap<String, McpServer>,
    pub problems: Vec<Problem>,
    #[serde(skip)]
    pub signature: String,
}

fn valid_id(id: &str) -> bool {
    let mut c = id.chars();
    matches!(c.next(), Some(f) if f.is_ascii_lowercase()) && c.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_member_name(name: &str) -> bool {
    let mut c = name.chars();
    matches!(c.next(), Some(f) if f.is_ascii_lowercase()) && c.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

// --- frontmatter ------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Fm {
    Str(String),
    List(Vec<String>),
}

impl Fm {
    fn text(&self) -> String {
        match self {
            Fm::Str(s) => s.clone(),
            Fm::List(l) => l.join(", "),
        }
    }
    /// A list; a plain value is split on commas (`excludeTools: edit, write`).
    fn list(&self) -> Vec<String> {
        match self {
            Fm::List(l) => l.clone(),
            Fm::Str(s) => s.split(',').map(|x| unquote(x.trim())).filter(|x| !x.is_empty()).collect(),
        }
    }
}

fn unquote(v: &str) -> String {
    let v = v.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        return v[1..v.len() - 1].replace("\\\"", "\"").replace("\\\\", "\\");
    }
    if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
        return v[1..v.len() - 1].replace("''", "'");
    }
    v.to_string()
}

/// Split a flow list `[a, "b, c", d]` on commas outside quotes.
fn flow_list(inner: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for ch in inner.chars() {
        match quote {
            Some(q) if ch == q => {
                quote = None;
                cur.push(ch);
            }
            Some(_) => cur.push(ch),
            None if ch == '"' || ch == '\'' => {
                quote = Some(ch);
                cur.push(ch);
            }
            None if ch == ',' => {
                items.push(unquote(&cur));
                cur.clear();
            }
            None => cur.push(ch),
        }
    }
    items.push(unquote(&cur));
    items.into_iter().filter(|s| !s.is_empty()).collect()
}

/// Frontmatter (`---` … `---`) and body. The frontmatter is a flat YAML subset:
/// `key: value`, flow lists `[a, b]`, block lists (`- item` lines) and block
/// text (`key: |` or `key: >` followed by indented lines).
fn parse_frontmatter(text: &str) -> Result<(Vec<(String, Fm)>, String), String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else {
        return Ok((Vec::new(), text.to_string()));
    };
    let mut lines: Vec<&str> = Vec::new();
    let mut body_start = None;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        offset += line.len();
        let l = line.trim_end_matches(['\n', '\r']);
        if l == "---" || l == "..." {
            body_start = Some(offset);
            break;
        }
        lines.push(l);
    }
    let Some(body_start) = body_start else { return Err("the frontmatter is not closed with ---".into()) };
    let body = rest[body_start..].trim_start_matches(['\n', '\r']).to_string();
    let mut fields = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            i += 1;
            continue;
        }
        if line.starts_with([' ', '\t']) {
            return Err(format!("line {}: unexpected indentation", i + 2));
        }
        let Some((key, value)) = line.split_once(':') else { return Err(format!("line {}: expected `key: value`", i + 2)) };
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(format!("line {}: bad key {key:?}", i + 2));
        }
        let value = value.trim();
        // The file's line of the key (the opening `---` is line 1).
        let at = i + 2;
        i += 1;
        // Continuation lines: indented (or blank inside a block).
        let mut block: Vec<&str> = Vec::new();
        while i < lines.len() && (lines[i].starts_with([' ', '\t']) || (lines[i].trim().is_empty() && value.starts_with(['|', '>']))) {
            block.push(lines[i]);
            i += 1;
        }
        let parsed = if value.starts_with('|') || value.starts_with('>') {
            let indent = block.iter().filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0);
            let texts: Vec<String> =
                block.iter().map(|l| if l.len() >= indent { l[indent..].to_string() } else { String::new() }).collect();
            let joined = if value.starts_with('|') {
                texts.join("\n")
            } else {
                texts
                    .iter()
                    .map(|l| if l.is_empty() { "\n".to_string() } else { l.clone() })
                    .collect::<Vec<_>>()
                    .join(" ")
                    .replace(" \n ", "\n")
            };
            Fm::Str(joined.trim().to_string())
        } else if value.is_empty() && !block.is_empty() {
            let mut items = Vec::new();
            for (n, b) in block.iter().enumerate() {
                let b = b.trim();
                if b.is_empty() || b.starts_with('#') {
                    continue;
                }
                let Some(item) = b.strip_prefix('-') else { return Err(format!("line {}: {key}: expected `- item` lines", at + 1 + n)) };
                items.push(unquote(item));
            }
            Fm::List(items.into_iter().filter(|s| !s.is_empty()).collect())
        } else if !block.is_empty() {
            return Err(format!("line {}: {key}: unexpected indented lines after a value", at + 1));
        } else if let Some(inner) = value.strip_prefix('[') {
            let Some(inner) = inner.strip_suffix(']') else { return Err(format!("line {at}: {key}: the list is not closed with ]")) };
            Fm::List(flow_list(inner))
        } else {
            Fm::Str(unquote(value))
        };
        fields.push((key.to_string(), parsed));
    }
    Ok((fields, body))
}

// --- roles ------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct RawRole {
    title: Option<String>,
    description: Option<String>,
    base: Option<String>,
    extends: Option<String>,
    model: Option<String>,
    thinking: Option<String>,
    names: Option<Vec<String>>,
    allow: Option<Vec<String>>,
    deny: Option<Vec<String>>,
    files: Option<String>,
    git: Option<String>,
    exclude_tools: Option<Vec<String>>,
    deny_commands: Option<Vec<String>>,
    skills: Option<Vec<String>>,
    mcp: Option<Vec<String>>,
    stages: Option<Vec<String>>,
    projects: Option<Vec<String>>,
    instructions: Option<String>,
    body: String,
}

const ROLE_KEYS: &[&str] = &[
    "title",
    "description",
    "base",
    "extends",
    "model",
    "thinking",
    "names",
    "allow",
    "deny",
    "files",
    "git",
    "excludeTools",
    "denyCommands",
    "skills",
    "mcp",
    "stages",
    "projects",
    "instructions",
];

fn parse_role(text: &str) -> Result<(RawRole, Vec<String>), String> {
    let (fields, body) = parse_frontmatter(text)?;
    let mut r = RawRole { body, ..Default::default() };
    let mut warnings = Vec::new();
    let non_empty = |s: String| (!s.trim().is_empty()).then_some(s.trim().to_string());
    for (k, v) in fields {
        match k.as_str() {
            "title" => r.title = non_empty(v.text()),
            "description" => r.description = non_empty(v.text()),
            "base" => r.base = non_empty(v.text()),
            "extends" => r.extends = non_empty(v.text()),
            "model" => r.model = non_empty(v.text()),
            "thinking" => r.thinking = non_empty(v.text()),
            "names" => r.names = Some(v.list()),
            "allow" => r.allow = Some(v.list()),
            "deny" => r.deny = Some(v.list()),
            "files" => r.files = non_empty(v.text()),
            "git" => r.git = non_empty(v.text()),
            "excludeTools" => r.exclude_tools = Some(v.list()),
            "denyCommands" => r.deny_commands = Some(v.list()),
            "skills" => r.skills = Some(v.list()),
            "mcp" => r.mcp = Some(v.list()),
            "stages" => r.stages = Some(v.list()),
            "projects" => r.projects = Some(v.list()),
            "instructions" => r.instructions = non_empty(v.text()),
            other => warnings.push(format!("unknown field `{other}` is ignored (known: {})", ROLE_KEYS.join(", "))),
        }
    }
    Ok((r, warnings))
}

impl RawRole {
    /// `over` on top of `self`: set fields win; a non-empty body replaces the prompt.
    fn overlay(&self, over: &RawRole) -> RawRole {
        macro_rules! pick {
            ($f:ident) => {
                over.$f.clone().or_else(|| self.$f.clone())
            };
        }
        RawRole {
            title: pick!(title),
            description: pick!(description),
            base: pick!(base),
            extends: pick!(extends),
            model: pick!(model),
            thinking: pick!(thinking),
            names: pick!(names),
            allow: pick!(allow),
            deny: pick!(deny),
            files: pick!(files),
            git: pick!(git),
            exclude_tools: pick!(exclude_tools),
            deny_commands: pick!(deny_commands),
            skills: pick!(skills),
            mcp: pick!(mcp),
            stages: pick!(stages),
            projects: pick!(projects),
            instructions: pick!(instructions),
            body: if over.body.trim().is_empty() { self.body.clone() } else { over.body.clone() },
        }
    }
}

fn class_files(class: Role) -> FileAccess {
    match class {
        Role::Analyst | Role::Reviewer => FileAccess::Read,
        _ => FileAccess::Write,
    }
}

fn class_stages(class: Role) -> Vec<Stage> {
    match class {
        Role::Analyst | Role::Reviewer | Role::Documenter => vec![Stage::Refinement, Stage::Delivery],
        Role::Executor | Role::Tester => vec![Stage::Delivery],
        Role::Orchestrator | Role::Human => Vec::new(),
    }
}

fn caps(list: &Option<Vec<String>>, errors: &mut Vec<String>) -> Vec<Capability> {
    let mut out = Vec::new();
    for c in list.iter().flatten() {
        match c.parse::<Capability>() {
            Ok(cap) => out.push(cap),
            Err(_) => errors.push(format!(
                "unknown permission `{c}` (known: {})",
                Capability::ALL.iter().map(|c| c.as_str()).collect::<Vec<_>>().join(", ")
            )),
        }
    }
    out
}

struct RoleSource {
    raw: RawRole,
    origin: Origin,
    path: Option<String>,
}

struct RoleResolver<'a> {
    sources: &'a HashMap<String, RoleSource>,
    done: HashMap<String, Result<RoleDef, Vec<String>>>,
}

impl RoleResolver<'_> {
    fn resolve(&mut self, id: &str, stack: &mut Vec<String>) -> Result<RoleDef, Vec<String>> {
        if let Some(r) = self.done.get(id) {
            return r.clone();
        }
        if stack.iter().any(|s| s == id) {
            return Err(vec![format!("`extends` makes a cycle: {} → {id}", stack.join(" → "))]);
        }
        let Some(src) = self.sources.get(id) else { return Err(vec![format!("unknown role `{id}`")]) };
        stack.push(id.to_string());
        let out = self.build(id, src, stack);
        stack.pop();
        self.done.insert(id.to_string(), out.clone());
        out
    }

    fn build(&mut self, id: &str, src: &RoleSource, stack: &mut Vec<String>) -> Result<RoleDef, Vec<String>> {
        let raw = &src.raw;
        let mut errors = Vec::new();
        let parent = match (&raw.extends, &raw.base) {
            (Some(_), Some(_)) => {
                errors.push("use either `base` (a class) or `extends` (a role), not both".into());
                None
            }
            (Some(p), None) => match self.resolve(p, stack) {
                Ok(def) if def.class == Role::Orchestrator => {
                    errors.push("a role cannot extend the orchestrator".into());
                    None
                }
                Ok(def) => Some(def),
                Err(e) => {
                    errors.push(format!("`extends: {p}`: {}", e.join("; ")));
                    None
                }
            },
            _ => None,
        };
        let class = match (&parent, &raw.base) {
            (Some(p), _) => p.class,
            (None, Some(b)) => match b.parse::<Role>() {
                Ok(r) if MEMBER_ROLES.contains(&r) => r,
                _ => {
                    errors.push(format!(
                        "`base: {b}` is not a team class (one of {})",
                        MEMBER_ROLES.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(", ")
                    ));
                    Role::Analyst
                }
            },
            (None, None) if id == ORCHESTRATOR && src.origin != Origin::Custom => Role::Orchestrator,
            (None, None) => match id.parse::<Role>() {
                Ok(r) if MEMBER_ROLES.contains(&r) => r,
                _ => {
                    if raw.extends.is_none() {
                        errors.push(format!(
                            "a new role needs `base` (a class: {}) or `extends` (another role)",
                            MEMBER_ROLES.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(", ")
                        ));
                    }
                    Role::Analyst
                }
            },
        };
        let allow = caps(&raw.allow, &mut errors);
        let deny = caps(&raw.deny, &mut errors);
        if class == Role::Orchestrator && (!allow.is_empty() || !deny.is_empty()) {
            errors.push("the orchestrator's permissions are fixed: remove `allow` and `deny`".into());
        }
        let base_caps = match &parent {
            Some(p) => p.capabilities.clone(),
            None if class == Role::Orchestrator => Capability::ALL.to_vec(),
            None => class_capabilities(class),
        };
        let capabilities = adjust_capabilities(&base_caps, &allow, &deny);
        let files = match raw.files.as_deref() {
            Some("write") => FileAccess::Write,
            Some("read") => FileAccess::Read,
            Some("none") => FileAccess::None,
            Some(other) => {
                errors.push(format!("`files: {other}`: expected write, read or none"));
                FileAccess::Read
            }
            None => match (&raw.exclude_tools, &parent) {
                (Some(x), _) if x.iter().any(|t| t == "edit" || t == "write") => FileAccess::Read,
                (_, Some(p)) => p.files,
                _ => class_files(class),
            },
        };
        let git = match raw.git.as_deref() {
            Some(g @ ("none" | "read" | "write")) => Some(g.to_string()),
            Some(other) => {
                errors.push(format!("`git: {other}`: expected none, read or write"));
                None
            }
            None => parent.as_ref().and_then(|p| p.git.clone()),
        };
        let mut stages = Vec::new();
        for s in raw.stages.iter().flatten() {
            match s.as_str() {
                "refinement" => stages.push(Stage::Refinement),
                "delivery" => stages.push(Stage::Delivery),
                other => errors.push(format!("unknown stage `{other}` (refinement or delivery)")),
            }
        }
        if raw.stages.is_none() {
            stages = parent.as_ref().map(|p| p.stages.clone()).unwrap_or_else(|| class_stages(class));
        }
        let names = raw
            .names
            .clone()
            .or_else(|| parent.as_ref().map(|p| p.names.clone()))
            .unwrap_or_else(|| genie_core::team::name_pool(class.as_str()).iter().map(|s| s.to_string()).collect());
        for n in &names {
            if !valid_member_name(n) {
                errors.push(format!("name `{n}` must match [a-z][a-z0-9_-]*"));
            }
        }
        if let Some(p) = raw.projects.as_ref().filter(|p| p.is_empty()) {
            let _ = p;
            errors.push("`projects: []` makes the role unusable; drop the field to allow every project".into());
        }
        let prompt =
            if raw.body.trim().is_empty() { parent.as_ref().map(|p| p.prompt.clone()).unwrap_or_default() } else { raw.body.clone() };
        if prompt.trim().is_empty() {
            errors.push("the role has no prompt: write it in the file body".into());
        }
        if !errors.is_empty() {
            return Err(errors);
        }
        let inherit = |own: &Option<String>, from: fn(&RoleDef) -> Option<String>| own.clone().or_else(|| parent.as_ref().and_then(from));
        Ok(RoleDef {
            id: id.to_string(),
            title: raw.title.clone().or_else(|| parent.as_ref().map(|p| p.title.clone())).unwrap_or_else(|| id.to_string()),
            description: raw.description.clone().or_else(|| parent.as_ref().map(|p| p.description.clone())).unwrap_or_default(),
            class,
            extends: raw.extends.clone(),
            model: inherit(&raw.model, |p| p.model.clone()),
            thinking: inherit(&raw.thinking, |p| p.thinking.clone()),
            names,
            allow,
            deny,
            capabilities,
            files,
            git,
            deny_commands: raw.deny_commands.clone().or_else(|| parent.as_ref().map(|p| p.deny_commands.clone())).unwrap_or_default(),
            skills: raw.skills.clone().or_else(|| parent.as_ref().and_then(|p| p.skills.clone())),
            mcp: raw.mcp.clone().or_else(|| parent.as_ref().map(|p| p.mcp.clone())).unwrap_or_default(),
            stages,
            projects: raw.projects.clone().or_else(|| parent.as_ref().and_then(|p| p.projects.clone())),
            instructions: inherit(&raw.instructions, |p| p.instructions.clone()),
            prompt,
            origin: src.origin,
            path: src.path.clone(),
            stale: false,
        })
    }
}

// --- teams ------------------------------------------------------------------------

const TEAM_KEYS: &[&str] =
    &["title", "description", "stage", "workspace", "worktree", "mail", "members", "relations", "charter", "projects"];

fn string_list(v: &Value, what: &str, errors: &mut Vec<String>) -> Option<Vec<String>> {
    match v {
        Value::Null => None,
        Value::String(s) => Some(vec![s.clone()]),
        Value::Array(a) => Some(
            a.iter()
                .filter_map(|x| match x.as_str() {
                    Some(s) => Some(s.to_string()),
                    None => {
                        errors.push(format!("{what}: expected strings"));
                        None
                    }
                })
                .collect(),
        ),
        _ => {
            errors.push(format!("{what}: expected a string or a list of strings"));
            None
        }
    }
}

fn opt_str(o: &Map<String, Value>, key: &str) -> Option<String> {
    o.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// A team template as written, before the roles are checked.
struct RawTeam {
    title: Option<String>,
    description: String,
    stage: Stage,
    workspace: Workspace,
    mail: MailMode,
    members: Vec<MemberDef>,
    relations: Option<Vec<Relation>>,
    charter: Option<String>,
    projects: Option<Vec<String>>,
}

fn parse_team(v: &Value, legacy: bool) -> Result<(RawTeam, Vec<String>), Vec<String>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let Some(o) = v.as_object() else { return Err(vec!["a team template is a JSON object".into()]) };
    for k in o.keys() {
        if !TEAM_KEYS.contains(&k.as_str()) {
            warnings.push(format!("unknown field `{k}` is ignored (known: {})", TEAM_KEYS.join(", ")));
        }
    }
    let stage = match opt_str(o, "stage").as_deref() {
        None | Some("delivery") => Stage::Delivery,
        Some("refinement") => Stage::Refinement,
        Some(other) => {
            errors.push(format!("`stage: {other}`: expected refinement or delivery"));
            Stage::Delivery
        }
    };
    let workspace = match (opt_str(o, "workspace").as_deref(), o.get("worktree").and_then(Value::as_bool)) {
        (Some("worktree"), _) => Workspace::Worktree,
        (Some("repo"), _) => Workspace::Repo,
        (Some("scratch"), _) => Workspace::Scratch,
        (Some(other), _) => {
            errors.push(format!("`workspace: {other}`: expected worktree, repo or scratch"));
            Workspace::Worktree
        }
        (None, Some(true)) => Workspace::Worktree,
        (None, Some(false)) => Workspace::Repo,
        // The old presets meant "no worktree" when the flag was missing.
        (None, None) if legacy => Workspace::Repo,
        (None, None) => Workspace::Worktree,
    };
    let mail = match opt_str(o, "mail").as_deref() {
        None | Some("open") => MailMode::Open,
        Some("flow") => MailMode::Flow,
        Some(other) => {
            errors.push(format!("`mail: {other}`: expected open or flow"));
            MailMode::Open
        }
    };
    let mut members = Vec::new();
    match o.get("members") {
        Some(Value::Array(list)) => {
            for (i, m) in list.iter().enumerate() {
                let Some(mo) = m.as_object() else {
                    errors.push(format!("members[{i}]: expected an object with `role`"));
                    continue;
                };
                let Some(role) = opt_str(mo, "role") else {
                    errors.push(format!("members[{i}]: `role` is required"));
                    continue;
                };
                members.push(MemberDef {
                    key: opt_str(mo, "key").unwrap_or_else(|| role.clone()),
                    role,
                    name: opt_str(mo, "name"),
                    model: opt_str(mo, "model"),
                    thinking: opt_str(mo, "thinking"),
                    instructions: opt_str(mo, "instructions"),
                });
            }
        }
        _ => errors.push("`members` is required: a list of `{\"role\": …}`".into()),
    }
    let relations = match o.get("relations") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) => {
            let mut out = Vec::new();
            for (i, r) in list.iter().enumerate() {
                let Some(ro) = r.as_object() else {
                    errors.push(format!("relations[{i}]: expected an object"));
                    continue;
                };
                let from = opt_str(ro, "from");
                let to = string_list(ro.get("to").unwrap_or(&Value::Null), &format!("relations[{i}].to"), &mut errors);
                let kind = match opt_str(ro, "type").as_deref() {
                    Some("handoff") => Some(RelKind::Handoff),
                    Some("returns") => Some(RelKind::Returns),
                    Some("reports") => Some(RelKind::Reports),
                    Some("consults") => Some(RelKind::Consults),
                    other => {
                        errors.push(format!(
                            "relations[{i}]: `type` {}: expected handoff, returns, reports or consults",
                            other.map(|o| format!("`{o}`")).unwrap_or_else(|| "is missing".into())
                        ));
                        None
                    }
                };
                let on = match opt_str(ro, "on") {
                    Some(s) => match s.parse::<Status>() {
                        Ok(st) => Some(st),
                        Err(_) => {
                            errors.push(format!("relations[{i}]: `on: {s}` is not a status"));
                            None
                        }
                    },
                    None => None,
                };
                match (from, to, kind) {
                    (Some(from), Some(to), Some(kind)) if !to.is_empty() => {
                        out.push(Relation { from, to, kind, on, note: opt_str(ro, "note") })
                    }
                    _ => errors.push(format!("relations[{i}]: `from`, `to` and `type` are required")),
                }
            }
            Some(out)
        }
        Some(_) => {
            errors.push("`relations` must be a list".into());
            None
        }
    };
    let projects = string_list(o.get("projects").unwrap_or(&Value::Null), "projects", &mut errors);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok((
        RawTeam {
            title: opt_str(o, "title"),
            description: opt_str(o, "description").unwrap_or_default(),
            stage,
            workspace,
            mail,
            members,
            relations,
            charter: opt_str(o, "charter"),
            projects,
        },
        warnings,
    ))
}

/// Relations the built-in process implies for a set of members (for templates
/// without `relations` and for ad-hoc teams). Mirrors the kickoffs genie gave
/// before relations were configurable.
pub fn default_relations(members: &[(String, Role)], refinement: bool) -> Vec<Relation> {
    let first = |class: Role| members.iter().find(|(_, c)| *c == class).map(|(k, _)| k.clone());
    let all = |class: Role| members.iter().filter(|(_, c)| *c == class).map(|(k, _)| k.clone()).collect::<Vec<_>>();
    let rel = |from: &str, to: Vec<String>, kind: RelKind, on: Option<Status>, note: &str| Relation {
        from: from.to_string(),
        to,
        kind,
        on,
        note: Some(note.to_string()),
    };
    let (analyst, executor, reviewer, tester, documenter) =
        (first(Role::Analyst), first(Role::Executor), first(Role::Reviewer), first(Role::Tester), first(Role::Documenter));
    let mut out = Vec::new();
    if refinement || executor.is_none() {
        if let (Some(a), Some(r)) = (&analyst, &reviewer) {
            out.push(rel(a, vec![r.clone()], RelKind::Handoff, None, "the findings"));
            out.push(rel(r, vec![a.clone()], RelKind::Returns, None, "gaps and risks in the findings"));
        }
        if let Some(voice) = analyst.clone().or(reviewer.clone()) {
            out.push(rel(&voice, vec![ORCHESTRATOR.into()], RelKind::Reports, None, "the findings"));
        }
        if let Some(d) = &documenter {
            out.push(rel(d, vec![ORCHESTRATOR.into()], RelKind::Reports, None, "the documentation is ready"));
        }
        return out;
    }
    let e = executor.expect("checked above");
    if let Some(a) = &analyst {
        out.push(rel(a, vec![e.clone()], RelKind::Handoff, None, "the plan is ready"));
        out.push(rel(&e, vec![a.clone()], RelKind::Consults, None, "questions about the plan"));
    }
    let mut checkers = all(Role::Tester);
    checkers.extend(all(Role::Reviewer));
    if !checkers.is_empty() {
        out.push(rel(&e, checkers, RelKind::Handoff, Some(Status::Review), "the work is submitted for review"));
    }
    for t in all(Role::Tester) {
        if let Some(r) = &reviewer {
            out.push(rel(&t, vec![r.clone()], RelKind::Handoff, None, "the test report"));
        }
        out.push(rel(&t, vec![e.clone()], RelKind::Returns, None, "failing tests"));
    }
    if let Some(r) = &reviewer {
        out.push(rel(r, vec![e.clone()], RelKind::Returns, Some(Status::ChangesRequested), "review findings"));
        if let Some(d) = &documenter {
            out.push(rel(r, vec![d.clone()], RelKind::Handoff, Some(Status::Approved), "the work is approved"));
        }
    }
    let voice = reviewer.or(tester).unwrap_or(e);
    out.push(rel(&voice, vec![ORCHESTRATOR.into()], RelKind::Reports, None, "the verdict"));
    if let Some(d) = &documenter {
        out.push(rel(d, vec![ORCHESTRATOR.into()], RelKind::Reports, None, "the documentation is ready"));
    }
    out
}

/// Check a template against the roles: errors make it unusable, warnings are advice.
fn check_team(raw: &RawTeam, roles: &BTreeMap<String, RoleDef>, max_members: usize) -> (Vec<String>, Vec<String>, Vec<Relation>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if raw.members.is_empty() {
        errors.push("a team needs at least one member".into());
    }
    if raw.members.len() > max_members {
        errors.push(format!("{} members; the limit is {max_members} per team (`limits.maxMembersPerTeam`)", raw.members.len()));
    }
    let mut keys = HashSet::new();
    let mut classes: Vec<(String, Role)> = Vec::new();
    for m in &raw.members {
        if !valid_id(&m.key) {
            errors.push(format!("member key `{}` must match [a-z][a-z0-9-]*", m.key));
        }
        if m.key == ORCHESTRATOR || !keys.insert(m.key.clone()) {
            errors.push(format!("two members share the key `{}`: give them different `key`s", m.key));
        }
        if let Some(n) = &m.name
            && !valid_member_name(n)
        {
            errors.push(format!("member name `{n}` must match [a-z][a-z0-9_-]*"));
        }
        match roles.get(&m.role) {
            Some(r) if r.class == Role::Orchestrator => errors.push("the orchestrator is not a team member".into()),
            Some(r) => {
                if raw.stage == Stage::Refinement && !r.stages.contains(&Stage::Refinement) {
                    errors.push(format!("role `{}` does not work before `ready`, but the template's stage is refinement", m.role));
                }
                classes.push((m.key.clone(), r.class));
            }
            None => errors.push(format!("unknown role `{}`", m.role)),
        }
    }
    let derived = raw.relations.is_none();
    let relations = raw.relations.clone().unwrap_or_else(|| default_relations(&classes, raw.stage == Stage::Refinement));
    for r in &relations {
        if !keys.contains(&r.from) {
            errors.push(format!("relation from `{}`: no such member", r.from));
        }
        for t in &r.to {
            let is_orch = t == ORCHESTRATOR;
            if !is_orch && !keys.contains(t) {
                errors.push(format!("relation to `{t}`: no such member"));
            }
            if is_orch && r.kind != RelKind::Reports {
                errors.push(format!("{:?} to the orchestrator: only `reports` goes to the orchestrator", r.kind).to_lowercase());
            }
            if !is_orch && r.kind == RelKind::Reports {
                errors.push(format!("`reports` goes to the orchestrator, not to `{t}`"));
            }
            if t == &r.from {
                errors.push(format!("`{t}` relates to itself"));
            }
        }
        if r.on.is_some() && !matches!(r.kind, RelKind::Handoff | RelKind::Returns) {
            errors.push(format!("`on` works only for handoff and returns (relation from `{}`)", r.from));
        }
        if let (Some(on), Some(role)) = (r.on, raw.members.iter().find(|m| m.key == r.from).and_then(|m| roles.get(&m.role)))
            && !genie_core::TEAM_TRANSITIONS.iter().any(|(_, to, cap)| *to == on && role.can(*cap))
        {
            warnings.push(format!("`{}` cannot move a task to {on} itself; the handoff happens only when someone else does", r.from));
        }
    }
    // Everyone must be able to start: without an incoming handoff, after one from
    // somebody who starts, or when genie hands over on a status.
    let mut started: HashSet<String> = raw
        .members
        .iter()
        .filter(|m| !relations.iter().any(|r| r.kind == RelKind::Handoff && r.to.contains(&m.key)))
        .map(|m| m.key.clone())
        .collect();
    loop {
        let before = started.len();
        for m in &raw.members {
            if started.contains(&m.key) {
                continue;
            }
            if relations
                .iter()
                .any(|r| r.kind == RelKind::Handoff && r.to.contains(&m.key) && (r.on.is_some() || started.contains(&r.from)))
            {
                started.insert(m.key.clone());
            }
        }
        if started.len() == before {
            break;
        }
    }
    for m in &raw.members {
        if !started.contains(&m.key) {
            errors.push(format!("`{}` waits for a handoff that never comes (the handoffs form a cycle)", m.key));
        }
    }
    // A refinement team with an explicit empty list talks with a person, not the
    // orchestrator (the idea planner): its result is the tracker's, nobody reports.
    let solo = raw.stage == Stage::Refinement && raw.relations.as_ref().is_some_and(|r| r.is_empty());
    if !raw.members.is_empty() && !solo && !relations.iter().any(|r| r.kind == RelKind::Reports) {
        warnings.push("nobody reports to the orchestrator: add a `reports` relation".into());
    }
    let has = |cap: Capability| raw.members.iter().filter_map(|m| roles.get(&m.role)).any(|r| r.can(cap));
    if raw.stage == Stage::Delivery && !raw.members.is_empty() {
        for (cap, what) in [(Capability::StatusSubmit, "submit the work for review"), (Capability::StatusApprove, "approve it")] {
            if !has(cap) {
                warnings.push(format!("no member can {what} (`{cap}`): the task cannot be closed without force"));
            }
        }
    }
    if raw.workspace == Workspace::Repo
        && let Some(m) = raw.members.iter().find(|m| roles.get(&m.role).is_some_and(|r| r.files == FileAccess::Write))
    {
        warnings.push(format!("`{}` may write files and works in the main working copy (`workspace: repo`)", m.key));
    }
    (errors, warnings, if derived { relations } else { raw.relations.clone().unwrap_or_default() })
}

// --- loading ----------------------------------------------------------------------

/// Edits not written yet (absolute path → new content, `None`: removed), so a
/// change can be checked exactly as the loader will see it before it is saved.
pub type Overlay = BTreeMap<PathBuf, Option<String>>;

fn read(path: &Path, overlay: &Overlay) -> std::io::Result<String> {
    match overlay.get(path) {
        Some(Some(text)) => Ok(text.clone()),
        Some(None) => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "removed")),
        None => std::fs::read_to_string(path),
    }
}

fn exists(path: &Path, overlay: &Overlay) -> bool {
    match overlay.get(path) {
        Some(v) => v.is_some(),
        None => path.exists(),
    }
}

fn md_files(dir: &Path, ext: &str, overlay: &Overlay) -> Vec<(String, PathBuf)> {
    let mut paths: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() && p.extension().and_then(|x| x.to_str()) == Some(ext) {
                paths.push(p);
            }
        }
    }
    for (p, v) in overlay {
        if p.parent() == Some(dir) && p.extension().and_then(|x| x.to_str()) == Some(ext) && v.is_some() && !paths.contains(p) {
            paths.push(p.clone());
        }
    }
    let mut out: Vec<(String, PathBuf)> = paths
        .into_iter()
        .filter(|p| exists(p, overlay))
        .filter_map(|p| p.file_stem().and_then(|s| s.to_str()).map(|stem| (stem.to_string(), p.clone())))
        .collect();
    out.sort();
    out
}

/// `SKILL.md` files under `dir` (a few levels deep, like harness discovery).
fn skill_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let skill = dir.join("SKILL.md");
    if skill.is_file() {
        out.push(skill);
        return;
    }
    if depth == 0 {
        return;
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        let mut dirs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir() && !p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.')))
            .collect();
        dirs.sort();
        for d in dirs {
            skill_files(&d, depth - 1, out);
        }
    }
}

/// `SKILL.md` files under `dir`, with pending edits applied.
fn skill_files_with(dir: &Path, overlay: &Overlay) -> Vec<PathBuf> {
    let mut out = Vec::new();
    skill_files(dir, 3, &mut out);
    for (p, v) in overlay {
        if v.is_some() && p.file_name().is_some_and(|n| n == "SKILL.md") && p.starts_with(dir) && !out.contains(p) {
            out.push(p.clone());
        }
    }
    out.retain(|p| exists(p, overlay));
    out.sort();
    out
}

fn skill_dirs(data: &Path, cfg: &Config) -> Vec<PathBuf> {
    let mut dirs = vec![data.join("skills")];
    dirs.extend(cfg.skills.paths.iter().map(|p| if p.is_absolute() { p.clone() } else { data.join(p) }));
    dirs
}

/// Everything the loader reads, as `path:mtime:len` — cheap change detection.
pub fn signature(data: &Path, cfg: &Config) -> String {
    let mut h = DefaultHasher::new();
    let mut add = |p: &Path| {
        if let Ok(m) = std::fs::metadata(p) {
            p.hash(&mut h);
            m.len().hash(&mut h);
            m.modified().ok().hash(&mut h);
        }
    };
    add(&data.join("config.json"));
    add(&data.join("mcp.json"));
    let none = Overlay::new();
    for (_, p) in md_files(&data.join("agents"), "md", &none).into_iter().chain(md_files(&data.join("teams"), "json", &none)) {
        add(&p);
    }
    let mut skills = Vec::new();
    for d in skill_dirs(data, cfg) {
        skill_files(&d, 3, &mut skills);
    }
    for p in &skills {
        add(p);
    }
    format!("{:x}", h.finish())
}

fn rel_path(data: &Path, p: &Path) -> String {
    p.strip_prefix(data).map(|r| r.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string_lossy().into_owned())
}

impl AgentConfig {
    /// Load the built-in configuration and the data directory's files. `previous`
    /// supplies the last valid version of items whose files are broken now.
    pub fn load(data: &Path, cfg: &Config, previous: Option<&AgentConfig>) -> AgentConfig {
        Self::load_with(data, cfg, previous, &Overlay::new())
    }

    /// `load` with pending edits applied on top of the files (to check them before saving).
    pub fn load_with(data: &Path, cfg: &Config, previous: Option<&AgentConfig>, overlay: &Overlay) -> AgentConfig {
        let mut problems = Vec::new();
        let mut problem = |level: Level, item: String, path: Option<String>, message: String| {
            problems.push(Problem { level, item, path, message });
        };

        // Skills.
        let mut skills = BTreeMap::new();
        let mut files = Vec::new();
        for d in skill_dirs(data, cfg) {
            files.extend(skill_files_with(&d, overlay));
        }
        for f in files {
            let dir = f.parent().map(Path::to_path_buf).unwrap_or_default();
            let path = rel_path(data, &f);
            let parsed = read(&f, overlay).map_err(|e| e.to_string()).and_then(|t| parse_frontmatter(&t));
            match parsed {
                Ok((fields, _)) => {
                    let get = |k: &str| fields.iter().find(|(n, _)| n == k).map(|(_, v)| v.text());
                    let name = get("name").unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
                    let Some(description) = get("description").filter(|d| !d.trim().is_empty()) else {
                        problem(Level::Error, format!("skill:{name}"), Some(path), "SKILL.md needs a `description`".into());
                        continue;
                    };
                    if skills.contains_key(&name) {
                        problem(
                            Level::Warning,
                            format!("skill:{name}"),
                            Some(path),
                            "another skill with this name was found first; this one is ignored".into(),
                        );
                        continue;
                    }
                    skills.insert(name.clone(), SkillDef { name, description, dir });
                }
                Err(e) => problem(Level::Error, "skill".into(), Some(path), e),
            }
        }

        // MCP connections.
        let mut mcp = BTreeMap::new();
        let mcp_file = data.join("mcp.json");
        if exists(&mcp_file, overlay) {
            let parsed: Result<Value, String> =
                read(&mcp_file, overlay).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string()));
            match parsed {
                Ok(v) => match v.get("mcpServers").and_then(Value::as_object) {
                    Some(servers) => {
                        for (id, entry) in servers {
                            let item = format!("mcp:{id}");
                            let Some(o) = entry.as_object() else {
                                problem(Level::Error, item, Some("mcp.json".into()), "expected an object".into());
                                continue;
                            };
                            let transport = if o.contains_key("url") {
                                "http"
                            } else if o.contains_key("command") {
                                "stdio"
                            } else {
                                problem(Level::Error, item, Some("mcp.json".into()), "needs `command` (stdio) or `url` (http)".into());
                                continue;
                            };
                            let mut errs = Vec::new();
                            let projects = string_list(o.get("projects").unwrap_or(&Value::Null), "projects", &mut errs);
                            if let Some(e) = errs.pop() {
                                problem(Level::Error, item, Some("mcp.json".into()), e);
                                continue;
                            }
                            let gateway = match o.get("gateway") {
                                None => true,
                                Some(Value::Bool(b)) => *b,
                                Some(_) => {
                                    problem(Level::Error, item, Some("mcp.json".into()), "`gateway` is true or false".into());
                                    continue;
                                }
                            };
                            let mut config = o.clone();
                            config.remove("description");
                            config.remove("projects");
                            config.remove("gateway");
                            mcp.insert(
                                id.clone(),
                                McpServer {
                                    id: id.clone(),
                                    description: opt_str(o, "description").unwrap_or_default(),
                                    transport: transport.into(),
                                    projects,
                                    gateway,
                                    config: Value::Object(config),
                                },
                            );
                        }
                    }
                    None => problem(Level::Error, "mcp.json".into(), Some("mcp.json".into()), "expected {\"mcpServers\": {…}}".into()),
                },
                Err(e) => problem(Level::Error, "mcp.json".into(), Some("mcp.json".into()), e),
            }
        }

        // Roles: built-in, then the data directory on top.
        let mut sources: HashMap<String, RoleSource> = HashMap::new();
        for (id, text) in BUILTIN_ROLES {
            match parse_role(text) {
                Ok((raw, _)) => {
                    sources.insert(id.to_string(), RoleSource { raw, origin: Origin::Builtin, path: None });
                }
                Err(e) => problem(Level::Error, format!("role:{id}"), None, format!("built-in role: {e}")),
            }
        }
        let mut broken_roles: HashSet<String> = HashSet::new();
        for (id, path) in md_files(&data.join("agents"), "md", overlay) {
            let item = format!("role:{id}");
            let shown = rel_path(data, &path);
            if !valid_id(&id) {
                problem(Level::Error, item, Some(shown), "the file name is the role id: [a-z][a-z0-9-]*.md".into());
                continue;
            }
            let parsed = read(&path, overlay).map_err(|e| e.to_string()).and_then(|t| parse_role(&t));
            match parsed {
                Ok((raw, warnings)) => {
                    for w in warnings {
                        problem(Level::Warning, item.clone(), Some(shown.clone()), w);
                    }
                    let entry = match sources.remove(&id) {
                        Some(b) => RoleSource { raw: b.raw.overlay(&raw), origin: Origin::Override, path: Some(shown) },
                        None => RoleSource { raw, origin: Origin::Custom, path: Some(shown) },
                    };
                    sources.insert(id, entry);
                }
                Err(e) => {
                    problem(Level::Error, item, Some(shown), e);
                    broken_roles.insert(id);
                }
            }
        }
        let mut resolver = RoleResolver { sources: &sources, done: HashMap::new() };
        let mut ids: Vec<&String> = sources.keys().collect();
        ids.sort();
        let mut roles = BTreeMap::new();
        for id in ids {
            match resolver.resolve(id, &mut Vec::new()) {
                Ok(def) => {
                    roles.insert(id.clone(), def);
                }
                Err(errors) => {
                    let src = &sources[id];
                    for e in &errors {
                        problem(Level::Error, format!("role:{id}"), src.path.clone(), e.clone());
                    }
                    broken_roles.insert(id.clone());
                }
            }
        }
        // A broken file: its last valid version, or the built-in role.
        for id in broken_roles {
            if roles.contains_key(&id) {
                continue;
            }
            if let Some(prev) = previous.and_then(|p| p.roles.get(&id)) {
                roles.insert(id.clone(), RoleDef { stale: true, ..prev.clone() });
            } else if let Some((_, text)) = BUILTIN_ROLES.iter().find(|(b, _)| *b == id)
                && let Ok((raw, _)) = parse_role(text)
            {
                let only: HashMap<String, RoleSource> =
                    HashMap::from([(id.clone(), RoleSource { raw, origin: Origin::Builtin, path: None })]);
                if let Ok(def) = (RoleResolver { sources: &only, done: HashMap::new() }).resolve(&id, &mut Vec::new()) {
                    roles.insert(id.clone(), RoleDef { stale: true, ..def });
                }
            }
        }
        for r in roles.values() {
            for s in r.skills.iter().flatten() {
                if !skills.contains_key(s) {
                    problem(Level::Warning, format!("role:{}", r.id), r.path.clone(), format!("skill `{s}` is not installed"));
                }
            }
            for g in &r.mcp {
                let server = g.split(':').next().unwrap_or_default();
                if g != "*" && !mcp.contains_key(server) {
                    problem(
                        Level::Warning,
                        format!("role:{}", r.id),
                        r.path.clone(),
                        format!("MCP connection `{server}` is not in mcp.json"),
                    );
                }
            }
        }

        // Team templates: built-in, the legacy `teams` key of config.json, then files.
        let mut raw_teams: BTreeMap<String, (Value, Origin, Option<String>)> = BTreeMap::new();
        for (id, text) in BUILTIN_TEAMS {
            match serde_json::from_str::<Value>(text) {
                Ok(v) => {
                    raw_teams.insert(id.to_string(), (v, Origin::Builtin, None));
                }
                Err(e) => problem(Level::Error, format!("team:{id}"), None, format!("built-in template: {e}")),
            }
        }
        let legacy: Option<Value> = read(&data.join("config.json"), overlay).ok().and_then(|t| serde_json::from_str(&t).ok());
        if let Some(Value::Object(teams)) = legacy.as_ref().and_then(|c| c.get("teams")) {
            for (id, v) in teams {
                let origin = if raw_teams.contains_key(id) { Origin::Override } else { Origin::Legacy };
                // A partial override of a built-in preset (the old deep merge) keeps its fields.
                let merged = match (raw_teams.get(id), v) {
                    (Some((Value::Object(b), ..)), Value::Object(o)) => {
                        let mut m = b.clone();
                        for (k, x) in o {
                            m.insert(k.clone(), x.clone());
                        }
                        Value::Object(m)
                    }
                    _ => v.clone(),
                };
                raw_teams.insert(id.clone(), (merged, origin, Some("config.json".into())));
            }
        }
        let mut broken_teams = HashSet::new();
        for (id, path) in md_files(&data.join("teams"), "json", overlay) {
            let shown = rel_path(data, &path);
            if !valid_id(&id) {
                problem(Level::Error, format!("team:{id}"), Some(shown), "the file name is the template id: [a-z][a-z0-9-]*.json".into());
                continue;
            }
            match read(&path, overlay).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string()))
            {
                Ok(v) => {
                    let origin = if BUILTIN_TEAMS.iter().any(|(b, _)| *b == id) { Origin::Override } else { Origin::Custom };
                    raw_teams.insert(id, (v, origin, Some(shown)));
                }
                Err(e) => {
                    problem(Level::Error, format!("team:{id}"), Some(shown), e);
                    broken_teams.insert(id);
                }
            }
        }
        let mut teams = BTreeMap::new();
        for (id, (v, origin, path)) in raw_teams {
            let item = format!("team:{id}");
            let parsed = parse_team(&v, origin == Origin::Legacy || path.as_deref() == Some("config.json"));
            let (raw, mut warnings) = match parsed {
                Ok(x) => x,
                Err(errors) => {
                    for e in errors {
                        problem(Level::Error, item.clone(), path.clone(), e);
                    }
                    broken_teams.insert(id);
                    continue;
                }
            };
            let (errors, more, relations) = check_team(&raw, &roles, cfg.limits.max_members_per_team);
            if !errors.is_empty() {
                for e in errors {
                    problem(Level::Error, item.clone(), path.clone(), e);
                }
                broken_teams.insert(id);
                continue;
            }
            warnings.extend(more);
            // Advice about the shipped presets (the ABAP team works in the main
            // working copy on purpose) stays on the template, not in the problem list.
            if origin != Origin::Builtin {
                for w in &warnings {
                    problem(Level::Warning, item.clone(), path.clone(), w.clone());
                }
            }
            teams.insert(
                id.clone(),
                TeamDef {
                    title: raw.title.unwrap_or_else(|| id.clone()),
                    id,
                    description: raw.description,
                    stage: raw.stage,
                    workspace: raw.workspace,
                    mail: raw.mail,
                    members: raw.members,
                    relations,
                    relations_derived: raw.relations.is_none(),
                    charter: raw.charter,
                    projects: raw.projects,
                    origin,
                    path,
                    warnings,
                    stale: false,
                },
            );
        }
        for id in broken_teams {
            if !teams.contains_key(&id)
                && let Some(prev) = previous.and_then(|p| p.teams.get(&id))
            {
                teams.insert(id.clone(), TeamDef { stale: true, ..prev.clone() });
            }
        }
        AgentConfig { roles, teams, skills, mcp, problems, signature: signature(data, cfg) }
    }

    pub fn errors(&self) -> impl Iterator<Item = &Problem> {
        self.problems.iter().filter(|p| p.level == Level::Error)
    }

    /// A role the project may use.
    pub fn role_for(&self, project: &str, id: &str) -> Result<&RoleDef, GenieError> {
        let r = self.roles.get(id).ok_or_else(|| {
            GenieError::invalid(format!(
                "unknown role {id}; roles: {}",
                self.roles
                    .values()
                    .filter(|r| r.class != Role::Orchestrator && r.available_in(project))
                    .map(|r| r.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        if !r.available_in(project) {
            return Err(GenieError::invalid(format!("role {id} is not available in project {project}")));
        }
        Ok(r)
    }

    /// A team template the project may use.
    pub fn team_for(&self, project: &str, id: &str) -> Result<&TeamDef, GenieError> {
        let t = self.teams.get(id).ok_or_else(|| {
            GenieError::invalid(format!(
                "unknown team template {id}; templates: {}",
                self.teams.values().filter(|t| t.available_in(project)).map(|t| t.id.as_str()).collect::<Vec<_>>().join(", ")
            ))
        })?;
        if !t.available_in(project) {
            return Err(GenieError::invalid(format!("team template {id} is not available in project {project}")));
        }
        Ok(t)
    }

    /// The orchestrator's role (always present: it is built in).
    pub fn orchestrator(&self) -> Option<&RoleDef> {
        self.roles.get(ORCHESTRATOR)
    }

    /// Environment variables that hold the secrets of connections agents reach
    /// through the gateway: agent processes do not get them. Common variables a
    /// connection may use for paths (HOME, PATH…) stay.
    pub fn mcp_secret_vars(&self) -> Vec<String> {
        const KEEP: &[&str] = &["HOME", "PATH", "USER", "LOGNAME", "SHELL", "PWD", "LANG", "LANGUAGE", "TMPDIR", "TERM", "TZ"];
        let direct: Vec<String> = self.mcp.values().filter(|s| !s.gateway).flat_map(McpServer::env_refs).collect();
        let mut out: Vec<String> = self
            .mcp
            .values()
            .filter(|s| s.gateway)
            .flat_map(McpServer::env_refs)
            .filter(|v| !KEEP.contains(&v.as_str()) && !v.starts_with("LC_") && !direct.contains(v))
            .collect();
        out.sort();
        out.dedup();
        out
    }

    /// MCP connections a role may use in a project.
    pub fn mcp_for(&self, project: &str, role: &RoleDef) -> Vec<(&McpServer, Option<Vec<String>>)> {
        let mut out: Vec<(&McpServer, Option<Vec<String>>)> = Vec::new();
        for s in self.mcp.values().filter(|s| s.available_in(project)) {
            let mut all = false;
            let mut tools = Vec::new();
            for g in &role.mcp {
                match g.split_once(':') {
                    _ if g == "*" => all = true,
                    None if g == &s.id => all = true,
                    Some((server, pattern)) if server == s.id => tools.push(pattern.to_string()),
                    _ => {}
                }
            }
            if all {
                out.push((s, None));
            } else if !tools.is_empty() {
                out.push((s, Some(tools)));
            }
        }
        out
    }
}

const WATCH_MIN: Duration = Duration::from_secs(3);
const WATCH_MAX: Duration = Duration::from_secs(15);

/// Keep the configuration in step with the files: reload when they change. Edits through the API
/// reload at once; hand edits are noticed by looking at the files, every few seconds while they
/// change and backing off while they are quiet.
pub fn start_watcher(app: &Arc<App>) {
    let app = app.clone();
    tokio::spawn(async move {
        let mut wait = WATCH_MIN;
        loop {
            tokio::time::sleep(wait).await;
            let res = app
                .blocking(|app| {
                    let sig = signature(&app.data, &app.cfg);
                    let changed = sig != app.agents().signature;
                    if changed {
                        let cfg = app.reload_agents();
                        let errors = cfg.errors().count();
                        println!(
                            "genie: agent configuration reloaded ({} roles, {} templates{})",
                            cfg.roles.len(),
                            cfg.teams.len(),
                            if errors > 0 { format!(", {errors} error(s): `genie agents check`") } else { String::new() }
                        );
                    }
                    Ok(changed)
                })
                .await;
            wait = match res {
                Ok(true) | Err(_) => WATCH_MIN,
                Ok(false) => (wait * 2).min(WATCH_MAX),
            };
            if let Err(e) = res {
                eprintln!("genie: agent configuration: {e}");
            }
        }
    });
}

/// A human-readable report for `genie agents check`.
pub fn report(cfg: &AgentConfig) -> String {
    let mut out = vec![format!(
        "{} roles, {} team templates, {} skills, {} MCP connections",
        cfg.roles.len(),
        cfg.teams.len(),
        cfg.skills.len(),
        cfg.mcp.len()
    )];
    if cfg.problems.is_empty() {
        out.push("no problems".into());
    }
    for p in &cfg.problems {
        out.push(format!(
            "{} {}{}: {}",
            match p.level {
                Level::Error => "error  ",
                Level::Warning => "warning",
            },
            p.item,
            p.path.as_deref().map(|x| format!(" ({x})")).unwrap_or_default(),
            p.message
        ));
    }
    out.join("\n")
}

/// The catalogue as JSON for the API (roles without prompts unless asked).
pub fn catalogue(cfg: &AgentConfig, project: Option<&str>) -> Value {
    let roles: Vec<Value> = cfg
        .roles
        .values()
        .filter(|r| project.is_none_or(|p| r.available_in(p)))
        .map(|r| {
            let mut v = serde_json::to_value(r).unwrap_or(Value::Null);
            if let Some(o) = v.as_object_mut() {
                o.remove("prompt");
            }
            v
        })
        .collect();
    let teams: Vec<&TeamDef> = cfg.teams.values().filter(|t| project.is_none_or(|p| t.available_in(p))).collect();
    let mcp: Vec<&McpServer> = cfg.mcp.values().filter(|s| project.is_none_or(|p| s.available_in(p))).collect();
    // The permissions each process class starts from (a role's `allow`/`deny` adjust them).
    let classes: serde_json::Map<String, Value> = [Role::Analyst, Role::Executor, Role::Reviewer, Role::Tester, Role::Documenter]
        .into_iter()
        .map(|c| (c.as_str().to_string(), json!(genie_core::class_capabilities(c))))
        .collect();
    json!({
        "roles": roles,
        "teams": teams,
        "skills": cfg.skills.values().collect::<Vec<_>>(),
        "mcp": mcp,
        "problems": cfg.problems,
        "permissions": Capability::ALL,
        "classes": classes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(files: &[(&str, &str)]) -> (tempfile::TempDir, AgentConfig) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let p = dir.path().join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let cfg = Config::load(dir.path()).unwrap();
        let a = AgentConfig::load(dir.path(), &cfg, None);
        (dir, a)
    }

    fn errors(a: &AgentConfig) -> Vec<String> {
        a.errors().map(|p| format!("{}: {}", p.item, p.message)).collect()
    }

    #[test]
    fn built_in_roles_and_presets_load_without_problems() {
        let (_d, a) = load(&[]);
        assert!(a.problems.is_empty(), "{:#?}", a.problems);
        assert_eq!(a.roles["reviewer"].files, FileAccess::Read);
        assert_eq!(a.roles["executor"].files, FileAccess::Write);
        assert_eq!(a.roles["orchestrator"].class, Role::Orchestrator);
        assert!(a.roles["researcher"].can(Capability::StatusSubmit));
        assert_eq!(a.roles["researcher"].class, Role::Analyst);
        assert_eq!(a.roles["analyst"].stages, vec![Stage::Refinement, Stage::Delivery]);
        for id in ["standard", "pair", "full", "abap", "spike", "research"] {
            assert!(!a.teams[id].relations_derived, "{id} has explicit relations");
        }
        assert_eq!(a.teams["research"].stage, Stage::Refinement);
        assert_eq!(a.teams["idea"].members.len(), 1);
        assert_eq!(a.roles["planner"].class, Role::Analyst);
        assert_eq!(a.teams["abap"].workspace, Workspace::Repo);
        assert!(a.teams["abap"].members[1].instructions.as_deref().unwrap().contains("read-only"));
    }

    #[test]
    fn frontmatter_subset() {
        let text = "---\ntitle: \"Ревьюер: безопасность\"\nnames: [argus, 'heim dall']\nmcp:\n  - semgrep\n  - \"github:get_*\"\ninstructions: |\n  Line one\n  line two\ndescription: >\n  folded\n  text\n# a comment\n---\nBody\n";
        let (fields, body) = parse_frontmatter(text).unwrap();
        let get = |k: &str| fields.iter().find(|(n, _)| n == k).unwrap().1.clone();
        assert_eq!(get("title"), Fm::Str("Ревьюер: безопасность".into()));
        assert_eq!(get("names"), Fm::List(vec!["argus".into(), "heim dall".into()]));
        assert_eq!(get("mcp"), Fm::List(vec!["semgrep".into(), "github:get_*".into()]));
        assert_eq!(get("instructions"), Fm::Str("Line one\nline two".into()));
        assert_eq!(get("description"), Fm::Str("folded text".into()));
        assert_eq!(body, "Body\n");
        assert!(parse_frontmatter("---\ntitle: x\n").unwrap_err().contains("not closed"));
        // Errors name the file's line (the opening `---` is line 1), for the web's editor.
        assert_eq!(
            parse_frontmatter("---\ntitle: x\n  more\n---\n").unwrap_err(),
            "line 3: title: unexpected indented lines after a value"
        );
        assert_eq!(parse_frontmatter("---\na: b\nmcp:\n  - x\n  y\n---\n").unwrap_err(), "line 5: mcp: expected `- item` lines");
        assert_eq!(parse_frontmatter("---\nallow: [a, b\n---\n").unwrap_err(), "line 2: allow: the list is not closed with ]");
        assert_eq!(Fm::Str("edit, write".into()).list(), vec!["edit", "write"]);
    }

    #[test]
    fn overrides_extends_and_custom_roles() {
        let (_d, a) = load(&[
            ("agents/executor.md", "---\nmodel: fake/cheap\nskills: [clean-abap]\ninstructions: Follow Clean ABAP.\n---\n"),
            (
                "agents/security-reviewer.md",
                "---\ntitle: Ревьюер безопасности\ndescription: Checks security.\nextends: reviewer\ndeny: [task.check]\nmcp: [semgrep, \"github:get_*\"]\nprojects: [shop]\n---\nYou review security.\n",
            ),
            ("agents/qa.md", "---\nbase: tester\nallow: [task.check]\n---\nYou test.\n"),
            ("agents/bad.md", "---\nbase: wizard\nallow: [fly]\n---\nx\n"),
            ("agents/nobase.md", "---\ntitle: x\n---\nx\n"),
        ]);
        let ex = &a.roles["executor"];
        assert_eq!((ex.origin, ex.model.as_deref()), (Origin::Override, Some("fake/cheap")));
        assert!(ex.prompt.contains("You are the **executor**"), "an empty body keeps the built-in prompt");
        assert!(ex.full_prompt().contains("Follow Clean ABAP."));
        let sec = &a.roles["security-reviewer"];
        assert_eq!((sec.class, sec.origin, sec.files), (Role::Reviewer, Origin::Custom, FileAccess::Read));
        assert!(!sec.can(Capability::TaskCheck) && sec.can(Capability::StatusApprove));
        assert!(sec.available_in("shop") && !sec.available_in("erp"));
        assert_eq!(sec.prompt.trim(), "You review security.");
        assert!(a.roles["qa"].can(Capability::TaskCheck));
        let errs = errors(&a);
        assert!(errs.iter().any(|e| e.contains("role:bad") && e.contains("not a team class")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("role:bad") && e.contains("unknown permission `fly`")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("role:nobase") && e.contains("needs `base`")), "{errs:?}");
        assert!(!a.roles.contains_key("bad") && !a.roles.contains_key("nobase"));
        let warnings: Vec<_> = a.problems.iter().filter(|p| p.level == Level::Warning).map(|p| p.message.clone()).collect();
        assert!(warnings.iter().any(|w| w.contains("skill `clean-abap` is not installed")), "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("`semgrep` is not in mcp.json")), "{warnings:?}");
    }

    #[test]
    fn a_broken_file_keeps_the_last_valid_version() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("agents")).unwrap();
        let cfg = Config::load(dir.path()).unwrap();
        std::fs::write(dir.path().join("agents/qa.md"), "---\nbase: tester\n---\nYou test.\n").unwrap();
        let first = AgentConfig::load(dir.path(), &cfg, None);
        assert!(first.roles.contains_key("qa"));
        std::fs::write(dir.path().join("agents/qa.md"), "---\nbase: tester\nallow: [oops]\n---\nYou test.\n").unwrap();
        let second = AgentConfig::load(dir.path(), &cfg, Some(&first));
        assert!(second.roles["qa"].stale, "the previous version stays in use");
        std::fs::write(dir.path().join("agents/executor.md"), "---\nfiles: sometimes\n---\n").unwrap();
        let third = AgentConfig::load(dir.path(), &cfg, None);
        assert!(third.roles["executor"].stale && third.roles["executor"].origin == Origin::Builtin, "falls back to the built-in role");
    }

    #[test]
    fn templates_are_checked_against_roles_and_relations() {
        let (_d, a) = load(&[
            (
                "teams/security-review.json",
                r#"{"title": "Проверка безопасности", "members": [{"role": "executor"}, {"role": "reviewer"}, {"role": "reviewer", "key": "second"}],
                   "relations": [{"from": "executor", "to": ["reviewer", "second"], "type": "handoff", "on": "review"},
                                 {"from": "reviewer", "to": "orchestrator", "type": "reports"}]}"#,
            ),
            (
                "teams/loop.json",
                r#"{"members": [{"role": "analyst"}, {"role": "executor"}], "relations": [{"from": "analyst", "to": "executor", "type": "handoff"}, {"from": "executor", "to": "analyst", "type": "handoff"}]}"#,
            ),
            ("teams/early.json", r#"{"stage": "refinement", "members": [{"role": "executor"}]}"#),
            ("teams/dup.json", r#"{"members": [{"role": "reviewer"}, {"role": "reviewer"}]}"#),
            ("teams/ghost.json", r#"{"members": [{"role": "wizard"}]}"#),
            ("teams/lonely.json", r#"{"members": [{"role": "documenter"}], "relations": []}"#),
            (
                "config.json",
                r#"{"teams": {"legacy": {"description": "old style", "worktree": true, "members": [{"role": "executor"}, {"role": "reviewer"}]}}}"#,
            ),
        ]);
        let sec = &a.teams["security-review"];
        assert_eq!((sec.origin, sec.workspace, sec.members.len()), (Origin::Custom, Workspace::Worktree, 3));
        let errs = errors(&a);
        assert!(errs.iter().any(|e| e.starts_with("team:loop") && e.contains("never comes")), "{errs:?}");
        assert!(errs.iter().any(|e| e.starts_with("team:early") && e.contains("does not work before `ready`")), "{errs:?}");
        assert!(errs.iter().any(|e| e.starts_with("team:dup") && e.contains("share the key")), "{errs:?}");
        assert!(errs.iter().any(|e| e.starts_with("team:ghost") && e.contains("unknown role `wizard`")), "{errs:?}");
        let lonely: Vec<_> = a.problems.iter().filter(|p| p.item == "team:lonely").map(|p| p.message.clone()).collect();
        assert!(lonely.iter().any(|w| w.contains("nobody reports")) && lonely.iter().any(|w| w.contains("status.submit")), "{lonely:?}");
        let legacy = &a.teams["legacy"];
        assert_eq!((legacy.origin, legacy.workspace), (Origin::Legacy, Workspace::Worktree));
        assert!(legacy.relations_derived);
        assert!(legacy.relations.iter().any(|r| r.kind == RelKind::Handoff && r.on == Some(Status::Review)));
    }

    #[test]
    fn default_relations_mirror_the_built_in_process() {
        let m = |v: &[(&str, Role)]| v.iter().map(|(k, r)| (k.to_string(), *r)).collect::<Vec<_>>();
        let full = default_relations(
            &m(&[
                ("analyst", Role::Analyst),
                ("executor", Role::Executor),
                ("tester", Role::Tester),
                ("reviewer", Role::Reviewer),
                ("documenter", Role::Documenter),
            ]),
            false,
        );
        let has = |from: &str, to: &str, kind: RelKind, on: Option<Status>| {
            full.iter().any(|r| r.from == from && r.to.iter().any(|t| t == to) && r.kind == kind && r.on == on)
        };
        assert!(has("analyst", "executor", RelKind::Handoff, None));
        assert!(has("executor", "reviewer", RelKind::Handoff, Some(Status::Review)));
        assert!(has("executor", "tester", RelKind::Handoff, Some(Status::Review)));
        assert!(has("reviewer", "documenter", RelKind::Handoff, Some(Status::Approved)));
        assert!(has("reviewer", "orchestrator", RelKind::Reports, None));
        let research = default_relations(&m(&[("analyst", Role::Analyst), ("reviewer", Role::Reviewer)]), true);
        assert!(research.iter().any(|r| r.from == "analyst" && r.kind == RelKind::Reports));
    }

    #[test]
    fn mcp_connections_and_grants() {
        // SAFETY: tests in this module do not read this variable concurrently.
        unsafe { std::env::set_var("GENIE_TEST_MCP_TOKEN", "s3cret") };
        let (_d, a) = load(&[
            (
                "mcp.json",
                r#"{"mcpServers": {"sap": {"command": "npx", "args": ["x"], "projects": ["erp"], "description": "SAP"},
                                    "github": {"type": "http", "url": "https://x", "headers": {"Authorization": "Bearer ${env:GENIE_TEST_MCP_TOKEN}"}},
                                    "broken": {"args": []}}}"#,
            ),
            ("agents/dev.md", "---\nbase: executor\nmcp: [sap, \"github:get_*\"]\n---\nx\n"),
        ]);
        assert!(errors(&a).iter().any(|e| e.contains("mcp:broken")));
        let dev = &a.roles["dev"];
        let erp: Vec<_> = a.mcp_for("erp", dev).into_iter().map(|(s, t)| (s.id.clone(), t)).collect();
        assert_eq!(erp, vec![("github".to_string(), Some(vec!["get_*".to_string()])), ("sap".to_string(), None)]);
        assert_eq!(a.mcp_for("shop", dev).len(), 1, "sap is only for erp");
        assert_eq!(a.mcp["github"].resolved()["headers"]["Authorization"], "Bearer s3cret");
        assert!(serde_json::to_string(&a.mcp["github"]).unwrap().find("Bearer").is_none(), "the entry is never serialised");
    }

    #[test]
    fn skills_are_found_with_their_descriptions() {
        let (_d, a) = load(&[
            ("skills/clean-abap/SKILL.md", "---\nname: clean-abap\ndescription: Clean ABAP rules.\n---\n# Clean ABAP\n"),
            ("skills/nested/deep/owasp/SKILL.md", "---\nname: owasp\ndescription: OWASP checks.\n---\nx"),
            ("skills/broken/SKILL.md", "---\nname: broken\n---\nx"),
        ]);
        assert_eq!(a.skills.keys().collect::<Vec<_>>(), vec!["clean-abap", "owasp"]);
        assert!(errors(&a).iter().any(|e| e.contains("skill:broken")));
    }
}
