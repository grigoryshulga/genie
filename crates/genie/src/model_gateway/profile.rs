//! The model metadata and private Pi profile used by bounded provider calls.
//! Metadata is read from the trusted harness registry without starting a model.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::llm_key::Keys;
use crate::state::App;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Model {
    pub provider: String,
    pub id: String,
    pub api: String,
    pub base_url: String,
    pub context_window: u64,
    pub max_tokens: u64,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub compat: Value,
    #[serde(default)]
    pub headers: Value,
    #[serde(default)]
    pub thinking_level_map: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Profile {
    pub model: Model,
    pub source: PathBuf,
}

pub(super) async fn resolve(app: &App, model: &str, keys: &Keys, dir: &Path) -> Result<Profile, String> {
    let pi = crate::doctor::pi_program(&app.cfg).ok_or("monetary budgets require the supported Pi harness")?;
    let source = match keys {
        Keys::Person { pi_dir: Some(dir), .. } => dir.clone(),
        _ => crate::runtime::pi_dir(app),
    };
    let (provider, id) = model.split_once('/').ok_or("bounded models must use an exact provider/model ID")?;
    let mut command = tokio::process::Command::new(pi);
    command
        .args([
            "--mode",
            "rpc",
            "--no-session",
            "--no-extensions",
            "--no-skills",
            "--no-context-files",
            "--no-prompt-templates",
            "--no-mcp",
            "--no-approve",
            "--offline",
        ])
        .current_dir(dir)
        .env("PI_CODING_AGENT_DIR", &source)
        .env("PI_OFFLINE", "1")
        .env("PI_SKIP_VERSION_CHECK", "1")
        .env("PI_TELEMETRY", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    for (name, value) in &app.cfg.runtime.env {
        command.env(name, value);
    }
    keys.apply(&mut command);
    command.env("PI_OFFLINE", "1").env("PI_SKIP_VERSION_CHECK", "1").env("PI_TELEMETRY", "0");
    // No mutable server/profile setting may turn this registry query into a
    // provider call or load an extension while the server holds credentials.
    for (name, _) in std::env::vars().chain(app.cfg.runtime.env.clone()) {
        if name.starts_with("PI_JEV_") {
            command.env_remove(name);
        }
    }
    let mut child = command.spawn().map_err(|_| "cannot start the model metadata query")?;
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let mut input = child.stdin.take().ok_or("model metadata query has no input")?;
        input
            .write_all(b"{\"id\":\"genie-models\",\"type\":\"get_available_models\"}\n")
            .await
            .map_err(|_| "cannot query model metadata")?;
        input.flush().await.map_err(|_| "cannot query model metadata")?;
        let output = child.stdout.take().ok_or("model metadata query has no output")?;
        let mut lines = BufReader::new(output).lines();
        while let Some(line) = lines.next_line().await.map_err(|_| "cannot read model metadata")? {
            let Ok(reply) = serde_json::from_str::<Value>(&line) else { continue };
            if reply["id"] != "genie-models" {
                continue;
            }
            let selected = reply["data"]["models"]
                .as_array()
                .and_then(|models| models.iter().find(|m| m["provider"] == provider && m["id"] == id))
                .ok_or("the configured model is absent from the trusted harness registry")?;
            let model = serde_json::from_value::<Model>(selected.clone()).map_err(|_| "the harness supplied incomplete model bounds")?;
            return Ok::<_, String>(Profile { model, source });
        }
        Err("model metadata query ended without an answer".into())
    })
    .await
    .map_err(|_| "model metadata query timed out".to_string());
    let _ = child.kill().await;
    let _ = child.wait().await;
    let profile = result??;
    profile.validate()?;
    Ok(profile)
}

impl Profile {
    pub(super) fn validate(&self) -> Result<(), String> {
        let m = &self.model;
        if m.api != "openai-completions" {
            return Err(format!("bounded provider calls do not yet support {}", m.api));
        }
        if m.provider == "github-copilot" {
            return Err("the bounded profile does not support Copilot's additional authentication headers".into());
        }
        let url = reqwest::Url::parse(&m.base_url).map_err(|_| "invalid upstream model URL")?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path().contains("/agent/model-gateway/")
        {
            return Err("bounded model URLs cannot contain credentials, queries or a recursive gateway".into());
        }
        if m.provider.is_empty()
            || m.id.is_empty()
            || m.context_window == 0
            || m.max_tokens == 0
            || m.max_tokens > m.context_window
            || m.context_window > i64::MAX as u64
        {
            return Err("the model needs valid context and output bounds".into());
        }
        if m.headers.as_object().is_some_and(|headers| !headers.is_empty()) || (!m.headers.is_null() && !m.headers.is_object()) {
            return Err("custom upstream authentication headers are not supported by the bounded profile".into());
        }
        if (!m.compat.is_null() && !m.compat.is_object())
            || m.compat["supportsUsageInStreaming"] == false
            || !m.compat["openRouterRouting"].is_null()
            || !m.compat["vercelGatewayRouting"].is_null()
        {
            return Err("the bounded profile requires final usage and the configured provider tariff without dynamic routing".into());
        }
        Ok(())
    }

    pub(super) fn materialize(&self, path: &Path, base_url: &str) -> Result<(), String> {
        std::fs::create_dir_all(path).map_err(|_| "cannot create the private bounded-model profile")?;
        let m = &self.model;
        let mut model = json!({"id":m.id,"name":m.id,"api":m.api,"reasoning":m.reasoning,"input":["text"],"contextWindow":m.context_window,"maxTokens":m.max_tokens,"compat":m.compat,"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0}});
        if m.compat.is_null() {
            model.as_object_mut().expect("model object").remove("compat");
        }
        if !m.thinking_level_map.is_null() {
            model["thinkingLevelMap"] = m.thinking_level_map.clone();
        }
        let models = json!({"providers":{(m.provider.clone()):{"baseUrl":base_url,"api":m.api,"apiKey":"$GENIE_TOKEN","models":[model]}}});
        crate::runtime::write_private(&path.join("models.json"), &models.to_string())
            .map_err(|_| "cannot write the private bounded-model profile")?;
        // All model calls, including manual compaction, use the gateway. Hidden
        // provider retries and third-party startup extensions remain disabled.
        let mut settings =
            json!({"packages":[],"extensions":[],"retry":{"enabled":false,"provider":{"maxRetries":0}},"cacheWarming":{"mode":"off"}});
        let source_settings = read_object(&self.source.join("settings.json"))?;
        let mut compaction = serde_json::Map::new();
        if let Some(enabled) = source_settings["compaction"]["enabled"].as_bool() {
            compaction.insert("enabled".into(), json!(enabled));
        }
        for name in ["reserveTokens", "keepRecentTokens"] {
            if let Some(value) = source_settings["compaction"][name].as_u64() {
                compaction.insert(name.into(), json!(value));
            }
        }
        if !compaction.is_empty() {
            settings["compaction"] = json!(compaction);
        }
        crate::runtime::write_private(&path.join("settings.json"), &settings.to_string())
            .map_err(|_| "cannot write the private bounded-model settings")?;
        crate::runtime::write_private(&path.join("auth.json"), "{}").map_err(|_| "cannot write the private bounded-model credentials")?;
        Ok(())
    }

    /// The same API-key priority as the runtime: a person's key, shared key,
    /// then the configured server environment/profile. OAuth and shell key
    /// resolvers fail closed rather than switching billing identities silently.
    pub(super) fn key(&self, app: &App, keys: &Keys, user: Option<i64>) -> Result<String, String> {
        let provider = &self.model.provider;
        let auth = read_object(&self.source.join("auth.json"))?;
        if auth[provider]["type"] == "oauth" {
            return Err("bounded provider calls currently require API-key authentication, not OAuth".into());
        }
        if let Some(known) = crate::llm_key::provider(provider) {
            if let Some(key) = sealed_key(app, user, provider)? {
                return valid_key(key);
            }
            if let Keys::Person { set, .. } = keys
                && let Some((_, key)) = set.iter().find(|(name, _)| *name == known.key_var)
            {
                return valid_key(key.clone());
            }
            if known.strict && app.with_server(|db| db.user_count()).map_err(|_| "cannot resolve model credential authority")? > 0 {
                return Err("the user's configured model key is no longer available".into());
            }
            if let Some(key) = environment(app, keys, known.key_var) {
                return valid_key(key);
            }
        }
        let config = read_object(&self.source.join("models.json"))?;
        let provider_config = &config["providers"][provider];
        if provider_config["headers"].as_object().is_some_and(|h| !h.is_empty()) {
            return Err("custom provider headers are outside the bounded API-key profile".into());
        }
        if let Some(value) = provider_config["apiKey"].as_str() {
            if value.starts_with('!') {
                return Err("shell credential resolvers are outside the bounded model profile".into());
            }
            if let Some(name) = value.strip_prefix('$') {
                if name.is_empty() || !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_') || name == "GENIE_TOKEN" {
                    return Err("invalid upstream credential variable".into());
                }
                if name == crate::llm_key::KEY_VAR {
                    if let Some(key) = sealed_key(app, user, "litellm")? {
                        return valid_key(key);
                    }
                    if app.with_server(|db| db.user_count()).map_err(|_| "cannot resolve model credential authority")? > 0 {
                        return Err("the user's configured LiteLLM key is no longer available".into());
                    }
                }
                return environment(app, keys, name)
                    .ok_or("the model credential environment variable is unavailable".into())
                    .and_then(valid_key);
            }
            return valid_key(value.into());
        }
        if auth[provider]["type"] == "api_key"
            && let Some(key) = auth[provider]["key"].as_str()
        {
            return valid_key(key.into());
        }
        Err("no supported API key is available for this bounded model".into())
    }

    pub(super) fn custom_key_variables(&self) -> Result<Vec<String>, String> {
        let config = read_object(&self.source.join("models.json"))?;
        let mut variables = Vec::new();
        let mut remember = |value: &Value| {
            if let Some(name) = value.as_str().and_then(|s| s.strip_prefix('$'))
                && !name.is_empty()
                && name != "GENIE_TOKEN"
                && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
            {
                variables.push(name.to_string());
            }
        };
        if let Some(providers) = config["providers"].as_object() {
            for provider in providers.values() {
                remember(&provider["apiKey"]);
                for headers in std::iter::once(&provider["headers"])
                    .chain(provider["models"].as_array().into_iter().flatten().map(|model| &model["headers"]))
                {
                    if let Some(headers) = headers.as_object() {
                        for value in headers.values() {
                            remember(value);
                        }
                    }
                }
            }
        }
        variables.sort();
        variables.dedup();
        Ok(variables)
    }
}

fn read_object(path: &Path) -> Result<Value, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(json!({})),
        Err(_) => return Err("cannot read the configured model credential profile".into()),
    };
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid model credential profile")?;
    if !value.is_object() {
        return Err("model credential profile must be an object".into());
    }
    Ok(value)
}
fn sealed_key(app: &App, user: Option<i64>, provider: &str) -> Result<Option<String>, String> {
    app.with_server(|db| {
        Ok(match user.map(|id| db.user_secret(id, provider)).transpose()?.flatten() {
            Some(key) => Some(key),
            None => db.global_secret(provider)?,
        })
    })
    .map_err(|_| "cannot read the model credential store".into())
}
fn environment(app: &App, keys: &Keys, name: &str) -> Option<String> {
    if let Keys::Person { set, clear, .. } = keys {
        if clear.contains(&name) {
            return None;
        }
        if let Some((_, value)) = set.iter().find(|(key, _)| *key == name) {
            return Some(value.clone());
        }
    }
    app.cfg.runtime.env.get(name).cloned().or_else(|| std::env::var(name).ok())
}
fn valid_key(key: String) -> Result<String, String> {
    if key.trim().is_empty() || reqwest::header::HeaderValue::from_str(&format!("Bearer {key}")).is_err() {
        return Err("the model API key has an invalid header format".into());
    }
    Ok(key)
}
