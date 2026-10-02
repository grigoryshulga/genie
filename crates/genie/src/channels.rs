//! Delivery channels: the outbox dispatcher (Telegram, e-mail) with retries,
//! and the Telegram bot (long polling — works without a public address).
//!
//! Telegram: `/start CODE` links a chat to a person (code from their profile),
//! `/new <text>` files a task into their project's inbox, option buttons and
//! replies to question messages answer questionnaires.

use std::sync::Arc;
use std::time::Duration;

use genie_core::inbox::OutboxItem;
use genie_core::{Actor, CreateInput, Role, Status};
use serde_json::{Value, json};

use crate::state::App;

pub fn start(app: &Arc<App>) {
    let a = app.clone();
    tokio::spawn(async move {
        loop {
            dispatch(&a).await;
            tokio::select! {
                _ = a.wake_outbox.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(5)) => {}
            }
        }
    });
    if app.cfg.telegram.is_some() {
        let a = app.clone();
        tokio::spawn(async move { telegram_poll(a).await });
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(60)).build().unwrap_or_default()
}

fn tg_url(app: &App, method: &str) -> Option<String> {
    let t = app.cfg.telegram.as_ref()?;
    let base = t.api_base.clone().unwrap_or_else(|| "https://api.telegram.org".into());
    Some(format!("{}/bot{}/{method}", base.trim_end_matches('/'), t.token))
}

async fn tg(app: &App, method: &str, body: Value) -> Result<Value, String> {
    let url = tg_url(app, method).ok_or("telegram is not configured")?;
    let res = http().post(url).json(&body).send().await.map_err(|e| format!("telegram: {e}"))?;
    let v: Value = res.json().await.map_err(|e| format!("telegram: {e}"))?;
    if v["ok"] != json!(true) {
        return Err(format!("telegram: {}", v["description"].as_str().unwrap_or("error")));
    }
    Ok(v["result"].clone())
}

/// The bot's username (for instructions), if Telegram is configured and reachable.
pub async fn telegram_username(app: &App) -> Option<String> {
    tg(app, "getMe", json!({})).await.ok()?["username"].as_str().map(str::to_string)
}

/// Deliver due outbox messages; failures are retried with backoff by the outbox.
pub async fn dispatch(app: &Arc<App>) {
    let Ok(items) = app.blocking(|app| app.with_server(|db| db.due_outbox(50))).await else { return };
    for item in items {
        let result = match item.channel.as_str() {
            "telegram" => send_telegram(app, &item).await,
            "email" => send_email(app, &item).await,
            other => Err(format!("unknown channel {other}")),
        };
        let id = item.id;
        let question = item.payload["question"].clone();
        let _ = app
            .blocking(move |app| {
                app.with_server(|db| match &result {
                    Ok(reference) => {
                        db.outbox_sent(id, reference.as_deref())?;
                        if let (Some(qn), Some(n), Some(r)) = (question["questionnaire"].as_i64(), question["n"].as_i64(), reference) {
                            db.set_question_message(qn, n, r)?;
                        }
                        Ok(())
                    }
                    Err(e) => db.outbox_failed(id, e),
                })
            })
            .await;
    }
}

async fn send_telegram(app: &App, item: &OutboxItem) -> Result<Option<String>, String> {
    let mut body =
        json!({ "chat_id": item.address, "text": item.body.chars().take(4000).collect::<String>(), "disable_web_page_preview": true });
    let buttons: Vec<(String, String)> = serde_json::from_value(item.payload["buttons"].clone()).unwrap_or_default();
    if !buttons.is_empty() {
        let rows: Vec<Vec<Value>> =
            buttons.chunks(2).map(|c| c.iter().map(|(label, data)| json!({ "text": label, "callback_data": data })).collect()).collect();
        body["reply_markup"] = json!({ "inline_keyboard": rows });
    } else if item.payload["forceReply"] == json!(true) {
        body["reply_markup"] = json!({ "force_reply": true, "input_field_placeholder": "Ваш ответ" });
    }
    let sent = tg(app, "sendMessage", body).await?;
    Ok(sent["message_id"].as_i64().map(|m| format!("{}:{m}", item.address)))
}

async fn send_email(app: &App, item: &OutboxItem) -> Result<Option<String>, String> {
    use lettre::message::Mailbox;
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
    let smtp = app.cfg.smtp.as_ref().ok_or("e-mail is not configured")?;
    let from: Mailbox = smtp.from.parse().map_err(|e| format!("smtp.from: {e}"))?;
    let to: Mailbox = item.address.parse().map_err(|e| format!("recipient {}: {e}", item.address))?;
    let msg = Message::builder()
        .from(from)
        .to(to)
        .subject(format!("[genie] {}", item.subject))
        .body(item.body.clone())
        .map_err(|e| e.to_string())?;
    let builder = match smtp.security.as_str() {
        "none" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&smtp.host),
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(&smtp.host).map_err(|e| e.to_string())?,
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp.host).map_err(|e| e.to_string())?,
    };
    let mut builder = builder.port(smtp.port).timeout(Some(Duration::from_secs(30)));
    if let (Some(u), Some(p)) = (&smtp.username, &smtp.password) {
        builder = builder.credentials(Credentials::new(u.clone(), p.clone()));
    }
    let res = builder.build().send(msg).await.map_err(|e| format!("smtp: {e}"))?;
    Ok(res.message().next().map(str::to_string))
}

async fn telegram_poll(app: Arc<App>) {
    let mut offset: i64 = app
        .blocking(|app| {
            app.with_server(|db| {
                Ok(db.conn().query_row("SELECT value FROM meta WHERE key = 'telegram_offset'", [], |r| r.get::<_, String>(0)).ok())
            })
        })
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    loop {
        let updates =
            tg(&app, "getUpdates", json!({ "offset": offset, "timeout": 25, "allowed_updates": ["message", "callback_query"] })).await;
        let updates = match updates {
            Ok(Value::Array(u)) => u,
            Ok(_) => Vec::new(),
            Err(e) => {
                eprintln!("genie telegram: {e}");
                tokio::time::sleep(Duration::from_secs(10)).await;
                continue;
            }
        };
        for u in updates {
            offset = offset.max(u["update_id"].as_i64().unwrap_or(0) + 1);
            if let Err(e) = handle_update(&app, &u).await {
                eprintln!("genie telegram: {e}");
            }
        }
        let off = offset;
        let _ = app
            .blocking(move |app| {
                app.with_server(|db| {
                    db.conn().execute(
                        "INSERT INTO meta(key, value) VALUES ('telegram_offset', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        [off.to_string()],
                    )?;
                    Ok(())
                })
            })
            .await;
    }
}

async fn reply(app: &App, chat: &str, text: &str) {
    let _ = tg(app, "sendMessage", json!({ "chat_id": chat, "text": text })).await;
}

/// Handle one Telegram update (public for tests).
pub async fn handle_update(app: &Arc<App>, u: &Value) -> Result<(), String> {
    if let Some(cb) = u.get("callback_query") {
        let chat = cb["message"]["chat"]["id"].as_i64().map(|c| c.to_string()).unwrap_or_default();
        let data = cb["data"].as_str().unwrap_or_default().to_string();
        let _ = tg(app, "answerCallbackQuery", json!({ "callback_query_id": cb["id"], "text": "Принято" })).await;
        let parts: Vec<&str> = data.split(':').collect();
        if let ["q", qn, n, opt] = parts.as_slice() {
            let (qn, n, opt): (i64, i64, usize) =
                (qn.parse().map_err(|_| "bad data")?, n.parse().map_err(|_| "bad data")?, opt.parse().map_err(|_| "bad data")?);
            let chat2 = chat.clone();
            let res = app
                .blocking(move |app| {
                    let q = app.with_server(|db| db.questionnaire(qn))?;
                    let owner = app.with_server(|db| db.user_by_channel("telegram", &chat2))?;
                    if owner != Some(q.recipient) {
                        return Ok(None);
                    }
                    let label = q.questions.iter().find(|x| x.n == n).and_then(|x| x.options.get(opt).cloned()).unwrap_or_default();
                    crate::questions::answer(app, qn, n, &label, "telegram").map(|(_, done)| Some((label, done)))
                })
                .await
                .map_err(|e| e.to_string())?;
            if let Some((label, done)) = res {
                reply(app, &chat, &format!("Ответ записан: {label}{}", if done { "\nВсе вопросы отвечены — спасибо!" } else { "" })).await;
            }
        }
        return Ok(());
    }
    let msg = &u["message"];
    let Some(chat) = msg["chat"]["id"].as_i64().map(|c| c.to_string()) else { return Ok(()) };
    let text = msg["text"].as_str().unwrap_or_default().trim().to_string();
    if let Some(code) = text.strip_prefix("/start") {
        let code = code.trim().to_string();
        let chat2 = chat.clone();
        let linked = app
            .blocking(move |app| app.with_server(|db| db.redeem_link_code(&code, "telegram", &chat2)))
            .await
            .map_err(|e| e.to_string())?;
        let answer = match linked {
            Some(_) => {
                "Готово: этот чат привязан к genie. Сюда будут приходить уведомления и вопросы. /new <текст> — новая задача во входящие."
            }
            None => "Чтобы привязать чат, откройте профиль в genie, получите код и отправьте: /start КОД",
        };
        reply(app, &chat, answer).await;
        return Ok(());
    }
    let chat2 = chat.clone();
    let user = app.blocking(move |app| app.with_server(|db| db.user_by_channel("telegram", &chat2))).await.map_err(|e| e.to_string())?;
    let Some(user) = user else {
        reply(app, &chat, "Этот чат не привязан к genie: получите код в профиле и отправьте /start КОД").await;
        return Ok(());
    };
    if let Some(reply_to) = msg["reply_to_message"]["message_id"].as_i64() {
        let key = format!("{chat}:{reply_to}");
        let text2 = text.clone();
        let res = app
            .blocking(move |app| {
                let Some((qn, n)) = app.with_server(|db| db.question_by_message(&key))? else { return Ok(None) };
                let q = app.with_server(|db| db.questionnaire(qn))?;
                if q.recipient != user || q.status != "open" {
                    return Ok(None);
                }
                crate::questions::answer(app, qn, n, &text2, "telegram").map(|(_, done)| Some(done))
            })
            .await
            .map_err(|e| e.to_string())?;
        if let Some(done) = res {
            reply(app, &chat, if done { "Все вопросы отвечены — спасибо!" } else { "Ответ записан." }).await;
            return Ok(());
        }
    }
    if let Some(title) = text.strip_prefix("/new") {
        let title = title.trim().to_string();
        if title.is_empty() {
            reply(app, &chat, "Напишите: /new что нужно сделать").await;
            return Ok(());
        }
        let created = app
            .blocking(move |app| {
                let u = app.with_server(|db| db.user(user))?;
                let project = app
                    .with_server(|db| db.projects())?
                    .into_iter()
                    .find(|p| app.with_server(|db| Ok(db.project_role(&p.slug, &u)?.is_some_and(|r| r.can_write()))).unwrap_or(false));
                let Some(p) = project else { return Ok(None) };
                let (first, rest) = title.split_once('\n').unwrap_or((&title, ""));
                let t = app.with_tracker(&p.slug, |t| {
                    t.create(
                        &Actor::new(u.login.clone(), Role::Human),
                        CreateInput {
                            title: first.to_string(),
                            description: Some(rest.trim().to_string()).filter(|d| !d.is_empty()),
                            status: Some(Status::Inbox),
                            ..Default::default()
                        },
                    )
                })?;
                app.wake_engine.notify_one();
                app.wake_runtime.notify_one();
                Ok(Some((p.name, t.id)))
            })
            .await
            .map_err(|e| e.to_string())?;
        match created {
            Some((project, id)) => reply(app, &chat, &format!("Задача {id} во входящих проекта «{project}».")).await,
            None => reply(app, &chat, "У вас нет проекта с правом записи.").await,
        }
        return Ok(());
    }
    reply(app, &chat, "Ответьте (reply) на сообщение с вопросом, чтобы ответить на него. /new <текст> — новая задача.").await;
    Ok(())
}
