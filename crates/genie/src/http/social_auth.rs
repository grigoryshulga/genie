//! Browser-bound authorization code flow. Provider tokens never leave the server.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use genie_core::server_db::{SESSION_DAYS, hash_secret, new_secret};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use ts_rs::TS;

use super::ctx::{Ctx, SESSION_COOKIE};
use super::{ApiError, ApiResult};
use crate::config::SocialClient as Client;
use crate::state::App;

const FLOW_COOKIE: &str = "genie_social_flow";
const TTL: Duration = Duration::from_secs(600);

struct Provider<'a> {
    id: &'a str,
    title: &'a str,
    authorize: &'a str,
    token: &'a str,
    profile: &'a str,
    scope: &'a str,
}

const PROVIDERS: [Provider<'static>; 2] = [
    Provider {
        id: "github",
        title: "GitHub",
        authorize: "https://github.com/login/oauth/authorize",
        token: "https://github.com/login/oauth/access_token",
        profile: "https://api.github.com/user",
        scope: "read:user",
    },
    Provider {
        id: "google",
        title: "Google",
        authorize: "https://accounts.google.com/o/oauth2/v2/auth",
        token: "https://oauth2.googleapis.com/token",
        profile: "https://openidconnect.googleapis.com/v1/userinfo",
        scope: "openid profile",
    },
];

#[derive(Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SocialProvider {
    pub id: String,
    pub title: String,
    pub connected: bool,
    pub registration: bool,
}

struct Flow {
    provider: String,
    browser_hash: String,
    verifier: String,
    session: Option<String>,
    created: Instant,
}

#[derive(Default)]
pub struct Registry(Mutex<HashMap<String, Flow>>);

impl Registry {
    fn insert(&self, state: &str, flow: Flow) -> ApiResult<()> {
        let mut flows = self.0.lock().map_err(|_| ApiError::bad("sign-in registry unavailable"))?;
        flows.retain(|_, f| f.created.elapsed() < TTL);
        if flows.len() >= 1024 {
            return Err(ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too many sign-in attempts; try later"));
        }
        flows.insert(hash_secret(state), flow);
        Ok(())
    }

    fn take(&self, state: &str, browser: &str, provider: &str) -> ApiResult<Flow> {
        let mut flows = self.0.lock().map_err(|_| ApiError::bad("sign-in registry unavailable"))?;
        let key = hash_secret(state);
        let valid =
            flows.get(&key).is_some_and(|f| f.created.elapsed() < TTL && f.browser_hash == hash_secret(browser) && f.provider == provider);
        if !valid {
            return Err(ApiError::bad("sign-in expired or browser did not match; start again"));
        }
        Ok(flows.remove(&key).expect("flow checked under lock"))
    }
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/auth/social", get(providers))
        .route("/auth/social/{provider}/start", post(start))
        .route("/auth/social/{provider}/callback", get(callback))
}

fn configured<'a>(app: &'a App, id: &str) -> ApiResult<(&'static Provider<'static>, &'a Client, String)> {
    let provider = PROVIDERS.iter().find(|p| p.id == id).ok_or_else(|| ApiError::bad("unknown sign-in provider"))?;
    let client = match id {
        "github" => &app.cfg.social_auth.github,
        _ => &app.cfg.social_auth.google,
    }
    .as_ref()
    .ok_or_else(|| ApiError::bad("sign-in provider is not configured"))?;
    let secret = std::env::var(&client.client_secret_env)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| ApiError::bad("sign-in provider credentials are unavailable"))?;
    if client.client_id.trim().is_empty() {
        return Err(ApiError::bad("sign-in provider client ID is missing"));
    }
    Ok((provider, client, secret))
}

pub(super) fn origin(app: &App) -> ApiResult<reqwest::Url> {
    let url = app
        .cfg
        .public_url
        .as_deref()
        .and_then(|s| reqwest::Url::parse(s).ok())
        .ok_or_else(|| ApiError::bad("social sign-in requires publicUrl"))?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ApiError::bad("publicUrl must be an HTTPS origin (HTTP is allowed only on loopback)"));
    }
    Ok(url)
}

fn callback_url(app: &App, provider: &str) -> ApiResult<String> {
    Ok(origin(app)?.join(&format!("/api/auth/social/{provider}/callback")).map_err(|_| ApiError::bad("invalid callback URL"))?.to_string())
}

pub(super) fn cookie(name: &str, value: &str, age: i64, secure: bool, same_site: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; HttpOnly; SameSite={same_site}; Max-Age={age}{}",
        if secure { "; Secure" } else { "" }
    ))
    .expect("generated cookie contains only ASCII")
}

async fn providers(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Vec<SocialProvider>>> {
    let connected = if let Ok(user) = ctx.user() {
        let id = user.id;
        app.blocking(move |app| app.with_server(|db| db.social_providers(id))).await?
    } else {
        Vec::new()
    };
    let valid_origin = origin(&app).is_ok();
    Ok(Json(
        PROVIDERS
            .iter()
            .filter(|p| valid_origin && configured(&app, p.id).is_ok())
            .map(|p| SocialProvider {
                id: p.id.into(),
                title: p.title.into(),
                connected: connected.iter().any(|id| id == p.id),
                registration: app.cfg.social_auth.allow_registration,
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
pub struct StartBody {
    #[serde(default)]
    link: bool,
}

async fn start(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Json(body): Json<StartBody>) -> ApiResult<Response> {
    let (provider, client, _) = configured(&app, &id)?;
    let redirect = callback_url(&app, &id)?;
    let session = if body.link {
        let user = ctx.user()?;
        if user.id <= 0 {
            return Err(ApiError::bad("create an account before connecting a provider"));
        }
        Some(ctx.session.ok_or_else(|| ApiError::bad("linking requires a browser session"))?)
    } else {
        None
    };
    let state = new_secret();
    let browser = new_secret();
    let verifier = new_secret();
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let mut url = reqwest::Url::parse(provider.authorize).expect("fixed provider URL");
    url.query_pairs_mut().extend_pairs([
        ("client_id", client.client_id.as_str()),
        ("redirect_uri", &redirect),
        ("response_type", "code"),
        ("scope", provider.scope),
        ("state", &state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ]);
    app.social_auth
        .insert(&state, Flow { provider: id, browser_hash: hash_secret(&browser), verifier, session, created: Instant::now() })?;
    let mut response = Json(json!({ "url": url.as_str() })).into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, cookie(FLOW_COOKIE, &browser, TTL.as_secs() as i64, origin(&app)?.scheme() == "https", "Lax"));
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

#[derive(Deserialize)]
struct Callback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}

async fn callback(State(app): State<Arc<App>>, Path(id): Path<String>, Query(query): Query<Callback>, headers: HeaderMap) -> Response {
    let linking =
        app.social_auth.0.lock().ok().and_then(|flows| flows.get(&hash_secret(&query.state)).map(|f| f.session.is_some())).unwrap_or(false);
    let result = complete(&app, &id, query, &headers).await;
    // Keep provider errors and credentials out of URLs, HTML and logs.
    let (destination, session) = match result {
        Ok(session) => (if linking { "/settings/me/security" } else { "/" }, Some(session)),
        Err(_) => (if linking { "/settings/me/security?socialError=1" } else { "/?socialError=1" }, None),
    };
    let secure = origin(&app).is_ok_and(|u| u.scheme() == "https");
    let mut response = Redirect::to(destination).into_response();
    response.headers_mut().append(header::SET_COOKIE, cookie(FLOW_COOKIE, "", 0, secure, "Lax"));
    if let Some(session) = session {
        response.headers_mut().append(header::SET_COOKIE, cookie(SESSION_COOKIE, &session, SESSION_DAYS * 86400, secure, "Strict"));
    }
    response.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

async fn complete(app: &Arc<App>, id: &str, query: Callback, headers: &HeaderMap) -> ApiResult<String> {
    let browser = headers
        .get(header::COOKIE)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.split(';').find_map(|c| c.trim().strip_prefix("genie_social_flow=")))
        .ok_or_else(|| ApiError::bad("missing sign-in cookie"))?;
    let flow = app.social_auth.take(&query.state, browser, id)?;
    if query.error.is_some() {
        return Err(ApiError::bad("provider sign-in cancelled"));
    }
    let code = query.code.filter(|c| !c.is_empty()).ok_or_else(|| ApiError::bad("missing authorization code"))?;
    let (provider, client, secret) = configured(app, id)?;
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("genie-social-auth")
        .build()
        .map_err(|_| ApiError::bad("provider client unavailable"))?;
    let (subject, name) = fetch_identity(&http, provider, client, &secret, &code, &callback_url(app, id)?, &flow.verifier).await?;
    let id = id.to_string();
    let registration = app.cfg.social_auth.allow_registration;
    app.blocking(move |app| {
        app.with_server(|db| {
            let link_user = if let Some(session) = flow.session {
                Some(
                    db.session_user(&session)?
                        .ok_or_else(|| genie_core::GenieError::Denied("linking session expired; sign in again".into()))?
                        .id,
                )
            } else {
                None
            };
            db.social_login(&id, &subject, &name, registration, link_user).map(|(session, _)| session)
        })
    })
    .await
    .map_err(Into::into)
}

async fn fetch_identity(
    http: &reqwest::Client,
    provider: &Provider<'_>,
    client: &Client,
    secret: &str,
    code: &str,
    redirect: &str,
    verifier: &str,
) -> ApiResult<(String, String)> {
    let token = provider_json(
        http.post(provider.token)
            .header(header::ACCEPT, "application/json")
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", client.client_id.as_str()),
                ("client_secret", secret),
                ("code", code),
                ("redirect_uri", redirect),
                ("code_verifier", verifier),
            ])
            .send()
            .await
            .map_err(|_| ApiError::bad("provider token exchange failed"))?
            .error_for_status()
            .map_err(|_| ApiError::bad("provider rejected authorization code"))?,
    )
    .await?;
    let access =
        token["access_token"].as_str().filter(|s| !s.is_empty()).ok_or_else(|| ApiError::bad("provider did not return an access token"))?;
    // Only trust a profile fetched with the code's token from this provider's fixed HTTPS API.
    // ID tokens are not used as authentication assertions in this integration.
    let profile = provider_json(
        http.get(provider.profile)
            .bearer_auth(access)
            .send()
            .await
            .map_err(|_| ApiError::bad("provider profile unavailable"))?
            .error_for_status()
            .map_err(|_| ApiError::bad("provider rejected profile request"))?,
    )
    .await?;
    identity(provider.id, &profile)
}

async fn provider_json(mut response: reqwest::Response) -> ApiResult<Value> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ApiError::bad("cannot read provider response"))? {
        if bytes.len() + chunk.len() > 64 * 1024 {
            return Err(ApiError::bad("provider response is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("invalid provider response"))
}

fn identity(provider: &str, profile: &Value) -> ApiResult<(String, String)> {
    let subject = match provider {
        "github" => profile["id"].as_u64().filter(|n| *n > 0).map(|n| n.to_string()),
        "google" => profile["sub"].as_str().filter(|s| !s.is_empty()).map(str::to_owned),
        _ => None,
    }
    .filter(|s| s.len() <= 255)
    .ok_or_else(|| ApiError::bad("provider profile has no stable identity"))?;
    let name = profile["name"].as_str().or_else(|| profile["login"].as_str()).unwrap_or(provider).chars().take(200).collect();
    Ok((subject, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_is_expiring_single_use_and_bound_to_browser_and_provider() {
        let registry = Registry::default();
        let make = |created| Flow {
            provider: "github".into(),
            browser_hash: hash_secret("browser"),
            verifier: "v".into(),
            session: None,
            created,
        };
        registry.insert("s", make(Instant::now())).unwrap();
        assert!(registry.take("s", "other", "github").is_err());
        assert!(registry.take("s", "browser", "google").is_err());
        registry.take("s", "browser", "github").unwrap();
        assert!(registry.take("s", "browser", "github").is_err());
        registry.insert("old", make(Instant::now() - TTL)).unwrap();
        assert!(registry.take("old", "browser", "github").is_err());
    }

    #[test]
    fn profile_uses_immutable_id_never_email_or_login() {
        assert_eq!(identity("github", &json!({"id": 42, "login": "new-name"})).unwrap().0, "42");
        assert_eq!(identity("google", &json!({"sub": "123", "email": "changed@example.com"})).unwrap().0, "123");
        assert!(identity("github", &json!({"login": "name"})).is_err());
        assert!(identity("google", &json!({"email": "same@example.com"})).is_err());
    }
    #[tokio::test]
    async fn exchanges_code_with_verifier_and_uses_only_authenticated_profile() {
        let mock = Router::new()
            .route(
                "/token",
                post(|headers: HeaderMap, body: String| async move {
                    assert_eq!(headers[header::ACCEPT], "application/json");
                    assert!(body.contains("client_secret=test-secret") && body.contains("code_verifier=test-verifier"));
                    if body.contains("code=good-code") {
                        Json(json!({"access_token":"test-access", "id_token":"ignored-untrusted-claims"}))
                    } else {
                        Json(json!({"error":"invalid_grant"}))
                    }
                }),
            )
            .route(
                "/profile",
                get(|headers: HeaderMap| async move {
                    assert_eq!(headers[header::AUTHORIZATION], "Bearer test-access");
                    Json(json!({"id":42,"name":"Test person"}))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, mock).await.unwrap();
        });
        let token = format!("{base}/token");
        let profile = format!("{base}/profile");
        let provider = Provider { token: &token, profile: &profile, ..PROVIDERS[0] };
        let client = Client { client_id: "test-client".into(), client_secret_env: "unused".into() };
        let http = reqwest::Client::new();
        let result =
            fetch_identity(&http, &provider, &client, "test-secret", "good-code", "https://genie.example/callback", "test-verifier")
                .await
                .unwrap();
        assert_eq!(result, ("42".into(), "Test person".into()));
        assert!(
            fetch_identity(&http, &provider, &client, "test-secret", "bad-code", "https://genie.example/callback", "test-verifier")
                .await
                .is_err()
        );
        server.abort();
    }
}
