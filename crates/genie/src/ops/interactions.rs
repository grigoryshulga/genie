//! One set of human interaction operations for CLI and MCP clients.
use super::{Cx, Entry, Need, Op, Out, enc, register};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

pub fn register(all: &mut Vec<Entry>) {
    register!(all, List, Show, Respond);
}

fn output(value: serde_json::Value) -> Out {
    let rows = if value.is_array() { value.as_array().cloned().unwrap_or_default() } else { vec![value.clone()] };
    let text = rows
        .iter()
        .map(|v| {
            let mut text = format!(
                "{} · {}\nVersion: {}\n{}\nActions: {}",
                v["id"].as_str().unwrap_or_default(),
                v["state"].as_str().unwrap_or_default(),
                v["version"].as_str().unwrap_or_default(),
                v["prompt"].as_str().unwrap_or_default(),
                v["actions"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", ")
            );
            if let Some(answer) = v["resolution"]["answer"].as_str() {
                text.push_str(&format!("\n→ {answer}"));
            }
            if let Some(actor) = v["resolution"]["actor"].as_str() {
                text.push_str(&format!("\nDecided by: {actor}"));
            }
            let deliveries = v["deliveries"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|d| {
                    format!(
                        "{} {} ({})",
                        d["channel"].as_str().unwrap_or_default(),
                        d["status"].as_str().unwrap_or_default(),
                        d["attempts"]
                    )
                })
                .collect::<Vec<_>>();
            if !deliveries.is_empty() {
                text.push_str(&format!("\nDelivery: {}", deliveries.join(", ")));
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    Out::new(if rows.is_empty() { "No pending interactions.".into() } else { text }, value)
}

/// List unresolved questions, proposals and execution pauses in this project.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct List {
    #[arg(long)]
    #[serde(default)]
    pub all: bool,
}
impl Op for List {
    const GROUP: &'static str = "interaction";
    const NAME: &'static str = "list";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(output(cx.call("GET", if self.all { "/interactions?all=true" } else { "/interactions" }, None).await?))
    }
}

/// Inspect one authoritative question or decision, including its current version.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Show {
    pub id: String,
}
impl Op for Show {
    const GROUP: &'static str = "interaction";
    const NAME: &'static str = "show";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(output(cx.call("GET", &format!("/interactions/{}", enc(&self.id)), None).await?))
    }
}

/// Answer, approve, reject or resume the exact version you inspected.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Respond {
    pub id: String,
    #[arg(long)]
    pub version: String,
    // MCP reserves `action` for the operation selector (`respond`).
    #[arg(long = "action")]
    pub decision: crate::interactions::Action,
    #[arg(long)]
    pub text: Option<String>,
}
impl Op for Respond {
    const GROUP: &'static str = "interaction";
    const NAME: &'static str = "respond";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(output(
            cx.call(
                "POST",
                &format!("/interactions/{}/respond", enc(&self.id)),
                Some(json!({"version":self.version,"action":self.decision,"text":self.text})),
            )
            .await?,
        ))
    }
}
