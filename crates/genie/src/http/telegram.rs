//! Project-scoped bot management and personal subscriptions.
use super::{ApiError, ApiResult, ctx::Ctx};
use crate::state::App;
use crate::tasks::TaskWatch;
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use genie_core::secrets::Secret;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TelegramStatus {
    pub username: Option<String>,
    pub connected: bool,
    pub available: bool,
}
#[derive(Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TelegramLink {
    pub url: String,
    pub expires_at: String,
}
#[derive(Deserialize)]
struct BotBody {
    token: Secret,
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/project/telegram", get(status).put(save).delete(remove))
        .route("/project/telegram/link", post(link).delete(unlink))
        .route("/project/telegram/test", post(test_message))
        .route("/tasks/{task}/watch", get(watch).put(follow).delete(unfollow))
}

async fn status(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<TelegramStatus>> {
    let user = ctx.user()?.id;
    let project = ctx.access(&app, ctx.project_hint.as_deref()).await?.project;
    let result = app
        .blocking(move |app| {
            app.with_server(|db| {
                let bot = db.project_bot(&project)?;
                let connected = match &bot {
                    Some(b) => db.channel_address(user, &b.scope)?.is_some(),
                    None => false,
                };
                Ok(TelegramStatus {
                    username: bot.as_ref().map(|b| b.username.clone()),
                    available: bot.as_ref().is_some_and(|b| !b.token.0.is_empty()),
                    connected,
                })
            })
        })
        .await?;
    Ok(Json(result))
}
async fn save(State(app): State<Arc<App>>, ctx: Ctx, Json(body): Json<BotBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, ctx.project_hint.as_deref()).await?;
    access.admin()?;
    let token = body.token.0.trim().to_string();
    if token.len() > 256
        || !token.split_once(':').is_some_and(|(id, secret)| {
            !id.is_empty()
                && id.bytes().all(|b| b.is_ascii_digit())
                && secret.len() >= 20
                && secret.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
    {
        return Err(ApiError::bad("Введите токен бота от BotFather"));
    }
    let me = crate::telegram::call(&app, &token, "getMe", json!({})).await.map_err(ApiError::bad)?;
    let id = me["id"].as_i64().filter(|id| *id > 0).ok_or_else(|| ApiError::bad("Telegram не вернул ID бота"))?;
    let username = me["username"]
        .as_str()
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .ok_or_else(|| ApiError::bad("У бота нет username"))?
        .to_string();
    if me["is_bot"] != true {
        return Err(ApiError::bad("Токен не принадлежит боту"));
    }
    // Do not hijack an existing webhook or a legacy server bot's polling stream.
    if app.cfg.telegram.as_ref().is_some_and(|b| b.token.split(':').next() == token.split(':').next()) {
        return Err(ApiError::bad("Этот бот уже используется сервером. Создайте отдельного бота проекта."));
    }
    let hook = crate::telegram::call(&app, &token, "getWebhookInfo", json!({})).await.map_err(ApiError::bad)?;
    if hook["url"].as_str().is_none_or(|url| !url.is_empty()) {
        return Err(ApiError::bad("У бота включён webhook. Используйте отдельного бота без webhook."));
    }
    app.blocking(move |app| app.with_server(|db| db.set_project_bot(&access.project, id, &username, &token))).await?;
    Ok(Json(json!({"ok": true})))
}
async fn remove(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, ctx.project_hint.as_deref()).await?;
    access.admin()?;
    app.blocking(move |app| app.with_server(|db| db.remove_project_bot(&access.project))).await?;
    Ok(Json(json!({"ok": true})))
}
async fn link(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<TelegramLink>> {
    let user = ctx.user()?.id;
    if user == 0 {
        return Err(ApiError::bad("Сначала создайте учётную запись"));
    }
    let project = ctx.access(&app, ctx.project_hint.as_deref()).await?.project;
    let result = app
        .blocking(move |app| {
            app.with_server(|db| {
                let bot = db
                    .project_bot(&project)?
                    .filter(|b| !b.token.0.is_empty())
                    .ok_or_else(|| genie_core::GenieError::invalid("Бот проекта не настроен"))?;
                let code = db.bot_link_code(&bot, user)?;
                Ok(TelegramLink {
                    url: format!("https://t.me/{}?start={code}", bot.username),
                    expires_at: genie_core::server_db::time_in(chrono::Duration::minutes(30)),
                })
            })
        })
        .await?;
    Ok(Json(result))
}
async fn unlink(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.id;
    let project = ctx.access(&app, ctx.project_hint.as_deref()).await?.project;
    app.blocking(move |app| {
        app.with_server(|db| {
            if let Some(bot) = db.project_bot(&project)? {
                db.unwatch_bot(&bot.scope, user)?;
            }
            Ok(())
        })
    })
    .await?;
    Ok(Json(json!({"ok": true})))
}
async fn watch_state(app: &Arc<App>, ctx: &Ctx, task: String, value: Option<bool>) -> ApiResult<TaskWatch> {
    ctx.user()?;
    let access = ctx.access(app, ctx.project_hint.as_deref()).await?;
    let caller = access.caller();
    Ok(app.blocking(move |app| crate::tasks::watch(app, &caller, &task, value)).await?)
}
async fn watch(State(app): State<Arc<App>>, ctx: Ctx, Path(task): Path<String>) -> ApiResult<Json<TaskWatch>> {
    Ok(Json(watch_state(&app, &ctx, task, None).await?))
}
async fn follow(State(app): State<Arc<App>>, ctx: Ctx, Path(task): Path<String>) -> ApiResult<Json<TaskWatch>> {
    Ok(Json(watch_state(&app, &ctx, task, Some(true)).await?))
}
async fn unfollow(State(app): State<Arc<App>>, ctx: Ctx, Path(task): Path<String>) -> ApiResult<Json<TaskWatch>> {
    Ok(Json(watch_state(&app, &ctx, task, Some(false)).await?))
}

async fn test_message(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.id;
    let project = ctx.access(&app, ctx.project_hint.as_deref()).await?.project;
    let (bot, chat) = app
        .blocking(move |app| {
            app.with_server(|db| {
                let bot = db.project_bot(&project)?.ok_or_else(|| genie_core::GenieError::invalid("Бот проекта не настроен"))?;
                let chat = db
                    .channel_address(user, &bot.scope)?
                    .ok_or_else(|| genie_core::GenieError::invalid("Сначала подключите уведомления"))?;
                Ok((bot, chat))
            })
        })
        .await?;
    crate::telegram::call(&app, &bot.token.0, "sendMessage", json!({"chat_id": chat, "text": "Проверка genie: уведомления подключены."}))
        .await
        .map_err(ApiError::bad)?;
    Ok(Json(json!({"ok": true})))
}
