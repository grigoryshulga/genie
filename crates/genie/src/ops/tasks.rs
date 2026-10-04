//! Tasks and epics.

use std::path::PathBuf;

use genie_core::{Capability, GenieError, OwnerAction, TEAM_TRANSITIONS};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{AgentKind, Cx, Entry, Listed, Need, Op, Out, enc, register, render};
use crate::tasks::{CommentBody, CreateBody, StatusBody, UpdateBody};

/// A task type, status or comment kind by its name.
fn parsed<T: std::str::FromStr<Err = GenieError>>(v: Option<String>) -> Result<Option<T>, String> {
    v.filter(|s| !s.is_empty()).map(|s| s.parse()).transpose().map_err(|e: GenieError| e.to_string())
}

pub fn register(all: &mut Vec<Entry>) {
    register!(all, Show, List, Board, Epics, Create, Update, Status, Accept, Comment, Check, Artifact, ArtifactRead, Split, Block, Unblock);
}

/// Show a task in full: description, criteria, plan, notes, artifacts, comments (default: your task).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Show {
    /// Task id, e.g. G-7.
    pub task: Option<String>,
    /// Add the history of changes.
    #[arg(long)]
    #[serde(default)]
    pub history: bool,
}

impl Op for Show {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "show";
    const LEGACY: Option<&'static str> = Some("show");
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", &format!("/tasks/{}", enc(&cx.task(self.task)?)), None).await?;
        Ok(Out::new(render::task(&v, self.history), v))
    }
}

/// List tasks: open ones by default; filter by status, epic, team, label or text.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct List {
    /// Statuses, comma-separated (inbox, draft, refining, ready, in_progress, review, changes_requested, approved, needs_owner, done, cancelled).
    #[arg(long)]
    pub status: Option<String>,
    /// The ready queue: ready, unblocked, dependencies done, no team yet.
    #[arg(long)]
    #[serde(default)]
    pub ready: bool,
    /// Tasks of this epic.
    #[arg(long)]
    pub epic: Option<String>,
    /// Tasks of this team.
    #[arg(long)]
    pub team: Option<String>,
    #[arg(long)]
    pub label: Option<String>,
    /// Words in the title or description.
    #[arg(long)]
    pub search: Option<String>,
    /// Include done and cancelled tasks.
    #[arg(long)]
    #[serde(default)]
    pub all: bool,
    /// Leave epics out.
    #[arg(long)]
    #[serde(default)]
    pub no_epics: bool,
}

impl Op for List {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "list";
    const LEGACY: Option<&'static str> = Some("list");
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let mut q = Vec::new();
        for (k, v) in [("status", self.status), ("parent", self.epic), ("team", self.team), ("label", self.label), ("q", self.search)] {
            if let Some(v) = v.filter(|v| !v.is_empty()) {
                q.push(format!("{k}={}", enc(&v)));
            }
        }
        if self.ready {
            q.push("ready=1".into());
        }
        if self.all {
            q.push("closed=1".into());
        }
        if self.no_epics {
            q.push("epics=0".into());
        }
        let v = cx.call("GET", &format!("/tasks?{}", q.join("&")), None).await?;
        Ok(Out::new(render::list(&v), v))
    }
}

/// The board: epics with progress, tasks grouped by status, active teams.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    /// Include done and cancelled tasks.
    #[arg(long)]
    #[serde(default)]
    pub all: bool,
}

impl Op for Board {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "board";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let closed = if self.all { "&closed=1" } else { "" };
        let epics = cx.call("GET", &format!("/tasks?type=epic{closed}"), None).await?;
        let tasks = cx.call("GET", &format!("/tasks?epics=0{closed}"), None).await?;
        let teams = cx.call("GET", "/teams", None).await?;
        let text = render::board(&epics, &tasks, &teams);
        Ok(Out::new(text, json!({ "epics": epics, "tasks": tasks, "teams": teams })))
    }
}

/// Epics with their progress.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Epics {
    /// Include done and cancelled epics.
    #[arg(long)]
    #[serde(default)]
    pub all: bool,
}

impl Op for Epics {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "epics";
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", &format!("/tasks?type=epic{}", if self.all { "&closed=1" } else { "" }), None).await?;
        let text = if v.as_array().is_none_or(|a| a.is_empty()) { "(no epics)".to_string() } else { render::list(&v) };
        Ok(Out::new(text, v))
    }
}

/// Create a task or an epic. People's tasks land in the inbox (the orchestrator takes them from there) unless --draft; agents create drafts, members only subtasks of their task.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Create {
    pub title: String,
    /// Markdown description (the goal, for an epic).
    #[arg(short = 'd', long, allow_hyphen_values = true)]
    pub description: Option<String>,
    /// An acceptance criterion (a success criterion, for an epic); repeat for more.
    #[arg(short = 'a', long = "ac")]
    #[serde(default)]
    pub acceptance: Vec<String>,
    /// task, bug, spike or epic.
    #[arg(long = "type", value_parser = ["task", "bug", "spike", "epic"])]
    #[serde(rename = "type")]
    pub task_type: Option<String>,
    /// The epic (or parent task) to create it in.
    #[arg(long, visible_alias = "epic")]
    pub parent: Option<String>,
    /// 0 (urgent) … 4 (low).
    #[arg(short = 'p', long)]
    pub priority: Option<i64>,
    /// A task this one depends on; repeat for more.
    #[arg(long = "dep")]
    #[serde(default)]
    pub deps: Vec<String>,
    /// A label; repeat for more.
    #[arg(long = "label")]
    #[serde(default)]
    pub labels: Vec<String>,
    /// The plan (the roadmap, for an epic).
    #[arg(long, allow_hyphen_values = true)]
    pub plan: Option<String>,
    /// A person's task starts as a draft instead of the inbox.
    #[arg(long)]
    #[serde(default)]
    pub draft: bool,
}

impl Op for Create {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "create";
    const LEGACY: Option<&'static str> = Some("create");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::TaskCreate];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let body = CreateBody {
            title: self.title,
            task_type: parsed(self.task_type)?,
            description: cx.text(self.description, None)?,
            acceptance: Some(self.acceptance),
            priority: self.priority,
            parent: self.parent,
            deps: Some(self.deps),
            labels: (!self.labels.is_empty()).then_some(self.labels),
            plan: self.plan,
            draft: Some(self.draft),
            ..Default::default()
        };
        let v = cx.send("POST", "/tasks", &body).await?;
        Ok(Out::new(format!("created {}", render::summary(&v)), v))
    }
}

/// Update fields of a task (default: your task).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    #[arg(long)]
    pub task: Option<String>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(short = 'd', long, allow_hyphen_values = true)]
    pub description: Option<String>,
    /// Replaces the plan.
    #[arg(long, allow_hyphen_values = true)]
    pub plan: Option<String>,
    /// Replaces the notes.
    #[arg(long, allow_hyphen_values = true)]
    pub notes: Option<String>,
    /// A timestamped entry added to the notes.
    #[arg(long, allow_hyphen_values = true)]
    pub append_notes: Option<String>,
    /// An acceptance criterion to add; repeat for more.
    #[arg(short = 'a', long = "ac")]
    #[serde(default)]
    pub acceptance: Vec<String>,
    /// Number of a criterion to remove; repeat for more.
    #[arg(long = "rm-ac")]
    #[serde(default)]
    pub remove_acceptance: Vec<i64>,
    /// A dependency to add; repeat for more.
    #[arg(long = "dep")]
    #[serde(default)]
    pub deps: Vec<String>,
    /// A dependency to remove; repeat for more.
    #[arg(long = "rm-dep")]
    #[serde(default)]
    pub remove_deps: Vec<String>,
    /// Labels: replaces them all; repeat for more.
    #[arg(long = "label")]
    pub labels: Option<Vec<String>>,
    /// How the result gets integrated (orchestrator).
    #[arg(long)]
    pub merge_strategy: Option<String>,
    /// The person responsible: a login of the project ("none" clears it).
    #[arg(long)]
    pub assignee: Option<String>,
    #[arg(short = 'p', long)]
    pub priority: Option<i64>,
    /// Move into this epic ("none" moves it out).
    #[arg(long, visible_alias = "epic")]
    pub parent: Option<String>,
    /// task, bug, spike or epic.
    #[arg(long = "type", value_parser = ["task", "bug", "spike", "epic"])]
    #[serde(rename = "type")]
    pub task_type: Option<String>,
}

impl Op for Update {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "update";
    const LEGACY: Option<&'static str> = Some("update");
    const NEED: Need = Need::Write;
    fn arg_allowed(arg: &str, can: &dyn Fn(Capability) -> bool) -> bool {
        match arg {
            "title" | "description" | "acceptance" | "remove_acceptance" | "deps" | "remove_deps" | "parent" | "task_type" => {
                can(Capability::TaskScope)
            }
            "plan" => can(Capability::TaskPlan),
            // The orchestrator's and people's.
            "priority" | "merge_strategy" | "assignee" => false,
            _ => true,
        }
    }
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let some = |v: Vec<String>| (!v.is_empty()).then_some(v);
        let clear = |v: Option<String>| v.map(|v| Some(v).filter(|v| v != "none" && !v.is_empty()));
        let body = UpdateBody {
            title: self.title,
            task_type: parsed(self.task_type)?,
            description: cx.text(self.description, None)?,
            plan: cx.text(self.plan, None)?,
            notes: cx.text(self.notes, None)?,
            append_notes: cx.text(self.append_notes, None)?,
            merge_strategy: self.merge_strategy,
            priority: self.priority,
            labels: self.labels,
            add_acceptance: some(self.acceptance),
            remove_acceptance: (!self.remove_acceptance.is_empty()).then_some(self.remove_acceptance),
            add_deps: some(self.deps),
            remove_deps: some(self.remove_deps),
            parent: clear(self.parent),
            assignee: clear(self.assignee),
            ..Default::default()
        };
        let v = cx.send("PATCH", &format!("/tasks/{}", enc(&cx.task(self.task)?)), &body).await?;
        Ok(Out::new(format!("updated {}", render::summary(&v)), v))
    }
}

/// Move a task to another status (default: your task). needs_owner takes the question as the note,
/// and optionally an action that gives the owner buttons: ask-owner-question (with 2-6 options),
/// ask-for-merge-pr (the task's request in a repository), ask-free-form (the default).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// inbox, draft, refining, ready, in_progress, review, changes_requested, approved, needs_owner, done, cancelled.
    pub status: String,
    #[arg(long)]
    pub task: Option<String>,
    /// The reason or a summary; the question for needs_owner.
    #[arg(long, short = 'm', allow_hyphen_values = true)]
    pub note: Option<String>,
    /// Skip the Definition of Ready and Done checks (orchestrator).
    #[arg(long)]
    #[serde(default)]
    pub force: bool,
    /// needs_owner: ask-owner-question, ask-for-merge-pr or ask-free-form. The owner can always answer in words.
    #[arg(long = "action")]
    pub owner_action: Option<String>,
    /// ask-owner-question: an option to pick; repeat for more (2-6).
    #[arg(long = "option")]
    #[serde(default)]
    pub options: Vec<String>,
    /// ask-for-merge-pr: the repository whose request to merge (default: the task's only open request).
    #[arg(long)]
    pub repo: Option<String>,
}

impl Op for Status {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "status";
    const LEGACY: Option<&'static str> = Some("status");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = STATUS_CAPS;
    fn arg_allowed(arg: &str, _can: &dyn Fn(Capability) -> bool) -> bool {
        // needs_owner and forcing are the orchestrator's.
        !matches!(arg, "force" | "owner_action" | "options" | "repo")
    }
    fn prompt_note(kind: AgentKind, can: &dyn Fn(Capability) -> bool) -> Option<String> {
        if kind == AgentKind::Orchestrator {
            return Some("review and approved are the team's verdicts: set them yourself only with --force".into());
        }
        let mut to: Vec<&str> = TEAM_TRANSITIONS.iter().filter(|(_, _, c)| can(*c)).map(|(_, to, _)| to.as_str()).collect();
        to.dedup();
        Some(format!("your role may set: {}", to.join(", ")))
    }
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let action = match (self.owner_action, self.options.is_empty(), &self.repo) {
            (Some(kind), _, _) => {
                let mut action = json!({ "kind": kind });
                if !self.options.is_empty() {
                    action["options"] = json!(self.options);
                }
                if let Some(repo) = self.repo {
                    action["repo"] = json!(repo);
                }
                Some(OwnerAction::parse(&action).map_err(|e| e.to_string())?)
            }
            (None, false, _) | (None, _, Some(_)) => return Err("--option and --repo go with --action".into()),
            (None, true, None) => None,
        };
        let body = StatusBody {
            status: self.status.parse().map_err(|e: GenieError| e.to_string())?,
            note: cx.text(self.note, None)?,
            force: Some(self.force),
            action,
        };
        let v = cx.send("POST", &format!("/tasks/{}/status", enc(&cx.task(self.task)?)), &body).await?;
        Ok(Out::new(render::summary(&v), v))
    }
}

/// Accept a task: move it to done, with a summary.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Accept {
    pub task: Option<String>,
    #[arg(long, short = 'm', allow_hyphen_values = true)]
    pub note: Option<String>,
}

impl Op for Accept {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "accept";
    const NEED: Need = Need::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx
            .send("POST", &format!("/tasks/{}/status", enc(&cx.task(self.task)?)), &StatusBody::to(genie_core::Status::Done, self.note))
            .await?;
        Ok(Out::new(render::summary(&v), v))
    }
}

/// Comment on a task (default: your task). `@login` notifies a person of the project.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    #[arg(allow_hyphen_values = true)]
    pub text: String,
    #[arg(long)]
    pub task: Option<String>,
    /// note, progress, question, decision, review or handoff.
    #[arg(long, default_value = "note", value_parser = ["note", "progress", "question", "decision", "review", "handoff"])]
    #[serde(default = "note_kind")]
    pub kind: String,
}

fn note_kind() -> String {
    "note".into()
}

impl Op for Comment {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "comment";
    const LEGACY: Option<&'static str> = Some("comment");
    const NEED: Need = Need::Write;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let body = CommentBody {
            text: cx.text(Some(self.text), None)?.unwrap_or_default(),
            kind: Some(self.kind.parse().map_err(|e: GenieError| e.to_string())?),
        };
        let v = cx.send("POST", &format!("/tasks/{}/comments", enc(&cx.task(self.task)?)), &body).await?;
        Ok(Out::new("comment added", v))
    }
}

/// Tick (or untick) acceptance criterion N (default: your task).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub n: i64,
    #[arg(long)]
    pub task: Option<String>,
    #[arg(long)]
    #[serde(default)]
    pub undo: bool,
}

impl Op for Check {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "check";
    const LEGACY: Option<&'static str> = Some("check");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::TaskCheck];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx
            .call("POST", &format!("/tasks/{}/acceptance/{}", enc(&cx.task(self.task)?), self.n), Some(json!({ "done": !self.undo })))
            .await?;
        Ok(Out::new(format!("criterion #{} {}", self.n, if self.undo { "unchecked" } else { "checked" }), v))
    }
}

/// Attach an artifact to a task (default: your task): text, or a file on the command line.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    /// analysis, plan, code, review, test-report, diff, doc, log or other.
    #[arg(long, default_value = "other", value_parser = ["analysis", "plan", "code", "review", "test-report", "diff", "doc", "log", "other"])]
    #[serde(default = "other_kind")]
    pub kind: String,
    /// File name, e.g. review.md.
    #[arg(long)]
    pub name: Option<String>,
    /// A file to attach (command line only).
    #[arg(long)]
    #[serde(skip)]
    #[schemars(skip)]
    pub file: Option<PathBuf>,
    /// The content (`-` reads stdin on the command line).
    #[arg(long, allow_hyphen_values = true)]
    pub text: Option<String>,
    #[arg(long)]
    pub task: Option<String>,
    #[arg(long, allow_hyphen_values = true)]
    pub note: Option<String>,
}

fn other_kind() -> String {
    "other".into()
}

impl Op for Artifact {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "artifact";
    const LEGACY: Option<&'static str> = Some("artifact");
    const NEED: Need = Need::Write;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let mut body = json!({ "kind": self.kind, "name": self.name, "note": self.note });
        match (self.file, cx.text(self.text, None)?) {
            (Some(f), _) => {
                if !cx.local {
                    return Err("files are read only on the command line; pass the text itself".into());
                }
                let bytes = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
                if body["name"].is_null() {
                    body["name"] = json!(f.file_name().map(|n| n.to_string_lossy().into_owned()));
                }
                match String::from_utf8(bytes) {
                    Ok(s) => body["text"] = json!(s),
                    Err(e) => {
                        body["contentBase64"] = json!(base64::Engine::encode(&base64::engine::general_purpose::STANDARD, e.into_bytes()))
                    }
                }
            }
            (None, Some(t)) => body["text"] = json!(t),
            (None, None) => return Err("pass the text (or --file on the command line)".into()),
        }
        let v = cx.call("POST", &format!("/tasks/{}/artifacts", enc(&cx.task(self.task)?)), Some(body)).await?;
        let last = v["artifacts"].as_array().and_then(|a| a.last()).cloned().unwrap_or(Value::Null);
        Ok(Out::new(format!("artifact #{} {} attached", last["id"], last["name"].as_str().unwrap_or_default()), v))
    }
}

/// Read artifact N of a task (default: your task); on the command line --out saves it.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRead {
    pub n: i64,
    #[arg(long)]
    pub task: Option<String>,
    /// Save to this file (command line only).
    #[arg(long)]
    #[serde(skip)]
    #[schemars(skip)]
    pub out: Option<PathBuf>,
}

impl Op for ArtifactRead {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "artifact-read";
    const LEGACY: Option<&'static str> = Some("artifact-read");
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let base64 = if self.out.is_some() { "?base64=1" } else { "" };
        let v = cx.call("GET", &format!("/tasks/{}/artifacts/{}{base64}", enc(&cx.task(self.task)?), self.n), None).await?;
        if let Some(path) = self.out {
            if !cx.local {
                return Err("files are written only on the command line".into());
            }
            let bytes = match (v["text"].as_str(), v["contentBase64"].as_str()) {
                (Some(t), _) => t.as_bytes().to_vec(),
                (None, Some(b)) => base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b).map_err(|e| e.to_string())?,
                _ => return Err("the artifact has no content to save".into()),
            };
            std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            return Ok(Out::new(format!("saved to {}", path.display()), v));
        }
        let text = match v["text"].as_str() {
            Some(t) => format!("# {} ({})\n\n{t}", v["name"].as_str().unwrap_or_default(), v["kind"].as_str().unwrap_or_default()),
            None => format!("# {} — binary, {} bytes (not shown)", v["name"].as_str().unwrap_or_default(), v["size"]),
        };
        Ok(Out::new(text, v))
    }
}

/// Slice a task into child tasks (default: your task); it becomes their epic.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Split {
    /// Titles of the children.
    #[arg(required = true)]
    pub titles: Vec<String>,
    #[arg(long)]
    pub task: Option<String>,
}

impl Op for Split {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "split";
    const LEGACY: Option<&'static str> = Some("split");
    const NEED: Need = Need::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", &format!("/tasks/{}/split", enc(&cx.task(self.task)?)), Some(json!({ "children": self.titles }))).await?;
        Ok(Out::new(format!("created:\n{}", render::list(&v)), v))
    }
}

/// Mark a task blocked, with the reason (default: your task).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    #[arg(allow_hyphen_values = true)]
    pub reason: String,
    #[arg(long)]
    pub task: Option<String>,
}

impl Op for Block {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "block";
    const LEGACY: Option<&'static str> = Some("block");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::TaskBlock];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", &format!("/tasks/{}/block", enc(&cx.task(self.task)?)), Some(json!({ "reason": self.reason }))).await?;
        Ok(Out::new("blocked", v))
    }
}

/// Clear the block of a task (default: your task).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Unblock {
    #[arg(long)]
    pub task: Option<String>,
}

impl Op for Unblock {
    const GROUP: &'static str = "task";
    const NAME: &'static str = "unblock";
    const LEGACY: Option<&'static str> = Some("unblock");
    const NEED: Need = Need::Write;
    const CAPS: &'static [Capability] = &[Capability::TaskBlock];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("DELETE", &format!("/tasks/{}/block", enc(&cx.task(self.task)?)), None).await?;
        Ok(Out::new("unblocked", v))
    }
}

/// Every permission to move a task.
const STATUS_CAPS: &[Capability] = &[
    Capability::StatusRefine,
    Capability::StatusStart,
    Capability::StatusRework,
    Capability::StatusSubmit,
    Capability::StatusApprove,
    Capability::StatusReturn,
];
