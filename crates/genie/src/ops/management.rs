//! The same management operations for people, CLI agents and MCP callers.
use super::{Cx, Entry, Listed, Need, Op, Out, enc, register};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn register(all: &mut Vec<Entry>) {
    register!(all, Show, Dispatch, Clarify, Answer, Escalate, Sources, Source, Retrospective, Retry, Memory, Remember);
}

fn out(value: Value) -> Out {
    Out::new(serde_json::to_string_pretty(&value).unwrap_or_default(), value)
}

/// Current task-manager state, brief, clarification requests and task evidence.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Show {
    pub task: Option<String>,
}
impl Op for Show {
    const GROUP: &'static str = "manager";
    const NAME: &'static str = "show";
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("GET", &format!("/management/tasks/{}", enc(&cx.task(self.task)?)), None).await?))
    }
}

macro_rules! task_action {
    ($name:ident, $command:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(clap::Args, Deserialize, JsonSchema)]
        pub struct $name {
            pub task: Option<String>,
        }
        impl Op for $name {
            const GROUP: &'static str = "manager";
            const NAME: &'static str = $command;
            const NEED: Need = Need::Orchestrator;
            const LISTED: Listed = Listed::Orchestrator;
            async fn run(self, cx: &Cx) -> Result<Out, String> {
                Ok(out(cx.call("POST", &format!("/management/tasks/{}/{}", enc(&cx.task(self.task)?), $command), Some(json!({}))).await?))
            }
        }
    };
}
/// Release a ready unblocked task to its own manager; repeated calls reuse the manager. In a manual
/// project the project manager dispatches only on behalf of the chat message that asked for it.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Dispatch {
    pub task: Option<String>,
    /// The chat message (`genie chat read`) in which a person asked for this dispatch.
    #[arg(long)]
    pub on_behalf: Option<i64>,
}
impl Op for Dispatch {
    const GROUP: &'static str = "manager";
    const NAME: &'static str = "dispatch";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let body = json!({ "onBehalf": self.on_behalf });
        Ok(out(cx.call("POST", &format!("/management/tasks/{}/dispatch", enc(&cx.task(self.task)?)), Some(body)).await?))
    }
}
task_action!(Retry, "retry", "Wake a task manager to resume execution or its mandatory retrospective.");

/// Ask the project manager; it answers with evidence or escalates to a person.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Clarify {
    #[arg(long)]
    pub task: Option<String>,
    #[arg(long)]
    pub question: String,
    #[arg(long)]
    #[serde(default)]
    pub nonblocking: bool,
}
impl Op for Clarify {
    const GROUP: &'static str = "manager";
    const NAME: &'static str = "clarify";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx
            .call(
                "POST",
                &format!("/management/tasks/{}/clarify", enc(&cx.task(self.task)?)),
                Some(json!({"question":self.question,"nonblocking":self.nonblocking})),
            )
            .await?))
    }
}

/// Answer a clarification using a confirmed decision, document or a person's response.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Answer {
    pub task: String,
    pub id: i64,
    #[arg(long)]
    pub text: String,
    #[arg(long)]
    pub source: String,
}
impl Op for Answer {
    const GROUP: &'static str = "manager";
    const NAME: &'static str = "answer";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx
            .call(
                "POST",
                &format!("/management/tasks/{}/answer", enc(&self.task)),
                Some(json!({"id":self.id,"text":self.text,"source":self.source})),
            )
            .await?))
    }
}

/// Escalate an unresolved clarification to a person via needs_owner.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Escalate {
    pub task: String,
    pub id: i64,
}
impl Op for Escalate {
    const GROUP: &'static str = "manager";
    const NAME: &'static str = "escalate";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("POST", &format!("/management/tasks/{}/escalate", enc(&self.task)), Some(json!({"id":self.id}))).await?))
    }
}

/// Enumerate all recorded execution sources, including failed attempts and former members.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Sources {
    pub task: Option<String>,
}
impl Op for Sources {
    const GROUP: &'static str = "manager";
    const NAME: &'static str = "sources";
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("GET", &format!("/management/tasks/{}/sources", enc(&cx.task(self.task)?)), None).await?))
    }
}

/// Read a recorded source; continue with nextOffset until the entire source was read.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Source {
    pub id: String,
    #[arg(long)]
    pub task: Option<String>,
    #[arg(long, default_value_t = 0)]
    #[serde(default)]
    pub offset: usize,
}
impl Op for Source {
    const GROUP: &'static str = "manager";
    const NAME: &'static str = "source";
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx
            .call("GET", &format!("/management/tasks/{}/source/{}?offset={}", enc(&cx.task(self.task)?), enc(&self.id), self.offset), None)
            .await?))
    }
}

/// Publish task retrospective; complete coverage finishes the manager separately from the result.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Retrospective {
    #[arg(long)]
    pub task: Option<String>,
    #[arg(long)]
    pub text: String,
    #[arg(long)]
    #[serde(default)]
    pub reviewed: Vec<String>,
    #[arg(long)]
    #[serde(default)]
    pub incomplete: bool,
    #[arg(long)]
    #[serde(default)]
    pub accept_missing: bool,
}
impl Op for Retrospective {
    const GROUP: &'static str = "manager";
    const NAME: &'static str = "retrospective";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx
            .call(
                "POST",
                &format!("/management/tasks/{}/retrospective", enc(&cx.task(self.task)?)),
                Some(json!({"text":self.text,"reviewed":self.reviewed,"incomplete":self.incomplete,"acceptMissing":self.accept_missing})),
            )
            .await?))
    }
}

/// Search current shared project memories and their sources.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Memory {
    #[arg(default_value = "")]
    #[serde(default)]
    pub query: String,
}
impl Op for Memory {
    const GROUP: &'static str = "manager";
    const NAME: &'static str = "memory";
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("GET", &format!("/management/memory?query={}", enc(&self.query)), None).await?))
    }
}

/// Preserve a confirmed shared project decision with its source; optionally supersede a memory.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Remember {
    #[arg(long)]
    pub text: String,
    #[arg(long)]
    pub source: String,
    #[arg(long)]
    pub replaces: Option<i64>,
}
impl Op for Remember {
    const GROUP: &'static str = "manager";
    const NAME: &'static str = "remember";
    const NEED: Need = Need::Write;
    const LISTED: Listed = Listed::Agents;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx
            .call("POST", "/management/memory", Some(json!({"text":self.text,"source":self.source,"replaces":self.replaces})))
            .await?))
    }
}
