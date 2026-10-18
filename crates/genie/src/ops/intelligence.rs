use super::{Cx, Entry, Listed, Need, Op, Out, enc, register};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn register(all: &mut Vec<Entry>) {
    register!(all, History, Assess, Reassess, Shadow, Resolve);
}
fn out(value: Value) -> Out {
    Out::new(serde_json::to_string_pretty(&value).unwrap_or_default(), value)
}
fn parse(s: &str) -> Result<Value, String> {
    serde_json::from_str(s).map_err(|e| e.to_string())
}

/// Read immutable task signals, routing, execution attempts and shadow policy evidence.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct History {
    #[arg(long)]
    pub task: Option<String>,
}
impl Op for History {
    const GROUP: &'static str = "intelligence";
    const NAME: &'static str = "history";
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("GET", &format!("/tasks/{}/intelligence", enc(&cx.task(self.task)?)), None).await?))
    }
}
/// Assess task signals. Only people may narrow scope with explicit affected paths.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Assess {
    #[arg(long)]
    pub task: Option<String>,
    #[arg(long = "affected-path")]
    #[serde(default)]
    pub affected_paths: Vec<String>,
}
impl Op for Assess {
    const GROUP: &'static str = "intelligence";
    const NAME: &'static str = "assess";
    const NEED: Need = Need::Write;
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx
            .call(
                "POST",
                &format!("/tasks/{}/intelligence/assess", enc(&cx.task(self.task)?)),
                Some(json!({"affectedPaths":self.affected_paths})),
            )
            .await?))
    }
}
/// Record a material discovery and add mandatory independent reviewers in the existing workspace.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Reassess {
    #[arg(long)]
    pub task: Option<String>,
    /// JSON: requestId, reason, evidenceRefs, affectedPaths?. Reuse requestId on retries.
    #[arg(long,value_parser=parse)]
    pub request: Value,
}
impl Op for Reassess {
    const GROUP: &'static str = "intelligence";
    const NAME: &'static str = "reassess";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("POST", &format!("/tasks/{}/intelligence/reassess", enc(&cx.task(self.task)?)), Some(self.request)).await?))
    }
}
/// Shadow-evaluate a candidate policy against a frozen snapshot. Never applies production changes.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Shadow {
    #[arg(long)]
    pub task: Option<String>,
    /// JSON: snapshotId, candidate (the shared Jev policy contract).
    #[arg(long,value_parser=parse)]
    pub request: Value,
}
impl Op for Shadow {
    const GROUP: &'static str = "intelligence";
    const NAME: &'static str = "shadow";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("POST", &format!("/tasks/{}/intelligence/shadow", enc(&cx.task(self.task)?)), Some(self.request)).await?))
    }
}

/// Explicitly resolve runtime constraints with human-reviewed evidence. Does not resume execution.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Resolve {
    #[arg(long)]
    pub task: Option<String>,
    #[arg(long,value_parser=parse)]
    pub request: Value,
}
impl Op for Resolve {
    const GROUP: &'static str = "intelligence";
    const NAME: &'static str = "resolve";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("POST", &format!("/tasks/{}/intelligence/resolve", enc(&cx.task(self.task)?)), Some(self.request)).await?))
    }
}
