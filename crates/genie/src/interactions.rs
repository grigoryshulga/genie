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

mod human;
pub(crate) use human::answer_in;
pub use human::{Action, Interaction, Response, access, answer_clarification, list, respond, show};

/// Both bot adapters decode the same vocabulary; message correlation and
/// current account/project authority remain in their response services.
pub(crate) enum TelegramCallback {
    Action(Action),
    Option(usize),
}

impl TelegramCallback {
    pub(crate) fn parse(data: &str) -> Option<Self> {
        let parts = data.split(':').collect::<Vec<_>>();
        match parts.as_slice() {
            ["interaction", "approve"] => Some(Self::Action(Action::Approve)),
            ["interaction", "reject"] => Some(Self::Action(Action::Reject)),
            ["interaction", "resume"] => Some(Self::Action(Action::Resume)),
            ["q", _, _, index] => index.parse().ok().map(Self::Option),
            _ => None,
        }
    }
}

/// A dedicated cursor backfills still-pending interactions when this capability is enabled.
/// Outbox writes precede acknowledgement; replay uses stable domain dedupe keys.
pub fn tick(app: &App, project: &str) -> AppResult<()> {
    const CURSOR: &str = "human-interactions-v2";
    let cursor = app.with_tracker(project, |t| t.event_cursor(CURSOR))?;
    let events = app.with_tracker(project, |t| t.events_after(cursor, 200))?;
    for event in &events {
        notify_event(app, project, event)?;
        app.with_tracker(project, |t| t.ack_events(CURSOR, event.id))?;
    }
    if events.len() == 200 {
        app.wake_engine.notify_one();
    }
    reconcile(app, project)?;
    Ok(())
}

pub fn notify_event(app: &App, project: &str, event: &Event) -> AppResult<()> {
    if event.kind == "task.execution_paused" {
        return notify_execution_pause(app, project, event);
    }
    if event.kind == "conversation.proposal_created" {
        return notify_proposal(app, project, event);
    }
    if !matches!(event.kind.as_str(), "conversation.question" | "manager.clarification_escalated") {
        return Ok(());
    }
    let Some(task) = event.subject.as_deref() else { return Ok(()) };
    let Some(question) = event.payload["question"].as_i64().or_else(|| event.payload["id"].as_i64()) else { return Ok(()) };
    let state = app.with_tracker(project, |t| {
        let q = t.management().clarifications(task)?.into_iter().find(|q| q.id == question);
        let c = t.conversations().conversation_for_task(task)?.map(|c| t.conversations().conversation(&c)).transpose()?;
        Ok((q, c))
    });
    let (q, c) = match state {
        Ok(v) => v,
        Err(AppError::Genie(genie_core::GenieError::NotFound(_))) => return Ok(()),
        Err(e) => return Err(e),
    };
    let Some(q) = q.filter(|q| q.state == "owner") else { return Ok(()) };
    let users = if let Some(c) = &c {
        recipient(app, project, &c.principal)?.map(|u| vec![u.id]).unwrap_or_default()
    } else {
        notify::resolve(app, project, &["task.author".into(), "project.owners".into()], &json!({"task":{"id":task}}))?
    };
    notify::send(
        app,
        &users,
        &notify::Message {
            kind: "clarification".into(),
            title: format!("{task} · Нужен ваш ответ"),
            body: format!("{}\n\nОтветьте на вопрос в Genie или на это сообщение в Telegram.", q.question),
            project: Some(project.into()),
            task: Some(task.into()),
            link: Some(format!("/task/{task}")),
            interaction: Some(
                json!({"id":format!("{project}:clarification:{task}:{question}"),"task":task,"question":question,"version":q.brief_version.to_string()}),
            ),
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
    telegram_response(app, bot, chat, message, text, false, None)
}

pub fn telegram_decision(app: &App, bot: &ProjectBot, chat: i64, message: i64, text: &str) -> AppResult<bool> {
    telegram_response(app, bot, chat, message, text, true, None)
}

fn telegram_response(
    app: &App,
    bot: &ProjectBot,
    chat: i64,
    message: i64,
    text: &str,
    decision_only: bool,
    selection: Option<usize>,
) -> AppResult<bool> {
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
    respond_payload(app, &access, &payload, text, decision_only, selection)
}

pub fn telegram_option(app: &App, bot: &ProjectBot, chat: i64, message: i64, index: usize) -> AppResult<bool> {
    telegram_response(app, bot, chat, message, "", false, Some(index))
}

pub(crate) fn human_access(app: &App, project: &str, uid: i64) -> AppResult<Access> {
    human::for_user(app, project, uid)
}

/// New global-bot deliveries have the same correlation and human authority as project bots.
/// Old unscoped messages retain their answer links, but cannot identify a different bot's message.
pub(crate) fn global_telegram_response(app: &App, chat: i64, message: i64, text: &str, selection: Option<usize>) -> AppResult<bool> {
    let Some(scope) = notify::global_scope(app) else { return Ok(false) };
    let reference = format!("{scope}:{chat}:{message}");
    let found = app.with_server(|db| {
        let Some(uid) = db.user_by_channel("telegram", &chat.to_string())? else { return Ok(None) };
        let Some(payload) = db.outbox_by_ref("telegram", &reference)? else { return Ok(None) };
        if payload["recipient"] != uid || payload["telegramScope"] != scope {
            return Ok(None);
        }
        Ok(Some((uid, payload)))
    })?;
    let Some((uid, payload)) = found else { return Ok(false) };
    let project = payload["project"].as_str().ok_or_else(|| AppError::Bad("missing interaction project".into()))?;
    let access = human_access(app, project, uid)?;
    respond_payload(app, &access, &payload, text, false, selection)
}

fn respond_payload(
    app: &App,
    a: &Access,
    payload: &serde_json::Value,
    text: &str,
    decision_only: bool,
    selection: Option<usize>,
) -> AppResult<bool> {
    let i = &payload["interaction"];
    if i.is_null() {
        return Ok(false);
    }
    let id = if let Some(id) = i["id"].as_str() {
        id.to_string()
    } else if let Some(proposal) = i["proposal"].as_str() {
        format!("{}:proposal:{proposal}", a.project)
    } else if let (Some(task), Some(question)) = (i["task"].as_str(), i["question"].as_i64()) {
        format!("{}:clarification:{task}:{question}", a.project)
    } else {
        return Ok(false);
    };
    let item = show(app, a, &id)?;
    if decision_only && item.kind != "proposal" {
        return Ok(false);
    }
    let version = i["version"]
        .as_str()
        .map(str::to_string)
        .or_else(|| i["version"].as_i64().map(|v| v.to_string()))
        .ok_or_else(|| AppError::Bad("missing interaction version".into()))?;
    let (action, text) = if let Some(index) = selection {
        let choice = item.options.get(index).ok_or_else(|| AppError::Bad("invalid answer option".into()))?;
        (Action::Answer, Some(choice.clone()))
    } else if item.kind == "proposal" {
        let action = match text.trim().to_lowercase().as_str() {
            "одобряю" | "approve" | "/approve" => Action::Approve,
            "отклоняю" | "reject" | "/reject" => Action::Reject,
            _ => return Err(AppError::Bad("reply with an explicit approval or rejection".into())),
        };
        (action, None)
    } else if item.kind == "execution_pause" {
        if !matches!(text.trim().to_lowercase().as_str(), "возобновить" | "resume" | "/resume") {
            return Err(AppError::Bad("reply with an explicit resume".into()));
        }
        (Action::Resume, None)
    } else {
        (Action::Answer, Some(text.into()))
    };
    respond(app, a, &id, Response { version, action, text }, "telegram")?;
    Ok(true)
}

pub(crate) fn delivery_current(app: &App, id: i64) -> AppResult<bool> {
    let delivery = app.with_server(|db| db.interaction_delivery(id))?;
    let Some(d) = delivery else { return Ok(true) };
    let item = human_access(app, &d.project, d.recipient).and_then(|a| show(app, &a, &d.reference));
    let state = match item {
        Ok(i) if i.state == "open" && i.version == d.version => return Ok(true),
        Ok(i) => i.state,
        Err(AppError::Forbidden(_) | AppError::Genie(genie_core::GenieError::Denied(_) | genie_core::GenieError::NotFound(_))) => {
            "unavailable".into()
        }
        Err(e) => return Err(e),
    };
    app.with_server(|db| db.close_interaction_delivery(id, &state))?;
    app.wake_outbox.notify_one();
    Ok(false)
}

pub fn reconcile(app: &App, project: &str) -> AppResult<()> {
    for d in app.with_server(|db| db.open_interaction_deliveries(project))? {
        delivery_current(app, d.outbox)?;
        app.with_server(|db| db.checked_interaction_delivery(d.outbox))?;
    }
    Ok(())
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
        "{action}: {}\n\n{}\n\nПлан: {}\nАртефактов: {}\nПредложение: {}\nВерсия: {}\n\nПолные условия и результат — в задаче по ссылке. В Telegram ответьте «одобряю» или «отклоняю» на это сообщение.",
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
            interaction: Some(json!({"id":format!("{project}:proposal:{}",p.id),"proposal":p.id,"task":p.task,"version":p.version})),
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
            interaction: Some(json!({"id":format!("{project}:pause:{task}:{}",state.generation),"version":state.generation.to_string()})),
            buttons: vec![("Возобновить".into(), "interaction:resume".into())],
            ..Default::default()
        },
        Some(&format!("execution:{project}:{task}:{}", state.generation)),
    )?;
    Ok(())
}
