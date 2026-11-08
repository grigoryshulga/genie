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

/// A dedicated cursor backfills still-pending interactions when this capability is enabled.
/// Outbox writes precede acknowledgement; replay uses stable domain dedupe keys.
pub fn tick(app: &App, project: &str) -> AppResult<()> {
    const CURSOR: &str = "human-interactions-v1";
    let cursor = app.with_tracker(project, |t| t.event_cursor(CURSOR))?;
    let events = app.with_tracker(project, |t| t.events_after(cursor, 200))?;
    for event in &events {
        notify_event(app, project, event)?;
        app.with_tracker(project, |t| t.ack_events(CURSOR, event.id))?;
    }
    if events.len() == 200 {
        app.wake_engine.notify_one();
    }
    Ok(())
}

pub fn notify_event(app: &App, project: &str, event: &Event) -> AppResult<()> {
    if event.kind == "task.execution_paused" {
        return notify_execution_pause(app, project, event);
    }
    if event.kind == "conversation.proposal_created" {
        return notify_proposal(app, project, event);
    }
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
    let Some(user) = recipient(app, project, &c.principal)? else { return Ok(()) };
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
    telegram_response(app, bot, chat, message, text, false)
}

pub fn telegram_decision(app: &App, bot: &ProjectBot, chat: i64, message: i64, text: &str) -> AppResult<bool> {
    telegram_response(app, bot, chat, message, text, true)
}

fn telegram_response(app: &App, bot: &ProjectBot, chat: i64, message: i64, text: &str, decision_only: bool) -> AppResult<bool> {
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
    if i.is_null() || (decision_only && i["proposal"].as_str().is_none()) {
        return Ok(false);
    }
    let bad = || AppError::Conflict("The question reference is no longer valid".into());
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
    if let Some(proposal) = i["proposal"].as_str() {
        let approved = match text.trim().to_lowercase().as_str() {
            "одобряю" | "approve" | "/approve" => true,
            "отклоняю" | "reject" | "/reject" => false,
            _ => return Err(AppError::Bad("Reply with an explicit approval or rejection".into())),
        };
        conversations::decide(app, &access, proposal, approved)?;
    } else {
        let conversation = i["conversation"].as_str().ok_or_else(bad)?;
        let task = i["task"].as_str().ok_or_else(bad)?;
        let question = i["question"].as_i64().ok_or_else(bad)?;
        conversations::answer(app, &access, conversation, task, question, text)?;
    }
    Ok(true)
}

fn notify_proposal(app: &App, project: &str, event: &Event) -> AppResult<()> {
    let Some(id) = event.payload["proposal_id"].as_str() else {
        return Ok(());
    };
    let prepared = app.with_tracker(project, |t| {
        let p = t.conversations().proposal(id)?;
        if p.status != genie_core::conversations::ProposalStatus::Pending {
            return Ok(None);
        }
        if p.version != conversations::task_version(t, &p.task)? {
            return Ok(None);
        }
        let c = t.conversations().conversation(&p.conversation_id)?;
        let task = t.get(&p.task)?;
        Ok(Some((p, c, task)))
    });
    let (p, c, task) = match prepared {
        Ok(Some(value)) => value,
        Ok(None) | Err(AppError::Genie(genie_core::GenieError::NotFound(_))) => return Ok(()),
        Err(error) => return Err(error),
    };
    let Some(user) = recipient(app, project, &c.principal)? else { return Ok(()) };
    let action = match p.action {
        genie_core::conversations::ProposalAction::Dispatch => "Запустить задачу",
        genie_core::conversations::ProposalAction::Accept => "Принять результат",
    };
    let body = format!(
        "{action}: {}\n\n{}\n\nПлан: {}\nАртефактов: {}\nПредложение: {}\nВерсия: {}\n\nПолные условия и результат — в задаче по ссылке. В Telegram-боте проекта ответьте «одобряю» или «отклоняю» на это сообщение.",
        task.title.chars().take(200).collect::<String>(),
        task.description.chars().take(1000).collect::<String>(),
        task.plan.chars().take(800).collect::<String>(),
        task.artifacts.len(),
        p.id,
        p.version
    );
    notify::send(
        app,
        &[user.id],
        &notify::Message {
            kind: "conversation_proposal".into(),
            title: format!("{} · Требуется решение", p.task),
            body,
            project: Some(project.into()),
            task: Some(p.task.clone()),
            link: Some(format!("/task/{}", p.task)),
            interaction: Some(json!({"proposal":p.id,"conversation":c.id,"task":p.task,"version":p.version})),
            buttons: vec![("Одобряю".into(), "interaction:approve".into()), ("Отклоняю".into(), "interaction:reject".into())],
            ..Default::default()
        },
        Some(&format!("interaction:{project}:proposal:{id}")),
    )?;
    Ok(())
}

// Display logins may change or be reused. Delivery follows the immutable authenticated principal.
fn recipient(app: &App, project: &str, principal: &str) -> AppResult<Option<genie_core::server_db::User>> {
    let Some(id) = principal.strip_prefix("user:").and_then(|s| s.parse::<i64>().ok()) else { return Ok(None) };
    app.with_server(|db| {
        let user = match db.user(id) {
            Ok(user) => user,
            Err(genie_core::GenieError::NotFound(_)) => return Ok(None),
            Err(error) => return Err(error),
        };
        let allowed = !user.disabled && db.project_role(project, &user)?.is_some_and(|r| r.can_write());
        Ok(allowed.then_some(user))
    })
}

fn notify_execution_pause(app: &App, project: &str, event: &Event) -> AppResult<()> {
    let Some(task) = event.subject.as_deref() else { return Ok(()) };
    let state = match app.with_tracker(project, |t| t.execution().state(task)) {
        Ok(Some(s)) if s.paused && s.reason != "owner" => s,
        Ok(_) | Err(AppError::Genie(genie_core::GenieError::NotFound(_))) => return Ok(()),
        Err(e) => return Err(e),
    };
    let users = notify::resolve(
        app,
        project,
        &["task.author".into(), "task.assignee".into(), "project.owners".into()],
        &json!({"task":{"id":task}}),
    )?;
    notify::send(
        app,
        &users,
        &notify::Message {
            kind: "execution_paused".into(),
            title: format!("{task} · Выполнение остановлено"),
            body: format!(
                "{}\n\nПрогресс и артефакты сохранены. Проверьте ограничения, затем отправьте /resume {task} в Genie. Возобновление проверяет текущие лимиты.",
                state.note
            ),
            project: Some(project.into()),
            task: Some(task.into()),
            link: Some(format!("/task/{task}")),
            ..Default::default()
        },
        Some(&format!("execution:{project}:{task}:{}", state.generation)),
    )?;
    Ok(())
}
