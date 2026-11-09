//! The caller: who they are, a person's notifications and the questions agents
//! asked them (the web's bell and answer page).

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Cx, Entry, Need, Op, Out, register};

pub fn register(all: &mut Vec<Entry>) {
    register!(all, Show, Notifications, Read, Questions, Answer, Delivery);
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_default()
}

/// Inspect or set your preferred external channel and explicit ordered fallbacks.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Delivery {
    /// Comma-separated channels, e.g. telegram,email. Used only after failed delivery.
    #[arg(long, value_delimiter = ',')]
    pub order: Option<Vec<String>>,
    #[arg(long)]
    pub after_attempts: Option<i64>,
    #[arg(long)]
    #[serde(default)]
    pub web_only: bool,
    #[arg(long)]
    #[serde(default)]
    pub reset: bool,
}
impl Op for Delivery {
    const GROUP: &'static str = "me";
    const NAME: &'static str = "delivery";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        if self.reset && (self.order.is_some() || self.web_only || self.after_attempts.is_some()) || self.web_only && self.order.is_some() {
            return Err("choose --reset, --web-only or --order".into());
        }
        if self.reset || self.web_only || self.order.is_some() {
            let policy = if self.reset {
                Value::Null
            } else {
                let policy = genie_core::human_delivery::DeliveryPolicy {
                    order: self.order.unwrap_or_default(),
                    fallback_after_attempts: self.after_attempts.unwrap_or(genie_core::inbox::MAX_DELIVERY_ATTEMPTS),
                };
                policy.validate().map_err(|e| e.to_string())?;
                json!(policy)
            };
            cx.call("PUT", "/me/delivery", Some(json!({"policy":policy}))).await?;
        } else if self.after_attempts.is_some() {
            return Err("specify --order with --after-attempts".into());
        }
        let value = cx.call("GET", "/me/delivery", None).await?;
        let text = if value["policy"].is_null() {
            "Default: web and Telegram when linked, otherwise email; no external fallback after failure.".into()
        } else if value["policy"]["order"].as_array().is_some_and(|v| v.is_empty()) {
            "Web only.".into()
        } else {
            format!(
                "External order: {}. Fallback after {} failed attempt(s).",
                value["policy"]["order"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(" → "),
                value["policy"]["fallbackAfterAttempts"]
            )
        };
        Ok(Out::new(text, value))
    }
}

/// Who you are to the server: a person and their projects, or an agent and its project and team.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Show {}

impl Op for Show {
    const GROUP: &'static str = "me";
    const NAME: &'static str = "show";
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/auth/me", None).await?;
        if !v["agent"].is_null() {
            let a = &v["agent"];
            let text = format!(
                "agent {} ({}) of project {}{}",
                s(a, "name"),
                s(a, "role"),
                s(a, "project"),
                a["team"].as_str().map(|t| format!(", team {t}")).unwrap_or_default()
            );
            return Ok(Out::new(text, v));
        }
        let u = &v["user"];
        let mut out = vec![format!(
            "{} ({}){}{}",
            s(u, "login"),
            s(u, "name"),
            if u["isAdmin"] == json!(true) { " · server admin" } else { "" },
            if s(&v, "mode") == "local" { " · on the server's machine" } else { "" }
        )];
        for p in v["projects"].as_array().cloned().unwrap_or_default() {
            out.push(format!(
                "  {} {:<16} {:<8} {}",
                if v["project"] == p["slug"] { "*" } else { " " },
                s(&p, "slug"),
                s(&p, "role"),
                s(&p, "name")
            ));
        }
        Ok(Out::new(out.join("\n"), v))
    }
}

/// Your notifications: unread ones, or all recent with --all.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Notifications {
    #[arg(long)]
    #[serde(default)]
    pub all: bool,
}

impl Op for Notifications {
    const GROUP: &'static str = "me";
    const NAME: &'static str = "notifications";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", if self.all { "/notifications" } else { "/notifications?unread=1" }, None).await?;
        let mut out = Vec::new();
        for n in v["items"].as_array().cloned().unwrap_or_default() {
            out.push(format!(
                "#{:<5} {}{} {}{}",
                n["id"].to_string(),
                if n["readAt"].is_null() { "● " } else { "" },
                s(&n, "created"),
                n["project"].as_str().map(|p| format!("[{p}] ")).unwrap_or_default(),
                s(&n, "title")
            ));
            for line in s(&n, "body").lines().filter(|l| !l.trim().is_empty()) {
                out.push(format!("       {line}"));
            }
        }
        let text = if out.is_empty() {
            if self.all { "no notifications".to_string() } else { "nothing unread".to_string() }
        } else {
            out.push(format!("{} unread", v["unread"]));
            out.join("\n")
        };
        Ok(Out::new(text, v))
    }
}

/// Mark a notification read, or all of them without --id.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Read {
    #[arg(long)]
    pub id: Option<i64>,
}

impl Op for Read {
    const GROUP: &'static str = "me";
    const NAME: &'static str = "read";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("POST", "/notifications/read", Some(json!({ "id": self.id }))).await?;
        Ok(Out::new(self.id.map_or("all notifications read".into(), |id| format!("notification #{id} read")), v))
    }
}

fn questionnaire(q: &Value) -> Vec<String> {
    let mut out = vec![format!(
        "#{} {} · {}{} · asked by {}{}",
        q["id"],
        s(q, "status"),
        s(q, "project"),
        q["task"].as_str().map(|t| format!(" {t}")).unwrap_or_default(),
        s(q, "askedBy"),
        q["due"].as_str().map(|d| format!(" · due {d}")).unwrap_or_default()
    )];
    for qu in q["questions"].as_array().cloned().unwrap_or_default() {
        out.push(format!("  {}. {}", qu["n"], s(&qu, "text")));
        if !s(&qu, "why").is_empty() {
            out.push(format!("     why: {}", s(&qu, "why")));
        }
        let options: Vec<&str> = qu["options"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        if !options.is_empty() {
            out.push(format!("     options: {}", options.join(" | ")));
        }
        if let Some(a) = qu["answer"].as_str() {
            out.push(format!("     → {a}"));
        }
    }
    out
}

/// Questions agents and automations asked you and are waiting for.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Questions {}

impl Op for Questions {
    const GROUP: &'static str = "me";
    const NAME: &'static str = "questions";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let v = cx.call("GET", "/questions", None).await?;
        let rows = v.as_array().cloned().unwrap_or_default();
        let text = if rows.is_empty() {
            "no questions waiting for you".to_string()
        } else {
            rows.iter().flat_map(questionnaire).collect::<Vec<_>>().join("\n")
        };
        Ok(Out::new(text, v))
    }
}

/// Answer questions you were asked: `<n>=<answer>` for each (see `genie me questions`).
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Answer {
    /// The questionnaire's number.
    pub questionnaire: i64,
    /// `<question number>=<answer>`; repeat for more.
    #[arg(required = true, allow_hyphen_values = true, value_name = "N=ANSWER")]
    pub answers: Vec<String>,
}

impl Op for Answer {
    const GROUP: &'static str = "me";
    const NAME: &'static str = "answer";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let answers = super::pairs(&self.answers, "answer")?;
        let v = cx.call("POST", &format!("/questions/{}/answer", self.questionnaire), Some(json!({ "answers": answers }))).await?;
        let mut out = questionnaire(&v["questionnaire"]);
        out.push(if v["complete"] == json!(true) {
            "all answered: the answers went to the task".into()
        } else {
            "saved; some questions are still open".into()
        });
        Ok(Out::new(out.join("\n"), v))
    }
}
