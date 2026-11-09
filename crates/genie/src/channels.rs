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

use crate::state::{App, SAFETY_NET, sleep_until};

pub fn start(app: &Arc<App>) {
    crate::telegram::start(app.clone());
    let a = app.clone();
    tokio::spawn(async move {
        loop {
            dispatch(&a).await;
            // New rows wake the dispatcher; a failed one waits for its backoff, which ends by itself.
            let due = a.blocking(|app| app.with_server(|db| db.next_outbox_at())).await.ok().flatten();
            let due = due.and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok()).map(|t| t.to_utc());
            tokio::select! {
                _ = a.wake_outbox.notified() => {}
                _ = tokio::time::sleep(sleep_until(due, SAFETY_NET).max(Duration::from_millis(100))) => {}
            }
        }
    });
    if app.cfg.telegram.is_some() {
        let a = app.clone();
        tokio::spawn(async move { telegram_poll(a).await });
    }
}

async fn tg(app: &App, method: &str, body: Value) -> Result<Value, String> {
    let config = app.cfg.telegram.as_ref().ok_or("telegram is not configured")?;
    crate::telegram::call(app, &config.token, method, body).await
}

/// The bot's username (for instructions), if Telegram is configured and reachable.
pub async fn telegram_username(app: &App) -> Option<String> {
    tg(app, "getMe", json!({})).await.ok()?["username"].as_str().map(str::to_string)
}

/// Deliver due outbox messages; failures are retried with backoff by the outbox.
pub async fn dispatch(app: &Arc<App>) {
    let Ok(items) = app.blocking(|app| app.with_server(|db| db.due_outbox(50))).await else { return };
    for item in items {
        let id = item.id;
        if !app.blocking(move |app| app.with_server(|db| db.claim_outbox(id))).await.unwrap_or(false) {
            continue;
        }
        let copy = item.clone();
        let ready = if item.channel == "interaction_sync" {
            true
        } else {
            app.blocking(move |app| {
                if !crate::interactions::delivery_current(app, copy.id)? {
                    app.with_server(|db| db.outbox_cancelled(copy.id, "interaction is closed"))?;
                    return Ok(false);
                }
                if copy.channel != "web" && !crate::notify::valid_target(app, &copy)? {
                    if !crate::notify::fallback(app, &copy)? {
                        app.with_server(|db| db.outbox_cancelled(copy.id, "recipient, channel or project rights changed"))?;
                    }
                    return Ok(false);
                }
                Ok(true)
            })
            .await
            .unwrap_or(false)
        };
        if !ready {
            continue;
        }
        let result = match item.channel.as_str() {
            "telegram" => send_telegram(app, &item).await,
            "email" => send_email(app, &item).await,
            "web" => Ok(None),
            "interaction_sync" => sync_interaction(app, &item).await,
            other => Err(format!("unknown channel {other}")),
        };
        let _ = app
            .blocking(move |app| {
                app.with_server(|db| {
                    db.tx(|| {
                        match &result {
                            Ok(reference) => {
                                db.outbox_sent(id, reference.as_deref())?;
                                let question = &item.payload["question"];
                                if let (Some(qn), Some(n), Some(r)) =
                                    (question["questionnaire"].as_i64(), question["n"].as_i64(), reference)
                                {
                                    db.set_question_message(qn, n, r)?;
                                }
                            }
                            Err(error) => {
                                db.outbox_failed(id, error)?;
                                if item.channel != "interaction_sync" {
                                    let current = db.outbox_item(id)?;
                                    let after = item.payload["delivery"]["afterAttempts"]
                                        .as_i64()
                                        .unwrap_or(genie_core::inbox::MAX_DELIVERY_ATTEMPTS);
                                    if current.attempts >= after
                                        && !crate::notify::fallback_in(db, app, &current)?
                                        && current.status == "failed"
                                    {
                                        crate::notify::delivery_failed_in(db, &current)?;
                                    }
                                }
                            }
                        }
                        Ok(())
                    })
                })?;
                if result.is_ok() && item.channel != "interaction_sync" {
                    crate::interactions::delivery_current(app, id)?;
                }
                Ok(())
            })
            .await;
    }
}

async fn sync_interaction(app: &Arc<App>, item: &OutboxItem) -> Result<Option<String>, String> {
    let id = item.payload["original"].as_i64().ok_or("missing original delivery")?;
    let original = app.blocking(move |app| app.with_server(|db| db.outbox_item(id))).await.map_err(|_| "cannot read original delivery")?;
    let Some(reference) = original.external_ref.as_deref() else {
        return if original.status == "delivering" { Err("original send is still in flight".into()) } else { Ok(None) };
    };
    let mut parts = reference.rsplitn(3, ':');
    let message = parts.next().and_then(|v| v.parse::<i64>().ok()).ok_or("invalid message reference")?;
    let chat = parts.next().ok_or("missing chat reference")?;
    let scope = parts.next();
    if chat != original.address || scope != original.payload["telegramScope"].as_str() {
        return Ok(None);
    }
    let body = json!({"chat_id":chat,"message_id":message,"text":format!("Обращение завершено или больше недоступно ({}). Текущее состояние — в Genie.",item.payload["state"].as_str().unwrap_or("closed")),"reply_markup":{"inline_keyboard":[]}});
    let result = if scope.is_some() && scope == crate::notify::global_scope(app).as_deref() {
        tg(app, "editMessageText", body).await
    } else if let Some(scope) = scope {
        let scope = scope.to_string();
        let bot = app
            .blocking(move |app| app.with_server(|db| Ok(db.project_bots()?.into_iter().find(|b| b.scope == scope))))
            .await
            .map_err(|_| "cannot read project bot")?;
        let Some(bot) = bot else { return Ok(None) };
        crate::telegram::call(app, &bot.token.0, "editMessageText", body).await
    } else {
        return Ok(None);
    };
    match result {
        Ok(_) => Ok(None),
        Err(e) if e.to_lowercase().contains("message is not modified") => Ok(None),
        Err(e) => Err(e),
    }
}

async fn send_telegram(app: &Arc<App>, item: &OutboxItem) -> Result<Option<String>, String> {
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
    let sent = if item.payload["telegramScope"].as_str() == crate::notify::global_scope(app).as_deref() {
        tg(app, "sendMessage", body).await?
    } else if let Some(scope) = item.payload["telegramScope"].as_str() {
        let scope = scope.to_string();
        let user = item.user;
        let chat = item.address.clone();
        let bot = app
            .blocking(move |app| {
                app.with_server(|db| {
                    let bot = db.project_bots()?.into_iter().find(|b| b.scope == scope);
                    if let (Some(bot), Some(user)) = (&bot, user) {
                        let u = db.user(user)?;
                        if u.disabled
                            || db.project_role(&bot.project, &u)?.is_none()
                            || db.channel_address(user, &scope)?.as_deref() != Some(&chat)
                        {
                            return Ok(None);
                        }
                    } else {
                        return Ok(None);
                    }
                    Ok(bot)
                })
            })
            .await
            .map_err(|_| "Cannot read Telegram configuration")?;
        let Some(bot) = bot else { return Err("recipient no longer authorized for this bot".into()) };
        crate::telegram::call(app, &bot.token.0, "sendMessage", body).await?
    } else {
        tg(app, "sendMessage", body).await?
    };
    Ok(sent["message_id"].as_i64().map(|m| match item.payload["telegramScope"].as_str() {
        Some(scope) => format!("{scope}:{}:{m}", item.address),
        None => format!("{}:{m}", item.address),
    }))
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
            if let Err(e) = handle_update(&app, &u).await {
                eprintln!("genie telegram: {e}");
                break;
            }
            let off = offset.max(u["update_id"].as_i64().unwrap_or(0).saturating_add(1));
            let saved = app
                .blocking(move |app| {
                    app.with_server(|db| {
                        db.conn().execute(
                            "INSERT INTO meta(key,value) VALUES ('telegram_offset',?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                            [off.to_string()],
                        )?;
                        Ok(())
                    })
                })
                .await;
            if saved.is_err() {
                break;
            }
            offset = off;
        }
    }
}

async fn reply(app: &App, chat: &str, text: &str) {
    let _ = tg(app, "sendMessage", json!({ "chat_id": chat, "text": text })).await;
}

/// Handle one Telegram update (public for tests).
pub async fn handle_update(app: &Arc<App>, u: &Value) -> Result<(), String> {
    if let Some(cb) = u.get("callback_query") {
        let Some(chat) = cb["message"]["chat"]["id"].as_i64() else { return Ok(()) };
        let Some(message) = cb["message"]["message_id"].as_i64() else { return Ok(()) };
        if cb["message"]["chat"]["type"] != "private" || cb["from"]["id"].as_i64() != Some(chat) {
            return Ok(());
        }
        let data = cb["data"].as_str().unwrap_or_default();
        let parts = data.split(':').collect::<Vec<_>>();
        let (text, selection) = match parts.as_slice() {
            ["interaction", "approve"] => ("approve", None),
            ["interaction", "reject"] => ("reject", None),
            ["interaction", "resume"] => ("resume", None),
            ["q", _, _, index] => match index.parse::<usize>() {
                Ok(i) => ("", Some(i)),
                Err(_) => return Ok(()),
            },
            _ => return Ok(()),
        };
        let result = app.blocking(move |app| crate::interactions::global_telegram_response(app, chat, message, text, selection)).await;
        let notice = crate::telegram::interaction_result(result)?;
        let _ = tg(app, "answerCallbackQuery", json!({"callback_query_id":cb["id"],"text":notice})).await;
        return Ok(());
    }
    let msg = &u["message"];
    if msg["chat"]["type"] != "private" {
        return Ok(());
    }
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
        let chat_id = chat.parse::<i64>().map_err(|_| "invalid chat")?;
        let answer = text.clone();
        let result = app.blocking(move |app| crate::interactions::global_telegram_response(app, chat_id, reply_to, &answer, None)).await;
        let notice = crate::telegram::interaction_result(result)?;
        reply(app, &chat, notice).await;
        return Ok(());
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
