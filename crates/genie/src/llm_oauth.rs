//! Per-user Pi OAuth sessions. The Pi extension handles provider-specific flows;
//! this module bridges its RPC UI requests to the signed-in user and stores only
//! the resulting auth record in the encrypted server secret store.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::llm_key::{self, Provider};
use crate::state::App;

const SESSION_TTL: Duration = Duration::from_secs(10 * 60);

/// Remove temporary Pi profiles left behind if the server stopped mid-login.
pub fn cleanup_stale(data: &Path) {
    let _ = std::fs::remove_dir_all(data.join("runtime").join("llm-oauth"));
}

/// Remove user profiles whose accounts have been deleted. Active profiles are
/// retained because Pi may have rotated an OAuth refresh token in auth.json.
pub fn cleanup_orphaned_profiles(data: &Path, users: &[i64]) {
    let root = data.join("runtime").join("pi-users");
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for entry in entries.flatten() {
        let Some(id) = entry.file_name().to_str().and_then(|name| name.parse::<i64>().ok()) else { continue };
        if !users.contains(&id) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[derive(Clone, Default)]
pub struct Registry {
    sessions: Arc<Mutex<HashMap<String, Arc<OAuthSession>>>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Authorization {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    pub id: String,
    pub method: String,
    pub title: String,
    pub secret: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub status: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorization: Option<Authorization>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Prompt>,
}

struct OAuthSession {
    id: String,
    user: i64,
    provider: &'static Provider,
    state: Mutex<Snapshot>,
    writer: Mutex<Option<mpsc::UnboundedSender<Value>>>,
    child: tokio::sync::Mutex<Option<Child>>,
    directory: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseBody {
    pub id: String,
    pub value: Option<String>,
    pub confirmed: Option<bool>,
    pub cancelled: Option<bool>,
}

impl Registry {
    pub async fn start(&self, app: &Arc<App>, user: i64, provider: &'static Provider) -> Result<String, String> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        if self.sessions.lock().unwrap_or_else(|e| e.into_inner()).values().any(|session| {
            session.user == user
                && session.provider.id == provider.id
                && matches!(session.state.lock().unwrap_or_else(|e| e.into_inner()).status.as_str(), "starting" | "waiting")
        }) {
            return Err(format!("OAuth for {} is already running", provider.title));
        }
        let id = format!("{}-{}-{}", std::process::id(), user, NEXT.fetch_add(1, Ordering::Relaxed));
        let directory = app.data.join("runtime").join("llm-oauth").join(&id);
        std::fs::create_dir_all(&directory).map_err(|e| format!("cannot create OAuth work directory: {e}"))?;
        restrict_dir(&directory);
        for name in ["models.json"] {
            let source = crate::runtime::pi_dir(app).join(name);
            if source.is_file() {
                if let Err(e) = std::fs::copy(source, directory.join(name)) {
                    let _ = std::fs::remove_dir_all(&directory);
                    return Err(format!("cannot prepare Pi profile: {e}"));
                }
            }
        }
        let extension = directory.join("genie-oauth.ts");
        if let Err(e) = std::fs::write(&extension, include_str!("../pi/genie-oauth.ts")) {
            let _ = std::fs::remove_dir_all(&directory);
            return Err(format!("cannot prepare Pi OAuth extension: {e}"));
        }
        let Some(pi) = crate::doctor::pi_program(&app.cfg) else {
            let _ = std::fs::remove_dir_all(&directory);
            return Err("Pi executable is not available".into());
        };
        let mut child = match Command::new(pi)
            .arg("-e")
            .arg(&extension)
            .args(["--mode", "rpc", "--no-session"])
            .env("PI_CODING_AGENT_DIR", &directory)
            .env("GENIE_PROVIDER", provider.id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&directory);
                return Err(format!("cannot start Pi OAuth: {e}"));
            }
        };
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.start_kill();
            let _ = std::fs::remove_dir_all(&directory);
            return Err("Pi OAuth pipes are unavailable".into());
        };
        let (writer, mut commands) = mpsc::unbounded_channel::<Value>();
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(value) = commands.recv().await {
                let Ok(mut line) = serde_json::to_vec(&value) else { continue };
                line.push(b'\n');
                if stdin.write_all(&line).await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
        });
        let session = Arc::new(OAuthSession {
            id: id.clone(),
            user,
            provider,
            state: Mutex::new(Snapshot {
                status: "starting".into(),
                message: "Запускаем OAuth-подключение…".into(),
                authorization: None,
                prompt: None,
            }),
            writer: Mutex::new(Some(writer.clone())),
            child: tokio::sync::Mutex::new(Some(child)),
            directory,
        });
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).insert(id.clone(), session.clone());

        let app = app.clone();
        let current = session.clone();
        tokio::spawn(async move { read_session(app, current, stdout).await });
        let _ = writer.send(json!({ "type": "prompt", "message": format!("/genie-oauth-connect {}", provider.id) }));

        let registry = self.clone();
        let expiring = session.clone();
        tokio::spawn(async move {
            tokio::time::sleep(SESSION_TTL).await;
            let expired = {
                let mut state = expiring.state.lock().unwrap_or_else(|e| e.into_inner());
                if state.status == "starting" || state.status == "waiting" {
                    state.status = "error".into();
                    state.message = "Время подключения истекло. Начните OAuth ещё раз.".into();
                    true
                } else {
                    false
                }
            };
            if expired {
                stop(&expiring).await;
                cleanup(&expiring);
            }
            tokio::time::sleep(Duration::from_secs(10 * 60)).await;
            registry.sessions.lock().unwrap_or_else(|e| e.into_inner()).remove(&expiring.id);
        });
        Ok(id)
    }

    pub fn snapshot(&self, user: i64, id: &str) -> Option<Snapshot> {
        let session = self.session(user, id)?;
        Some(session.state.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    pub fn respond(&self, user: i64, session_id: &str, body: ResponseBody) -> Result<(), String> {
        let session = self.session(user, session_id).ok_or_else(|| "OAuth session not found".to_string())?;
        let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
        let prompt = state.prompt.as_ref().filter(|prompt| prompt.id == body.id).ok_or_else(|| "OAuth prompt has expired".to_string())?;
        let mut response = json!({ "type": "extension_ui_response", "id": body.id });
        if body.cancelled.unwrap_or(false) {
            response["cancelled"] = json!(true);
        } else {
            match prompt.method.as_str() {
                "confirm" => response["confirmed"] = json!(body.confirmed.unwrap_or(false)),
                "select" | "input" | "editor" => {
                    let value = body.value.unwrap_or_default();
                    if value.len() > 4096 {
                        return Err("OAuth response is too long".into());
                    }
                    if prompt.method == "select" && !prompt.options.as_ref().is_some_and(|options| options.contains(&value)) {
                        return Err("Choose one of the listed options".into());
                    }
                    response["value"] = json!(value);
                }
                _ => return Err("Unsupported OAuth prompt".into()),
            }
        }
        state.prompt = None;
        state.status = "waiting".into();
        state.message = "Ожидаем ответ провайдера…".into();
        drop(state);
        session
            .writer
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .ok_or_else(|| "Pi OAuth process has stopped".to_string())?
            .send(response)
            .map_err(|_| "Pi OAuth process has stopped".to_string())
    }

    pub async fn cancel(&self, user: i64, id: &str) -> Result<(), String> {
        let session = self.session(user, id).ok_or_else(|| "OAuth session not found".to_string())?;
        {
            let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
            if state.status == "complete" || state.status == "error" {
                return Ok(());
            }
            state.status = "cancelled".into();
            state.message = "Подключение отменено".into();
            state.prompt = None;
        }
        stop(&session).await;
        cleanup(&session);
        Ok(())
    }

    pub async fn disconnect(&self, app: &Arc<App>, user: i64, provider: &'static Provider) -> Result<(), String> {
        let name = llm_key::oauth_secret(provider.id);
        app.blocking(move |app| app.with_server(|db| db.delete_user_secret(user, &name))).await.map_err(|e| e.to_string())?;
        crate::llm_key::refresh_user_pi_profile(app, user)?;
        crate::http::teams::invalidate_models(Some(user));
        Ok(())
    }

    fn session(&self, user: i64, id: &str) -> Option<Arc<OAuthSession>> {
        self.sessions.lock().unwrap_or_else(|e| e.into_inner()).get(id).filter(|s| s.user == user).cloned()
    }
}

async fn read_session(app: Arc<App>, session: Arc<OAuthSession>, stdout: tokio::process::ChildStdout) {
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
        match value["type"].as_str().unwrap_or_default() {
            "extension_ui_request" => handle_ui_request(&session, &value),
            "genie_oauth_authorization" => handle_authorization(&session, &value),
            "genie_oauth_progress" => {
                let message = safe_text(value["message"].as_str().unwrap_or_default());
                if !message.is_empty() {
                    let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
                    state.status = "waiting".into();
                    state.message = message;
                }
            }
            "genie_oauth_error" => {
                set_state(&session, "error", safe_text(value["message"].as_str().unwrap_or("OAuth не завершён")), None, None);
                stop(&session).await;
                cleanup(&session);
                return;
            }
            "genie_oauth_result" => {
                if let Err(message) = store_result(&app, &session, &value).await {
                    set_state(&session, "error", message, None, None);
                } else {
                    set_state(&session, "complete", "Провайдер подключён".into(), None, None);
                }
                stop(&session).await;
                cleanup(&session);
                return;
            }
            _ => {}
        }
    }
    let status = session.state.lock().unwrap_or_else(|e| e.into_inner()).status.clone();
    if status == "starting" || status == "waiting" {
        set_state(&session, "error", "Pi завершил OAuth до сохранения учётных данных".into(), None, None);
    }
    stop(&session).await;
    cleanup(&session);
}

fn handle_ui_request(session: &OAuthSession, value: &Value) {
    let method = value["method"].as_str().unwrap_or_default();
    if method == "notify" {
        let message = value["message"].as_str().map(safe_text).unwrap_or_default();
        if !message.is_empty() {
            let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
            state.status = "waiting".into();
            state.message = message;
        }
        return;
    }
    if !matches!(method, "input" | "select" | "confirm" | "editor") {
        return;
    }
    let Some(id) = value["id"].as_str().filter(|id| id.len() < 128).map(str::to_string) else { return };
    let options = value["options"].as_array().map(|items| items.iter().filter_map(Value::as_str).map(safe_text).collect::<Vec<_>>());
    let raw_title = value["title"].as_str().filter(|s| !s.is_empty()).unwrap_or("Подтверждение OAuth");
    let secret = raw_title.starts_with("[genie-secret] ");
    let title = raw_title.strip_prefix("[genie-secret] ").unwrap_or(raw_title);
    let prompt = Prompt {
        id: id.clone(),
        method: method.into(),
        title: safe_text(title),
        secret,
        message: value["message"].as_str().map(safe_text),
        placeholder: value["placeholder"].as_str().map(safe_text),
        options,
    };
    let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
    state.status = "waiting".into();
    state.message = "Нужен ваш ответ, чтобы продолжить вход".into();
    state.prompt = Some(prompt);
}

fn handle_authorization(session: &OAuthSession, value: &Value) {
    let url = value["url"].as_str().filter(|url| url.starts_with("https://") || url.starts_with("http://")).map(safe_text);
    let authorization =
        Authorization { url, code: value["code"].as_str().map(safe_text), instructions: value["instructions"].as_str().map(safe_text) };
    let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
    state.status = "waiting".into();
    state.message = "Откройте страницу провайдера и завершите вход".into();
    state.authorization = Some(authorization);
}

async fn store_result(app: &Arc<App>, session: &OAuthSession, value: &Value) -> Result<(), String> {
    if value["provider"].as_str() != Some(session.provider.id) {
        return Err("Pi вернул данные не того провайдера".into());
    }
    let auth = value.get("auth").filter(|value| value.is_object()).ok_or_else(|| "Pi не вернул данные OAuth".to_string())?;
    let encoded = serde_json::to_string(auth).map_err(|e| e.to_string())?;
    let user = session.user;
    let name = llm_key::oauth_secret(session.provider.id);
    crate::llm_key::sync_user_pi_profile(app, user)?;
    app.blocking(move |app| app.with_server(|db| db.set_user_secret(user, &name, &encoded))).await.map_err(|e| e.to_string())?;
    crate::llm_key::write_user_pi_profile(app, user)?;
    crate::http::teams::invalidate_models(Some(user));
    Ok(())
}

fn set_state(session: &OAuthSession, status: &str, message: String, authorization: Option<Authorization>, prompt: Option<Prompt>) {
    let mut state = session.state.lock().unwrap_or_else(|e| e.into_inner());
    state.status = status.into();
    state.message = message;
    state.authorization = authorization;
    state.prompt = prompt;
}

async fn stop(session: &OAuthSession) {
    session.writer.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(mut child) = session.child.lock().await.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

fn cleanup(session: &OAuthSession) {
    let _ = std::fs::remove_dir_all(&session.directory);
}

fn safe_text(text: &str) -> String {
    text.replace(['\n', '\r'], " ").chars().take(2000).collect()
}

#[cfg(unix)]
fn restrict_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict_dir(_path: &Path) {}
