//! Accounts and projects: login, invitations, users, projects, memberships.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use genie_core::server_db::{ProjectRole, SESSION_DAYS};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ctx::{Ctx, PROJECT_COOKIE, SESSION_COOKIE, Who};
use super::{ApiError, ApiResult};
use crate::state::App;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/auth/me", get(me))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/invite", post(accept_invite))
        .route("/auth/password", post(change_password))
        .route("/auth/tokens", get(list_tokens).post(create_token))
        .route("/auth/tokens/{id}", delete(revoke_token))
        .route("/me/litellm-key", get(litellm_key).put(set_litellm_key).delete(delete_litellm_key))
        .route("/session/project", post(select_project))
        .route("/users", get(list_users).post(create_user))
        .route("/users/{id}", patch(update_user))
        .route("/users/{id}/password", post(set_user_password))
        .route("/users/{id}/tokens", post(create_user_token))
        .route("/users/{id}/avatar", get(avatar).put(set_avatar).delete(clear_avatar))
        .route("/projects", get(list_projects).post(create_project))
        .route("/projects/{slug}", patch(update_project))
        .route("/projects/{slug}/members", get(list_members))
        .route("/projects/{slug}/members/{user}", put(set_member).delete(remove_member))
        .route("/projects/{slug}/invites", post(create_invite))
        .route("/doctor", get(doctor))
        .route("/model-prices", get(model_prices).post(refresh_model_prices))
        .route("/runtime", get(runtime_view))
        .route("/stats", get(stats))
        .route("/vault/sync", get(vault_sync).post(vault_sync_now))
}

fn cookie_header(name: &str, value: &str, max_age_secs: i64) -> HeaderValue {
    HeaderValue::from_str(&format!("{name}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age_secs}")).expect("cookie is ascii")
}

async fn me(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let mode = if matches!(&ctx.who, Who::User { local: true, .. }) { "local" } else { "users" };
    let user = match &ctx.who {
        Who::User { user, .. } => user.clone(),
        Who::Agent { project, role, name, team, .. } => {
            return Ok(Json(json!({ "agent": { "project": project, "role": role, "name": name, "team": team } })));
        }
        Who::Anonymous => {
            let has_users = app.blocking(|app| app.with_server(|db| db.user_count())).await? > 0;
            return Err(ApiError::new(
                StatusCode::UNAUTHORIZED,
                if has_users { "login required" } else { "open the server from this machine to set it up" },
            ));
        }
    };
    let current = ctx.access(&app, None).await.ok().map(|a| a.project);
    let u = user.clone();
    let projects = app
        .blocking(move |app| {
            app.with_server(|db| {
                let mut out = Vec::new();
                for p in db.projects()? {
                    if let Some(role) = db.project_role(&p.slug, &u)? {
                        out.push(
                            json!({ "slug": p.slug, "name": p.name, "role": role, "autonomy": p.autonomy, "hasRepo": p.repo.is_some() }),
                        );
                    }
                }
                Ok(out)
            })
        })
        .await?;
    Ok(Json(json!({ "user": user, "mode": mode, "projects": projects, "project": current })))
}

#[derive(Deserialize)]
struct LoginBody {
    login: String,
    password: String,
}

async fn login(State(app): State<Arc<App>>, Json(body): Json<LoginBody>) -> ApiResult<impl IntoResponse> {
    let found = app
        .blocking(move |app| {
            app.with_server(|db| {
                Ok(match db.authenticate(&body.login, &body.password)? {
                    Some(u) => Some((db.create_session(u.id)?, u)),
                    None => None,
                })
            })
        })
        .await?;
    let Some((secret, user)) = found else {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "wrong login or password"));
    };
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, cookie_header(SESSION_COOKIE, &secret, SESSION_DAYS * 86400));
    Ok((headers, Json(json!({ "user": user }))))
}

async fn logout(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<impl IntoResponse> {
    if let Some(s) = ctx.session.clone() {
        app.blocking(move |app| app.with_server(|db| db.delete_session(&s))).await?;
    }
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, cookie_header(SESSION_COOKIE, "", 0));
    Ok((headers, Json(json!({ "ok": true }))))
}

#[derive(Deserialize)]
struct InviteBody {
    token: String,
    login: String,
    #[serde(default)]
    name: String,
    password: String,
}

async fn accept_invite(State(app): State<Arc<App>>, Json(b): Json<InviteBody>) -> ApiResult<impl IntoResponse> {
    let (secret, user) = app
        .blocking(move |app| {
            app.with_server(|db| {
                let u = db.accept_invite(&b.token, &b.login, &b.name, &b.password)?;
                Ok((db.create_session(u.id)?, u))
            })
        })
        .await?;
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, cookie_header(SESSION_COOKIE, &secret, SESSION_DAYS * 86400));
    Ok((StatusCode::CREATED, headers, Json(json!({ "user": user }))))
}

#[derive(Deserialize)]
struct PasswordBody {
    #[serde(default)]
    current: String,
    password: String,
}

async fn change_password(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<PasswordBody>) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    if user.id == 0 {
        return Err(ApiError::bad("create a user first: genie user add <login> --admin"));
    }
    app.blocking(move |app| {
        app.with_server(|db| {
            if db.authenticate(&user.login, &b.current)?.is_none() {
                return Err(genie_core::GenieError::Denied("the current password is wrong".into()));
            }
            db.set_password(user.id, &b.password)
        })
    })
    .await?;
    Ok(Json(json!({ "ok": true, "note": "all sessions were closed; log in again" })))
}

#[derive(Deserialize)]
struct TokenBody {
    #[serde(default)]
    label: String,
}

async fn create_token(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<TokenBody>) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    if user.id == 0 {
        return Err(ApiError::bad("personal tokens need a real user: genie user add <login> --admin"));
    }
    let token = app.blocking(move |app| app.with_server(|db| db.create_user_token(user.id, &b.label))).await?;
    Ok(Json(json!({ "token": token })))
}

/// The caller's own live personal tokens (never their secrets).
async fn list_tokens(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    if user.id == 0 {
        return Ok(Json(json!([])));
    }
    let tokens = app.blocking(move |app| app.with_server(|db| db.user_tokens(user.id))).await?;
    Ok(Json(json!(tokens)))
}

/// Revoke one of the caller's personal tokens; the next request with it is refused.
async fn revoke_token(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    let done = app.blocking(move |app| app.with_server(|db| db.revoke_user_token(user.id, id))).await?;
    if !done {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no such token"));
    }
    Ok(Json(json!({ "ok": true })))
}

/// The caller's LiteLLM key, as far as it may be shown: set or not, its last characters.
async fn litellm_key(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    let info = app.blocking(move |app| app.with_server(|db| db.user_secret_info(user.id, genie_core::secrets::LITELLM))).await?;
    Ok(Json(json!({ "key": info })))
}

#[derive(Deserialize)]
struct LitellmKeyBody {
    key: String,
}

/// Set the caller's LiteLLM key: the agents started on their behalf use it.
async fn set_litellm_key(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<LitellmKeyBody>) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    if user.id == 0 {
        return Err(ApiError::bad("a LiteLLM key needs a real user: genie user add <login> --admin"));
    }
    if b.key.trim().is_empty() {
        return Err(ApiError::bad("the key is empty"));
    }
    let info = app
        .blocking(move |app| {
            app.with_server(|db| {
                db.set_user_secret(user.id, genie_core::secrets::LITELLM, &b.key)?;
                db.user_secret_info(user.id, genie_core::secrets::LITELLM)
            })
        })
        .await?;
    // Agents that waited for the key start now.
    crate::sessions::retry_now(&app);
    Ok(Json(json!({ "key": info })))
}

async fn delete_litellm_key(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    app.blocking(move |app| app.with_server(|db| db.delete_user_secret(user.id, genie_core::secrets::LITELLM))).await?;
    Ok(Json(json!({ "key": null })))
}

#[derive(Deserialize)]
struct SelectBody {
    project: String,
}

async fn select_project(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<SelectBody>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, Some(&b.project)).await?;
    if access.project != b.project {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "no access to this project"));
    }
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, cookie_header(PROJECT_COOKIE, &access.project, 365 * 86400));
    Ok((headers, Json(json!({ "project": access.project, "role": access.role }))))
}

async fn list_users(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    ctx.user()?;
    let users = app.blocking(|app| app.with_server(|db| db.users())).await?;
    Ok(Json(serde_json::to_value(users).unwrap_or_default()))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewUser {
    login: String,
    #[serde(default)]
    name: String,
    email: Option<String>,
    password: Option<String>,
    #[serde(default)]
    is_admin: bool,
}

async fn create_user(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<NewUser>) -> ApiResult<impl IntoResponse> {
    ctx.server_admin()?;
    let user = app
        .blocking(move |app| app.with_server(|db| db.create_user(&b.login, &b.name, b.email.as_deref(), b.password.as_deref(), b.is_admin)))
        .await?;
    Ok((StatusCode::CREATED, Json(json!(user))))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UserPatch {
    login: Option<String>,
    name: Option<String>,
    email: Option<Option<String>>,
    is_admin: Option<bool>,
    disabled: Option<bool>,
}

async fn update_user(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>, Json(b): Json<UserPatch>) -> ApiResult<Json<Value>> {
    let me = ctx.user()?.clone();
    let admin_fields = b.is_admin.is_some() || b.disabled.is_some();
    if (me.id != id || admin_fields) && !me.is_admin {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "server admin rights required"));
    }
    if id == 0 {
        return Err(ApiError::bad("create a user first: genie user add <login> --admin"));
    }
    let user = app
        .blocking(move |app| {
            let before = app.with_server(|db| db.user(id))?;
            let user = app.with_server(|db| {
                db.tx(|| {
                    if let Some(login) = &b.login {
                        db.rename_user(id, login)?;
                    }
                    db.update_user(id, b.name.as_deref(), b.email.as_ref().map(|e| e.as_deref()), b.is_admin, b.disabled)
                })
            })?;
            if user.login != before.login {
                // Tasks name the person responsible by login: they follow the new one.
                for p in app.with_server(|db| db.projects())? {
                    app.with_tracker(&p.slug, |t| t.rename_assignee(&before.login, &user.login))?;
                }
            }
            Ok(user)
        })
        .await?;
    Ok(Json(json!(user)))
}

/// Only the person themself, or a server admin, changes someone's photo.
fn may_edit(ctx: &Ctx, id: i64) -> ApiResult<()> {
    let me = ctx.user()?;
    if id == 0 || me.id == 0 {
        return Err(ApiError::bad("create a user first: genie user add <login> --admin"));
    }
    if me.id != id && !me.is_admin {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "server admin rights required"));
    }
    Ok(())
}

async fn avatar(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<impl IntoResponse> {
    ctx.user()?;
    let found = app.blocking(move |app| app.with_server(|db| db.avatar(id))).await?;
    let Some((bytes, kind)) = found else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "no photo"));
    };
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(&kind).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    // The address carries the photo's version, so a changed photo is a new address.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=31536000, immutable"));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    Ok((headers, bytes))
}

async fn set_avatar(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>, body: Bytes) -> ApiResult<Json<Value>> {
    may_edit(&ctx, id)?;
    let user = app.blocking(move |app| app.with_server(|db| db.set_avatar(id, &body))).await?;
    Ok(Json(json!(user)))
}

async fn clear_avatar(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    may_edit(&ctx, id)?;
    let user = app.blocking(move |app| app.with_server(|db| db.clear_avatar(id))).await?;
    Ok(Json(json!(user)))
}

#[derive(Deserialize)]
struct NewPassword {
    password: String,
}

/// A server admin sets someone's password; their sessions close.
async fn set_user_password(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(id): Path<i64>,
    Json(b): Json<NewPassword>,
) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    app.blocking(move |app| {
        app.with_server(|db| {
            db.user(id)?;
            db.set_password(id, &b.password)
        })
    })
    .await?;
    Ok(Json(json!({ "ok": true })))
}

/// A personal token for the command line and MCP: for oneself, or — by the
/// operator of the server's machine — for anyone.
async fn create_user_token(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>, Json(b): Json<TokenBody>) -> ApiResult<Json<Value>> {
    let operator = matches!(&ctx.who, Who::User { user, local: true } if user.is_admin);
    if ctx.user()?.id != id && !operator {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "a token is issued to its owner, or by the operator on the server's machine"));
    }
    let token = app
        .blocking(move |app| {
            app.with_server(|db| {
                if db.user(id)?.disabled {
                    return Err(genie_core::GenieError::invalid("the user is disabled"));
                }
                db.create_user_token(id, if b.label.is_empty() { "cli" } else { &b.label })
            })
        })
        .await?;
    Ok(Json(json!({ "token": token })))
}

async fn list_projects(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    let out = app
        .blocking(move |app| {
            app.with_server(|db| {
                let mut out = Vec::new();
                for p in db.projects()? {
                    if let Some(role) = db.project_role(&p.slug, &user)? {
                        let mut v = json!(p);
                        v["role"] = json!(role);
                        out.push(v);
                    }
                }
                Ok(out)
            })
        })
        .await?;
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NewProject {
    slug: String,
    #[serde(default)]
    name: String,
    repo: Option<String>,
    /// An existing tracker directory to register in place.
    tracker: Option<String>,
    prefix: Option<String>,
}

async fn create_project(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<NewProject>) -> ApiResult<impl IntoResponse> {
    ctx.server_admin()?;
    let project = app
        .blocking(move |app| {
            let repo = b.repo.filter(|r| !r.trim().is_empty());
            // A repository that already has a genie tracker keeps its tasks.
            let tracker = b
                .tracker
                .filter(|t| !t.trim().is_empty())
                .map(std::path::PathBuf::from)
                .or_else(|| repo.as_ref().map(|r| std::path::Path::new(r).join(".genie")).filter(|d| d.join("genie.db").exists()));
            app.create_project(&b.slug, &b.name, repo.as_deref(), tracker.as_ref().and_then(|t| t.to_str()), b.prefix.as_deref())
        })
        .await?;
    Ok((StatusCode::CREATED, Json(json!(project))))
}

#[derive(Deserialize)]
struct ProjectPatch {
    name: Option<String>,
    autonomy: Option<String>,
    integration: Option<String>,
}

async fn update_project(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(slug): Path<String>,
    Json(b): Json<ProjectPatch>,
) -> ApiResult<Json<Value>> {
    ctx.access(&app, Some(&slug)).await?.admin()?;
    let project = app
        .blocking(move |app| {
            app.with_server(|db| {
                if let Some(a) = &b.autonomy {
                    db.set_autonomy(&slug, a)?;
                }
                db.update_project(&slug, b.name.as_deref(), b.integration.as_deref())
            })
        })
        .await?;
    // Leaving `manual` lets the orchestrator take the mail that waited.
    app.wake_runtime.notify_one();
    Ok(Json(json!(project)))
}

async fn list_members(State(app): State<Arc<App>>, ctx: Ctx, Path(slug): Path<String>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, Some(&slug)).await?;
    let members = app.blocking(move |app| app.with_server(|db| db.members_of(&access.project))).await?;
    Ok(Json(json!(members.into_iter().map(|(u, r)| json!({ "user": u, "role": r })).collect::<Vec<_>>())))
}

#[derive(Deserialize)]
struct MemberBody {
    role: String,
}

async fn set_member(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path((slug, user)): Path<(String, i64)>,
    Json(b): Json<MemberBody>,
) -> ApiResult<Json<Value>> {
    ctx.access(&app, Some(&slug)).await?.admin()?;
    let role = ProjectRole::parse(&b.role)?;
    app.blocking(move |app| app.with_server(|db| db.set_membership(&slug, user, role))).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn remove_member(State(app): State<Arc<App>>, ctx: Ctx, Path((slug, user)): Path<(String, i64)>) -> ApiResult<Json<Value>> {
    ctx.access(&app, Some(&slug)).await?.admin()?;
    app.blocking(move |app| app.with_server(|db| db.remove_membership(&slug, user))).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct InviteNew {
    #[serde(default = "default_role")]
    role: String,
    email: Option<String>,
}

fn default_role() -> String {
    "member".into()
}

async fn create_invite(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(slug): Path<String>,
    Json(b): Json<InviteNew>,
) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, Some(&slug)).await?;
    access.admin()?;
    let role = ProjectRole::parse(&b.role)?;
    let by = access.user.as_ref().map(|u| u.id).filter(|id| *id > 0);
    let project = access.project.clone();
    let secret = app.blocking(move |app| app.with_server(|db| db.create_invite(by, Some(&project), role, b.email.as_deref()))).await?;
    let url = format!("{}/invite?token={secret}", app.cfg.public_url());
    Ok((StatusCode::CREATED, Json(json!({ "token": secret, "url": url }))))
}

/// The server's preflight (`genie doctor`) for its admins.
async fn doctor(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    let checks = app.blocking(|app| Ok(crate::doctor::run(&app.data, &app.cfg, &app.agents(), app.web_root.as_deref()))).await?;
    Ok(Json(json!({ "checks": checks })))
}

/// What the scheduler is doing: running turns, the queue waiting for a slot by
/// kind, and the slots themselves (`genie server runtime`).
async fn runtime_view(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    let v = app.sched.snapshot(app.cfg.runtime.max_concurrent, app.slots.available_permits());
    Ok(Json(serde_json::to_value(&v).unwrap_or_default()))
}

/// The model prices in effect: `modelPrices` of config.json over what LiteLLM
/// returned, with where each model's price comes from and when it was received.
async fn model_prices(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    let state = app
        .blocking(|app| {
            let s = app.prices.read().map_err(|_| genie_core::GenieError::invalid("prices lock poisoned"))?;
            let manual = &app.cfg.model_prices;
            let models: Vec<Value> = s
                .prices
                .iter()
                .map(|(model, p)| {
                    let source = if manual.contains_key(model) {
                        match (p.cache_read.is_some(), manual[model].cache_read.is_some()) {
                            (true, false) => "config.json + LiteLLM (cache)",
                            _ => "config.json",
                        }
                    } else {
                        "LiteLLM"
                    };
                    json!({ "model": model, "price": p, "source": source })
                })
                .collect();
            Ok(json!({ "models": models, "fetchedAt": s.fetched_at, "lastError": s.last_error }))
        })
        .await?;
    Ok(Json(state))
}

/// «Pull the config»: ask LiteLLM for the tariffs again (an administrator's
/// action — there is no timer) and make them the prices in effect.
async fn refresh_model_prices(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    let n = crate::model_prices::refresh(&app).await.map_err(genie_core::GenieError::invalid)?;
    let state = app
        .blocking(move |app| {
            let s = app.prices.read().map_err(|_| genie_core::GenieError::invalid("prices lock poisoned"))?;
            Ok(json!({ "received": n, "priced": s.prices.len(), "fetchedAt": s.fetched_at }))
        })
        .await?;
    Ok(Json(state))
}

/// How the vault syncs with its git remote (`vault.remote`).
async fn vault_sync(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    let remote = app.cfg.vault.remote.clone().filter(|r| !r.trim().is_empty());
    Ok(Json(json!({ "remote": remote, "every": app.cfg.vault.sync_secs.unwrap_or(120), "last": crate::vault_sync::state() })))
}

/// Sync the vault now.
async fn vault_sync_now(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    let st = app.blocking(crate::vault_sync::sync).await?.ok_or_else(|| ApiError::bad("vault.remote is not set in config.json"))?;
    Ok(Json(json!({ "last": st })))
}

#[derive(Deserialize)]
struct StatsQuery {
    days: Option<i64>,
    project: Option<String>,
}

/// What happened over the last days (`genie stats`), for the server's admins.
async fn stats(State(app): State<Arc<App>>, ctx: Ctx, axum::extract::Query(q): axum::extract::Query<StatsQuery>) -> ApiResult<Json<Value>> {
    ctx.server_admin()?;
    let stats = app
        .blocking(move |app| {
            let prices = app.prices()?;
            crate::stats::collect(&app.data, q.days.unwrap_or(7), q.project.as_deref(), &prices)
                .map_err(|e| genie_core::GenieError::invalid(e).into())
        })
        .await?;
    Ok(Json(json!(stats)))
}
