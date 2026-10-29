//! Project chat: people and the project manager talk in one thread per project.
//!
//! `GET /chat` reads a page of it with the project manager's state. `POST /chat` posts: people,
//! and among agents only the project manager (not a task manager, not a member, not a job). A
//! person's message that tags the manager (`@pm`, `@manager`, `@менеджер`) is mail for the project
//! manager, with the messages before it as context; `@login` of a project member notifies that
//! member on the web. Waking the manager is the scheduler's job (`runtime`): in a `manual` project
//! it runs for chat mail and nothing else. `POST /chat/warm` starts the manager ahead of a message
//! (when a person opens the chat), so its first answer is not a cold start.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use genie_core::chat::{ChatMessage, MANAGER};
use genie_core::team::{CHAT_MAIL, ORCHESTRATOR};
use genie_core::{Actor, Role};
use serde::{Deserialize, Serialize};

use super::ctx::Ctx;
use super::{ApiError, ApiResult, Body};
use crate::management;
use crate::notify::{self, Message};
use crate::runtime::{self, AgentKey};
use crate::sessions;
use crate::state::{App, AppResult};

/// Messages before a manager-tagged one that its mail carries as context.
const CONTEXT: i64 = 10;
/// How much of a message the context and the notification show, in characters.
const CLIP: usize = 300;
/// The shortest time between two warm-ups of one project; a request inside it starts nothing.
const WARM_EVERY: Duration = Duration::from_secs(30);

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/chat", get(page).post(post_message)).route("/chat/warm", axum::routing::post(warm))
}

#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct ChatPage {
    pub messages: Vec<ChatMessage>,
    pub manager: ManagerChat,
}

/// The project manager as the chat shows it.
#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
pub struct ManagerChat {
    /// The state of its live session (`starting`, `idle`, `working`, `stopping`); `null` when none runs.
    pub state: Option<String>,
    /// Chat messages it has not received yet.
    pub pending: i64,
}

#[derive(Deserialize, Default)]
struct PageQuery {
    before: Option<i64>,
    limit: Option<i64>,
}

async fn page(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<PageQuery>) -> ApiResult<Json<ChatPage>> {
    let access = ctx.access(&app, None).await?;
    let slug = access.project.clone();
    let (messages, pending) = app
        .blocking(move |app| {
            app.with_tracker(&slug, |t| Ok((t.chat().list(q.before, q.limit)?, t.bus().undelivered_of_kind(ORCHESTRATOR, CHAT_MAIL)?)))
        })
        .await?;
    let state = app.sessions.get(&AgentKey::Orchestrator { project: access.project.clone() }).map(|s| s.live().state);
    Ok(Json(ChatPage { messages, manager: ManagerChat { state, pending } }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PostBody {
    text: String,
    reply_to: Option<i64>,
}

async fn post_message(State(app): State<Arc<App>>, ctx: Ctx, Body(b): Body<PostBody>) -> ApiResult<(StatusCode, Json<ChatMessage>)> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    if access.agent {
        // Only the project manager speaks for the project here; its refusal names the reason.
        management::project_manager(&access.actor)?;
    }
    let (slug, actor) = (access.project.clone(), access.actor.clone());
    let message = app.blocking(move |app| post(app, &slug, &actor, &b.text, b.reply_to)).await?;
    Ok((StatusCode::CREATED, Json(message)))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WarmReply {
    /// A session was started now (`false`: one runs already, the manager is not wanted, the session
    /// limit is reached, or the last warm-up was less than 30 s ago).
    started: bool,
}

/// A person opened the project chat or its composer: start the project manager ahead of their
/// message, when it would run now anyway. A warm-up also opens the manager window of a `manual`
/// project (the same as a message that tags it). People only, once per project per 30 s.
async fn warm(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<WarmReply>> {
    let access = ctx.access(&app, None).await?;
    if access.agent {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "a person warms the project chat, not an agent"));
    }
    let slug = access.project.clone();
    if !app.sessions.warm(&slug, app.clock.now(), WARM_EVERY) {
        return Ok(Json(WarmReply { started: false }));
    }
    let wanted_slug = slug.clone();
    let wanted = app.blocking(move |app| runtime::manager_wanted_in(app, &wanted_slug)).await?;
    if !wanted {
        return Ok(Json(WarmReply { started: false }));
    }
    let started = sessions::prewarm(&app, &AgentKey::Orchestrator { project: slug }).await?;
    Ok(Json(WarmReply { started }))
}

/// Posts a message and routes what it asks for. The message, its `chat.posted` event and the
/// manager's mail commit in one tracker transaction: a failure leaves neither a message without its
/// mail nor a retry that would post it twice. The web notices come after the commit; they are
/// best effort and never fail a post that was stored.
fn post(app: &App, slug: &str, actor: &Actor, text: &str, reply_to: Option<i64>) -> AppResult<ChatMessage> {
    let (message, to_manager) = app.with_tracker(slug, |t| {
        t.tx(|| {
            let message = t.chat().post(&actor.name, actor.role, text, reply_to)?;
            let to_manager = message.author_role == Role::Human && message.mentions.iter().any(|m| m == MANAGER);
            if to_manager {
                let context = t.chat().list(Some(message.id), Some(CONTEXT))?;
                let mail = manager_mail(&message, &context);
                t.bus().notify_manager(ORCHESTRATOR, &message.author, "human", CHAT_MAIL, &mail, None)?;
            }
            Ok((message, to_manager))
        })
    })?;
    if to_manager {
        app.wake_runtime.notify_one();
    }
    if let Err(e) = notify_members(app, slug, &message) {
        eprintln!("genie chat: {slug}: notices for message #{}: {e}", message.id);
    }
    Ok(message)
}

/// A project member named by login hears of a message on the web (whoever wrote it).
fn notify_members(app: &App, slug: &str, m: &ChatMessage) -> AppResult<()> {
    let users = app.with_server(|db| {
        let mut ids = Vec::new();
        for login in m.mentions.iter().filter(|t| *t != MANAGER && **t != m.author) {
            if let Some(u) = db.user_by_login(login)?
                && !u.disabled
                && db.project_role(slug, &u)?.is_some()
            {
                ids.push(u.id);
            }
        }
        Ok(ids)
    })?;
    if !users.is_empty() {
        let msg = Message {
            kind: "chat".into(),
            title: format!("@{} упомянул вас в чате", m.author),
            body: clip(&m.text, CLIP),
            project: Some(slug.into()),
            link: Some("/chat".into()),
            ..Default::default()
        };
        notify::send(app, &users, &msg, Some(&format!("chat:{slug}:{}", m.id)))?;
    }
    Ok(())
}

/// The project manager's mail for a tagged message: the message, the messages before it, and how to answer.
fn manager_mail(m: &ChatMessage, context: &[ChatMessage]) -> String {
    let mut out = format!("Project chat message #{} from @{}:\n\n{}\n", m.id, m.author, m.text.trim());
    if !context.is_empty() {
        out.push_str("\nEarlier in the chat:\n");
        for c in context {
            out.push_str(&format!("#{} {} {}: {}\n", c.id, c.at.get(11..16).unwrap_or_default(), speaker(c), line(&c.text, CLIP)));
        }
    }
    out.push_str(&format!(
        "\nAnswer in the chat, briefly and in the language of the message: genie chat post --reply-to {} \"…\"\nOlder messages: genie chat read --before {}\n",
        m.id, m.id
    ));
    out
}

/// Who wrote a message as the chat names it.
fn speaker(m: &ChatMessage) -> String {
    match m.author_role {
        Role::Human => format!("@{}", m.author),
        _ => "project manager".into(),
    }
}

/// A text on one line, cut to `max` characters.
fn line(text: &str, max: usize) -> String {
    clip(&text.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    format!("{}…", text.chars().take(max).collect::<String>())
}
