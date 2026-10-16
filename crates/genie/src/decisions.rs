//! One Jev service for explicit agent tools and automatic team decisions.
//! HTTP happens without a tracker/server lock. Policy and task snapshots are checked
//! again by the team boundary before a recommendation can change execution.
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use genie_core::Actor;
use genie_core::decisions::{Decision, NewDecision};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::state::{App, AppError, AppResult};
pub(crate) mod team;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    #[default]
    Off,
    Observe,
    Advise,
    Joint,
    Select,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub description: String,
    pub model: String,
    pub thinking: Option<String>,
    /// Configured role ids this pair has been checked for; empty does not grant all roles.
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Policy {
    pub allow_external: bool,
    pub tool_enabled: bool,
    pub tool_roles: Vec<String>,
    pub team: Mode,
    pub model: Mode,
    pub thinking: Mode,
    pub templates: Vec<String>,
    pub profiles: Vec<Profile>,
    pub min_confidence: f64,
    pub min_margin: f64,
    pub max_calls_per_task: u32,
    pub max_cost_per_task_usd: f64,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            allow_external: false,
            tool_enabled: false,
            tool_roles: vec!["orchestrator".into(), "analyst".into()],
            team: Mode::Off,
            model: Mode::Off,
            thinking: Mode::Off,
            templates: vec![],
            profiles: vec![],
            min_confidence: 0.7,
            min_margin: 0.2,
            max_calls_per_task: 40,
            max_cost_per_task_usd: 0.1,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct Config {
    /// Server-wide availability. Projects opt into external calls separately.
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    pub timeout_ms: u64,
    pub max_request_bytes: usize,
    pub input_price: f64,
    pub defaults: Policy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: "https://api.typesafe.ai/v1/".into(),
            model: "jev-1.13.0".into(),
            timeout_ms: 5000,
            max_request_bytes: 32_000,
            input_price: 0.042,
            defaults: Policy::default(),
        }
    }
}

/// Only simultaneous spawns of the same task wait for each other. No database lock
/// or global task lock is held during the provider call. Weak entries are reclaimed.
#[derive(Default)]
pub struct Gates(Mutex<HashMap<String, std::sync::Weak<Mutex<()>>>>);
impl Gates {
    pub fn task(&self, project: &str, task: &str) -> Arc<Mutex<()>> {
        let mut gates = self.0.lock().unwrap_or_else(|e| e.into_inner());
        gates.retain(|_, v| v.strong_count() > 0);
        gates.entry(format!("{project}/{task}")).or_default().upgrade().unwrap_or_else(|| {
            let gate = Arc::new(Mutex::new(()));
            gates.insert(format!("{project}/{task}"), Arc::downgrade(&gate));
            gate
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "lowercase", deny_unknown_fields)]
pub enum Question {
    Choice { instructions: String, criteria: BTreeMap<String, String> },
    Score { instructions: String, criteria: Vec<String> },
    Bool { instructions: String, criteria: BoolCriteria },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoolCriteria {
    #[serde(rename = "true")]
    pub yes: String,
    #[serde(rename = "false")]
    pub no: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Request {
    /// Task scope; the CLI uses GENIE_TASK when omitted.
    pub task: Option<String>,
    pub purpose: String,
    pub state: Value,
    pub questions: BTreeMap<String, Question>,
}

pub fn policy(app: &App, project: &str) -> AppResult<Policy> {
    let value = app.with_server(|db| db.decision_policy(project))?;
    let p = match value {
        Some(v) => serde_json::from_value(v).map_err(|e| AppError::Internal(format!("decision policy: {e}")))?,
        None => app.cfg.decisions.defaults.clone(),
    };
    validate_policy(app, project, &p)?;
    Ok(p)
}

pub fn validate_policy(app: &App, project: &str, p: &Policy) -> AppResult<()> {
    let bad = |message: &str| AppError::Bad(message.into());
    if !(0.0..=1.0).contains(&p.min_confidence)
        || !(0.0..=1.0).contains(&p.min_margin)
        || !(1..=1000).contains(&p.max_calls_per_task)
        || !p.max_cost_per_task_usd.is_finite()
        || p.max_cost_per_task_usd <= 0.0
        || p.max_cost_per_task_usd > 100.0
    {
        return Err(bad("invalid Jev thresholds or task allowance"));
    }
    if p.templates.len() > 24 || p.profiles.len() > 48 || p.tool_roles.len() > 64 {
        return Err(bad("too many Jev templates, profiles or tool roles"));
    }
    let agents = app.agents();
    for id in &p.tool_roles {
        agents.role_for(project, id)?;
    }
    for id in &p.templates {
        let t = agents.team_for(project, id)?;
        if t.stage != crate::agent_config::Stage::Delivery || t.members.len() > app.cfg.limits.max_members_per_team {
            return Err(bad("Jev templates must be delivery teams within the member limit"));
        }
    }
    let mut ids = BTreeSet::new();
    for profile in &p.profiles {
        if profile.id.is_empty()
            || profile.id.len() > 64
            || !ids.insert(&profile.id)
            || !profile.model.split_once('/').is_some_and(|(provider, model)| !provider.is_empty() && !model.is_empty())
            || profile.model.chars().any(|c| c.is_whitespace() || c.is_control())
            || profile.description.trim().is_empty()
            || profile.description.len() > 2000
            || profile.roles.is_empty()
            || profile.thinking.as_deref().is_some_and(|t| !crate::http::teams::THINKING.contains(&t))
        {
            return Err(bad("each Jev profile needs a unique id, provider/model, description, allowed roles and a valid thinking level"));
        }
        for id in &profile.roles {
            agents.role_for(project, id)?;
        }
    }
    Ok(())
}

fn validate_request(r: &Request) -> AppResult<()> {
    if r.purpose.trim().is_empty() || r.purpose.len() > 120 || r.questions.is_empty() || r.questions.len() > 32 {
        return Err(AppError::Bad("a decision needs a purpose and 1–32 questions".into()));
    }
    for (id, question) in &r.questions {
        let (instructions, valid) = match question {
            Question::Choice { instructions, criteria } => (
                instructions,
                (2..=255).contains(&criteria.len()) && criteria.iter().all(|(k, v)| !k.trim().is_empty() && !v.trim().is_empty()),
            ),
            Question::Score { instructions, criteria } => {
                (instructions, (2..=10).contains(&criteria.len()) && criteria.iter().all(|v| !v.trim().is_empty()))
            }
            Question::Bool { instructions, criteria } => (instructions, !criteria.yes.trim().is_empty() && !criteria.no.trim().is_empty()),
        };
        if id.is_empty() || id.len() > 100 || instructions.trim().is_empty() || !valid {
            return Err(AppError::Bad(format!("invalid decision question {id}")));
        }
    }
    Ok(())
}

fn wire(r: &Request, model: &str) -> Value {
    let mut questions = serde_json::to_value(&r.questions).expect("questions serialize");
    for question in questions.as_object_mut().unwrap().values_mut() {
        if question["type"] == "bool" {
            question["type"] = json!("noul");
        }
    }
    json!({"model":model,"state":r.state,"questions":questions})
}

/// Profile/shared key first; only a userless local installation may use the server environment.
fn key(app: &App, initiator: Option<&str>) -> AppResult<Option<String>> {
    app.with_server(|db| {
        let personal = match initiator {
            Some(login) => match db.user_by_login(login)?.filter(|u| !u.disabled) {
                Some(u) => db.user_secret(u.id, "typesafe")?,
                None => None,
            },
            None => None,
        };
        let shared = db.global_secret("typesafe")?;
        Ok(personal
            .or(shared)
            .or_else(|| (db.user_count().ok() == Some(0)).then(|| std::env::var("TYPESAFE_API_KEY").ok()).flatten())
            .filter(|s| !s.trim().is_empty()))
    })
}

fn checked_answers(r: &Request, raw: &Value, p: &Policy) -> Result<(Value, bool), String> {
    let answers = raw["answers"].as_object().ok_or("Jev returned no answers")?;
    let mut out = serde_json::Map::new();
    let mut uncertain = false;
    for (id, question) in &r.questions {
        let answer = answers.get(id).ok_or_else(|| format!("Jev omitted question {id}"))?;
        let probability = |v: &Value| v.as_f64().filter(|x| (0.0..=1.0).contains(x));
        let value = match question {
            Question::Bool { .. } => {
                let value = probability(&answer["noul"]).ok_or("invalid noul answer")?;
                if answer["type"] != "noul" {
                    return Err("invalid noul type".into());
                }
                uncertain |= (2.0 * value - 1.0).abs() < p.min_confidence;
                json!({"type":"bool","probability":value})
            }
            Question::Choice { criteria, .. } => {
                let choice = answer["choice"].as_str().filter(|c| criteria.contains_key(*c)).ok_or("unknown Jev choice")?;
                let probabilities = answer["probabilities"].as_object().ok_or("invalid choice probabilities")?;
                if answer["type"] != "choice" || probabilities.len() != criteria.len() {
                    return Err("invalid choice shape".into());
                }
                let mut values = criteria
                    .keys()
                    .map(|k| probability(&probabilities[k]).ok_or("invalid choice probability"))
                    .collect::<Result<Vec<_>, _>>()?;
                if (values.iter().sum::<f64>() - 1.0).abs() > 0.02 {
                    return Err("choice probabilities do not sum to one".into());
                }
                values.sort_by(|a, b| b.total_cmp(a));
                if probability(&probabilities[choice]).unwrap() + 1e-6 < values[0] {
                    return Err("choice is not a top option".into());
                }
                let confidence = probability(&answer["confidence"]).ok_or("invalid confidence")?;
                // Derive our own gate as well; malformed confidence cannot overrule a flat distribution.
                let derived = (values[0] - 1.0 / values.len() as f64) / (1.0 - 1.0 / values.len() as f64);
                let qualified = confidence.min(derived) >= p.min_confidence && values[0] - values[1] >= p.min_margin;
                uncertain |= !qualified;
                let mut v = answer.clone();
                v["qualified"] = json!(qualified);
                v
            }
            Question::Score { criteria, .. } => {
                let score = answer["score"].as_f64().filter(|s| *s >= 0.0 && *s <= (criteria.len() - 1) as f64).ok_or("invalid score")?;
                let confidence = probability(&answer["confidence"]).ok_or("invalid score confidence")?;
                if answer["type"] != "score" {
                    return Err("invalid score type".into());
                }
                uncertain |= confidence < p.min_confidence;
                json!({"type":"score","score":score,"confidence":confidence,"probabilities":answer["probabilities"]})
            }
        };
        out.insert(id.clone(), value);
    }
    Ok((Value::Object(out), uncertain))
}

async fn send(app: &App, payload: &Value, key: &str) -> Result<Value, String> {
    let c = &app.cfg.decisions;
    let base = reqwest::Url::parse(&format!("{}/", c.base_url.trim_end_matches('/'))).map_err(|_| "invalid Jev baseUrl")?;
    let loopback = matches!(base.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if base.scheme() != "https" && !(base.scheme() == "http" && loopback) {
        return Err("Jev baseUrl requires HTTPS".into());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(c.timeout_ms.clamp(100, 30_000)))
        .build()
        .map_err(|_| "cannot create Jev client")?;
    let mut response = client
        .post(base.join("systemone").map_err(|_| "invalid Jev endpoint")?)
        .bearer_auth(key)
        .json(payload)
        .send()
        .await
        .map_err(|_| "Jev request failed or timed out")?;
    if !response.status().is_success() {
        return Err(format!("Jev HTTP {}", response.status().as_u16()));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| "cannot read Jev response")? {
        if bytes.len() + chunk.len() > 262_144 {
            return Err("Jev response exceeds size limit".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Jev returned invalid JSON".into())
}

/// The caller's authorization is checked at the HTTP entrance (or trusted team boundary).
/// No provider retry: a timeout may already have been billed. Identical automatic requests
/// reuse their durable record; explicit calls are each charged against the task allowance.
pub async fn evaluate(
    app: &App,
    project: &str,
    actor: &Actor,
    initiator: Option<&str>,
    r: &Request,
    request_key: Option<&str>,
) -> AppResult<Decision> {
    let p = policy(app, project)?;
    if !app.cfg.decisions.enabled || !p.allow_external {
        return Err(AppError::Forbidden("Jev external calls are disabled for this project".into()));
    }
    validate_request(r)?;
    let task = r.task.as_deref().ok_or_else(|| AppError::Bad("a decision requires a task".into()))?;
    if let Some(exceeded) = crate::budget::check(app, project, Some(task))? {
        return Err(AppError::Conflict(exceeded.explain()));
    }
    let payload = wire(r, &app.cfg.decisions.model);
    let bytes = serde_json::to_vec(&payload).map_err(|e| AppError::Bad(e.to_string()))?.len();
    if bytes > app.cfg.decisions.max_request_bytes.min(128_000) {
        return Err(AppError::Bad("Jev request exceeds the configured byte limit".into()));
    }
    if !app.cfg.decisions.input_price.is_finite() || app.cfg.decisions.input_price <= 0.0 {
        return Err(AppError::Bad("Jev inputPrice must be positive".into()));
    }
    if let Some(k) = request_key
        && let Some(record) = app.with_server(|db| db.decision_by_key(project, task, k))?
    {
        let expired = record.status == "pending"
            && chrono::DateTime::parse_from_rfc3339(&record.created).is_ok_and(|created| {
                (chrono::Utc::now() - created.with_timezone(&chrono::Utc)).num_milliseconds()
                    > (app.cfg.decisions.timeout_ms.clamp(100, 30_000) * 2 + 1000) as i64
            });
        if expired {
            app.with_server(|db| {
                db.finish_decision(
                    project,
                    record.id,
                    "unavailable",
                    &json!({"error":"An interrupted Jev request has no response; its allowance remains reserved"}),
                    None,
                    0,
                )
            })?;
        } else if record.status != "unavailable" {
            return Ok(record);
        }
    }
    // One token per byte plus framing is a conservative reservation, not a token measurement.
    let reserve = (bytes + 1024) as f64 * app.cfg.decisions.input_price / 1_000_000.0;
    let record = app.with_server(|db| {
        db.begin_decision(NewDecision {
            project,
            task,
            actor: &actor.name,
            purpose: &r.purpose,
            request: &payload,
            key: request_key,
            reserve,
            max_calls: p.max_calls_per_task,
            max_cost: p.max_cost_per_task_usd,
        })
    })?;
    let start = Instant::now();
    let (raw, sent) = match key(app, initiator) {
        Ok(Some(k)) => (send(app, &payload, &k).await, true),
        Ok(None) => (Err("No TypeSafe key: set your typesafe provider key or a shared key".into()), false),
        Err(_) => (Err("Cannot read the TypeSafe key".into()), false),
    };
    let (status, result, cost) = match raw {
        Ok(raw) => {
            let input = raw["usage"]["input_tokens"].as_u64();
            let cost = input.map(|n| n as f64 * app.cfg.decisions.input_price / 1_000_000.0);
            let result = checked_answers(r, &raw, &p);
            let (status, answers, error) = match result {
                Ok((answers, uncertain)) => (if uncertain { "uncertain" } else { "ok" }, answers, None),
                Err(error) => ("unavailable", json!({}), Some(error)),
            };
            let model = raw["model"].as_str().unwrap_or(&app.cfg.decisions.model);
            // Account for billed malformed responses too. The service price is independent
            // of pi's classifier catalogue, which currently lists a zero cost for Jev.
            if let Some(input) = input {
                // Price under the configured id (which may be an alias); the actual
                // responding version remains in the decision evidence.
                let model_id = format!("typesafe/{}", app.cfg.decisions.model);
                app.with_tracker(project, |t| {
                    t.record_usage(
                        "jev",
                        Some(task),
                        &model_id,
                        &genie_core::usage::Tokens {
                            input,
                            output: raw["usage"]["output_tokens"].as_u64().unwrap_or(0),
                            ..Default::default()
                        },
                    )
                })?;
            }
            (status, json!({"model":model,"answers":answers,"usage":raw["usage"],"error":error}), cost)
        }
        Err(error) => ("unavailable", json!({"error":error}), if sent { None } else { Some(0.0) }),
    };
    app.with_server(|db| db.finish_decision(project, record.id, status, &result, cost, start.elapsed().as_millis() as u64))
}

pub fn initiator(app: &App, project: &str, actor: &Actor, task: &str) -> Option<String> {
    crate::llm_key::initiator_of(app, project, actor, Some(task))
}

pub fn choice(instructions: &str, criteria: BTreeMap<String, String>) -> Question {
    Question::Choice { instructions: instructions.into(), criteria }
}

/// A clear choice, excluding the explicit abstention option. Each question is gated
/// separately, so an uncertain answer cannot accidentally grant authority elsewhere.
pub fn selected<'a>(record: &'a Decision, id: &str) -> Option<&'a str> {
    let answer = &record.result["answers"][id];
    (answer["qualified"] == true).then(|| answer["choice"].as_str()).flatten().filter(|s| *s != "need_evidence")
}
