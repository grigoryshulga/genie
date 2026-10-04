//! Questionnaires: questions from agents to a person. Sent through the person's
//! channel (Telegram: one message per question with option buttons; e-mail and
//! web: a link to the answer page), answered anywhere, recorded in the task as
//! the owner's comment when complete (which also wakes the orchestrator).

use chrono::Duration as ChronoDuration;
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
    let channel = if app.with_server(|db| db.channel_address(a.recipient, "telegram"))?.is_some() && app.cfg.telegram.is_some() {
        "telegram"
    } else {
        "web"
    };
    let (qn, secret) = app.with_server(|db| {
        db.create_questionnaire(a.project, a.task, a.asked_by, a.recipient, channel, &a.questions, a.run_step, a.remind_after, a.timeout)
    })?;
    send(app, &qn, &secret, false)?;
    Ok(qn.id)
}

fn answer_link(app: &App, secret: &str) -> String {
    format!("{}/answer?token={secret}", app.cfg.public_url())
}

/// Deliver (or remind about) a questionnaire.
pub fn send(app: &App, qn: &Questionnaire, secret: &str, reminder: bool) -> AppResult<()> {
    let task = qn.task.clone().unwrap_or_default();
    let title = format!(
        "{}{} · {} вопрос(а) от {}",
        if reminder { "Напоминание: " } else { "" },
        if task.is_empty() { qn.project.clone() } else { task.clone() },
        qn.questions.iter().filter(|q| q.answer.is_none()).count(),
        qn.asked_by
    );
    let link = answer_link(app, secret);
    let listing = qn
        .questions
        .iter()
        .filter(|q| q.answer.is_none())
        .map(|q| {
            format!("{}. {}{}", q.n, q.text, if q.options.is_empty() { String::new() } else { format!(" ({})", q.options.join(" / ")) })
        })
        .collect::<Vec<_>>()
        .join("\n");
    // Web center (+ e-mail when the person has no Telegram).
    let web = Message {
        kind: "question".into(),
        title: title.clone(),
        body: format!("{listing}\n\nОтветить: {link}"),
        project: Some(qn.project.clone()),
        task: qn.task.clone(),
        link: Some(format!("/answer?token={secret}")),
        channels: if qn.channel == "telegram" { vec!["web".into()] } else { vec![] },
        ..Default::default()
    };
    let key = format!("qn:{}:{}", qn.id, if reminder { "remind" } else { "ask" });
    notify::send(app, &[qn.recipient], &web, Some(&key))?;
    if qn.channel == "telegram" {
        let chat = app.with_server(|db| db.channel_address(qn.recipient, "telegram"))?.unwrap_or_default();
        app.with_server(|db| {
            db.enqueue(
                Some(&format!("{key}:head")),
                Some(qn.recipient),
                "telegram",
                &chat,
                &title,
                &format!("{title}\n\nОтветьте на каждое сообщение (reply) или нажмите вариант. Все вопросы сразу: {link}"),
                &json!({}),
            )?;
            for q in qn.questions.iter().filter(|q| q.answer.is_none()) {
                let buttons: Vec<(String, String)> =
                    q.options.iter().enumerate().map(|(i, o)| (o.clone(), format!("q:{}:{}:{i}", qn.id, q.n))).collect();
                let text = format!("{}. {}{}", q.n, q.text, if q.why.is_empty() { String::new() } else { format!("\n\nЗачем: {}", q.why) });
                db.enqueue(
                    Some(&format!("{key}:q{}", q.n)),
                    Some(qn.recipient),
                    "telegram",
                    &chat,
                    &title,
                    &text,
                    &json!({ "buttons": buttons, "question": { "questionnaire": qn.id, "n": q.n }, "forceReply": q.options.is_empty() }),
                )?;
            }
            Ok(())
        })?;
        app.wake_outbox.notify_one();
    }
    Ok(())
}

/// Record one answer; when the questionnaire is complete, write the answers into the task.
pub fn answer(app: &App, id: i64, n: i64, text: &str, via: &str) -> AppResult<(Questionnaire, bool)> {
    let (qn, done) = app.with_server(|db| db.answer_question(id, n, text, via))?;
    if done {
        record(app, &qn)?;
    }
    app.wake_engine.notify_one();
    Ok((qn, done))
}

/// Answers → one owner comment on the task (wakes the orchestrator).
fn record(app: &App, qn: &Questionnaire) -> AppResult<()> {
    let Some(task) = &qn.task else { return Ok(()) };
    let user = app.with_server(|db| db.user(qn.recipient))?;
    let text = format!(
        "Ответы на вопросы ({}):\n\n{}",
        qn.asked_by,
        qn.questions
            .iter()
            .map(|q| format!("{}. {}\n   → {}", q.n, q.text, q.answer.clone().unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("\n")
    );
    crate::tasks::comment(
        app,
        &crate::tasks::Caller::person(&qn.project, &user.login),
        task,
        crate::tasks::CommentBody { text, kind: None },
    )?;
    Ok(())
}

/// Reminders and deadlines of open questionnaires (called by the engine tick).
pub fn tick(app: &App) -> AppResult<()> {
    let now = genie_core::db::now();
    for qn in app.with_server(|db| db.open_questionnaires())? {
        if qn.due.as_ref().is_some_and(|d| *d <= now) {
            app.with_server(|db| db.close_questionnaire(qn.id, "expired"))?;
            app.wake_engine.notify_one();
            continue;
        }
        if qn.remind_at.as_ref().is_some_and(|r| *r <= now) {
            app.with_server(|db| db.clear_reminder(qn.id))?;
            // The secret is not stored; a reminder carries a fresh answer link.
            let secret = genie_core::server_db::new_secret();
            app.with_server(|db| {
                db.conn().execute(
                    "UPDATE questionnaires SET token_hash = ?1 WHERE id = ?2",
                    rusqlite::params![genie_core::server_db::hash_secret(&secret), qn.id],
                )?;
                Ok(())
            })?;
            send(app, &qn, &secret, true)?;
        }
    }
    Ok(())
}
