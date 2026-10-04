//! The catalog of genie operations: every action a person or an agent takes in
//! genie, described once and served through two entrances — the `genie` command
//! line and the genie MCP server — and listed in the role prompts.
//!
//! An operation is a struct of its arguments: clap reads it from the command
//! line, serde from an MCP call, and schemars gives MCP its JSON schema. Its
//! handler calls the server's HTTP API ([`api::Api`]) — over the network with a
//! token, or inside the server's process — and renders the answer as text for
//! people and language models. The API checks every right; the catalog only
//! describes and renders.

use std::future::Future;
use std::path::PathBuf;
use std::sync::OnceLock;

use clap::{ArgAction, ArgMatches, Command};
use futures_util::future::BoxFuture;
use genie_core::Capability;
use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

mod admin;
mod agents;
pub mod api;
mod automations;
mod docs;
mod mail;
mod me;
pub mod render;
mod repos;
mod tasks;
mod teams;
pub mod tools;

pub use api::{Api, Auth, InProcess, Payload, Remote};

/// Who an operation is for. The API decides every call; this only keeps an
/// operation out of the lists of those who cannot use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Need {
    /// Anyone with access to the project.
    Read,
    /// People who write in the project, and agents whose role allows it.
    Write,
    /// The orchestrator and people.
    Orchestrator,
    /// Agents of a team and the orchestrator; not people.
    Agent,
    /// A member of a team (its own status line, replies).
    Member,
    /// A one-shot job of an automation.
    Job,
    /// People of the project, whatever their role; never agents.
    Person,
    /// People who administer the server or a project; never agents.
    Admin,
}

/// An agent genie runs, as the catalog sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentKind {
    /// Holds every permission.
    Orchestrator,
    /// A member of a team, with its role's permissions.
    Member,
    /// A one-shot job: no team, no mail.
    Job,
}

/// Which agents' prompts list an operation in their command table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listed {
    Nobody,
    /// Every agent that may use it.
    Agents,
    /// The orchestrator only, though others may use it.
    Orchestrator,
}

/// What an operation gives back: text for people and models, the API's data for scripts.
#[derive(Debug, Clone)]
pub struct Out {
    pub text: String,
    pub data: Value,
    /// A report that found problems: shown in full, then the command fails with this.
    pub failed: Option<String>,
}

impl Out {
    pub fn new(text: impl Into<String>, data: Value) -> Out {
        Out { text: text.into(), data, failed: None }
    }

    /// Fail after showing the report when `problem` holds.
    pub fn failing_if(mut self, problem: bool, why: impl Into<String>) -> Out {
        if problem {
            self.failed = Some(why.into());
        }
        self
    }
}

/// Where an operation runs: its way to the API and the caller's defaults.
pub struct Cx {
    pub api: Box<dyn Api>,
    /// The project the caller named (`--project`), if any; the API acts in it.
    pub project: Option<String>,
    /// The caller's own task and team (an agent's), for operations that omit them.
    pub task: Option<String>,
    pub team: Option<String>,
    /// The caller's files and stdin are at hand (the command line); not over MCP.
    pub local: bool,
}

impl Cx {
    pub async fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        self.api.call(method, path, body).await
    }

    /// Call with a typed request body (the server reads the same struct).
    pub async fn send<B: serde::Serialize>(&self, method: &str, path: &str, body: &B) -> Result<Value, String> {
        self.call(method, path, Some(serde_json::to_value(body).map_err(|e| e.to_string())?)).await
    }

    /// A request whose body is a file's bytes.
    pub async fn upload(&self, method: &str, path: &str, bytes: Vec<u8>) -> Result<Value, String> {
        self.api.request(method, path, Payload::Bytes(bytes)).await
    }

    /// The task an operation is about: the given one, else the caller's own.
    pub fn task(&self, task: Option<String>) -> Result<String, String> {
        task.or_else(|| self.task.clone()).filter(|t| !t.is_empty()).ok_or_else(|| "which task? pass it".to_string())
    }

    /// The team an operation is about: the given one, else the caller's own.
    pub fn team(&self, team: Option<String>) -> Result<String, String> {
        team.or_else(|| self.team.clone()).filter(|t| !t.is_empty()).ok_or_else(|| "which team? pass --team".to_string())
    }

    /// Text given inline, as `-` (stdin) or as a file: the last two only on the command line.
    pub fn text(&self, inline: Option<String>, file: Option<PathBuf>) -> Result<Option<String>, String> {
        if let Some(f) = file {
            if !self.local {
                return Err("files are read only on the command line; pass the text itself".into());
            }
            return std::fs::read_to_string(&f).map(Some).map_err(|e| format!("{}: {e}", f.display()));
        }
        match inline.as_deref() {
            Some("-") if self.local => {
                let mut s = String::new();
                std::io::Read::read_to_string(&mut std::io::stdin(), &mut s).map_err(|e| e.to_string())?;
                Ok(Some(s))
            }
            _ => Ok(inline),
        }
    }
}

/// An operation of the catalog.
pub trait Op: clap::Args + DeserializeOwned + JsonSchema + Send + Sized + 'static {
    /// `genie <GROUP> <NAME>` on the command line; tool `genie_<GROUP>`, action `NAME` over MCP.
    const GROUP: &'static str;
    const NAME: &'static str;
    /// The command under `genie agent …` before the catalog, kept working for older prompts.
    const LEGACY: Option<&'static str> = None;
    const NEED: Need = Need::Read;
    /// For agents: any of these permissions of their role (the orchestrator holds them all).
    const CAPS: &'static [Capability] = &[];
    /// In agents' command tables: the commands agents knew are.
    const LISTED: Listed = if Self::LEGACY.is_some() { Listed::Agents } else { Listed::Nobody };
    /// Whether an agent's role uses an argument (by its id); the command tables leave
    /// out the others. The orchestrator uses them all; the API decides anyway.
    fn arg_allowed(_arg: &str, _can: &dyn Fn(Capability) -> bool) -> bool {
        true
    }
    /// A remark for an agent's command table, e.g. the statuses its role may set.
    fn prompt_note(_kind: AgentKind, _can: &dyn Fn(Capability) -> bool) -> Option<String> {
        None
    }
    fn run(self, cx: &Cx) -> impl Future<Output = Result<Out, String>> + Send;
}

type RunCli = for<'a> fn(&'a ArgMatches, &'a Cx) -> BoxFuture<'a, Result<Out, String>>;
type RunJson = for<'a> fn(Value, &'a Cx) -> BoxFuture<'a, Result<Out, String>>;
type ArgAllowed = fn(&str, &dyn Fn(Capability) -> bool) -> bool;
type PromptNote = fn(AgentKind, &dyn Fn(Capability) -> bool) -> Option<String>;

/// An operation as the entrances see it.
pub struct Entry {
    pub group: &'static str,
    pub name: &'static str,
    pub legacy: Option<&'static str>,
    pub need: Need,
    pub caps: &'static [Capability],
    pub listed: Listed,
    /// What it does: the doc comment of its arguments.
    pub about: String,
    /// JSON schema of its arguments.
    pub schema: Value,
    augment: fn(Command) -> Command,
    run_cli: RunCli,
    run_json: RunJson,
    arg_allowed: ArgAllowed,
    prompt_note: PromptNote,
}

fn run_cli<'a, T: Op>(m: &'a ArgMatches, cx: &'a Cx) -> BoxFuture<'a, Result<Out, String>> {
    match T::from_arg_matches(m) {
        Ok(args) => Box::pin(args.run(cx)),
        Err(e) => Box::pin(async move { Err(e.to_string()) }),
    }
}

fn run_json<'a, T: Op>(args: Value, cx: &'a Cx) -> BoxFuture<'a, Result<Out, String>> {
    match serde_json::from_value::<T>(args) {
        Ok(args) => Box::pin(args.run(cx)),
        Err(e) => Box::pin(async move { Err(format!("invalid arguments: {e}")) }),
    }
}

impl Entry {
    fn of<T: Op>() -> Entry {
        let mut schema = schemars::schema_for!(T).to_value();
        let about = schema.get("description").and_then(Value::as_str).unwrap_or_default().to_string();
        if let Some(o) = schema.as_object_mut() {
            for k in ["$schema", "title", "description"] {
                o.remove(k);
            }
        }
        Entry {
            group: T::GROUP,
            name: T::NAME,
            legacy: T::LEGACY,
            need: T::NEED,
            caps: T::CAPS,
            listed: T::LISTED,
            about,
            schema,
            augment: <T as clap::Args>::augment_args,
            run_cli: run_cli::<T>,
            run_json: run_json::<T>,
            arg_allowed: T::arg_allowed,
            prompt_note: T::prompt_note,
        }
    }

    /// Whether an agent may use it: its need, then its role's permissions.
    pub fn for_agent(&self, kind: AgentKind, can: &dyn Fn(Capability) -> bool) -> bool {
        let fits = match self.need {
            Need::Read | Need::Write => true,
            Need::Orchestrator => kind == AgentKind::Orchestrator,
            Need::Agent => kind != AgentKind::Job,
            Need::Member => kind == AgentKind::Member,
            Need::Job => kind == AgentKind::Job,
            Need::Person | Need::Admin => false,
        };
        let alone = kind == AgentKind::Job && matches!(self.group, "team" | "mail");
        fits && !alone && (kind == AgentKind::Orchestrator || self.caps.is_empty() || self.caps.iter().any(|c| can(*c)))
    }

    /// Whether an agent's command table lists it.
    pub fn listed_for(&self, kind: AgentKind, can: &dyn Fn(Capability) -> bool) -> bool {
        let listed = match self.listed {
            Listed::Nobody => false,
            Listed::Agents => true,
            Listed::Orchestrator => kind == AgentKind::Orchestrator,
        };
        listed && self.for_agent(kind, can)
    }

    /// Its command line in short, with the arguments the agent uses:
    /// `genie task status <status> [--task …] [--note …]`.
    pub fn usage_for(&self, kind: AgentKind, can: &dyn Fn(Capability) -> bool) -> String {
        let cmd = self.command(self.name);
        let uses = |a: &&clap::Arg| !a.is_hide_set() && (kind == AgentKind::Orchestrator || (self.arg_allowed)(a.get_id().as_str(), can));
        let mut out = format!("genie {} {}", self.group, self.name);
        let (positionals, options): (Vec<&clap::Arg>, Vec<&clap::Arg>) = cmd.get_arguments().filter(uses).partition(|a| a.is_positional());
        for a in positionals.into_iter().chain(options) {
            let values: Vec<String> = a.get_possible_values().iter().map(|v| v.get_name().to_string()).collect();
            let value = if !values.is_empty() {
                values.join("|")
            } else if a.is_positional() {
                format!(
                    "<{}>",
                    a.get_value_names().and_then(|v| v.first()).map_or_else(|| a.get_id().as_str().replace('_', "-"), |v| v.to_string())
                )
            } else {
                "…".into()
            };
            let item = match (a.is_positional(), a.get_long()) {
                (true, _) => value,
                (false, Some(long)) if a.get_action().takes_values() => format!("--{long} {value}"),
                (false, Some(long)) => format!("--{long}"),
                (false, None) => continue,
            };
            let many = matches!(a.get_action(), ArgAction::Append);
            out.push(' ');
            out.push_str(&match (a.is_required_set(), many) {
                (true, false) => item,
                (true, true) => format!("{item}..."),
                (false, false) => format!("[{item}]"),
                (false, true) => format!("[{item}]..."),
            });
        }
        out
    }

    /// A remark for an agent's command table.
    pub fn note_for(&self, kind: AgentKind, can: &dyn Fn(Capability) -> bool) -> Option<String> {
        (self.prompt_note)(kind, can)
    }

    /// The first line of what it does.
    pub fn summary(&self) -> &str {
        self.about.lines().next().unwrap_or_default()
    }

    /// Its command line: `name` with its arguments (`list` answers to `ls` too).
    pub fn command(&self, name: &str) -> Command {
        let mut c = Command::new(name.to_string()).about(self.summary().to_string()).long_about(self.about.clone());
        if name == "list" {
            c = c.alias("ls");
        }
        (self.augment)(c)
    }

    pub fn run_cli<'a>(&self, m: &'a ArgMatches, cx: &'a Cx) -> BoxFuture<'a, Result<Out, String>> {
        (self.run_cli)(m, cx)
    }

    pub fn run_json<'a>(&self, args: Value, cx: &'a Cx) -> BoxFuture<'a, Result<Out, String>> {
        (self.run_json)(args, cx)
    }
}

/// The groups of operations, in the order people read them.
pub const GROUPS: &[(&str, &str)] = &[
    ("task", "Tasks and epics: read, create, update, move through statuses, comment, attach artifacts"),
    ("team", "Teams of agents: assemble, look at, steer and stop them"),
    ("mail", "Mail between the members of a team, the orchestrator and people"),
    ("docs", "The project's knowledge base: search, read, write pages"),
    ("repos", "Git repositories of a project: hosts, attach, policy, checks, and what a task may do in them"),
    ("pr", "Pull/merge requests of a task's branches: open, look at, comment, merge"),
    ("job", "One-shot jobs: start one, see what it did"),
    ("automation", "Automations: rules that act on events, schedules and webhooks, and their runs"),
    ("agents", "Roles, team templates, skills and MCP connections of the server"),
    ("project", "Projects of the server: add, settings, people, invitations, what happened"),
    ("user", "People with access to the server"),
    ("me", "You: who you are, your notifications and the questions agents asked you"),
    ("server", "The running server: readiness, what happened, knowledge sync"),
];

/// Every operation of genie.
pub fn catalog() -> &'static [Entry] {
    static ALL: OnceLock<Vec<Entry>> = OnceLock::new();
    ALL.get_or_init(|| {
        let mut all = Vec::new();
        tasks::register(&mut all);
        teams::register(&mut all);
        mail::register(&mut all);
        docs::register(&mut all);
        automations::register(&mut all);
        agents::register(&mut all);
        admin::register(&mut all);
        me::register(&mut all);
        repos::register(&mut all);
        all
    })
}

pub fn find(group: &str, name: &str) -> Option<&'static Entry> {
    catalog().iter().find(|e| e.group == group && e.name == name)
}

/// Push [`Entry::of`] for each type.
macro_rules! register {
    ($all:expr, $($t:ty),+ $(,)?) => {
        $($all.push($crate::ops::Entry::of::<$t>());)+
    };
}
pub(crate) use register;

/// The command line of the catalog: a subcommand per group, and `agent` with the
/// commands agents knew before it (`genie agent show` is `genie task show`).
pub fn commands(mut root: Command) -> Command {
    for (group, about) in GROUPS {
        let mut g = Command::new(*group).about(*about).subcommand_required(true).arg_required_else_help(true);
        for e in catalog().iter().filter(|e| e.group == *group) {
            g = g.subcommand(e.command(e.name));
        }
        root = root.subcommand(g);
    }
    let mut agent = Command::new("agent")
        .about("The commands of agents under their earlier names (genie agent show = genie task show)")
        .subcommand_required(true)
        .arg_required_else_help(true);
    let mut docs = Command::new("docs").about("Project knowledge (vault)").subcommand_required(true);
    for e in catalog() {
        match e.legacy {
            Some(l) if l.starts_with("docs ") => docs = docs.subcommand(e.command(&l[5..])),
            Some(l) => agent = agent.subcommand(e.command(l)),
            None => {}
        }
    }
    root.subcommand(agent.subcommand(docs))
}

/// The operation a command line names, with its arguments.
pub fn chosen(m: &ArgMatches) -> Option<(&'static Entry, &ArgMatches)> {
    let (group, gm) = m.subcommand()?;
    let (name, am) = gm.subcommand()?;
    if group == "agent" {
        if name == "docs" {
            let (sub, sm) = am.subcommand()?;
            let legacy = format!("docs {sub}");
            return catalog().iter().find(|e| e.legacy == Some(legacy.as_str())).map(|e| (e, sm));
        }
        return catalog().iter().find(|e| e.legacy == Some(name)).map(|e| (e, am));
    }
    find(group, name).map(|e| (e, am))
}

/// JSON given as text (the command line) or as itself (MCP).
pub fn json_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) => s,
        v => v.to_string(),
    })
}

/// [`json_text`] for an argument that may be left out (with `#[serde(default)]`).
pub fn opt_json_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Null => None,
        Value::String(s) => Some(s),
        v => Some(v.to_string()),
    })
}

/// `key=value` pairs as a JSON object.
pub fn pairs(items: &[String], what: &str) -> Result<serde_json::Map<String, Value>, String> {
    let mut out = serde_json::Map::new();
    for kv in items {
        let (k, v) = kv.split_once('=').ok_or(format!("{what} {kv}: expected key=value"))?;
        out.insert(k.trim().to_string(), Value::String(v.to_string()));
    }
    Ok(out)
}

/// The command table of an agent's prompt: every operation its role uses, its
/// name as the role guides and the MCP server give it (`genie_task` action
/// `show`), the command, what it does.
pub fn command_table(kind: AgentKind, can: &dyn Fn(Capability) -> bool) -> String {
    let mut out = String::from("| Tool | Command | What it does |\n|---|---|---|\n");
    for e in catalog().iter().filter(|e| e.listed_for(kind, can)) {
        let tool = format!("`genie_{}` {}", e.group, e.name);
        let note = e.note_for(kind, can).map(|n| format!(" ({n})")).unwrap_or_default();
        out.push_str(&format!("| {tool} | `{}`{note} | {} |\n", e.usage_for(kind, can), e.summary()));
    }
    out
}

/// URL-encode one path segment or query value.
pub fn enc(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.~".contains(c) {
                c.to_string()
            } else {
                c.to_string().bytes().map(|b| format!("%{b:02X}")).collect()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_operation_has_a_command_line_and_a_schema() {
        let mut seen = std::collections::BTreeSet::new();
        for e in catalog() {
            assert!(seen.insert((e.group, e.name)), "{} {} twice", e.group, e.name);
            assert!(GROUPS.iter().any(|(g, _)| *g == e.group), "{} is not a group", e.group);
            assert!(!e.summary().is_empty(), "{} {} says nothing about itself", e.group, e.name);
            assert_eq!(e.schema["type"], "object", "{} {}: {}", e.group, e.name, e.schema);
            e.command(e.name).debug_assert();
        }
        commands(Command::new("genie")).debug_assert();
    }

    fn caps(role: genie_core::Role) -> impl Fn(Capability) -> bool {
        let c = genie_core::class_capabilities(role);
        move |x| c.contains(&x)
    }

    #[test]
    fn command_tables_follow_the_role() {
        use genie_core::Role;
        let orch = command_table(AgentKind::Orchestrator, &|_| true);
        assert!(orch.contains("| `genie_team` spawn | `genie team spawn <TASK> [--template …] [--member …]... [--note …]` |"), "{orch}");
        assert!(
            orch.contains("`genie task status <STATUS> [--task …] [--note …] [--force] [--action …] [--option …]... [--repo …]` (review and approved are the team's verdicts"),
            "{orch}"
        );
        assert!(
            orch.contains("--merge-strategy") && orch.contains("| `genie_task` split |") && orch.contains("| `genie_team` templates |")
        );
        assert!(!orch.contains("set-status") && !orch.contains("genie job output"), "a member's and a job's own commands");

        let reviewer = command_table(AgentKind::Member, &caps(Role::Reviewer));
        assert!(
            reviewer.contains("`genie task status <STATUS> [--task …] [--note …]` (your role may set: changes_requested, approved)"),
            "{reviewer}"
        );
        assert!(
            reviewer.contains("| `genie_task` check |")
                && reviewer.contains("| `genie_team` set-status | `genie team set-status <TEXT>` |")
        );
        assert!(reviewer.contains(
            "`genie mail send <TO> <TEXT> [--level low|normal|high|interrupt] [--intent question|blocker|verdict|done|fyi] [--topic …]`"
        ));
        for other in ["genie task create", "--title", "--plan", "--force", "genie team spawn", "genie team templates", "genie task split"] {
            assert!(!reviewer.contains(other), "a reviewer has no {other}");
        }

        let executor = command_table(AgentKind::Member, &caps(Role::Executor));
        assert!(executor.contains("`genie task update [--task …] [--plan …] [--notes …] [--append-notes …] [--label …]...`"), "{executor}");
        assert!(executor.contains("(your role may set: in_progress, review)") && !executor.contains("genie task check"));
        let analyst = command_table(AgentKind::Member, &caps(Role::Analyst));
        assert!(analyst.contains("| `genie_task` create |") && analyst.contains("--title …"), "{analyst}");

        let job = command_table(AgentKind::Job, &caps(Role::Documenter));
        assert!(job.contains("| `genie_job` output | `genie job output <JSON>` |") && job.contains("| `genie_docs` write |"), "{job}");
        assert!(!job.contains("genie mail") && !job.contains("genie team"), "a job works alone: {job}");

        // Every row is a command line the catalog parses.
        for line in [orch, reviewer, executor, analyst, job].concat().lines().filter(|l| l.starts_with("| `")) {
            let cmd = line.split('`').nth(3).unwrap_or_default();
            let words: Vec<&str> = cmd.split_whitespace().take(3).collect();
            assert!(find(words[1], words[2]).is_some(), "{line}");
        }
    }

    #[test]
    fn lists_answer_to_ls() {
        let m = commands(Command::new("genie")).try_get_matches_from(["genie", "task", "ls", "--status", "ready"]).unwrap();
        let (e, _) = chosen(&m).unwrap();
        assert_eq!((e.group, e.name), ("task", "list"));
    }

    #[test]
    fn urls_are_encoded_by_bytes() {
        assert_eq!(enc("G-7"), "G-7");
        assert_eq!(enc("a b/ц"), "a%20b%2F%D1%86");
    }
}
