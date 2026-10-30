//! Whose model keys an agent uses.
//!
//! Every person keeps their own keys in their profile (`genie_core::secrets`),
//! one per provider: this deployment's LiteLLM proxy and the providers pi
//! supports natively (Anthropic, OpenAI, Google, xAI, … — [`PROVIDERS`]). An
//! agent the server starts runs on behalf of its *initiator* and gets that
//! person's keys as environment variables, which pi's providers read:
//!
//! - a team member — the person who assembled the team; for a team the
//!   orchestrator or an automation assembled, the person the task is for (its
//!   assignee, else its author);
//! - a job — the person who started it; for an automation's job, the person
//!   whose action triggered the run, else the task's person, else the author of
//!   the automation;
//! - the orchestrator — the person whose mail it is answering (the sender, or the
//!   initiator of the team or task the mail is about).
//!
//! The LiteLLM key is owed: an agent on a `litellm/…` model whose initiator has
//! no key is not started, the person is told, and the server's own
//! `LITELLM_API_KEY` never reaches agents — except on a server without users
//! (local mode), where there are no profiles and agents keep using it. The keys
//! of pi's own providers are voluntary: a person's key, when set, overrides what
//! the server's environment would have given the agent; without one the agent
//! runs as before.
//!
//! A provider's key the administration shares with everyone (`global_secrets`)
//! stands in where the person keeps none: a person's own key of the same
//! provider comes first, and a shared LiteLLM key satisfies the owed one.

use genie_core::automation::Run;
use genie_core::secrets::LITELLM;
use genie_core::team::{Mail, ORCHESTRATOR};
use genie_core::{Actor, Role};
use serde::Serialize;
use std::sync::Mutex;

use crate::agent_config::TeamSpec;
use crate::notify::{self, Message};
use crate::state::App;

/// The variable the `litellm` provider of pi reads its key from (`models.json`: `"apiKey": "$LITELLM_API_KEY"`).
pub const KEY_VAR: &str = "LITELLM_API_KEY";

static USER_PI_PROFILE_LOCK: Mutex<()> = Mutex::new(());

/// A provider whose API key a person may keep in their profile: the variable
/// pi's provider reads it from.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    /// The prefix of the provider's models (`provider/model`).
    pub id: &'static str,
    pub title: &'static str,
    /// The environment variable pi's provider takes its key from.
    pub key_var: &'static str,
    /// Whether a key can be entered for this provider.
    pub api_key: bool,
    /// Whether pi supports OAuth login for this provider.
    pub oauth: bool,
    /// The key is owed: an agent on this provider does not start without it.
    pub strict: bool,
}

const fn pi(id: &'static str, title: &'static str, key_var: &'static str) -> Provider {
    Provider { id, title, key_var, api_key: true, oauth: false, strict: false }
}

const fn pi_oauth(id: &'static str, title: &'static str, key_var: &'static str) -> Provider {
    Provider { id, title, key_var, api_key: true, oauth: true, strict: false }
}

const fn oauth_only(id: &'static str, title: &'static str) -> Provider {
    Provider { id, title, key_var: "", api_key: false, oauth: true, strict: false }
}

/// Providers available to personal and shared-key settings. OAuth is exposed
/// for the Pi providers whose registry supports it; providers with more complex
/// configuration (Bedrock, Vertex, Azure, Cloudflare) remain server-managed.
pub const PROVIDERS: &[Provider] = &[
    Provider { id: "litellm", title: "LiteLLM", key_var: KEY_VAR, api_key: true, oauth: false, strict: true },
    pi_oauth("anthropic", "Anthropic", "ANTHROPIC_API_KEY"),
    pi("ant-ling", "Ant Ling", "ANT_LING_API_KEY"),
    pi_oauth("openai", "OpenAI", "OPENAI_API_KEY"),
    pi("deepseek", "DeepSeek", "DEEPSEEK_API_KEY"),
    pi("nvidia", "NVIDIA NIM", "NVIDIA_API_KEY"),
    pi("google", "Google Gemini", "GEMINI_API_KEY"),
    pi_oauth("github-copilot", "GitHub Copilot", "COPILOT_GITHUB_TOKEN"),
    pi("mistral", "Mistral", "MISTRAL_API_KEY"),
    pi("groq", "Groq", "GROQ_API_KEY"),
    pi("cerebras", "Cerebras", "CEREBRAS_API_KEY"),
    pi_oauth("xai", "xAI", "XAI_API_KEY"),
    pi_oauth("openrouter", "OpenRouter", "OPENROUTER_API_KEY"),
    pi("vercel-ai-gateway", "Vercel AI Gateway", "AI_GATEWAY_API_KEY"),
    pi("zai", "ZAI Coding Plan", "ZAI_API_KEY"),
    pi("zai-coding-cn", "ZAI Coding Plan (China)", "ZAI_CODING_CN_API_KEY"),
    pi("opencode", "OpenCode Zen", "OPENCODE_API_KEY"),
    pi("opencode-go", "OpenCode Zen Go", "OPENCODE_API_KEY"),
    pi_oauth("radius", "Radius", "RADIUS_API_KEY"),
    pi("typesafe", "TypeSafe", "TYPESAFE_API_KEY"),
    pi("huggingface", "Hugging Face", "HF_TOKEN"),
    pi("fireworks", "Fireworks", "FIREWORKS_API_KEY"),
    pi("together", "Together AI", "TOGETHER_API_KEY"),
    pi("baseten", "Baseten", "BASETEN_API_KEY"),
    pi_oauth("kimi-coding", "Kimi For Coding", "KIMI_API_KEY"),
    pi_oauth("meta", "Meta", "META_API_KEY"),
    pi("minimax", "MiniMax", "MINIMAX_API_KEY"),
    pi("minimax-cn", "MiniMax (China)", "MINIMAX_CN_API_KEY"),
    pi("moonshotai", "Moonshot AI", "MOONSHOT_API_KEY"),
    pi("moonshotai-cn", "Moonshot AI (China)", "MOONSHOT_API_KEY"),
    pi("qwen-token-plan", "Qwen Token Plan", "QWEN_TOKEN_PLAN_API_KEY"),
    pi("qwen-token-plan-cn", "Qwen Token Plan (China)", "QWEN_TOKEN_PLAN_CN_API_KEY"),
    pi("qwen-token-plan-individual", "Qwen Token Plan (Individual)", "QWEN_TOKEN_PLAN_API_KEY"),
    pi("xiaomi", "Xiaomi MiMo", "XIAOMI_API_KEY"),
    pi("xiaomi-token-plan-ams", "Xiaomi MiMo Token Plan (Amsterdam)", "XIAOMI_TOKEN_PLAN_AMS_API_KEY"),
    pi("xiaomi-token-plan-cn", "Xiaomi MiMo Token Plan (China)", "XIAOMI_TOKEN_PLAN_CN_API_KEY"),
    pi("xiaomi-token-plan-sgp", "Xiaomi MiMo Token Plan (Singapore)", "XIAOMI_TOKEN_PLAN_SGP_API_KEY"),
    oauth_only("openai-codex", "OpenAI Codex"),
];

/// Secret name used to keep Pi OAuth credentials encrypted in the user's profile.
pub fn oauth_secret(id: &str) -> String {
    format!("pi-oauth:{id}")
}

/// A provider by its id (the `provider/` prefix of its models).
pub fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// The provider a `provider/model` id names by its prefix, when it is one of
/// the registry's.
fn of_model(model: Option<&str>) -> Option<&'static Provider> {
    let prefix = model.map(str::trim).filter(|m| !m.is_empty())?.split('/').next()?;
    provider(prefix)
}

/// The provider a `provider/model` id belongs to, when it is one of pi's own
/// with a key variable: `litellm` is handled by its own rule, a provider the
/// server configured in `models.json` has no variable of its own.
fn native(model: Option<&str>) -> Option<&'static Provider> {
    of_model(model).filter(|p| !p.strict && p.api_key)
}

/// Does an agent on `model` need a key it cannot start without — the strict
/// providers, of which LiteLLM is the one?
pub fn needs_key(model: Option<&str>) -> bool {
    of_model(model).is_some_and(|p| p.strict)
}

/// What an agent's model keys are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keys {
    /// A server without users: the server's own environment, untouched.
    Server,
    /// A person's agent: the variables their keys set, and those through which
    /// the server's own keys must not reach it.
    Person { set: Vec<(&'static str, String)>, clear: Vec<&'static str>, pi_dir: Option<std::path::PathBuf> },
}

impl Keys {
    pub fn apply(&self, cmd: &mut tokio::process::Command) {
        if let Keys::Person { set, clear, pi_dir } = self {
            for (var, value) in set {
                cmd.env(var, value);
            }
            for var in clear {
                cmd.env_remove(var);
            }
            if let Some(dir) = pi_dir {
                cmd.env("PI_CODING_AGENT_DIR", dir);
            }
        }
    }
}

/// The keys for an agent `who` (for messages) on `model`, run on behalf of `initiator`.
/// Fails when the agent needs a LiteLLM key and cannot have one; the person is told.
pub fn resolve(app: &App, project: &str, who: &str, initiator: Option<&str>, model: Option<&str>) -> Result<Keys, String> {
    if app.with_server(|db| db.user_count()).map_err(|e| e.to_string())? == 0 {
        return Ok(Keys::Server);
    }
    let user = match initiator {
        Some(login) => app.with_server(|db| db.user_by_login(login)).map_err(|e| e.to_string())?.filter(|u| !u.disabled),
        None => None,
    };
    let secret = |name: &str| {
        let id = user.as_ref().map(|u| u.id);
        id.and_then(|id| app.with_server(|db| db.user_secret(id, name)).ok().flatten())
    };
    let global = |name: &str| app.with_server(|db| db.global_secret(name)).ok().flatten();
    let mut set: Vec<(&'static str, String)> = Vec::new();
    let mut clear: Vec<&'static str> = Vec::new();
    // The LiteLLM key is the person's own, else the shared one; set or cleared:
    // the server's own never reaches a person's agent.
    let litellm = secret(LITELLM).or_else(|| global(LITELLM));
    match &litellm {
        Some(k) => set.push((KEY_VAR, k.clone())),
        None => clear.push(KEY_VAR),
    }
    // The model's own provider: the person's key, else the shared one, overrides
    // the server's environment.
    if let Some(p) = native(model)
        && let Some(k) = secret(p.id).or_else(|| global(p.id))
    {
        set.push((p.key_var, k));
    }
    if needs_key(model) && litellm.is_none() {
        let model = model.unwrap_or_default();
        return Err(match (&user, initiator) {
            (Some(u), _) => {
                tell_missing(app, project, u.id, &u.login, who, model);
                format!("{who} runs on {model} on behalf of @{}, who has no LiteLLM key: set it in the profile", u.login)
            }
            (None, Some(login)) => {
                format!("{who} runs on {model} on behalf of {login}, who has no account on this server to hold a LiteLLM key")
            }
            (None, None) => format!("{who} runs on {model}, but nobody it works for is known to take its LiteLLM key from"),
        });
    }
    let pi_dir = match &user {
        Some(user) => user_pi_dir(app, user.id)?,
        None => None,
    };
    Ok(Keys::Person { set, clear, pi_dir })
}

/// Materialize OAuth credentials for this user into a private Pi profile. Pi
/// reads OAuth from auth.json; the database remains the encrypted source of truth.
fn user_pi_dir(app: &App, user: i64) -> Result<Option<std::path::PathBuf>, String> {
    sync_user_pi_profile(app, user)?;
    let personal = user_oauth(app, user)?;
    if personal.is_empty() {
        remove_user_pi_profile(app, user)?;
        return Ok(None);
    }

    let dir = app.data.join("runtime").join("pi-users").join(user.to_string());
    let auth_path = dir.join("auth.json");
    let valid_profile = std::fs::read(&auth_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .is_some_and(|value| value.is_object());
    if !valid_profile {
        write_user_pi_profile(app, user)?;
    }
    restrict_dir(&dir);
    restrict_file(&auth_path);
    Ok(Some(dir))
}

/// Keep Pi's refreshed OAuth token in the encrypted secret store. Pi updates
/// auth.json while using OAuth; retain the private profile across server restarts
/// so its rotated refresh token can be picked up before another agent starts.
pub fn sync_user_pi_profile(app: &App, user: i64) -> Result<(), String> {
    let _guard = USER_PI_PROFILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = app.data.join("runtime").join("pi-users").join(user.to_string()).join("auth.json");
    let Some(profile) = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.as_object().cloned())
    else {
        return Ok(());
    };
    app.with_server(|db| {
        for provider in PROVIDERS.iter().filter(|provider| provider.oauth) {
            let Some(on_disk) = profile.get(provider.id).filter(|value| value["type"] == "oauth") else { continue };
            let name = oauth_secret(provider.id);
            let Some(stored) = db.user_secret(user, &name)? else { continue };
            let Ok(stored) = serde_json::from_str::<serde_json::Value>(&stored) else { continue };
            let (Some(disk_expiry), Some(stored_expiry)) = (on_disk["expires"].as_f64(), stored["expires"].as_f64()) else {
                continue;
            };
            if disk_expiry >= stored_expiry && on_disk != &stored {
                let updated = serde_json::to_string(on_disk).map_err(|e| genie_core::GenieError::invalid(e.to_string()))?;
                db.set_user_secret(user, &name, &updated)?;
            }
        }
        Ok::<_, genie_core::GenieError>(())
    })
    .map_err(|e| e.to_string())
}

/// Rebuild the private Pi profile from the encrypted user credentials.
pub fn write_user_pi_profile(app: &App, user: i64) -> Result<(), String> {
    let _guard = USER_PI_PROFILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let personal = user_oauth(app, user)?;
    if personal.is_empty() {
        return remove_user_pi_profile(app, user);
    }

    let base = crate::runtime::pi_dir(app);
    let dir = app.data.join("runtime").join("pi-users").join(user.to_string());
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create the user's pi profile: {e}"))?;
    restrict_dir(&dir);
    let name = "models.json";
    let src = base.join(name);
    if src.is_file() {
        std::fs::copy(src, dir.join(name)).map_err(|e| format!("cannot copy pi {name}: {e}"))?;
    }
    let mut auth = std::fs::read(base.join("auth.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    auth.extend(personal);
    let path = dir.join("auth.json");
    let bytes = serde_json::to_vec(&auth).map_err(|e| e.to_string())?;
    std::fs::write(&path, bytes).map_err(|e| format!("cannot write the user's pi auth file: {e}"))?;
    restrict_file(&path);
    Ok(())
}

fn user_oauth(app: &App, user: i64) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    app.with_server(|db| {
        let mut auth = serde_json::Map::new();
        for provider in PROVIDERS.iter().filter(|provider| provider.oauth) {
            if let Some(raw) = db.user_secret(user, &oauth_secret(provider.id))?
                && let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw)
            {
                auth.insert(provider.id.into(), value);
            }
        }
        Ok::<_, genie_core::GenieError>(auth)
    })
    .map_err(|e| e.to_string())
}

/// Refresh or remove the local runtime profile after OAuth credentials change.
pub fn refresh_user_pi_profile(app: &App, user: i64) -> Result<(), String> {
    sync_user_pi_profile(app, user)?;
    write_user_pi_profile(app, user)
}

fn remove_user_pi_profile(app: &App, user: i64) -> Result<(), String> {
    let dir = app.data.join("runtime").join("pi-users").join(user.to_string());
    std::fs::remove_dir_all(dir)
        .or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
        .map_err(|e| format!("cannot remove the user's pi profile: {e}"))
}

#[cfg(unix)]
fn restrict_dir(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict_dir(_path: &std::path::Path) {}

#[cfg(unix)]
fn restrict_file(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn restrict_file(_path: &std::path::Path) {}

/// Tell a person (once a day per project) that their agents wait for their key.
fn tell_missing(app: &App, project: &str, user: i64, login: &str, who: &str, model: &str) {
    let msg = Message {
        kind: "litellm-key".into(),
        title: "Агенты ждут ваш ключ LiteLLM".into(),
        body: format!(
            "{who} в проекте {project} работает от вашего имени на модели {model}, но в вашем профиле нет ключа LiteLLM. Укажите его в профиле — агент запустится сам."
        ),
        project: Some(project.to_string()),
        link: Some("/settings/me/keys".into()),
        ..Default::default()
    };
    let day = chrono::Utc::now().format("%Y-%m-%d");
    if let Err(e) = notify::send(app, &[user], &msg, Some(&format!("litellm-key:{project}:{login}:{day}"))) {
        eprintln!("genie runtime: {project}: cannot tell @{login} about the LiteLLM key: {e}");
    }
}

/// The person a task is for: its assignee, else the person who created it.
pub fn task_person(app: &App, project: &str, task: &str) -> Option<String> {
    let t = app.with_tracker(project, |t| t.get(task)).ok()?;
    t.assignee.clone().or_else(|| t.history.iter().find(|h| h.role == Role::Human).map(|h| h.actor.clone()))
}

/// The person a team works on behalf of.
pub fn team_initiator(app: &App, project: &str, team: &str) -> Option<String> {
    let t = app.with_tracker(project, |t| t.bus().get(team)).ok()?;
    t.spec.as_ref().and_then(TeamSpec::from_value).and_then(|s| s.initiator).or_else(|| task_person(app, project, &t.task))
}

/// The initiator of a new team or job started by `by` for `task`: a person is
/// their own initiator, an agent works for the task's person.
pub fn initiator_of(app: &App, project: &str, by: &Actor, task: Option<&str>) -> Option<String> {
    if by.role == Role::Human {
        return Some(by.name.clone());
    }
    task.and_then(|t| task_person(app, project, t))
}

/// The initiator of what an automation's run starts: the person whose action
/// triggered the run, else the task's person, else the automation's author.
pub fn automation_initiator(app: &App, run: &Run, task: Option<&str>) -> Option<String> {
    let actor = run.trigger["context"]["event"]["actor"].as_str().filter(|a| !a.is_empty());
    let person = actor.filter(|a| app.with_server(|db| db.user_by_login(a)).ok().flatten().is_some_and(|u| !u.disabled));
    person
        .map(str::to_string)
        .or_else(|| task.and_then(|t| task_person(app, &run.project, t)))
        .or_else(|| app.with_server(|db| db.automation(run.automation)).ok().map(|a| a.created_by))
}

/// Whether `login` is an enabled account of this server (`git-host`, which reports merges and
/// checks as a human, is not).
fn is_account(app: &App, login: &str) -> bool {
    app.with_server(|db| db.user_by_login(login)).ok().flatten().is_some_and(|u| !u.disabled)
}

/// The person the orchestrator answers in `mail`: the first person with an account who wrote,
/// else the initiator of the first team or task a message is about.
pub fn mail_initiator(app: &App, project: &str, mail: &[Mail]) -> Option<String> {
    if let Some(m) = mail.iter().find(|m| m.from_role == Role::Human.as_str() && is_account(app, &m.from)) {
        return Some(m.from.clone());
    }
    mail.iter().find_map(|m| {
        if let Some(team) = &m.team {
            return team_initiator(app, project, team);
        }
        let task = m.task.as_deref()?;
        let team = app.with_tracker(project, |t| t.get(task)).ok().and_then(|t| t.team);
        team.and_then(|team| team_initiator(app, project, &team)).or_else(|| task_person(app, project, task))
    })
}

/// The person the orchestrator would answer now: from the mail waiting for it.
pub fn orchestrator_initiator(app: &App, project: &str) -> Option<String> {
    // Only the mail the manager takes: a manual project's queued system notes do not name a person.
    let row = app.with_server(|db| db.project(project)).ok()?;
    let mail = app
        .with_tracker(project, |t| t.bus().pending_kind(None, ORCHESTRATOR, crate::runtime::taken_kind(&row, None, ORCHESTRATOR)))
        .ok()?;
    mail_initiator(app, project, &mail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_litellm_models_need_a_key() {
        assert!(needs_key(Some("litellm/gpt-6-sol")));
        assert!(!needs_key(Some("openai-codex/gpt-6")));
        assert!(!needs_key(None));
    }

    #[test]
    fn the_registry_covers_the_providers_pi_takes_a_key_variable_for() {
        assert_eq!(provider("litellm").unwrap().key_var, "LITELLM_API_KEY");
        assert!(provider("litellm").unwrap().strict);
        assert_eq!(provider("anthropic").unwrap().key_var, "ANTHROPIC_API_KEY");
        assert_eq!(provider("zai-coding-cn").unwrap().key_var, "ZAI_CODING_CN_API_KEY");
        let codex = provider("openai-codex").unwrap();
        assert!(!codex.api_key, "an OAuth-only provider holds no key");
        assert!(codex.oauth);
        assert!(native(Some("openai-codex/gpt-6")).is_none());
        assert_eq!(provider("moonshotai").unwrap().key_var, provider("moonshotai-cn").unwrap().key_var);
    }

    #[test]
    fn a_native_provider_comes_from_the_prefix_of_the_model() {
        assert_eq!(native(Some("anthropic/claude-opus-5-5")).unwrap().id, "anthropic");
        assert!(native(Some("litellm/gpt-6-sol")).is_none(), "litellm is set by its own rule");
        assert!(native(Some("claude-bridge/claude")).is_none(), "a provider of models.json has no variable of its own");
        assert!(native(None).is_none(), "no model, no provider");
    }
}
