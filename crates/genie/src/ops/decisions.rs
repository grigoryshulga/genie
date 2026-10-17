use super::{Cx, Entry, Listed, Need, Op, Out, enc, register};
use crate::decisions::Request;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn register(all: &mut Vec<Entry>) {
    register!(all, Evaluate, List, Show, Policy);
}
fn out(v: Value) -> Out {
    Out::new(serde_json::to_string_pretty(&v).unwrap_or_default(), v)
}
fn request(s: &str) -> Result<Request, String> {
    serde_json::from_str(s).map_err(|e| e.to_string())
}

/// Ask Jev typed questions about supplied facts and alternatives. Returns probabilities,
/// uncertainty and a durable decision id; does not execute the chosen action. Jev cannot
/// verify missing facts. Use this when another judgement will help, within the task allowance.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evaluate {
    /// JSON object: task?, purpose, state, questions (choice, score or bool).
    #[arg(long,value_parser=request)]
    pub request: Request,
}
impl Op for Evaluate {
    const GROUP: &'static str = "decision";
    const NAME: &'static str = "evaluate";
    const NEED: Need = Need::Write;
    const LISTED: Listed = Listed::Agents;
    async fn run(mut self, cx: &Cx) -> Result<Out, String> {
        self.request.task = Some(cx.task(self.request.task)?);
        Ok(out(cx.call("POST", "/decisions/evaluate", Some(json!(self.request))).await?))
    }
}

/// Recorded decisions for a task, including their requests, results and application.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct List {
    #[arg(long)]
    pub task: Option<String>,
}
impl Op for List {
    const GROUP: &'static str = "decision";
    const NAME: &'static str = "list";
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("GET", &format!("/decisions?task={}", enc(&cx.task(self.task)?)), None).await?))
    }
}

/// Read a decision by its id, including uncertainty, usage and final application.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Show {
    pub id: i64,
}
impl Op for Show {
    const GROUP: &'static str = "decision";
    const NAME: &'static str = "show";
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("GET", &format!("/decisions/{}", self.id), None).await?))
    }
}

/// Read the project's Jev modes, tool grants, profiles and task allowance.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Policy {}
impl Op for Policy {
    const GROUP: &'static str = "decision";
    const NAME: &'static str = "policy";
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("GET", "/decisions/policy", None).await?))
    }
}
