//! The project chat: people and the project manager talk here. The manager reads the history
//! around a message it was sent and answers in the chat.
use super::{Cx, Entry, Listed, Need, Op, Out, register};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

pub fn register(all: &mut Vec<Entry>) {
    register!(all, Read, Post);
}

/// The project chat, oldest first: the newest messages, or the page before `--before`.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Read {
    /// How many messages (1 to 200, default 50).
    #[arg(long)]
    pub limit: Option<i64>,
    /// Only messages older than this id.
    #[arg(long)]
    pub before: Option<i64>,
}
impl Op for Read {
    const GROUP: &'static str = "chat";
    const NAME: &'static str = "read";
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let query: Vec<String> =
            self.limit.map(|n| format!("limit={n}")).into_iter().chain(self.before.map(|b| format!("before={b}"))).collect();
        let path = if query.is_empty() { "/chat".to_string() } else { format!("/chat?{}", query.join("&")) };
        let v = cx.call("GET", &path, None).await?;
        Ok(Out::new(render(&v), v))
    }
}

/// Post a message to the project chat; `--reply-to` answers one. `-` reads the text from stdin.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    /// What to say (`-` for stdin on the command line).
    #[arg(allow_hyphen_values = true)]
    pub text: String,
    /// The chat message this answers.
    #[arg(long)]
    pub reply_to: Option<i64>,
}
impl Op for Post {
    const GROUP: &'static str = "chat";
    const NAME: &'static str = "post";
    const NEED: Need = Need::Write;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let text = cx.text(Some(self.text), None)?.unwrap_or_default();
        let v = cx.call("POST", "/chat", Some(json!({ "text": text, "replyTo": self.reply_to }))).await?;
        Ok(Out::new(format!("posted #{}", v["id"]), v))
    }
}

/// The page as people read it: one message per line block, `#id time who [↩ reply]: text`, and the
/// project manager's state last.
fn render(v: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    for m in v["messages"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let at = m["at"].as_str().and_then(|a| a.get(11..16)).unwrap_or_default();
        let who = match m["authorRole"].as_str() {
            Some("human") => format!("@{}", m["author"].as_str().unwrap_or_default()),
            _ => "project manager".to_string(),
        };
        let reply = m["replyTo"].as_i64().map(|r| format!(" ↩ #{r}")).unwrap_or_default();
        lines.push(format!("#{} {at} {who}{reply}: {}", m["id"], m["text"].as_str().unwrap_or_default()));
    }
    if lines.is_empty() {
        lines.push("(the chat is empty)".into());
    }
    let state = v["manager"]["state"].as_str().unwrap_or("not running");
    let waiting = v["manager"]["pending"].as_i64().unwrap_or_default();
    lines.push(format!("project manager: {state} · {waiting} chat message(s) not yet read by it"));
    lines.join("\n")
}
