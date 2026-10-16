//! Teams of agents and one-shot jobs.

use genie_core::Capability;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Cx, Entry, Listed, Need, Op, Out, enc, register, render};

pub fn register(all: &mut Vec<Entry>) {
    register!(
        all,
        Show,
        List,
        Board,
        Peek,
        Spawn,
        Recommend,
        AddMember,
        RemoveMember,
        Stop,
        Delete,
        Restart,
        Pause,
        Resume,
        SetModel,
        Interrupt,
        SetStatus,
        Templates,
        Roles,
        Turns,
        Output,
        JobList,
        JobShow,
        JobStart
    );
}

/// A team: its members and what they do, and recent mail (default: your team).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Show {
    pub team: Option<String>,
    /// How many recent messages to show.
    #[arg(long, default_value_t = 10)]
    #[serde(default = "ten")]
    pub mail: usize,
}

fn ten() -> usize {
    10
}

impl Op for Show {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "show";
    const LEGACY: Option<&'static str> = Some("team");
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", &format!("/teams/{}", enc(&cx.team(self.team)?)), None).await?;
        Ok(Out::new(render::team(&v, self.mail), v))
    }
}

/// Teams of the project: active ones, or all with --all.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct List {
    #[arg(long)]
    #[serde(default)]
    pub all: bool,
}

impl Op for List {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "list";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", if self.all { "/teams?all=1" } else { "/teams" }, None).await?;
        Ok(Out::new(render::teams(&v), v))
    }
}

/// Every agent of the project: its state, current step and waiting mail.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Board {}

impl Op for Board {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "board";
    const LEGACY: Option<&'static str> = Some("board");
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/agents", None).await?;
        Ok(Out::new(render::agents(&v), v))
    }
}

/// What an agent is doing now; --deep adds its latest conversation.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Peek {
    /// A member of the team, or `orchestrator`.
    pub member: String,
    #[arg(long)]
    #[serde(default)]
    pub deep: bool,
    #[arg(long)]
    pub team: Option<String>,
}

impl Op for Peek {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "peek";
    const LEGACY: Option<&'static str> = Some("peek");
    const CAPS: &'static [Capability] = &[Capability::TeamPeek];
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let team = if self.member == "orchestrator" { "orchestrator".to_string() } else { cx.team(self.team)? };
        let deep = if self.deep { "?deep=1" } else { "" };
        let v = cx.call("GET", &format!("/agents/{}/{}/peek{deep}", enc(&team), enc(&self.member)), None).await?;
        Ok(Out::new(render::peek(&v), v))
    }
}

/// An explicit model/thinking pair can be copied from a Jev recommendation.
fn member_spec(text: &str) -> Value {
    let Some((role, model)) = text.split_once(':') else {
        return json!({"role":text});
    };
    match model.rsplit_once(':') {
        Some((model, thinking)) if crate::http::teams::THINKING.contains(&thinking) => {
            json!({"role":role,"model":model,"thinking":thinking})
        }
        _ => json!({"role":role,"model":model}),
    }
}

/// Assemble a team for a task from a template, or member by member (orchestrator).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Spawn {
    /// Keep this proposal after considering Jev; record a specific justification.
    #[arg(long)]
    pub decision_override: Option<String>,
    pub task: String,
    /// A team template (`genie team templates`).
    #[arg(long)]
    pub template: Option<String>,
    /// A member by role instead of the template's roster: `role`, `role:provider/model` or `role:provider/model:thinking`; repeat for more.
    #[arg(long = "member")]
    #[serde(default)]
    pub members: Vec<String>,
    /// What the team should know from you.
    #[arg(long, allow_hyphen_values = true)]
    pub note: Option<String>,
}

impl Op for Spawn {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "spawn";
    const LEGACY: Option<&'static str> = Some("spawn");
    const NEED: Need = Need::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let members: Vec<Value> = self.members.iter().map(|m| member_spec(m)).collect();
        let v = cx
            .call("POST", "/teams", Some(json!({ "task": self.task, "template": self.template, "members": members, "note": self.note, "decisionOverride":self.decision_override })))
            .await?;
        let names: Vec<String> = v["members"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|m| format!("{} ({})", m["name"].as_str().unwrap_or_default(), m["role"].as_str().unwrap_or_default()))
            .collect();
        let text = format!(
            "team {} started for {}: {}",
            v["id"].as_str().unwrap_or_default(),
            v["task"].as_str().unwrap_or_default(),
            names.join(", ")
        );
        Ok(Out::new(text, v))
    }
}

/// Preview Jev's recommendation before assembling a team; does not start agents.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Recommend {
    pub task: String,
    #[arg(long)]
    pub template: Option<String>,
    #[arg(long = "member")]
    #[serde(default)]
    pub members: Vec<String>,
}
impl Op for Recommend {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "recommend";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let members: Vec<Value> = self.members.iter().map(|s| member_spec(s)).collect();
        let v = cx.call("POST", "/teams/recommend", Some(json!({"task":self.task,"template":self.template,"members":members}))).await?;
        Ok(Out::new(serde_json::to_string_pretty(&v).unwrap_or_default(), v))
    }
}

/// Add a member to a running team (orchestrator).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AddMember {
    pub team: String,
    /// A role (`genie team roles`).
    pub role: String,
    #[arg(long)]
    pub name: Option<String>,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long, allow_hyphen_values = true)]
    pub instructions: Option<String>,
}

impl Op for AddMember {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "add-member";
    const LEGACY: Option<&'static str> = Some("add-member");
    const NEED: Need = Need::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let body = json!({ "role": self.role, "name": self.name, "model": self.model, "instructions": self.instructions });
        let v = cx.call("POST", &format!("/teams/{}/members", enc(&self.team)), Some(body)).await?;
        let m = v.as_array().and_then(|a| a.first()).cloned().unwrap_or(Value::Null);
        let text =
            format!("{} ({}) joined team {}", m["name"].as_str().unwrap_or_default(), m["role"].as_str().unwrap_or_default(), self.team);
        Ok(Out::new(text, v))
    }
}

/// Stop a member and take it out of the team (orchestrator).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RemoveMember {
    pub team: String,
    pub member: String,
}

impl Op for RemoveMember {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "remove-member";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("DELETE", &format!("/teams/{}/members/{}", enc(&self.team), enc(&self.member)), None).await?;
        Ok(Out::new(format!("{} left team {}", self.member, self.team), v))
    }
}

fn report(v: &Value) -> String {
    v["report"].as_array().map(|r| r.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("\n")).unwrap_or_default()
}

/// Stop a team: its members stop, the task is released if still open (orchestrator).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Stop {
    pub team: String,
    /// Also remove the team's worktree.
    #[arg(long)]
    #[serde(default)]
    pub remove_worktree: bool,
}

impl Op for Stop {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "stop";
    const LEGACY: Option<&'static str> = Some("stop-team");
    const NEED: Need = Need::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v =
            cx.call("POST", &format!("/teams/{}/stop", enc(&self.team)), Some(json!({ "removeWorktree": self.remove_worktree }))).await?;
        Ok(Out::new(report(&v), v))
    }
}

/// Stop a team and delete it with its mail (orchestrator).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Delete {
    pub team: String,
    /// Also remove the team's worktree.
    #[arg(long)]
    #[serde(default)]
    pub remove_worktree: bool,
}

impl Op for Delete {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "delete";
    const NEED: Need = Need::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let q = if self.remove_worktree { "?removeWorktree=1" } else { "" };
        let v = cx.call("DELETE", &format!("/teams/{}{q}", enc(&self.team)), None).await?;
        Ok(Out::new(report(&v), v))
    }
}

/// Let a member in error work again (orchestrator).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Restart {
    pub team: String,
    pub member: String,
}

impl Op for Restart {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "restart";
    const LEGACY: Option<&'static str> = Some("restart");
    const NEED: Need = Need::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", &format!("/teams/{}/members/{}/restart", enc(&self.team), enc(&self.member)), Some(json!({}))).await?;
        Ok(Out::new(format!("{} restarted", self.member), v))
    }
}

/// Hold an agent: its session stops, its mail waits (orchestrator).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Pause {
    pub team: String,
    pub member: String,
}

impl Op for Pause {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "pause";
    const LEGACY: Option<&'static str> = Some("pause");
    const NEED: Need = Need::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", &format!("/agents/{}/{}/pause", enc(&self.team), enc(&self.member)), Some(json!({}))).await?;
        Ok(Out::new(format!("{} paused; its mail waits until `genie team resume {} {}`", self.member, self.team, self.member), v))
    }
}

/// Let a paused agent work again (orchestrator).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Resume {
    pub team: String,
    pub member: String,
}

impl Op for Resume {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "resume";
    const LEGACY: Option<&'static str> = Some("resume");
    const NEED: Need = Need::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", &format!("/agents/{}/{}/resume", enc(&self.team), enc(&self.member)), Some(json!({}))).await?;
        Ok(Out::new(format!("{} resumed", self.member), v))
    }
}

/// Give one agent its own model and thinking level; without them it goes back to its role's (orchestrator).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetModel {
    pub team: String,
    pub member: String,
    /// `provider/model` (`genie doctor` checks what pi can use); omit for the role's.
    #[arg(long)]
    pub model: Option<String>,
    /// off, minimal, low, medium, high, xhigh or max; omit for the role's.
    #[arg(long)]
    pub thinking: Option<String>,
}

impl Op for SetModel {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "set-model";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let body = json!({ "model": self.model, "thinking": self.thinking });
        let v = cx.call("PATCH", &format!("/teams/{}/members/{}", enc(&self.team), enc(&self.member)), Some(body)).await?;
        let now = match (v["model"].as_str(), v["thinking"].as_str()) {
            (None, None) => "its role's model".to_string(),
            (m, t) => format!("{}{}", m.unwrap_or("its role's model"), t.map(|t| format!(" · thinking {t}")).unwrap_or_default()),
        };
        Ok(Out::new(format!("{} runs on {now} from its next step; its conversation goes on", self.member), v))
    }
}

/// Stop an agent's current step and give it new instructions (orchestrator).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Interrupt {
    pub team: String,
    pub member: String,
    #[arg(allow_hyphen_values = true)]
    pub text: String,
}

impl Op for Interrupt {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "interrupt";
    const LEGACY: Option<&'static str> = Some("interrupt");
    const NEED: Need = Need::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let body = json!({ "to": self.member, "text": self.text, "level": "interrupt" });
        let v = cx.call("POST", &format!("/teams/{}/mail", enc(&self.team)), Some(body)).await?;
        Ok(Out::new(format!("{} is interrupted: its current step stops and your message comes first", self.member), v))
    }
}

/// Your short status line in the team card.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetStatus {
    #[arg(allow_hyphen_values = true)]
    pub text: String,
}

impl Op for SetStatus {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "set-status";
    const LEGACY: Option<&'static str> = Some("set-status");
    const NEED: Need = Need::Member;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", "/agent/status", Some(json!({ "text": self.text }))).await?;
        Ok(Out::new("status set", v))
    }
}

/// Team templates of the project: members, how they hand work over.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Templates {}

impl Op for Templates {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "templates";
    const LEGACY: Option<&'static str> = Some("templates");
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/agent-config", None).await?;
        let mut out = Vec::new();
        for t in v["teams"].as_array().cloned().unwrap_or_default() {
            let roles: Vec<&str> =
                t["members"].as_array().map(|m| m.iter().filter_map(|x| x["role"].as_str()).collect()).unwrap_or_default();
            out.push(format!(
                "{} — {}\n    {} · {} · {}",
                t["id"].as_str().unwrap_or_default(),
                t["description"].as_str().unwrap_or_default(),
                t["stage"].as_str().unwrap_or_default(),
                t["workspace"].as_str().unwrap_or_default(),
                roles.join(", ")
            ));
            for r in t["relations"].as_array().cloned().unwrap_or_default() {
                let to: Vec<&str> = r["to"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                out.push(format!(
                    "    {} {} {}{}{}",
                    r["from"].as_str().unwrap_or_default(),
                    r["type"].as_str().unwrap_or_default(),
                    to.join(", "),
                    r["on"].as_str().map(|s| format!(" on {s}")).unwrap_or_default(),
                    r["note"].as_str().map(|n| format!(" — {n}")).unwrap_or_default()
                ));
            }
        }
        Ok(Out::new(out.join("\n"), v["teams"].clone()))
    }
}

/// Roles of the project that can join a team.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Roles {}

impl Op for Roles {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "roles";
    const LEGACY: Option<&'static str> = Some("roles");
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/agent-config", None).await?;
        let text = v["roles"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|r| r["class"] != json!("orchestrator"))
            .map(|r| {
                let class = r["class"].as_str().unwrap_or_default();
                let id = r["id"].as_str().unwrap_or_default();
                let stages: Vec<&str> = r["stages"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                format!(
                    "{id}{} — {}\n    stages: {} · files: {}",
                    if class == id { String::new() } else { format!(" ({class})") },
                    r["description"].as_str().unwrap_or_default(),
                    stages.join(", "),
                    r["files"].as_str().unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Out::new(text, v["roles"].clone()))
    }
}

/// Report the structured result of a one-shot job: a JSON object.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    /// The result: a JSON object (its text on the command line, `-` reads stdin).
    #[arg(allow_hyphen_values = true, value_name = "JSON")]
    #[serde(deserialize_with = "super::json_text")]
    #[schemars(with = "serde_json::Map<String, Value>")]
    pub result: String,
}

impl Op for Output {
    const GROUP: &'static str = "job";
    const NAME: &'static str = "output";
    const LEGACY: Option<&'static str> = Some("output");
    const NEED: Need = Need::Job;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let text = cx.text(Some(self.result), None)?.unwrap_or_default();
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("the output must be JSON: {e}"))?;
        let r = cx.call("POST", "/agent/output", Some(json!({ "output": v }))).await?;
        Ok(Out::new("result recorded", r))
    }
}

/// Recent turns of the project's agents: when each ran, how it ended, why it failed.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Turns {
    /// One agent only: `orchestrator`, `<team>/<member>` or `job-<id>`.
    #[arg(long)]
    pub agent: Option<String>,
    #[arg(long, default_value_t = 20)]
    #[serde(default = "twenty")]
    pub limit: usize,
    /// Print each turn's log too.
    #[arg(long)]
    #[serde(default)]
    pub log: bool,
}

fn twenty() -> usize {
    20
}

impl Op for Turns {
    const GROUP: &'static str = "team";
    const NAME: &'static str = "turns";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let agent = self.agent.as_deref().map(|a| format!("&agent={}", enc(a))).unwrap_or_default();
        let v = cx.call("GET", &format!("/turns?limit={}{agent}", self.limit), None).await?;
        let mut out = Vec::new();
        for t in v.as_array().cloned().unwrap_or_default() {
            out.push(format!(
                "#{:<5} {:<24} {:<9} {} → {}{}",
                t["id"].to_string(),
                t["agent"].as_str().unwrap_or_default(),
                t["status"].as_str().unwrap_or_default(),
                t["started"].as_str().unwrap_or_default(),
                t["finished"].as_str().unwrap_or("…"),
                t["exitCode"].as_i64().map(|c| format!(" exit {c}")).unwrap_or_default()
            ));
            if let Some(e) = t["error"].as_str() {
                out.push(format!("       error: {e}"));
            }
            if self.log
                && let Some(log) = t["log"].as_str().filter(|l| !l.trim().is_empty())
            {
                out.extend(log.lines().map(|l| format!("       | {l}")));
            }
        }
        Ok(Out::new(if out.is_empty() { "no turns".into() } else { out.join("\n") }, v))
    }
}

fn job_line(j: &Value) -> String {
    let goal = j["goal"].as_str().unwrap_or_default().lines().next().unwrap_or_default();
    let goal: String =
        if goal.chars().count() > 80 { format!("{}…", goal.chars().take(80).collect::<String>()) } else { goal.to_string() };
    format!(
        "#{:<5} {:<10} {:<14} {}{}",
        j["id"].to_string(),
        j["status"].as_str().unwrap_or_default(),
        j["role"].as_str().unwrap_or_default(),
        j["task"].as_str().map(|t| format!("for {t}: ")).unwrap_or_default(),
        goal
    )
}

/// One-shot jobs of the project, newest first.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct JobList {}

impl Op for JobList {
    const GROUP: &'static str = "job";
    const NAME: &'static str = "list";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/jobs", None).await?;
        let rows = v.as_array().cloned().unwrap_or_default();
        Ok(Out::new(if rows.is_empty() { "no jobs".into() } else { rows.iter().map(job_line).collect::<Vec<_>>().join("\n") }, v))
    }
}

/// A one-shot job: its goal, inputs, and the result or the error.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct JobShow {
    pub id: i64,
}

impl Op for JobShow {
    const GROUP: &'static str = "job";
    const NAME: &'static str = "show";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let j = cx.call("GET", &format!("/jobs/{}", self.id), None).await?;
        let pretty = |v: &Value| serde_json::to_string_pretty(v).unwrap_or_default();
        let mut out = vec![
            job_line(&j),
            format!(
                "workspace {} · model {} · attempts {} · created {}{}",
                j["workspace"].as_str().unwrap_or_default(),
                j["model"].as_str().unwrap_or("default"),
                j["attempts"],
                j["created"].as_str().unwrap_or_default(),
                j["finished"].as_str().map(|f| format!(" · finished {f}")).unwrap_or_default()
            ),
            String::new(),
            j["goal"].as_str().unwrap_or_default().to_string(),
        ];
        if j["inputs"].as_object().is_some_and(|o| !o.is_empty()) {
            out.push(format!("\ninputs: {}", pretty(&j["inputs"])));
        }
        if !j["output"].is_null() {
            out.push(format!("\noutput: {}", pretty(&j["output"])));
        }
        if let Some(e) = j["error"].as_str() {
            out.push(format!("\nerror: {e}"));
        }
        Ok(Out::new(out.join("\n"), j))
    }
}

/// Start a one-shot job: an agent of a role does one thing and reports a result (orchestrator and people).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct JobStart {
    /// A role that is not the orchestrator (`genie team roles`).
    #[arg(long)]
    pub role: String,
    /// What to do.
    #[arg(allow_hyphen_values = true)]
    pub goal: String,
    /// The task it works for.
    #[arg(long)]
    pub task: Option<String>,
    #[arg(long)]
    pub model: Option<String>,
    /// Inputs as key=value; repeat for more.
    #[arg(long = "input")]
    #[serde(default)]
    pub inputs: Vec<String>,
    /// JSON schema the result must follow.
    #[arg(long, allow_hyphen_values = true, value_name = "JSON")]
    #[serde(default, deserialize_with = "super::opt_json_text")]
    #[schemars(with = "Option<serde_json::Map<String, Value>>")]
    pub output_schema: Option<String>,
    /// Where it works: none, read-only (the project's code) or worktree.
    #[arg(long)]
    pub workspace: Option<String>,
}

impl Op for JobStart {
    const GROUP: &'static str = "job";
    const NAME: &'static str = "start";
    const NEED: Need = Need::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let schema: Option<Value> = match cx.text(self.output_schema, None)? {
            Some(t) => Some(serde_json::from_str(&t).map_err(|e| format!("the output schema must be JSON: {e}"))?),
            None => None,
        };
        let body = json!({
            "role": self.role,
            "goal": self.goal,
            "task": self.task,
            "model": self.model,
            "inputs": super::pairs(&self.inputs, "--input")?,
            "outputSchema": schema,
            "workspace": self.workspace,
        });
        let j = cx.call("POST", "/jobs", Some(body)).await?;
        Ok(Out::new(format!("started {}; see `genie job show {}`", job_line(&j), j["id"]), j))
    }
}
