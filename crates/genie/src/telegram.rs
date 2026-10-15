//! Project bot transport. Workers reconcile persisted configuration without a restart.
use crate::state::App;
use genie_core::telegram::ProjectBot;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc, time::Duration};

/// Never include a reqwest error or provider description: either may expose the token URL.
pub async fn call(app: &App, token: &str, method: &str, body: Value) -> Result<Value, String> {
    let base = app.cfg.telegram.as_ref().and_then(|t| t.api_base.as_deref()).unwrap_or("https://api.telegram.org");
    let url = format!("{}/bot{token}/{method}", base.trim_end_matches('/'));
    let res = reqwest::Client::new()
        .post(url)
        .timeout(Duration::from_secs(35))
        .json(&body)
        .send()
        .await
        .map_err(|_| "Telegram недоступен. Повторите попытку.".to_string())?;
    let value: Value = res.json().await.map_err(|_| "Некорректный ответ Telegram".to_string())?;
    if value["ok"] != true {
        return Err("Telegram отклонил запрос. Проверьте токен и доступность бота.".into());
    }
    Ok(value["result"].clone())
}

pub fn start(app: Arc<App>) {
    tokio::spawn(async move {
        let mut workers: HashMap<String, (String, tokio::task::JoinHandle<()>)> = HashMap::new();
        loop {
            if let Ok(bots) = app.blocking(|app| app.with_server(|db| db.project_bots())).await {
                workers.retain(|scope, (token, worker)| {
                    let keep =
                        bots.iter().any(|b| &b.scope == scope && &b.token.0 == token && !b.token.0.is_empty()) && !worker.is_finished();
                    if !keep {
                        worker.abort();
                    }
                    keep
                });
                for bot in bots.into_iter().filter(|b| !b.token.0.is_empty()) {
                    workers.entry(bot.scope.clone()).or_insert_with(|| {
                        let token = bot.token.0.clone();
                        let a = app.clone();
                        (token, tokio::spawn(async move { poll(a, bot).await }))
                    });
                }
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
}

async fn poll(app: Arc<App>, mut bot: ProjectBot) {
    loop {
        let updates =
            call(&app, &bot.token.0, "getUpdates", json!({"offset": bot.offset, "timeout": 25, "allowed_updates": ["message"]})).await;
        let Ok(Value::Array(updates)) = updates else {
            tokio::time::sleep(Duration::from_secs(10)).await;
            continue;
        };
        for update in updates {
            if handle_update(&app, &bot, &update).await.is_err() {
                break;
            }
            bot.offset = bot.offset.max(update["update_id"].as_i64().unwrap_or(0) + 1);
            let (scope, offset) = (bot.scope.clone(), bot.offset);
            let _ = app
                .blocking(move |app| {
                    app.with_server(|db| {
                        db.conn().execute("UPDATE project_telegram SET offset = ?1 WHERE scope = ?2", rusqlite::params![offset, scope])?;
                        Ok(())
                    })
                })
                .await;
        }
    }
}

/// Linking is allowed only in private chats and only against the currently configured bot.
pub async fn handle_update(app: &Arc<App>, bot: &ProjectBot, update: &Value) -> Result<(), String> {
    let msg = &update["message"];
    if msg["chat"]["type"] != "private" {
        return Ok(());
    }
    let Some(chat) = msg["chat"]["id"].as_i64() else { return Ok(()) };
    let text = msg["text"].as_str().unwrap_or_default();
    let mut parts = text.split_whitespace();
    let command = parts.next().unwrap_or_default();
    let reply = if command == "/start" || command == format!("/start@{}", bot.username) {
        let code = parts.next().unwrap_or_default().to_string();
        let scope = bot.scope.clone();
        let linked = app
            .blocking(move |app| app.with_server(|db| db.redeem_bot_code(&scope, &code, &chat.to_string())))
            .await
            .map_err(|_| "Cannot link Telegram")?;
        if linked {
            "Подключено. Здесь будут уведомления о ваших задачах и задачах, за которыми вы следите."
        } else {
            "Ссылка устарела или недоступна. Получите новую в настройках Telegram в genie."
        }
    } else {
        "Уведомления о задачах приходят сюда. Подключение: настройки Telegram в genie. Для ответа откройте ссылку в уведомлении."
    };
    let _ = call(app, &bot.token.0, "sendMessage", json!({"chat_id": chat, "text": reply})).await;
    Ok(())
}
