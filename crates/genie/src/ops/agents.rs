//! The agent configuration of the server: roles, team templates, skills and MCP
//! connections. They are files in the data directory; the server reads and
//! writes them, checks a file before saving it and keeps the history of changes.
//! Everyone in a project reads them, server admins change them.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Cx, Entry, Need, Op, Out, enc, register};

pub fn register(all: &mut Vec<Entry>) {
    register!(all, List, Check, Show, Save, Delete, History, Preview, McpCheck, McpCalls);
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default()
}

fn strs(v: &Value) -> Vec<&str> {
    v.as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default()
}

/// `a, b` or `-` for nothing.
fn joined(v: &Value) -> String {
    let items = strs(v);
    if items.is_empty() { "-".into() } else { items.join(", ") }
}

/// Problems of the configuration, one per line.
fn problems(v: &Value) -> Vec<String> {
    v.as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|p| {
            format!(
                "{} {}{}: {}",
                if s(p, "level") == "error" { "error  " } else { "warning" },
                s(p, "item"),
                p["path"].as_str().map(|x| format!(" ({x})")).unwrap_or_default(),
                s(p, "message")
            )
        })
        .collect()
}

/// What an item of the configuration names.
enum Item {
    Role(String),
    Team(String),
    Skill(String),
    /// A skill's file other than SKILL.md.
    SkillFile(String, String),
    Mcp,
}

const ITEMS: &str = "role:<id>, team:<id>, skill:<name>, skill:<name>/<file> or mcp";

impl Item {
    fn parse(text: &str) -> Result<Item, String> {
        let bad = || format!("{text}: name a {ITEMS}");
        if text == "mcp" || text == "mcp.json" {
            return Ok(Item::Mcp);
        }
        let (kind, rest) = text.split_once(':').ok_or_else(bad)?;
        let rest = rest.trim().trim_end_matches('/');
        if rest.is_empty() {
            return Err(bad());
        }
        Ok(match kind {
            "role" => Item::Role(rest.into()),
            "team" | "template" => Item::Team(rest.into()),
            "skill" => match rest.split_once('/') {
                Some((name, file)) => Item::SkillFile(name.into(), file.into()),
                None => Item::Skill(rest.into()),
            },
            _ => return Err(bad()),
        })
    }

    /// Where it is in the API.
    fn path(&self) -> String {
        match self {
            Item::Role(id) => format!("/roles/{}", enc(id)),
            Item::Team(id) => format!("/templates/{}", enc(id)),
            Item::Skill(name) => format!("/skills/{}", enc(name)),
            Item::SkillFile(name, file) => {
                format!("/skills/{}/files/{}", enc(name), file.split('/').map(enc).collect::<Vec<_>>().join("/"))
            }
            Item::Mcp => "/mcp".into(),
        }
    }
}

/// Roles, team templates, skills and MCP connections of the server, whatever the project (server admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct List {}

impl Op for List {
    const GROUP: &'static str = "agents";
    const NAME: &'static str = "list";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/agent-config/report", None).await?;
        let mut out = vec!["Roles:".to_string()];
        for r in v["roles"].as_array().cloned().unwrap_or_default() {
            let statuses: Vec<&str> = strs(&r["capabilities"]).into_iter().filter(|c| c.starts_with("status.")).collect();
            out.push(format!(
                "  {:<20} {:<12} {:<9} {}{}",
                s(&r, "id"),
                s(&r, "class"),
                s(&r, "origin"),
                s(&r, "title"),
                if statuses.is_empty() { String::new() } else { format!(" · {}", statuses.join(", ")) }
            ));
        }
        out.push("Team templates:".into());
        for t in v["teams"].as_array().cloned().unwrap_or_default() {
            out.push(format!("  {:<20} {:<9} {} · {}", s(&t, "id"), s(&t, "origin"), s(&t, "title"), joined(&t["roles"])));
        }
        if !strs(&v["skills"]).is_empty() {
            out.push(format!("Skills: {}", joined(&v["skills"])));
        }
        if !strs(&v["mcp"]).is_empty() {
            out.push(format!("MCP connections: {}", joined(&v["mcp"])));
        }
        let n = v["problems"].as_array().map_or(0, Vec::len);
        if n > 0 {
            out.push(format!("{n} problem(s): genie agents check"));
        }
        Ok(Out::new(out.join("\n"), v))
    }
}

/// Check the configuration files as they are now: errors and warnings of roles, templates, skills and MCP, the sandbox; fails on errors (server admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Check {}

impl Op for Check {
    const GROUP: &'static str = "agents";
    const NAME: &'static str = "check";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/agent-config/report", None).await?;
        let mut out = vec![s(&v, "report").to_string()];
        if v["mcpAdapterLoaded"] == json!(true) {
            out.push("warning: pi loads pi-mcp-adapter, which replaces pi's native MCP support: `pi remove npm:pi-mcp-adapter`".into());
        }
        let sandbox = &v["sandbox"];
        out.push(format!("{}: {}", if sandbox["active"] == json!(true) { "sandbox" } else { "warning" }, s(sandbox, "note")));
        let errors = v["errors"].as_u64().unwrap_or(0);
        let why = format!("{errors} error(s) in the agent configuration of {}", s(&v, "data"));
        Ok(Out::new(out.join("\n"), v).failing_if(errors > 0, why))
    }
}

/// A role, team template, skill (or one of its files) or the MCP connections: the file, what uses it, its problems.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Show {
    /// role:<id>, team:<id>, skill:<name>, skill:<name>/<file> or mcp.
    pub item: String,
}

fn file_lines(out: &mut Vec<String>, v: &Value, builtin_hint: &str) {
    let f = &v["file"];
    match f["content"].as_str() {
        Some(content) => {
            out.push(format!("file {} · hash {}", s(f, "path"), s(f, "hash")));
            out.push(String::new());
            out.push(content.to_string());
        }
        None => {
            out.push(format!("built in, no file yet: {builtin_hint} writes {}", s(f, "path")));
            if let Some(text) = v["builtin"].as_str() {
                out.push(String::new());
                out.push(text.to_string());
            }
        }
    }
}

fn role_text(v: &Value, id: &str) -> String {
    let r = &v["role"];
    let mut head = vec![format!("class {}", s(r, "class")), s(r, "origin").to_string()];
    for (k, label) in [("extends", "extends"), ("model", "model"), ("thinking", "thinking")] {
        if let Some(x) = r[k].as_str() {
            head.push(format!("{label} {x}"));
        }
    }
    let mut out = vec![format!("role {id} — {}", s(r, "title")), head.join(" · ")];
    if !s(r, "description").is_empty() {
        out.push(s(r, "description").to_string());
    }
    out.push(format!("permissions: {}", joined(&r["capabilities"])));
    out.push(format!(
        "files {} · skills {} · mcp {} · stages {}{}",
        s(r, "files"),
        if r["skills"].is_null() { "all installed".to_string() } else { joined(&r["skills"]) },
        joined(&r["mcp"]),
        joined(&r["stages"]),
        if r["projects"].is_null() { String::new() } else { format!(" · projects {}", joined(&r["projects"])) }
    ));
    if !strs(&r["denyCommands"]).is_empty() {
        out.push(format!("denied commands: {}", joined(&r["denyCommands"])));
    }
    used_by(&mut out, &v["usedBy"]);
    out.extend(problems(&v["problems"]));
    file_lines(&mut out, v, &format!("`genie agents save role:{id} --file <role.md>`"));
    out.join("\n")
}

fn used_by(out: &mut Vec<String>, u: &Value) {
    let mut parts = Vec::new();
    if !strs(&u["templates"]).is_empty() {
        parts.push(format!("templates {}", joined(&u["templates"])));
    }
    let autos: Vec<String> = u["automations"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|a| format!("{} ({})", s(a, "name"), a["project"].as_str().unwrap_or_default()))
        .collect();
    if !autos.is_empty() {
        parts.push(format!("automations {}", autos.join(", ")));
    }
    if !parts.is_empty() {
        out.push(format!("used by {}", parts.join("; ")));
    }
}

fn template_text(v: &Value, id: &str) -> String {
    let t = &v["template"];
    let mut out = vec![
        format!("template {id} — {}", s(t, "title")),
        format!("stage {} · workspace {} · mail {} · {}", s(t, "stage"), s(t, "workspace"), s(t, "mail"), s(t, "origin")),
    ];
    if !s(t, "description").is_empty() {
        out.push(s(t, "description").to_string());
    }
    let members: Vec<String> = t["members"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|m| if s(m, "key") == s(m, "role") { s(m, "role").to_string() } else { format!("{} ({})", s(m, "key"), s(m, "role")) })
        .collect();
    out.push(format!("members: {}", members.join(", ")));
    for r in t["relations"].as_array().cloned().unwrap_or_default() {
        out.push(format!(
            "  {} {} {}{}",
            s(&r, "from"),
            s(&r, "type"),
            joined(&r["to"]),
            r["on"].as_str().map(|x| format!(" on {x}")).unwrap_or_default()
        ));
    }
    for w in strs(&t["warnings"]) {
        out.push(format!("warning: {w}"));
    }
    used_by(&mut out, &v["usedBy"]);
    out.extend(problems(&v["problems"]));
    file_lines(&mut out, v, &format!("`genie agents save team:{id} --file <template.json>`"));
    out.join("\n")
}

fn skill_text(v: &Value) -> String {
    let sk = &v["skill"];
    let mut out = vec![
        format!("skill {} — {}", s(sk, "name"), s(sk, "description")),
        format!(
            "{} · {}",
            s(sk, "dir"),
            if v["editable"] == json!(true) { "genie changes it" } else { "installed outside genie: change it there" }
        ),
    ];
    if !strs(&v["usedBy"]).is_empty() {
        out.push(format!("roles: {}", joined(&v["usedBy"])));
    }
    out.push(format!("files: {}", joined(&v["files"])));
    out.push(format!("hash {}", s(v, "hash")));
    out.push(String::new());
    out.push(v["content"].as_str().unwrap_or("(no SKILL.md)").to_string());
    out.join("\n")
}

fn mcp_text(v: &Value) -> String {
    let mut out = vec!["MCP connections:".to_string()];
    for m in v["servers"].as_array().cloned().unwrap_or_default() {
        out.push(format!(
            "  {:<16} {:<6} {:<8} projects {}{}",
            s(&m, "id"),
            s(&m, "transport"),
            if m["gateway"] == json!(false) { "direct" } else { "gateway" },
            if m["projects"].is_null() { "all".to_string() } else { joined(&m["projects"]) },
            if s(&m, "description").is_empty() { String::new() } else { format!(" — {}", s(&m, "description")) }
        ));
    }
    if out.len() == 1 {
        out.push("  (none)".into());
    }
    out.extend(problems(&v["problems"]));
    let f = &v["file"];
    match f["content"].as_str() {
        _ if f.is_null() => {}
        Some(content) => {
            out.push(format!("file {} · hash {}", s(f, "path"), s(f, "hash")));
            out.push(String::new());
            out.push(content.to_string());
        }
        None => out.push("no mcp.json yet: `genie agents save mcp --file mcp.json` writes it".into()),
    }
    out.join("\n")
}

impl Op for Show {
    const GROUP: &'static str = "agents";
    const NAME: &'static str = "show";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let item = Item::parse(&self.item)?;
        let v = cx.call("GET", &item.path(), None).await?;
        let text = match &item {
            Item::Role(id) => role_text(&v, id),
            Item::Team(id) => template_text(&v, id),
            Item::Skill(_) => skill_text(&v),
            Item::SkillFile(..) => match v["text"].as_str() {
                Some(text) => format!("{} ({} bytes)\n\n{text}", s(&v, "path"), v["size"]),
                None => format!("{} ({} bytes, not text)", s(&v, "path"), v["size"]),
            },
            Item::Mcp => mcp_text(&v),
        };
        Ok(Out::new(text, v))
    }
}

/// Write a role (Markdown with front matter), a team template (JSON), a skill's SKILL.md or another file of it, or mcp.json; the server checks it and refuses a broken one (server admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Save {
    /// role:<id>, team:<id>, skill:<name>, skill:<name>/<file> or mcp.
    pub item: String,
    /// The whole file (`-` reads stdin).
    #[arg(long, allow_hyphen_values = true)]
    pub text: Option<String>,
    /// Read the file from here (command line only).
    #[arg(long)]
    #[serde(skip)]
    #[schemars(skip)]
    pub file: Option<PathBuf>,
    /// The hash `genie agents show` printed: nothing is saved if the file changed since.
    #[arg(long)]
    pub base_hash: Option<String>,
}

impl Op for Save {
    const GROUP: &'static str = "agents";
    const NAME: &'static str = "save";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let item = Item::parse(&self.item)?;
        if let Item::SkillFile(..) = item {
            // Any file, text or not: its bytes as they are.
            let bytes = match (&self.file, self.text) {
                (Some(f), _) if cx.local => std::fs::read(f).map_err(|e| format!("{}: {e}", f.display()))?,
                (None, text) => cx.text(text, None)?.ok_or("pass the file's content with --text (or --file)")?.into_bytes(),
                (Some(_), _) => return Err("files are read only on the command line; pass the text itself".into()),
            };
            let v = cx.upload("PUT", &item.path(), bytes).await?;
            return Ok(Out::new(format!("saved {} ({} bytes)", s(&v, "path"), v["size"]), v));
        }
        let content = cx.text(self.text, self.file)?.ok_or("pass the file with --text (`-` reads stdin) or --file")?;
        let v = cx.call("PUT", &item.path(), Some(json!({ "content": content, "baseHash": self.base_hash }))).await?;
        let mut out = vec![format!("saved {} · hash {}", s(&v, "path"), s(&v, "hash"))];
        out.extend(problems(&v["problems"]));
        Ok(Out::new(out.join("\n"), v))
    }
}

/// Delete a role or template file (a built-in one goes back to its default), a skill with its files, or one file of a skill (server admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Delete {
    /// role:<id>, team:<id>, skill:<name> or skill:<name>/<file>.
    pub item: String,
    /// The hash `genie agents show` printed: nothing is deleted if the file changed since (roles and templates).
    #[arg(long)]
    pub base_hash: Option<String>,
}

impl Op for Delete {
    const GROUP: &'static str = "agents";
    const NAME: &'static str = "delete";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let item = Item::parse(&self.item)?;
        let query = match (&item, &self.base_hash) {
            (Item::Mcp, _) => return Err("mcp.json stays: save it without the connection (genie agents save mcp)".into()),
            (Item::Role(_) | Item::Team(_), Some(h)) => format!("?baseHash={}", enc(h)),
            _ => String::new(),
        };
        let v = cx.call("DELETE", &format!("{}{query}", item.path()), None).await?;
        let mut out = vec![format!("deleted {}", self.item)];
        out.extend(problems(&v["problems"]));
        Ok(Out::new(out.join("\n"), v))
    }
}

/// Changes to the agent configuration made through genie: who, when, which file (server admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct History {
    /// One item only: role:<id>, team:<id>, skill:<name> or mcp.
    #[arg(long)]
    pub item: Option<String>,
    #[arg(long, default_value_t = 20)]
    #[serde(default = "twenty")]
    pub limit: i64,
    /// Print the file before and after each change.
    #[arg(long)]
    #[serde(default)]
    pub full: bool,
}

fn twenty() -> i64 {
    20
}

impl Op for History {
    const GROUP: &'static str = "agents";
    const NAME: &'static str = "history";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let item = self.item.as_deref().map(|i| format!("&item={}", enc(if i == "mcp.json" { "mcp" } else { i }))).unwrap_or_default();
        let v = cx.call("GET", &format!("/agent-config/history?limit={}{item}", self.limit), None).await?;
        let mut out = Vec::new();
        for c in v.as_array().cloned().unwrap_or_default() {
            let what = match (c["before"].is_null(), c["after"].is_null()) {
                (true, _) => "created",
                (_, true) => "deleted",
                _ => "changed",
            };
            out.push(format!(
                "#{:<4} {} {:<12} {what} {} ({})",
                c["id"].to_string(),
                s(&c, "at"),
                s(&c, "user"),
                s(&c, "path"),
                s(&c, "item")
            ));
            if self.full {
                for (label, k) in [("before", "before"), ("after", "after")] {
                    if let Some(text) = c[k].as_str() {
                        out.push(format!("--- {label}"));
                        out.push(text.to_string());
                    }
                }
            }
        }
        Ok(Out::new(if out.is_empty() { "no changes made through genie".into() } else { out.join("\n") }, v))
    }
}

/// What each member of a team template gets as its kickoff, for a task of the project (or an example task).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Preview {
    pub template: String,
    #[arg(long)]
    pub task: Option<String>,
}

impl Op for Preview {
    const GROUP: &'static str = "agents";
    const NAME: &'static str = "preview";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", &format!("/templates/{}/preview", enc(&self.template)), Some(json!({ "task": self.task }))).await?;
        let mut out = vec![format!("template {} for {}", s(&v, "template"), s(&v, "task"))];
        for w in strs(&v["warnings"]) {
            out.push(format!("warning: {w}"));
        }
        for m in v["members"].as_array().cloned().unwrap_or_default() {
            out.push(String::new());
            out.push(format!("## {} ({})", s(&m, "name"), s(&m, "role")));
            out.push(s(&m, "kickoff").to_string());
        }
        Ok(Out::new(out.join("\n"), v))
    }
}

/// Connect to an MCP connection the way the gateway does for agents, and list its tools (server admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct McpCheck {
    pub server: String,
}

impl Op for McpCheck {
    const GROUP: &'static str = "agents";
    const NAME: &'static str = "mcp-check";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", &format!("/mcp/{}/check", enc(&self.server)), Some(json!({}))).await?;
        if v["ok"] != json!(true) {
            let why = format!("{}: {}", self.server, s(&v, "error"));
            return Ok(
                Out::new(format!("{} does not work ({} ms): {}", self.server, v["ms"], s(&v, "error")), v.clone()).failing_if(true, why)
            );
        }
        let info = &v["serverInfo"];
        let mut out = vec![format!(
            "{} works ({} ms): {} {} · protocol {}",
            self.server,
            v["ms"],
            s(info, "name"),
            s(info, "version"),
            s(&v, "protocolVersion")
        )];
        for t in v["tools"].as_array().cloned().unwrap_or_default() {
            let about = s(&t, "description").lines().next().unwrap_or_default();
            out.push(format!("  {}{}", s(&t, "name"), if about.is_empty() { String::new() } else { format!(" — {about}") }));
        }
        Ok(Out::new(out.join("\n"), v))
    }
}

/// Recent calls the project's agents made through the MCP gateway: tool, result, time (people of the project).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct McpCalls {
    #[arg(long, default_value_t = 30)]
    #[serde(default = "thirty")]
    pub limit: i64,
}

fn thirty() -> i64 {
    30
}

impl Op for McpCalls {
    const GROUP: &'static str = "agents";
    const NAME: &'static str = "mcp-calls";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", &format!("/mcp/calls?limit={}", self.limit), None).await?;
        let mut out = Vec::new();
        for e in v.as_array().cloned().unwrap_or_default() {
            let p = &e["payload"];
            out.push(format!(
                "{} {}{} ({}) {}:{} {} {} ms",
                s(&e, "at"),
                e["subject"].as_str().map(|t| format!("{t} ")).unwrap_or_default(),
                s(&e, "actor"),
                s(p, "role"),
                s(p, "server"),
                s(p, "tool"),
                if p["refused"] == json!(true) {
                    "refused"
                } else if p["ok"] == json!(true) {
                    "ok"
                } else {
                    "failed"
                },
                p["ms"]
            ));
            if let Some(err) = p["error"].as_str() {
                out.push(format!("    {err}"));
            }
        }
        Ok(Out::new(if out.is_empty() { "no MCP calls yet".into() } else { out.join("\n") }, v))
    }
}
