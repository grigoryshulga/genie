//! Channel-independent references to durable domain questions. Delivery is a projection:
//! the journal consumer acknowledges only after the deduplicated outbox write succeeds.
use crate::{
    conversations,
    http::ctx::Access,
    notify,
    state::{App, AppError, AppResult},
};
use genie_core::{Actor, Role, events::Event, telegram::ProjectBot};
use serde_json::json;

pub fn notify_event(app: &App, project: &str, event: &Event) -> AppResult<()> {
    if event.kind != "conversation.question" {
        return Ok(());
    }
    let Some(task) = event.subject.as_deref() else {
        return Ok(());
    };
    let Some(conversation) = event.payload["conversation_id"].as_str() else {
        return Ok(());
    };
    let Some(question) = event.payload["question"].as_i64() else {
        return Ok(());
    };
    let question_state = app.with_tracker(project, |t| {
        Ok((t.conversations().conversation(conversation)?, t.management().clarifications(task)?.into_iter().find(|q| q.id == question)))
    });
    let (c, q) = match question_state {
        Ok(state) => state,
        Err(AppError::Genie(genie_core::GenieError::NotFound(_))) => return Ok(()),
        Err(error) => return Err(error),
    };
    let Some(q) = q.filter(|q| q.state == "owner") else {
        return Ok(());
    };
    let Some(login) = c.initiator else {
        return Ok(());
    };
    let user = app.with_server(|db| db.user_by_login(&login))?;
    let Some(user) = user.filter(|u| !u.disabled) else {
        return Ok(());
    };
    if !app.with_server(|db| Ok(db.project_role(project, &user)?.is_some_and(|r| r.can_write())))? {
        return Ok(());
    }
    notify::send(
        app,
        &[user.id],
        &notify::Message {
            kind: "clarification".into(),
            title: format!("{task} · Нужен ваш ответ"),
            body: format!("{}\n\nОтветьте на вопрос в Genie. В Telegram-боте проекта можно ответить на это сообщение.", q.question),
            project: Some(project.into()),
            task: Some(task.into()),
            link: Some(format!("/task/{task}")),
            interaction: Some(json!({"conversation":conversation,"task":task,"question":question,"version":q.brief_version})),
            ..Default::default()
        },
        Some(&format!("interaction:{project}:clarification:{question}")),
    )?;
    Ok(())
}

/// Both account linking and current project rights are rechecked on every inbound reply.
/// The domain service supplies the same answer provenance as the native API, making a
/// redelivery after a crash harmless while rejecting a conflicting later answer.
pub fn telegram_reply(app: &App, bot: &ProjectBot, chat: i64, message: i64, text: &str) -> AppResult<bool> {
    let reference = format!("{}:{chat}:{message}", bot.scope);
    let found = app.with_server(|db| {
        let current = db.project_bot(&bot.project)?;
        if !current.is_some_and(|b| b.scope == bot.scope && b.token.0 == bot.token.0) {
            return Ok(None);
        }
        let Some(uid) = db.user_by_channel(&bot.scope, &chat.to_string())? else {
            return Ok(None);
        };
        let user = db.user(uid)?;
        let Some(role) = db.project_role(&bot.project, &user)? else {
            return Ok(None);
        };
        if user.disabled || !role.can_write() {
            return Ok(None);
        }
        let Some(payload) = db.outbox_by_ref("telegram", &reference)? else {
            return Ok(None);
        };
        if payload["recipient"] != uid || payload["telegramScope"] != bot.scope || payload["project"] != bot.project {
            return Ok(None);
        }
        Ok(Some((user, role, payload)))
    })?;
    let Some((user, role, payload)) = found else {
        return Ok(false);
    };
    let i = &payload["interaction"];
    if i.is_null() {
        return Ok(false);
    }
    let bad = || AppError::Conflict("The question reference is no longer valid".into());
    let conversation = i["conversation"].as_str().ok_or_else(bad)?;
    let task = i["task"].as_str().ok_or_else(bad)?;
    let question = i["question"].as_i64().ok_or_else(bad)?;
    let access = Access {
        project: bot.project.clone(),
        actor: Actor::new(&user.login, Role::Human),
        role,
        user: Some(user),
        agent: false,
        agent_team: None,
        agent_job: None,
        agent_role_id: None,
    };
    conversations::answer(app, &access, conversation, task, question, text)?;
    Ok(true)
}
