//! The GitHub App side of a fake GitHub: the OAuth code exchange, the installations a person can
//! see, the repositories an installation reaches, and installation tokens minted against an app JWT.
//! The repository API itself is `fakehost`, which accepts only the token minted here.

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Debug)]
pub struct AppFake {
    pub app_id: i64,
    pub client_secret: String,
    /// What the repository API accepts (the token an installation mint hands out).
    pub minted_token: String,
    pub installation: i64,
    pub account: String,
    pub removed: bool,
    pub suspended: bool,
    pub code: String,
    pub user_token: String,
    /// The PKCE challenge the browser sent to the authorize page (the test copies it here).
    pub challenge: Option<String>,
    /// `(id, full_name, can_push)` of the repositories the installation reaches.
    pub repos: Vec<(i64, String, bool)>,
    /// `repository_ids` of every installation token minted.
    pub mints: Vec<Vec<i64>>,
    pub jwt_issuers: Vec<i64>,
}

pub type AppShared = Arc<Mutex<AppFake>>;

pub fn state(minted_token: &str) -> AppShared {
    Arc::new(Mutex::new(AppFake {
        app_id: 4242,
        client_secret: "shh".into(),
        minted_token: minted_token.into(),
        installation: 42,
        account: "acme".into(),
        removed: false,
        suspended: false,
        code: "the-code".into(),
        user_token: "ghu_user".into(),
        challenge: None,
        repos: vec![(501, "acme/api".into(), true), (502, "acme/web".into(), true)],
        mints: Vec::new(),
        jwt_issuers: Vec::new(),
    }))
}

fn bearer(h: &HeaderMap) -> String {
    h.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or_default().to_string()
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, axum::Json(json!({ "message": "Bad credentials" }))).into_response()
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, axum::Json(json!({ "message": "Not Found" }))).into_response()
}

/// The app id a JWT claims (`iss`), when it is shaped like one.
fn jwt_issuer(token: &str) -> Option<i64> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let claims: Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).ok()?).ok()?;
    claims["iss"].as_i64()
}

fn installation_json(f: &AppFake) -> Value {
    json!({
        "id": f.installation, "account": { "login": f.account, "id": 7, "type": "Organization" },
        "repository_selection": "selected",
        "permissions": { "contents": "write", "pull_requests": "write", "metadata": "read" },
        "suspended_at": if f.suspended { json!("2026-10-25T00:00:00Z") } else { Value::Null },
    })
}

pub fn routes(s: AppShared) -> Router {
    Router::new()
        .route(
            "/login/oauth/access_token",
            post(|State(s): State<AppShared>, body: String| async move {
                let form: HashMap<String, String> =
                    reqwest::Url::parse(&format!("http://x/?{body}")).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
                let f = s.lock().unwrap();
                let get = |k: &str| form.get(k).cloned().unwrap_or_default();
                let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(get("code_verifier").as_bytes()));
                let ok = get("client_id") == "cid"
                    && get("client_secret") == f.client_secret
                    && get("code") == f.code
                    && f.challenge.as_deref() == Some(challenge.as_str());
                if ok {
                    axum::Json(json!({ "access_token": f.user_token, "token_type": "bearer" })).into_response()
                } else {
                    axum::Json(json!({ "error": "bad_verification_code" })).into_response()
                }
            }),
        )
        .route(
            "/api/v3/user/installations",
            get(|State(s): State<AppShared>, h: HeaderMap| async move {
                let f = s.lock().unwrap();
                if bearer(&h) != f.user_token {
                    return unauthorized();
                }
                let list = if f.removed { Vec::new() } else { vec![installation_json(&f)] };
                axum::Json(json!({ "total_count": list.len(), "installations": list })).into_response()
            }),
        )
        .route(
            "/api/v3/user/installations/{id}/repositories",
            get(|State(s): State<AppShared>, h: HeaderMap, Path(id): Path<i64>| async move {
                let f = s.lock().unwrap();
                if bearer(&h) != f.user_token {
                    return unauthorized();
                }
                if f.removed || id != f.installation {
                    return not_found();
                }
                let repos: Vec<Value> = f
                    .repos
                    .iter()
                    .map(|(i, n, push)| json!({ "id": i, "full_name": n, "default_branch": "main", "private": true, "permissions": { "push": push } }))
                    .collect();
                axum::Json(json!({ "total_count": repos.len(), "repositories": repos })).into_response()
            }),
        )
        .route(
            "/api/v3/app/installations/{id}",
            get(|State(s): State<AppShared>, h: HeaderMap, Path(id): Path<i64>| async move {
                let mut f = s.lock().unwrap();
                match jwt_issuer(&bearer(&h)) {
                    Some(iss) => f.jwt_issuers.push(iss),
                    None => return unauthorized(),
                }
                if f.removed || id != f.installation {
                    return not_found();
                }
                axum::Json(installation_json(&f)).into_response()
            }),
        )
        .route(
            "/api/v3/app/installations/{id}/access_tokens",
            post(|State(s): State<AppShared>, h: HeaderMap, Path(id): Path<i64>, axum::Json(b): axum::Json<Value>| async move {
                let mut f = s.lock().unwrap();
                match jwt_issuer(&bearer(&h)) {
                    Some(iss) => f.jwt_issuers.push(iss),
                    None => return unauthorized(),
                }
                if f.removed || id != f.installation {
                    return not_found();
                }
                if f.suspended {
                    return (StatusCode::FORBIDDEN, axum::Json(json!({ "message": "This installation has been suspended" }))).into_response();
                }
                let ids: Vec<i64> = b["repository_ids"].as_array().map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
                f.mints.push(ids);
                let expires = (chrono::Utc::now() + chrono::Duration::hours(1)).to_rfc3339();
                (StatusCode::CREATED, axum::Json(json!({ "token": f.minted_token, "expires_at": expires }))).into_response()
            }),
        )
        .with_state(s)
}

/// A PKCS#1 PEM key of the form GitHub hands out, written where the config points.
pub fn private_key(dir: &std::path::Path) -> String {
    use aws_lc_rs::encoding::AsDer;
    use base64::Engine;
    let key = aws_lc_rs::signature::RsaKeyPair::generate(aws_lc_rs::rsa::KeySize::Rsa2048).unwrap();
    let pkcs8 = AsDer::<aws_lc_rs::encoding::Pkcs8V1Der>::as_der(&key).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&pkcs8.as_ref()[26..]);
    let lines: Vec<String> = b64.as_bytes().chunks(64).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
    let path = dir.join("app.pem");
    std::fs::write(&path, format!("-----BEGIN RSA PRIVATE KEY-----\n{}\n-----END RSA PRIVATE KEY-----\n", lines.join("\n"))).unwrap();
    path.to_string_lossy().into_owned()
}
