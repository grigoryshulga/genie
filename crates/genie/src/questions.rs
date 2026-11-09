//! Questionnaires: questions from agents to a person. Sent through the person's
//! channel (Telegram: one message per question with option buttons; e-mail and
//! web: a link to the answer page), answered anywhere, recorded in the task as
//! the owner's comment when complete (which also wakes the orchestrator).

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use genie_core::inbox::{NewQuestion, Questionnaire};
use serde_json::json;

use crate::notify::{self, Message};
use crate::state::{App, AppResult};

pub struct Ask<'a> {
    pub project: &'a str,
    pub task: Option<&'a str>,
    pub asked_by: &'a str,
    pub recipient: i64,
    pub questions: Vec<NewQuestion>,
    pub run_step: Option<i64>,
    pub remind_after: Option<ChronoDuration>,
    pub timeout: Option<ChronoDuration>,
}

/// Create a questionnaire and send it; returns its id.
pub fn ask(app: &App, a: Ask<'_>) -> AppResult<i64> {
    let channel = if app.with_server(|db| db.project_bot(a.project))?.is_none()
        && app.with_server(|db| db.channel_address(a.recipient, "telegram"))?.is_some()
        && app.cfg.telegram.is_some()
    {
        "telegram"
    } else {
        "web"
    };
    let qn = app.with_server(|db| {
        db.tx(|| {
            let user = db.user(a.recipient)?;
            if user.disabled || !db.project_role(a.project, &user)?.is_some_and(|r| r.can_write()) {
                return Err(genie_core::GenieError::Denied("questions require a current project member as recipient".into()));
            }
            let (qn, _) = db.create_questionnaire(
                a.project,
                a.task,
                a.asked_by,
                a.recipient,
                channel,
                &a.questions,
                a.run_step,
                a.remind_after,
                a.timeout,
            )?;
            db.queue_questionnaire_send(qn.id, false)?;
            Ok(qn)
        })
    })?;
    flush_sends(app)?;
    // Its reminder and deadline are new deadlines for the engine to sleep until.
    app.wake_engine.notify_one();
    Ok(qn.id)
}

fn answer_link(app: &App, secret: &str) -> String {
    format!("{}/answer?token={secret}", app.cfg.public_url())
}

/// The send intent survives a crash before projection into the notification/outbox tables.
/// Replaying a partially projected questionnaire uses the same per-item delivery keys.
pub fn flush_sends(app: &App) -> AppResult<()> {
    for (id, reminder) in app.with_server(|db| db.due_questionnaire_sends())? {
        let qn = app.with_server(|db| db.questionnaire(id))?;
        let result = if qn.status == "open" && qn.due.as_ref().is_none_or(|d| *d > genie_core::db::now()) {
            let secret = app.with_server(|db| db.questionnaire_link(id))?;
            send(app, &qn, &secret, reminder)
        } else {
            Ok(())
        };
        app.with_server(|db| db.questionnaire_sent(id, reminder, result.as_ref().err().map(|e| e.to_string()).as_deref()))?;
    }
    Ok(())
}

/// Deliver (or remind about) a questionnaire.
pub fn send(app: &App, qn: &Questionnaire, secret: &str, reminder: bool) -> AppResult<()> {
    let link = answer_link(app, secret);
    for q in qn.questions.iter().filter(|q| q.answer.is_none()) {
        let title = format!(
            "{}{} · Вопрос {} от {}",
            if reminder { "Напоминание: " } else { "" },
            qn.task.as_deref().unwrap_or(&qn.project),
            q.n,
            qn.asked_by
        );
        let id = format!("{}:questionnaire:{}:{}", qn.project, qn.id, q.n);
        let buttons = q.options.iter().enumerate().map(|(i, o)| (o.clone(), format!("q:{}:{}:{i}", qn.id, q.n))).collect();
        notify::send(
            app,
            &[qn.recipient],
            &Message {
                kind: "question".into(),
                title,
                body: format!(
                    "{}{}\n\nОтветить в Genie: {link}\nВ Telegram можно ответить на это сообщение или выбрать вариант.",
                    q.text,
                    if q.why.is_empty() { String::new() } else { format!("\n\nЗачем: {}", q.why) }
                ),
                project: Some(qn.project.clone()),
                task: qn.task.clone(),
                link: Some(format!("/answer?token={secret}")),
                buttons,
                interaction: Some(json!({"id":id,"version":format!("{}:{}",qn.created,q.n)})),
                ..Default::default()
            },
            Some(&format!("qn:{}:{}:q{}", qn.id, if reminder { "remind" } else { "ask" }, q.n)),
        )?;
    }
    Ok(())
}

/// Check the current recipient account and project rights, including for a scoped answer link.
pub fn authorize(db: &genie_core::server_db::ServerDb, qn: &Questionnaire, recipient: i64) -> genie_core::Result<()> {
    let user = db.user(recipient)?;
    if qn.recipient != recipient || user.disabled || !db.project_role(&qn.project, &user)?.is_some_and(|r| r.can_write()) {
        return Err(genie_core::GenieError::Denied("this person cannot answer this questionnaire".into()));
    }
    Ok(())
}

pub(crate) fn authorization_error(error: crate::state::AppError) -> crate::state::AppError {
    match error {
        crate::state::AppError::Genie(genie_core::GenieError::Denied(message)) => crate::state::AppError::Forbidden(message),
        other => other,
    }
}

pub fn answer(app: &App, recipient: i64, id: i64, n: i64, text: &str, via: &str) -> AppResult<(Questionnaire, bool)> {
    answer_many(app, recipient, id, &[(n, text.to_string())], via)
}

/// Accept one whole submission and its durable continuation together. Transports never write
/// answers themselves and cannot bypass a revoked account with an old message or link.
pub fn answer_many(app: &App, recipient: i64, id: i64, answers: &[(i64, String)], via: &str) -> AppResult<(Questionnaire, bool)> {
    let qn = app
        .with_server(|db| {
            db.tx(|| {
                let qn = db.questionnaire(id)?;
                authorize(db, &qn, recipient)?;
                let (qn, completed) = db.answer_questions(id, answers, via)?;
                if completed && qn.task.is_some() {
                    db.queue_answer_record(qn.id, &db.user(recipient)?.login, &record_text(&qn))?;
                }
                Ok(qn)
            })
        })
        .map_err(authorization_error)?;
    // A failed task-side write leaves a pending receipt; accepting the answer is already durable.
    flush_records(app)?;
    app.wake_engine.notify_one();
    let done = qn.status == "answered";
    Ok((qn, done))
}

fn record_text(qn: &Questionnaire) -> String {
    format!(
        "Ответы на вопросы ({}):\n\n{}",
        qn.asked_by,
        qn.questions
            .iter()
            .map(|q| format!("{}. {}\n   → {}", q.n, q.text, q.answer.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// The server DB and a project tracker commit separately. The tracker receipt makes replay
/// after a crash between those commits harmless, including orchestrator mail.
pub fn flush_records(app: &App) -> AppResult<()> {
    for record in app.with_server(|db| db.due_answer_records())? {
        let qn = app.with_server(|db| db.questionnaire(record.questionnaire))?;
        let Some(task) = qn.task else { continue };
        match crate::tasks::record_answers(app, &qn.project, &record.actor, &task, &record.id, &record.body) {
            Ok(()) => app.with_server(|db| db.answer_recorded(&record.id))?,
            Err(e) => app.with_server(|db| db.answer_record_failed(&record.id, &e.to_string()))?,
        }
    }
    Ok(())
}

/// Reminders and deadlines of open questionnaires (called by the engine's pass). Returns when the
/// next reminder or deadline falls: the engine sleeps until then.
pub fn tick(app: &App) -> AppResult<Option<DateTime<Utc>>> {
    flush_records(app)?;
    let now = genie_core::db::now();
    let mut next: Option<DateTime<Utc>> = None;
    let mut soonest = |at: &str| {
        if let Ok(t) = DateTime::parse_from_rfc3339(at) {
            let t = t.with_timezone(&Utc);
            next = Some(next.map_or(t, |n| n.min(t)));
        }
    };
    if let Some(at) = app.with_server(|db| db.next_answer_record_at())? {
        soonest(&at);
    }
    for qn in app.with_server(|db| db.open_questionnaires())? {
        if qn.due.as_ref().is_some_and(|d| *d <= now) {
            app.with_server(|db| db.close_questionnaire(qn.id, "expired"))?;
            app.wake_engine.notify_one();
            continue;
        }
        if let Some(due) = &qn.due {
            soonest(due);
        }
        match &qn.remind_at {
            Some(r) if *r <= now => {
                app.with_server(|db| {
                    db.tx(|| {
                        db.clear_reminder(qn.id)?;
                        db.queue_questionnaire_send(qn.id, true)
                    })
                })?;
            }
            Some(r) => soonest(r),
            None => {}
        }
    }
    flush_sends(app)?;
    if let Some(at) = app.with_server(|db| db.next_questionnaire_send_at())? {
        soonest(&at);
    }
    Ok(next)
}
