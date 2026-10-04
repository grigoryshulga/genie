//! HTTP API and web UI.
//!
//! Project scope comes from the project cookie, `X-Genie-Project` or `?project=`. Protections: loopback bind by default,
//! Host allowlist (DNS rebinding), `X-Genie: 1` on cookie-authenticated writes
//! (cross-site forms cannot send it), no CORS.

pub mod account;
pub mod agent_config;
pub mod agents;
pub mod automations;
pub mod console;
pub mod ctx;
pub mod docs;
pub mod git;
pub mod ideas;
pub mod images;
pub mod live;
pub mod mcp_gateway;
pub mod mcp_server;
pub mod repos;
pub mod tasks;
pub mod teams;
pub mod web;

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, Uri, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use genie_core::GenieError;
use serde_json::json;
use tower_http::services::{ServeDir, ServeFile};

use crate::state::{App, AppError};

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        ApiError { status, message: message.into() }
    }
    pub fn unauthorized() -> Self {
        ApiError::new(StatusCode::UNAUTHORIZED, "login required")
    }
    pub fn bad(message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::BAD_REQUEST, message)
    }
}

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        match e {
            // Same contract as the TypeScript server: domain errors are 422 with their message.
            AppError::Genie(GenieError::Denied(m) | GenieError::NotFound(m) | GenieError::Invalid(m)) => {
                ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, m)
            }
            AppError::Genie(other) => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
            AppError::Bad(m) => ApiError::bad(m),
            AppError::Forbidden(m) => ApiError::new(StatusCode::FORBIDDEN, m),
            AppError::Conflict(m) => ApiError::new(StatusCode::CONFLICT, m),
            AppError::Internal(m) => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, m),
        }
    }
}

impl From<GenieError> for ApiError {
    fn from(e: GenieError) -> Self {
        AppError::Genie(e).into()
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;

/// A JSON request body of a known shape: a malformed one, or one with a key nobody knows, is
/// refused with `{ "error": … }` like any other bad request.
pub struct Body<T>(pub T);

impl<S: Send + Sync, T: serde::de::DeserializeOwned> axum::extract::FromRequest<S> for Body<T> {
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, ApiError> {
        let Json(v) = Json::<serde_json::Value>::from_request(req, state).await.map_err(|e| ApiError::bad(e.body_text()))?;
        serde_json::from_value(v).map(Body).map_err(|e| ApiError::bad(e.to_string()))
    }
}

pub fn router(app: Arc<App>) -> Router {
    let api = Router::new()
        .route("/health", get(|| async { Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") })) }))
        .merge(account::routes())
        .merge(tasks::routes())
        .merge(ideas::routes())
        .merge(teams::routes())
        .merge(agents::routes())
        .merge(docs::routes())
        .merge(images::routes())
        .merge(automations::routes())
        .merge(console::routes())
        .merge(agent_config::routes())
        .merge(mcp_gateway::routes())
        .merge(repos::routes())
        .merge(live::routes())
        .fallback(|| async { ApiError::new(StatusCode::NOT_FOUND, "not found") });
    let router = Router::new()
        .nest("/api", api)
        .merge(git::routes())
        .route("/mcp", post(mcp_server::endpoint).get(mcp_server::no_stream).delete(mcp_server::no_stream));
    // Client-side routes (/board, /team/G-7…) fall back to the SPA entry.
    let router = match web::resolve(app.web_root.as_deref()) {
        web::WebUi::BuiltIn => {
            router.fallback(|method: Method, uri: Uri| async move { web::respond(web::WEB_ASSETS, &method, uri.path()) })
        }
        web::WebUi::Dir(dir) => router.fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(dir.join("index.html")))),
        web::WebUi::Missing(why) => router.fallback(move || async move { (StatusCode::SERVICE_UNAVAILABLE, why) }),
    };
    router.layer(middleware::from_fn_with_state(app.clone(), guard)).with_state(app)
}

async fn guard(State(app): State<Arc<App>>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or_default();
    let port = app.cfg.port;
    let allowed = ["127.0.0.1", "localhost", "[::1]"].iter().any(|h| host == format!("{h}:{port}"))
        || app.cfg.allow_hosts.iter().any(|h| h == host || format!("{h}:{port}") == host);
    if !allowed {
        return ApiError::new(StatusCode::MISDIRECTED_REQUEST, "unexpected Host header").into_response();
    }
    let write = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    let bearer = req.headers().contains_key(header::AUTHORIZATION);
    // Webhooks come from other services and carry their own secret instead.
    // The git proxy takes tokens only (never cookies), so it has no cross-site risk either.
    let hook = req.uri().path().starts_with("/api/hooks/") || req.uri().path().starts_with("/git/");
    if write && !bearer && !hook && req.headers().get("x-genie").and_then(|v| v.to_str().ok()) != Some("1") {
        return ApiError::new(StatusCode::FORBIDDEN, "missing X-Genie header").into_response();
    }
    next.run(req).await
}
