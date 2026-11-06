//! Scoped incoming work: the same native commands for an intake agent, CLI and MCP caller.
use super::{Cx, Entry, Listed, Need, Op, Out, register};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn register(all: &mut Vec<Entry>) {
    register!(all, Show, Reply, Task, Propose, Dispatch, Decide, Answer);
}
fn out(v: Value) -> Out {
    Out::new(serde_json::to_string_pretty(&v).unwrap_or_default(), v)
}

/// Read this incoming conversation's responses, tasks, questions and proposals.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Show {
    pub id: String,
}
impl Op for Show {
    const GROUP: &'static str = "conversation";
    const NAME: &'static str = "show";
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("GET", &format!("/conversations/{}", self.id), None).await?))
    }
}

/// Finish your incoming response with the complete text for its requester; then end the turn.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Reply {
    pub response: String,
    #[arg(long)]
    pub text: String,
}
impl Op for Reply {
    const GROUP: &'static str = "conversation";
    const NAME: &'static str = "reply";
    const NEED: Need = Need::Write;
    const LISTED: Listed = Listed::Request;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let text = cx.text(Some(self.text), None)?.unwrap_or_default();
        Ok(out(cx.call("POST", &format!("/conversations/{}/reply", self.response), Some(json!({"text":text}))).await?))
    }
}

/// Create the conversation's primary task; retries return the same task. Update it with task commands.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Task {
    pub response: String,
    #[arg(long)]
    pub title: String,
    #[arg(long)]
    pub description: String,
    #[arg(long = "ac")]
    #[serde(default)]
    pub acceptance: Vec<String>,
    #[arg(long)]
    pub plan: Option<String>,
}
impl Op for Task {
    const GROUP: &'static str = "conversation";
    const NAME: &'static str = "task";
    const NEED: Need = Need::Write;
    const LISTED: Listed = Listed::Request;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx
            .call(
                "POST",
                &format!("/conversations/{}/tasks", self.response),
                Some(json!({
                    "title":self.title,"description":self.description,"acceptance":self.acceptance,"plan":self.plan
                })),
            )
            .await?))
    }
}

/// Ask the requester to approve dispatch or acceptance of this exact task/result version.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Propose {
    pub response: String,
    #[arg(long)]
    pub task: String,
    // `action` selects the operation in MCP; keep the proposal's action separate.
    #[arg(long = "action", value_parser=["dispatch","accept"])]
    pub proposal_action: String,
}
impl Op for Propose {
    const GROUP: &'static str = "conversation";
    const NAME: &'static str = "propose";
    const NEED: Need = Need::Write;
    const LISTED: Listed = Listed::Request;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx
            .call(
                "POST",
                &format!("/conversations/{}/proposals", self.response),
                Some(json!({"task":self.task,"action":self.proposal_action})),
            )
            .await?))
    }
}

/// Start this conversation's prepared task within project autonomy; manual projects need a proposal.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Dispatch {
    pub response: String,
    #[arg(long)]
    pub task: String,
}
impl Op for Dispatch {
    const GROUP: &'static str = "conversation";
    const NAME: &'static str = "dispatch";
    const NEED: Need = Need::Write;
    const LISTED: Listed = Listed::Request;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("POST", &format!("/conversations/{}/dispatch", self.response), Some(json!({"task":self.task}))).await?))
    }
}

/// Approve or reject a proposal as its authenticated requester, never as an intake agent.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Decide {
    pub proposal: String,
    #[arg(long, required_unless_present = "reject", conflicts_with = "reject")]
    #[serde(default)]
    pub approve: bool,
    #[arg(long, required_unless_present = "approve", conflicts_with = "approve")]
    #[serde(default)]
    pub reject: bool,
}
impl Op for Decide {
    const GROUP: &'static str = "conversation";
    const NAME: &'static str = "decide";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        if self.approve == self.reject {
            return Err("choose exactly one of approve or reject".into());
        }
        Ok(out(cx
            .call("POST", &format!("/conversation-proposals/{}/decide", self.proposal), Some(json!({"approved":self.approve})))
            .await?))
    }
}

/// Answer a task clarification as the conversation's requester; obsolete questions are refused.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Answer {
    pub conversation: String,
    #[arg(long)]
    pub task: String,
    #[arg(long)]
    pub question: i64,
    #[arg(long)]
    pub text: String,
}
impl Op for Answer {
    const GROUP: &'static str = "conversation";
    const NAME: &'static str = "answer";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx
            .call(
                "POST",
                &format!("/conversations/{}/answer", self.conversation),
                Some(json!({"task":self.task,"question":self.question,"text":self.text})),
            )
            .await?))
    }
}
