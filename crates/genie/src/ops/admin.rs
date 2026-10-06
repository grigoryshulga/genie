//! The server and its projects and people: what admins do in the web's settings.

use std::path::Path;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Cx, Entry, Need, Op, Out, enc, register};

pub fn register(all: &mut Vec<Entry>) {
    register!(
        all,
        ProjectList,
        ProjectAdd,
        ProjectUpdate,
        ProjectMembers,
        ProjectMember,
        ProjectRemoveMember,
        ProjectInvite,
        ProjectEvents,
        UserList,
        UserAdd,
        UserUpdate,
        UserPasswd,
        UserToken,
        Doctor,
        Stats,
        VaultSync,
        ModelPrices
    );
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default()
}

/// A path on the server: relative ones that exist here are made absolute (the command line on the server's machine).
fn server_path(cx: &Cx, p: Option<String>) -> Option<String> {
    let p = p.filter(|p| !p.trim().is_empty())?;
    if cx.local
        && Path::new(&p).is_relative()
        && let Ok(abs) = std::fs::canonicalize(&p)
    {
        return Some(abs.to_string_lossy().into_owned());
    }
    Some(p)
}

/// The id of the user with this login.
async fn user_id(cx: &Cx, login: &str) -> Result<i64, String> {
    let users = cx.call("GET", "/users", None).await?;
    let login = login.trim().to_lowercase();
    users
        .as_array()
        .and_then(|a| a.iter().find(|u| u["login"] == json!(login)))
        .and_then(|u| u["id"].as_i64())
        .ok_or(format!("no user {login}"))
}

/// The project named, else the one the caller acts in.
async fn project(cx: &Cx, p: Option<String>) -> Result<String, String> {
    if let Some(p) = p.or_else(|| cx.project.clone()).filter(|p| !p.is_empty()) {
        return Ok(p);
    }
    let me = cx.call("GET", "/auth/me", None).await?;
    me["project"].as_str().or(me["agent"]["project"].as_str()).map(str::to_string).ok_or("which project? pass it".into())
}

/// Projects you have access to: slug, name, autonomy, repository, tracker.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct ProjectList {}

impl Op for ProjectList {
    const GROUP: &'static str = "project";
    const NAME: &'static str = "list";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/projects", None).await?;
        let text = v
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|p| {
                format!(
                    "{:<16} {:<24} {:<10} {}  tracker {}",
                    s(p, "slug"),
                    s(p, "name"),
                    s(p, "autonomy"),
                    p["repo"].as_str().unwrap_or("(no code)"),
                    s(p, "trackerDir")
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Out::new(if text.is_empty() { "no projects".into() } else { text }, v))
    }
}

/// Add a project (server admins). With --repo, a repository's existing `.genie/` tracker is reused.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectAdd {
    /// Lowercase latin letters, digits and dashes.
    pub slug: String,
    #[arg(long, default_value = "")]
    #[serde(default)]
    pub name: String,
    /// The project's git repository on the server (omit for projects without code).
    #[arg(long)]
    pub repo: Option<String>,
    /// An existing tracker directory on the server to register in place.
    #[arg(long)]
    pub tracker: Option<String>,
    /// Task id prefix for a new tracker (G, PAY…).
    #[arg(long)]
    pub prefix: Option<String>,
}

impl Op for ProjectAdd {
    const GROUP: &'static str = "project";
    const NAME: &'static str = "add";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let body = json!({ "slug": self.slug, "name": self.name, "repo": server_path(cx, self.repo), "tracker": server_path(cx, self.tracker), "prefix": self.prefix });
        let p = cx.call("POST", "/projects", Some(body)).await?;
        let text = format!(
            "project {} ({}) — tracker {}{}",
            s(&p, "slug"),
            s(&p, "name"),
            s(&p, "trackerDir"),
            p["repo"].as_str().map(|r| format!(", repo {r}")).unwrap_or_default()
        );
        Ok(Out::new(text, p))
    }
}

/// Change a project's name, autonomy (autonomous, assisted, manual) or how finished work is integrated (project admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUpdate {
    pub slug: String,
    #[arg(long)]
    pub name: Option<String>,
    /// autonomous: the orchestrator closes tasks itself; assisted: people close them; manual: no server orchestrator.
    #[arg(long)]
    pub autonomy: Option<String>,
    /// How finished work gets integrated, e.g. "the owner reviews the branch and merges it".
    #[arg(long, allow_hyphen_values = true)]
    pub integration: Option<String>,
}

impl Op for ProjectUpdate {
    const GROUP: &'static str = "project";
    const NAME: &'static str = "update";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let body = json!({ "name": self.name, "autonomy": self.autonomy, "integration": self.integration });
        let p = cx.call("PATCH", &format!("/projects/{}", enc(&self.slug)), Some(body)).await?;
        let text = format!("project {} ({}) · {} · integration: {}", s(&p, "slug"), s(&p, "name"), s(&p, "autonomy"), s(&p, "integration"));
        Ok(Out::new(text, p))
    }
}

/// People of a project and their roles (default: the project you act in).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct ProjectMembers {
    pub project: Option<String>,
}

impl Op for ProjectMembers {
    const GROUP: &'static str = "project";
    const NAME: &'static str = "members";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let slug = project(cx, self.project).await?;
        let v = cx.call("GET", &format!("/projects/{}/members", enc(&slug)), None).await?;
        let text = v
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|m| format!("{:<16} {:<24} {}", s(&m["user"], "login"), s(&m["user"], "name"), m["role"].as_str().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Out::new(if text.is_empty() { format!("nobody in {slug} yet") } else { text }, v))
    }
}

/// Give a person a role in a project: viewer, member, admin or owner (project admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct ProjectMember {
    pub project: String,
    pub login: String,
    pub role: String,
}

impl Op for ProjectMember {
    const GROUP: &'static str = "project";
    const NAME: &'static str = "member";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = user_id(cx, &self.login).await?;
        let v = cx.call("PUT", &format!("/projects/{}/members/{id}", enc(&self.project)), Some(json!({ "role": self.role }))).await?;
        Ok(Out::new(format!("{} is {} in {}", self.login, self.role, self.project), v))
    }
}

/// Take a person out of a project (project admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct ProjectRemoveMember {
    pub project: String,
    pub login: String,
}

impl Op for ProjectRemoveMember {
    const GROUP: &'static str = "project";
    const NAME: &'static str = "remove-member";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = user_id(cx, &self.login).await?;
        let v = cx.call("DELETE", &format!("/projects/{}/members/{id}", enc(&self.project)), None).await?;
        Ok(Out::new(format!("{} left {}", self.login, self.project), v))
    }
}

/// An invitation link to a project (project admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct ProjectInvite {
    pub project: String,
    /// viewer, member, admin or owner.
    #[arg(long, default_value = "member")]
    #[serde(default = "member")]
    pub role: String,
    /// Only this address may use it.
    #[arg(long)]
    pub email: Option<String>,
}

fn member() -> String {
    "member".into()
}

impl Op for ProjectInvite {
    const GROUP: &'static str = "project";
    const NAME: &'static str = "invite";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx
            .call("POST", &format!("/projects/{}/invites", enc(&self.project)), Some(json!({ "role": self.role, "email": self.email })))
            .await?;
        Ok(Out::new(s(&v, "url").to_string(), v))
    }
}

/// What happened in the project, event by event: the most recent ones, or those after --after (oldest first).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct ProjectEvents {
    /// Events after this one (the last id printed before).
    #[arg(long)]
    pub after: Option<i64>,
    #[arg(long, default_value_t = 30)]
    #[serde(default = "thirty")]
    pub limit: i64,
}

fn thirty() -> i64 {
    30
}

impl Op for ProjectEvents {
    const GROUP: &'static str = "project";
    const NAME: &'static str = "events";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let limit = self.limit.clamp(1, 1000);
        let after = match self.after {
            Some(a) => a,
            None => (cx.call("GET", "/journal?limit=0", None).await?["last"].as_i64().unwrap_or(0) - limit).max(0),
        };
        let v = cx.call("GET", &format!("/journal?after={after}&limit={limit}"), None).await?;
        let mut out = Vec::new();
        for e in v["events"].as_array().cloned().unwrap_or_default() {
            let payload = match &e["payload"] {
                Value::Object(o) if o.is_empty() => String::new(),
                Value::Null => String::new(),
                p => {
                    let text = p.to_string();
                    if text.chars().count() > 160 {
                        format!(" {}…", text.chars().take(160).collect::<String>())
                    } else {
                        format!(" {text}")
                    }
                }
            };
            out.push(format!(
                "#{:<6} {} {:<22} {}{} ({}){payload}",
                e["id"].to_string(),
                s(&e, "at"),
                s(&e, "type"),
                e["subject"].as_str().map(|x| format!("{x} ")).unwrap_or_default(),
                s(&e, "actor"),
                s(&e, "actorRole")
            ));
        }
        Ok(Out::new(if out.is_empty() { "nothing yet".into() } else { out.join("\n") }, v))
    }
}

/// People with access to the server.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct UserList {}

impl Op for UserList {
    const GROUP: &'static str = "user";
    const NAME: &'static str = "list";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/users", None).await?;
        let text = v
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|u| {
                format!(
                    "{:<16} {:<24} {}{}",
                    s(u, "login"),
                    s(u, "name"),
                    if u["isAdmin"] == json!(true) { "admin" } else { "" },
                    if u["disabled"] == json!(true) { " (disabled)" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Out::new(if text.is_empty() { "no users: the server is open from this machine only".into() } else { text }, v))
    }
}

/// The password: from stdin on the command line, never as an argument.
fn password(cx: &Cx, stdin: bool, given: Option<String>) -> Result<Option<String>, String> {
    if stdin && cx.local {
        let mut p = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut p).map_err(|e| e.to_string())?;
        return Ok(Some(p.trim_end_matches(['\n', '\r']).to_string()));
    }
    Ok(given.filter(|p| !p.is_empty()))
}

/// Add a person to the server (server admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserAdd {
    pub login: String,
    #[arg(long, default_value = "")]
    #[serde(default)]
    pub name: String,
    #[arg(long)]
    pub email: Option<String>,
    /// A server admin: adds projects and people.
    #[arg(long)]
    #[serde(default)]
    pub admin: bool,
    /// Read the password from stdin.
    #[arg(long)]
    #[serde(skip)]
    #[schemars(skip)]
    pub password_stdin: bool,
    /// The password (at least 8 characters); on the command line use --password-stdin.
    #[arg(skip)]
    pub password: Option<String>,
}

impl Op for UserAdd {
    const GROUP: &'static str = "user";
    const NAME: &'static str = "add";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let password = password(cx, self.password_stdin, self.password)?;
        let body = json!({ "login": self.login, "name": self.name, "email": self.email, "password": password, "isAdmin": self.admin });
        let u = cx.call("POST", "/users", Some(body)).await?;
        let mut text = format!("user {} ({}){}", s(&u, "login"), s(&u, "name"), if u["isAdmin"] == json!(true) { ", admin" } else { "" });
        if password.is_none() {
            text.push_str(&format!("\nno password yet: echo '<password>' | genie user passwd {}", s(&u, "login")));
        }
        Ok(Out::new(text, u))
    }
}

/// Change a person's name, email, admin rights, or disable them (server admins; your own name and email yourself).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserUpdate {
    pub login: String,
    #[arg(long)]
    pub name: Option<String>,
    /// An empty value removes it.
    #[arg(long)]
    pub email: Option<String>,
    #[arg(long)]
    pub admin: Option<bool>,
    /// A disabled person cannot log in; their tokens stop working.
    #[arg(long)]
    pub disabled: Option<bool>,
}

impl Op for UserUpdate {
    const GROUP: &'static str = "user";
    const NAME: &'static str = "update";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = user_id(cx, &self.login).await?;
        let mut body = json!({ "name": self.name, "isAdmin": self.admin, "disabled": self.disabled });
        if let Some(e) = self.email {
            body["email"] = if e.is_empty() { Value::Null } else { json!(e) };
        }
        let u = cx.call("PATCH", &format!("/users/{id}"), Some(body)).await?;
        let text = format!(
            "user {} ({}){}{}",
            s(&u, "login"),
            s(&u, "name"),
            if u["isAdmin"] == json!(true) { ", admin" } else { "" },
            if u["disabled"] == json!(true) { ", disabled" } else { "" }
        );
        Ok(Out::new(text, u))
    }
}

/// Set a person's password (server admins); their sessions close.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct UserPasswd {
    pub login: String,
    /// The new password; on the command line it is read from stdin.
    #[arg(skip)]
    pub password: Option<String>,
}

impl Op for UserPasswd {
    const GROUP: &'static str = "user";
    const NAME: &'static str = "passwd";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let password = password(cx, true, self.password)?.ok_or("pass the new password")?;
        let id = user_id(cx, &self.login).await?;
        let v = cx.call("POST", &format!("/users/{id}/password"), Some(json!({ "password": password }))).await?;
        Ok(Out::new(format!("password of {} updated", self.login), v))
    }
}

/// A personal token for the command line and MCP clients (GENIE_TOKEN): your own, or anyone's on the server's machine.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct UserToken {
    pub login: String,
    /// What the token is for.
    #[arg(long, default_value = "cli")]
    #[serde(default)]
    pub label: String,
}

impl Op for UserToken {
    const GROUP: &'static str = "user";
    const NAME: &'static str = "token";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = user_id(cx, &self.login).await?;
        let v = cx.call("POST", &format!("/users/{id}/tokens"), Some(json!({ "label": self.label }))).await?;
        Ok(Out::new(s(&v, "token").to_string(), v))
    }
}

/// Whether the running server is ready: data, web, people, projects, pi and models, sandbox, git, channels (server admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Doctor {}

impl Op for Doctor {
    const GROUP: &'static str = "server";
    const NAME: &'static str = "doctor";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/doctor", None).await?;
        let checks = v["checks"].as_array().cloned().unwrap_or_default();
        let mut lines = Vec::new();
        for c in &checks {
            let mark = match s(c, "level") {
                "ok" => "ok  ",
                "warn" => "warn",
                _ => "FAIL",
            };
            lines.push(format!("{mark} {:<9} {}", s(c, "area"), s(c, "text")));
            if let Some(h) = c["hint"].as_str() {
                lines.push(format!("               → {h}"));
            }
        }
        let count = |l: &str| checks.iter().filter(|c| c["level"] == json!(l)).count();
        let failed = count("fail");
        lines.push(String::new());
        lines.push(match (failed, count("warn")) {
            (0, 0) => "everything is ready".to_string(),
            (0, w) => format!("ready, {w} warning(s)"),
            (f, _) => format!("{f} problem(s) to fix before people and agents can work"),
        });
        Ok(Out::new(lines.join("\n"), v).failing_if(failed > 0, format!("{failed} check(s) failed")))
    }
}

/// What happened over the last days: tasks, decisions, reviews, agent runs, knowledge; with --project one project only (server admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Stats {
    #[arg(long, default_value_t = 7)]
    #[serde(default = "week")]
    pub days: i64,
}

fn week() -> i64 {
    7
}

impl Op for Stats {
    const GROUP: &'static str = "server";
    const NAME: &'static str = "stats";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let q = cx.project.as_ref().map(|p| format!("&project={}", enc(p))).unwrap_or_default();
        let v = cx.call("GET", &format!("/stats?days={}{q}", self.days), None).await?;
        let stats: crate::stats::Stats = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
        Ok(Out::new(crate::stats::render(&stats), v))
    }
}

/// Sync the knowledge vault with its git remote now (server admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct VaultSync {}

impl Op for VaultSync {
    const GROUP: &'static str = "server";
    const NAME: &'static str = "vault-sync";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", "/vault/sync", Some(json!({}))).await?;
        let st = &v["last"];
        if st["ok"] != json!(true) {
            return Err(st["error"].as_str().unwrap_or("the sync failed").to_string());
        }
        let mut text =
            format!("vault synced with {} ({}): {} commit(s) in, {} out", s(st, "remote"), s(st, "branch"), st["pulled"], st["pushed"]);
        let both: Vec<&str> = st["both"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        if !both.is_empty() {
            text.push_str(&format!("\nchanged on both sides (the server's lines kept where they overlap): {}", both.join(", ")));
        }
        Ok(Out::new(text, v))
    }
}

/// The model prices in effect and where each one comes from; --refresh asks LiteLLM for the tariffs again (server admins).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct ModelPrices {
    /// Pull the config: fetch the tariffs from LiteLLM now (no timer does it).
    #[arg(long)]
    pub refresh: bool,
}

impl Op for ModelPrices {
    const GROUP: &'static str = "server";
    const NAME: &'static str = "prices";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = if self.refresh {
            cx.call("POST", "/model-prices", Some(json!({}))).await?
        } else {
            cx.call("GET", "/model-prices", None).await?
        };
        let mut lines = Vec::new();
        if let Some(n) = v["received"].as_u64() {
            lines.push(format!("received: {n} model price(s) from LiteLLM at {}", s(&v, "fetchedAt")));
            let models = v["priced"].as_u64().unwrap_or(n);
            lines.push(format!("in effect: {models} price(s) (modelPrices of config.json overrides per field)"));
            return Ok(Out::new(lines.join("\n"), v));
        }
        let models = v["models"].as_array().cloned().unwrap_or_default();
        lines.push(match v["fetchedAt"].as_str() {
            Some(at) => format!("LiteLLM's tariffs received at {at}"),
            None => "LiteLLM's tariffs have not been received yet".to_string(),
        });
        if let Some(e) = v["lastError"].as_str().filter(|e| !e.is_empty()) {
            lines.push(format!("the last fetch failed: {e}"));
        }
        lines.push(String::new());
        for m in &models {
            let p = &m["price"];
            let price = |x: &Value| x.as_f64().map(|x| format!("${}", (x * 10_000.0).round() / 10_000.0)).unwrap_or_else(|| "—".into());
            lines.push(format!(
                "{:<42} in {} out {} cache {} / {}  ({})",
                s(m, "model"),
                price(&p["input"]),
                price(&p["output"]),
                price(&p["cacheRead"]),
                price(&p["cacheWrite"]),
                s(m, "source")
            ));
        }
        Ok(Out::new(lines.join("\n"), v))
    }
}
