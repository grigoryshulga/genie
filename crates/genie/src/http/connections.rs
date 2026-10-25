//! Provider connections: a person connects the deployment's GitHub App, picks repositories and
//! binds them to a project (docs/development/git-provider-connection-research.md).
//!
//! The browser flow has two legs on one callback address, both bound to the browser that started
//! them (`genie_git_flow` cookie) and to one unpredictable `state`:
//!
//! 1. the *setup* leg: GitHub returns the person from the installation page with an `installation_id`,
//!    which anyone can forge, so it is only a claim;
//! 2. the *authorization* leg: GitHub returns an authorization `code` (PKCE `S256`). The server
//!    exchanges it for a user token, and accepts the installation only if that token lists it
//!    among the person's installations.
//!
//! The user token is kept in memory for a quarter of an hour so the person can pick repositories
//! that both they and the installation reach, and is never stored. Installation tokens, which do
//! the lasting work, are minted per repository (`git::tokens`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use genie_core::connections::{Connection, Health, NewConnection};
use genie_core::events;
use genie_core::server_db::{hash_secret, new_secret};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::ctx::Ctx;
use super::social_auth::{cookie, origin};
use super::{ApiError, ApiResult};
use crate::config::GithubApp;
use crate::git::github_app;
use crate::git::hosts::{self, Host, Kind};
use crate::git::provider::ApiError as HostError;
use crate::state::App;

const FLOW_COOKIE: &str = "genie_git_flow";
const FLOW_TTL: Duration = Duration::from_secs(600);
const PICK_TTL: Duration = Duration::from_secs(900);
/// Where the person lands after the flow: the project's repositories.
const LANDING: &str = "/manage/project/repos?tab=repositories";

enum Purpose {
    Connect,
    /// Authorize again to pick repositories of an existing connection.
    Pick(String),
}

struct Flow {
    user: i64,
    purpose: Purpose,
    browser_hash: String,
    verifier: String,
    /// The installation the setup leg claimed.
    installation: Option<i64>,
    created: Instant,
}

/// Flows in progress and the user tokens of people picking repositories; memory only.
#[derive(Default)]
pub struct Registry {
    flows: Mutex<HashMap<String, Flow>>,
    pickers: Mutex<HashMap<(i64, String), (String, Instant)>>,
}

impl Registry {
    fn insert(&self, state: &str, flow: Flow) -> ApiResult<()> {
        let mut flows = self.flows.lock().map_err(|_| ApiError::bad("connection registry unavailable"))?;
        flows.retain(|_, f| f.created.elapsed() < FLOW_TTL);
        if flows.len() >= 1024 {
            return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too many connection attempts; try later"));
        }
        flows.insert(hash_secret(state), flow);
        Ok(())
    }

    fn take(&self, state: &str, browser: &str) -> ApiResult<Flow> {
        let mut flows = self.flows.lock().map_err(|_| ApiError::bad("connection registry unavailable"))?;
        let key = hash_secret(state);
        let valid = flows.get(&key).is_some_and(|f| f.created.elapsed() < FLOW_TTL && f.browser_hash == hash_secret(browser));
        if !valid {
            return Err(ApiError::bad("the connection expired or the browser did not match; start again"));
        }
        Ok(flows.remove(&key).expect("flow checked under lock"))
    }

    fn keep_picker(&self, user: i64, connection: &str, token: String) {
        let mut p = self.pickers.lock().unwrap_or_else(|e| e.into_inner());
        p.retain(|_, (_, at)| at.elapsed() < PICK_TTL);
        p.insert((user, connection.to_string()), (token, Instant::now()));
    }

    fn picker(&self, user: i64, connection: &str) -> Option<String> {
        let p = self.pickers.lock().unwrap_or_else(|e| e.into_inner());
        p.get(&(user, connection.to_string())).filter(|(_, at)| at.elapsed() < PICK_TTL).map(|(t, _)| t.clone())
    }

    fn drop_pickers(&self, connection: &str) {
        self.pickers.lock().unwrap_or_else(|e| e.into_inner()).retain(|(_, c), _| c != connection);
    }

    fn drop_picker(&self, user: i64, connection: &str) {
        self.pickers.lock().unwrap_or_else(|e| e.into_inner()).remove(&(user, connection.to_string()));
    }
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/git/connections", get(list))
        .route("/git/connections/{id}", axum::routing::delete(disconnect))
        .route("/git/connections/{id}/reassign", post(reassign))
        .route("/git/connections/{id}/check", post(check))
        .route("/git/connections/{id}/authorize", post(authorize))
        .route("/git/connections/{id}/repositories", get(repositories))
        .route("/git/github/connect", post(connect))
        .route("/git/github/callback", get(callback))
}

/// The configured app, its client secret and its host.
fn github(app: &App) -> ApiResult<(GithubApp, String, Host)> {
    let cfg = app
        .cfg
        .git_apps
        .github
        .clone()
        .ok_or_else(|| ApiError::bad("no GitHub App is configured on this server (gitApps.github in config.json)"))?;
    let secret = std::env::var(&cfg.client_secret_env)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| ApiError::bad("the GitHub App's client secret is unavailable"))?;
    let host = hosts::load(&app.data)
        .map
        .get(&cfg.host)
        .cloned()
        .filter(|h| h.kind == Kind::Github)
        .ok_or_else(|| ApiError::bad(format!("the GitHub App's host {} is not a github host in git.json", cfg.host)))?;
    Ok((cfg, secret, host))
}

fn callback_url(app: &App) -> ApiResult<String> {
    Ok(origin(app)?.join("/api/git/github/callback").map_err(|_| ApiError::bad("invalid callback URL"))?.to_string())
}

fn host_error(e: HostError) -> ApiError {
    match e {
        HostError::Auth(m) => ApiError::new(StatusCode::BAD_GATEWAY, format!("GitHub refused: {m}")),
        HostError::RateLimited(_) => ApiError::new(StatusCode::TOO_MANY_REQUESTS, "GitHub's rate limit is reached; try again later"),
        other => ApiError::new(StatusCode::BAD_GATEWAY, other.to_string()),
    }
}

/// What the person sees of a connection.
fn connection_json(c: &Connection, host: Option<&Host>) -> Value {
    let mut v = serde_json::to_value(c).unwrap_or_default();
    v["blocked"] = json!(c.blocked());
    if let Some(h) = host {
        v["manageUrl"] = json!(format!("{}/settings/installations/{}", h.url, c.installation_id));
    }
    v
}

async fn list(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    let configured = github(&app).ok();
    let admin = app.blocking({
        let user = user.clone();
        move |app| app.with_server(|db| db.administers_a_project(&user))
    });
    let can_connect = admin.await?;
    let conns = app.blocking(move |app| app.with_server(|db| db.connections(if user.is_admin { None } else { Some(user.id) }))).await?;
    let host = configured.as_ref().map(|(_, _, h)| h);
    Ok(Json(json!({
        "github": {
            "configured": configured.is_some(),
            "slug": configured.as_ref().map(|(c, _, _)| c.slug.clone()),
            "host": configured.as_ref().map(|(c, _, _)| c.host.clone()),
        },
        "canConnect": can_connect,
        "connections": conns.iter().map(|c| connection_json(c, host.filter(|h| h.id == c.host))).collect::<Vec<_>>(),
    })))
}

fn audit(app: &App, actor: &str, connection: &str, action: &str, detail: Value) {
    let _ = app
        .with_server(|db| db.record_config_change(actor, &format!("git.connection:{connection}"), action, None, Some(&detail.to_string())));
}

/// Tell the journal of every project that uses the connection (best effort).
fn journal(app: &App, connection: &str, actor: &str, action: &str, detail: Value) {
    let Ok(rows) = app.with_server(|db| db.connection_repos(connection)) else { return };
    let mut projects: Vec<String> = rows.into_iter().map(|(p, _, _)| p).collect();
    projects.dedup();
    for p in projects {
        let payload = json!({ "connection": connection, "action": action, "detail": detail });
        let _ = app.with_tracker(&p, |tr| tr.append_event(events::GIT_CONNECTION, None, actor, "human", payload).map(|_| ()));
    }
}

fn flow_cookie(app: &App, browser: &str, age: i64) -> ApiResult<HeaderValue> {
    Ok(cookie(FLOW_COOKIE, browser, age, origin(app)?.scheme() == "https", "Lax"))
}

fn json_with_cookie(app: &App, body: Value, browser: &str) -> ApiResult<Response> {
    let mut response = Json(body).into_response();
    response.headers_mut().insert(header::SET_COOKIE, flow_cookie(app, browser, FLOW_TTL.as_secs() as i64)?);
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// Start connecting: the address of the app's installation page.
async fn connect(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Response> {
    let user = ctx.user()?.clone();
    if user.id <= 0 {
        return Err(ApiError::bad("create an account before connecting a provider"));
    }
    let allowed = app.blocking({
        let user = user.clone();
        move |app| app.with_server(|db| db.administers_a_project(&user))
    });
    if !allowed.await? {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "only an administrator of a project can connect an account"));
    }
    let (cfg, _, host) = github(&app)?;
    callback_url(&app)?;
    let (state, browser) = (new_secret(), new_secret());
    let url = format!("{}/apps/{}/installations/new?state={state}", host.url, cfg.slug);
    app.git_connect.insert(
        &state,
        Flow {
            user: user.id,
            purpose: Purpose::Connect,
            browser_hash: hash_secret(&browser),
            verifier: String::new(),
            installation: None,
            created: Instant::now(),
        },
    )?;
    json_with_cookie(&app, json!({ "url": url }), &browser)
}

/// Authorize again to pick repositories of an existing connection.
async fn authorize(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Response> {
    let user = ctx.user()?.clone();
    let conn = own_connection(&app, &user, &id).await?;
    if let Some(why) = conn.blocked() {
        return Err(ApiError::new(StatusCode::CONFLICT, why));
    }
    let (cfg, _, host) = github(&app)?;
    let (state, browser) = (new_secret(), new_secret());
    let verifier = new_secret();
    let url = authorize_url(&app, &cfg, &host, &state, &verifier)?;
    app.git_connect.insert(
        &state,
        Flow {
            user: user.id,
            purpose: Purpose::Pick(id),
            browser_hash: hash_secret(&browser),
            verifier,
            installation: None,
            created: Instant::now(),
        },
    )?;
    json_with_cookie(&app, json!({ "url": url }), &browser)
}

fn authorize_url(app: &App, cfg: &GithubApp, host: &Host, state: &str, verifier: &str) -> ApiResult<String> {
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let redirect = callback_url(app)?;
    let mut url =
        reqwest::Url::parse(&format!("{}/login/oauth/authorize", host.url)).map_err(|_| ApiError::bad("invalid GitHub host URL"))?;
    url.query_pairs_mut().extend_pairs([
        ("client_id", cfg.client_id.as_str()),
        ("redirect_uri", redirect.as_str()),
        ("state", state),
        ("code_challenge", challenge.as_str()),
        ("code_challenge_method", "S256"),
    ]);
    Ok(url.to_string())
}

/// The connection when the caller owns it (server administrators may act on any).
async fn own_connection(app: &Arc<App>, user: &genie_core::server_db::User, id: &str) -> ApiResult<Connection> {
    let id = id.to_string();
    let conn = app.blocking(move |app| app.with_server(|db| db.connection(&id))).await?;
    if conn.owner != Some(user.id) && !user.is_admin {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "this connection belongs to someone else"));
    }
    Ok(conn)
}

#[derive(Deserialize)]
struct Callback {
    state: String,
    code: Option<String>,
    installation_id: Option<i64>,
    error: Option<String>,
}

async fn callback(State(app): State<Arc<App>>, Query(query): Query<Callback>, headers: HeaderMap) -> Response {
    let secure = origin(&app).is_ok_and(|u| u.scheme() == "https");
    let mut response = match proceed(&app, query, &headers).await {
        Ok(Next::Authorize(url)) => Redirect::to(&url).into_response(),
        Ok(Next::Done(connection)) => {
            let mut r = Redirect::to(&format!("{LANDING}&connection={connection}")).into_response();
            r.headers_mut().append(header::SET_COOKIE, cookie(FLOW_COOKIE, "", 0, secure, "Lax"));
            r
        }
        // Keep provider errors and credentials out of URLs, HTML and logs.
        Err(_) => {
            let mut r = Redirect::to(&format!("{LANDING}&gitError=1")).into_response();
            r.headers_mut().append(header::SET_COOKIE, cookie(FLOW_COOKIE, "", 0, secure, "Lax"));
            r
        }
    };
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

enum Next {
    /// Send the browser to GitHub's authorization page.
    Authorize(String),
    /// The connection to show.
    Done(String),
}

async fn proceed(app: &Arc<App>, q: Callback, headers: &HeaderMap) -> ApiResult<Next> {
    let browser = headers
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.split(';').find_map(|c| c.trim().strip_prefix("genie_git_flow=")))
        .ok_or_else(|| ApiError::bad("missing connection cookie"))?;
    let mut flow = app.git_connect.take(&q.state, browser)?;
    if q.error.is_some() {
        return Err(ApiError::bad("the provider cancelled the connection"));
    }
    let (cfg, secret, host) = github(app)?;
    let Some(code) = q.code.filter(|c| !c.is_empty()) else {
        // The setup leg: remember the claimed installation and go on to authorize the person.
        let Purpose::Connect = flow.purpose else { return Err(ApiError::bad("unexpected setup")) };
        flow.installation = Some(q.installation_id.filter(|i| *i > 0).ok_or_else(|| ApiError::bad("GitHub returned no installation"))?);
        flow.verifier = new_secret();
        flow.created = Instant::now();
        let url = authorize_url(app, &cfg, &host, &q.state, &flow.verifier)?;
        app.git_connect.insert(&q.state, flow)?;
        return Ok(Next::Authorize(url));
    };
    let token = github_app::exchange_code(&host, &cfg, &secret, &code, &flow.verifier, &callback_url(app)?).await.map_err(host_error)?;
    let installations = github_app::user_installations(&host, &token).await.map_err(host_error)?;
    let (claimed, existing) = match &flow.purpose {
        Purpose::Connect => (flow.installation.ok_or_else(|| ApiError::bad("the installation was not claimed"))?, None),
        Purpose::Pick(id) => {
            let id = id.clone();
            let conn = app.blocking(move |app| app.with_server(|db| db.connection(&id))).await?;
            (conn.installation_id, Some(conn))
        }
    };
    // The installation counts only when GitHub lists it for the person who just authorized.
    let installation = installations
        .into_iter()
        .find(|i| i.id == claimed)
        .ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "that installation is not one of yours"))?;
    let user_id = flow.user;
    let connection = match existing {
        Some(conn) => {
            if conn.owner != Some(user_id) {
                let admin = app.blocking(move |app| Ok(app.with_server(|db| db.user(user_id))?.is_admin)).await?;
                if !admin {
                    return Err(ApiError::new(StatusCode::FORBIDDEN, "this connection belongs to someone else"));
                }
            }
            conn.id
        }
        None => {
            let new = NewConnection {
                provider: "github".into(),
                host: host.id.clone(),
                owner: user_id,
                account_login: installation.account_login,
                account_id: installation.account_id,
                account_type: installation.account_type,
                installation_id: installation.id,
                selection: installation.selection,
                permissions: installation.permissions,
            };
            let conn = app
                .blocking(move |app| {
                    let conn = app.with_server(|db| db.upsert_connection(new))?;
                    let login = app.with_server(|db| db.user(user_id))?.login;
                    audit(
                        app,
                        &login,
                        &conn.id,
                        "connected",
                        json!({ "account": conn.account_login, "installation": conn.installation_id }),
                    );
                    crate::git::tokens::forget(app, &conn.id);
                    Ok(conn)
                })
                .await?;
            conn.id
        }
    };
    app.git_connect.keep_picker(user_id, &connection, token);
    Ok(Next::Done(connection))
}

/// The repositories the person and the installation both reach.
async fn repositories(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    let conn = own_connection(&app, &user, &id).await?;
    if let Some(why) = conn.blocked() {
        return Err(ApiError::new(StatusCode::CONFLICT, why));
    }
    let Some(token) = app.git_connect.picker(user.id, &id) else {
        return Err(ApiError::new(StatusCode::CONFLICT, "authorize to list repositories"));
    };
    let (_, _, host) = github(&app)?;
    let list = match github_app::user_installation_repos(&host, &token, conn.installation_id).await {
        Ok(l) => l,
        Err(HostError::Auth(_)) => {
            app.git_connect.drop_picker(user.id, &id);
            return Err(ApiError::new(StatusCode::CONFLICT, "authorize to list repositories"));
        }
        Err(e) => return Err(host_error(e)),
    };
    let project = ctx.access(&app, None).await.ok().map(|a| a.project);
    let bound = app
        .blocking({
            let id = id.clone();
            move |app| app.with_server(|db| db.connection_repos(&id))
        })
        .await?;
    Ok(Json(json!({
        "connection": id,
        "repositories": list.iter().map(|r| {
            let here = bound.iter().find(|(p, _, rid)| *rid == r.id && Some(p) == project.as_ref()).map(|(_, n, _)| n.clone());
            json!({
                "id": r.id, "fullName": r.full_name, "defaultBranch": r.default_branch, "private": r.private,
                "canPush": r.can_push, "boundAs": here,
                "boundElsewhere": here.is_none() && bound.iter().any(|(_, _, rid)| *rid == r.id),
            })
        }).collect::<Vec<_>>(),
    })))
}

/// Resolve what a repository add from a connection stands for: the verified repository on the provider.
/// The caller must own the connection and have authorized recently; only repositories the person
/// and the installation both reach can be bound.
pub async fn resolve_binding(
    app: &Arc<App>,
    ctx: &Ctx,
    connection: &str,
    provider_repo_id: i64,
) -> ApiResult<(Connection, github_app::ProviderRepo)> {
    let user = ctx.user()?.clone();
    let conn = own_connection(app, &user, connection).await?;
    if let Some(why) = conn.blocked() {
        return Err(ApiError::new(StatusCode::CONFLICT, why));
    }
    let token =
        app.git_connect.picker(user.id, connection).ok_or_else(|| ApiError::new(StatusCode::CONFLICT, "authorize to list repositories"))?;
    let (_, _, host) = github(app)?;
    let list = github_app::user_installation_repos(&host, &token, conn.installation_id).await.map_err(host_error)?;
    let repo = list
        .into_iter()
        .find(|r| r.id == provider_repo_id)
        .ok_or_else(|| ApiError::new(StatusCode::FORBIDDEN, "that repository is not available through this connection"))?;
    Ok((conn, repo))
}

/// Disconnect: operations stop at once; repositories, deliveries and links to requests stay.
async fn disconnect(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    own_connection(&app, &user, &id).await?;
    app.git_connect.drop_pickers(&id);
    let out = app
        .blocking(move |app| {
            app.with_server(|db| db.revoke_connection(&id, &format!("disconnected by {}", user.login)))?;
            crate::git::tokens::forget(app, &id);
            audit(app, &user.login, &id, "disconnected", json!({}));
            journal(app, &id, &user.login, "disconnected", json!({}));
            Ok(connection_json(&app.with_server(|db| db.connection(&id))?, None))
        })
        .await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
struct ReassignBody {
    login: String,
}

/// An administrator gives a connection to another person (needed when its owner was removed).
async fn reassign(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Json(b): Json<ReassignBody>) -> ApiResult<Json<Value>> {
    let admin = ctx.server_admin()?.clone();
    app.git_connect.drop_pickers(&id);
    let out = app
        .blocking(move |app| {
            let target = app
                .with_server(|db| db.user_by_login(b.login.trim()))?
                .ok_or_else(|| genie_core::GenieError::not_found(format!("no user {}", b.login)))?;
            let before = app.with_server(|db| db.connection(&id))?;
            let conn = app.with_server(|db| db.reassign_connection(&id, target.id))?;
            crate::git::tokens::forget(app, &id);
            audit(app, &admin.login, &id, "reassigned", json!({ "from": before.owner_login, "to": target.login }));
            journal(app, &id, &admin.login, "reassigned", json!({ "to": target.login }));
            Ok(connection_json(&conn, None))
        })
        .await?;
    Ok(Json(out))
}

/// Ask the provider how the installation stands and record it.
async fn check(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let user = ctx.user()?.clone();
    own_connection(&app, &user, &id).await?;
    Ok(Json(refresh(&app, &id).await?))
}

/// Re-read an installation from GitHub: suspension, removal, selection and permissions.
pub async fn refresh(app: &Arc<App>, id: &str) -> ApiResult<Value> {
    let (cfg, _, host) = github(app)?;
    let conn = {
        let id = id.to_string();
        app.blocking(move |app| app.with_server(|db| db.connection(&id))).await?
    };
    let answer = github_app::installation(&host, &cfg, conn.installation_id).await;
    let id = id.to_string();
    let out = app
        .blocking(move |app| {
            let (health, note) = match &answer {
                Ok(i) if i.suspended => (Health::Suspended, "the installation is suspended on GitHub".to_string()),
                Ok(i) => {
                    app.with_server(|db| db.touch_connection(&id, &i.selection, &i.permissions))?;
                    (Health::Ok, String::new())
                }
                Err(e) => crate::git::tokens::classify(e),
            };
            // Disconnecting in Genie stays a disconnect until the person connects again.
            if conn.health != Health::Revoked {
                app.with_server(|db| db.set_connection_health(&id, health, &note))?;
                if health.blocks() {
                    crate::git::tokens::forget(app, &id);
                }
            }
            Ok(connection_json(&app.with_server(|db| db.connection(&id))?, None))
        })
        .await?;
    Ok(out)
}

/// Check every live connection now and then, so a removed or suspended installation shows up
/// before an agent runs into it.
pub fn spawn_health(app: Arc<App>) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(60)).await;
        loop {
            if github(&app).is_ok()
                && let Ok(conns) = app.blocking(|app| app.with_server(|db| db.connections(None))).await
            {
                for c in conns.into_iter().filter(|c| c.provider == "github" && c.health != Health::Revoked) {
                    if let Err(e) = refresh(&app, &c.id).await {
                        eprintln!("genie git: connection {}: {}", c.id, e.message);
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(1800)).await;
        }
    });
}
