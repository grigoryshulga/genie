//! Fake Prometheus- and Sentry-compatible servers on a real port, with the answers of the real APIs:
//! `/api/v1/query` and `/api/0/organizations/{org}/events/`. A test scripts answers per fragment of
//! the query, can replace them between passes, and reads back every request the fake received.

#![allow(dead_code)]

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex, MutexGuard};

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::Engine;
use serde_json::{Value, json};

/// One request the fake received.
#[derive(Debug, Clone)]
pub struct Call {
    /// The full request URL, query string as sent.
    pub url: String,
    /// The `Authorization` header as sent, if any.
    pub authorization: Option<String>,
}

#[derive(Debug)]
pub struct Fake {
    base: String,
    /// When set, a request is answered 401 unless its Authorization is `Bearer <token>` or
    /// `Basic <base64 of token>`.
    pub token: Option<String>,
    rules: Vec<(String, Value)>,
    /// Overrides the HTTP status of every scripted answer.
    pub status: Option<u16>,
    /// Answers with a chunked body of more than 1 MiB (no Content-Length, not JSON).
    pub oversize: bool,
    pub calls: Vec<Call>,
}

impl Fake {
    /// The first rule whose fragment is contained in the request's query wins. A later call with
    /// the same fragment replaces that answer.
    pub fn respond(&mut self, fragment: &str, body: Value) {
        match self.rules.iter_mut().find(|(f, _)| f == fragment) {
            Some(rule) => rule.1 = body,
            None => self.rules.push((fragment.to_string(), body)),
        }
    }
}

#[derive(Clone)]
pub struct Shared(Arc<Mutex<Fake>>);

impl Shared {
    pub fn lock(&self) -> MutexGuard<'_, Fake> {
        self.0.lock().unwrap()
    }
}

pub struct FakeMetrics {
    pub url: String,
    pub state: Shared,
}

/// A fake accepting `token` (or any request when `None`), on a fresh local port.
pub async fn spawn(token: Option<&str>) -> FakeMetrics {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let state = Shared(Arc::new(Mutex::new(Fake {
        base: url.clone(),
        token: token.map(str::to_string),
        rules: Vec::new(),
        status: None,
        oversize: false,
        calls: Vec::new(),
    })));
    let app = Router::new()
        .route("/api/v1/query", get(answer))
        .route("/api/0/organizations/{org}/events/", get(answer))
        .with_state(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    FakeMetrics { url, state }
}

async fn answer(State(state): State<Shared>, headers: HeaderMap, uri: Uri, Query(params): Query<HashMap<String, String>>) -> Response {
    let authorization = headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(str::to_string);
    let mut f = state.lock();
    let url = format!("{}{}", f.base, uri);
    f.calls.push(Call { url, authorization: authorization.clone() });
    if let Some(token) = &f.token {
        let basic = base64::engine::general_purpose::STANDARD.encode(token);
        let accepted = authorization.as_deref().is_some_and(|a| a == format!("Bearer {token}") || a == format!("Basic {basic}"));
        if !accepted {
            return (StatusCode::UNAUTHORIZED, Json(json!({ "status": "error", "error": "unauthorized" }))).into_response();
        }
    }
    if f.oversize {
        let chunks = (0..3).map(|_| Ok::<Bytes, Infallible>(Bytes::from(vec![b' '; 512 * 1024])));
        return Response::new(Body::from_stream(futures_util::stream::iter(chunks)));
    }
    let query = params.get("query").map(String::as_str).unwrap_or_default();
    match f.rules.iter().find(|(fragment, _)| query.contains(fragment.as_str())) {
        Some((_, body)) => {
            let code = StatusCode::from_u16(f.status.unwrap_or(200)).unwrap();
            (code, Json(body.clone())).into_response()
        }
        None => (StatusCode::NOT_FOUND, Json(json!({ "status": "error", "error": "no scripted answer" }))).into_response(),
    }
}
