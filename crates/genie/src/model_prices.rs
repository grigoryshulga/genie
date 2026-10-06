//! The models' tariffs, as LiteLLM knows them.
//!
//! Prices load once at start and by an administrator's «pull the config» action
//! (`POST /api/model-prices`, the button on the costs page) — never on a timer.
//! They are read with a service token that may see model info and nothing else,
//! kept in `server.db`, and merged with `modelPrices` of config.json, which
//! stays the manual override: per field, so a manual entry can fix a price
//! while LiteLLM still fills the ones it left out (the cache, usually).
//!
//! A model LiteLLM prices without a cache price keeps its cache tokens
//! **unpriced** — never billed as input (see [`genie_core::usage::ModelPrice`]).

use std::collections::BTreeMap;

use genie_core::usage::ModelPrice;

use crate::config::Config;
use crate::spend::Prices;
use crate::state::App;

/// The environment variable the service (admin) token for `/model/info` comes
/// from (`LITELLM_INFO_TOKEN_FILE` works wherever secrets become environment
/// variables). Unlike `LITELLM_API_KEY`, it never reaches agents.
pub const INFO_TOKEN: &str = "LITELLM_INFO_TOKEN";

/// The prices in effect: `modelPrices` of config.json over what LiteLLM
/// returned, per field. Manual `input`/`output` always win; a manual cache
/// price wins when it is set, and LiteLLM's fills the gap when it is not.
pub fn effective(cfg: &Config, fetched: &[(String, ModelPrice)]) -> Prices {
    let mut prices: BTreeMap<String, ModelPrice> = fetched.iter().cloned().collect();
    for (model, manual) in &cfg.model_prices {
        match prices.remove(model) {
            // A manual entry alone, or a fully manual model: as written.
            None => {
                prices.insert(model.clone(), *manual);
            }
            Some(mut p) => {
                p.input = manual.input;
                p.output = manual.output;
                p.cache_read = manual.cache_read.or(p.cache_read);
                p.cache_write = manual.cache_write.or(p.cache_write);
                prices.insert(model.clone(), p);
            }
        }
    }
    prices
}

#[derive(Debug, Default, Clone)]
pub struct State {
    /// The prices in effect right now (what spend and budgets price with).
    pub prices: Prices,
    /// When LiteLLM's prices were last received, `None` if never.
    pub fetched_at: Option<String>,
    /// Why the last fetch failed; `None` after a success.
    pub last_error: Option<String>,
}

impl State {
    pub fn load(cfg: &Config, db: &genie_core::server_db::ServerDb) -> State {
        let (fetched_at, last_error) = db.prices_fetch_state().unwrap_or((None, None));
        let fetched = db.model_prices().unwrap_or_default();
        State { prices: effective(cfg, &fetched), fetched_at, last_error }
    }

    /// What spend and budgets price with.
    pub fn prices(&self) -> Prices {
        self.prices.clone()
    }
}

/// What LiteLLM's `/model/info` says, reduced to what genie prices with. The
/// response is large and versioned by deployment: only these fields are read,
/// and anything missing is simply absent from the answer.
#[derive(serde::Deserialize)]
struct ModelInfo {
    model_name: String,
    #[serde(default)]
    model_info: Option<Info>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct Info {
    /// `chat`, `embedding`, … — only chat models are priced.
    #[serde(default)]
    mode: Option<String>,
    input_cost_per_token: Option<f64>,
    output_cost_per_token: Option<f64>,
    cache_read_input_token_cost: Option<f64>,
    cache_creation_input_token_cost: Option<f64>,
}

/// Fetch the tariffs from `base` (`…/v1`) with the service token. Dollars per
/// token become dollars per million, as `modelPrices` is written in; the models
/// keep the `litellm/` prefix the agents' models carry. A model without an
/// input or output price is dropped (embeddings and the like).
pub async fn fetch(base: &str, token: &str) -> Result<Vec<(String, ModelPrice)>, String> {
    let resp = reqwest::Client::new()
        .get(format!("{}/model/info", base.trim_end_matches('/')))
        .bearer_auth(token)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("LiteLLM at {base}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("LiteLLM at {base}: {}", resp.status()));
    }
    let v: serde_json::Value = resp.json().await.map_err(|e| format!("LiteLLM at {base}: {e}"))?;
    let entries: Vec<ModelInfo> = serde_json::from_value(v["data"].clone()).map_err(|e| format!("LiteLLM's /model/info: {e}"))?;
    let per_million = |p: f64| p * 1_000_000.0;
    // One entry per model: a proxy may list several deployments under the same
    // name, and the last one wins (they differ only in tiers we do not read).
    let mut models = std::collections::BTreeMap::new();
    for e in entries {
        let Some(i) = e.model_info else { continue };
        if i.mode.as_deref() != Some("chat") {
            continue;
        }
        let (input, output) = match (i.input_cost_per_token, i.output_cost_per_token) {
            (Some(input), Some(output)) => (input, output),
            _ => continue,
        };
        models.insert(
            format!("litellm/{}", e.model_name),
            ModelPrice {
                input: per_million(input),
                output: per_million(output),
                // A zero tariff is a price (DeepSeek writes cache for free);
                // only an absent one leaves tokens unpriced.
                cache_read: i.cache_read_input_token_cost.map(per_million),
                cache_write: i.cache_creation_input_token_cost.map(per_million),
            },
        );
    }
    Ok(models.into_iter().collect())
}

/// Ask LiteLLM for the tariffs, keep them in `server.db` and make them the
/// prices in effect. The count of priced models is returned for the answer.
pub async fn refresh(app: &App) -> Result<usize, String> {
    let base = app.cfg.litellm.base_url.clone().ok_or_else(|| "litellm.baseUrl is not set in config.json".to_string())?;
    let token = std::env::var(INFO_TOKEN).unwrap_or_default();
    if token.is_empty() {
        return Err(format!("{INFO_TOKEN} is not set: give the server a service token that may read LiteLLM's model info"));
    }
    let fetched = match fetch(&base, &token).await {
        Ok(f) => f,
        Err(e) => {
            record_error(app, &e);
            return Err(e);
        }
    };
    let n = fetched.len();
    let at = genie_core::db::now();
    app.with_server(|db| db.set_model_prices(&fetched, &at)).map_err(|e| e.to_string())?;
    if let Ok(mut state) = app.prices.write() {
        *state = State { prices: effective(&app.cfg, &fetched), fetched_at: Some(at), last_error: None };
    }
    Ok(n)
}

/// Remember why a fetch failed, without touching the prices that work.
pub fn record_error(app: &App, error: &str) {
    let at = genie_core::db::now();
    let _ = app.with_server(|db| db.set_prices_error(&at, error));
    if let Ok(mut state) = app.prices.write() {
        state.fetched_at = Some(at);
        state.last_error = Some(error.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Manual = (&'static str, f64, f64, Option<f64>, Option<f64>);

    fn cfg(prices: &[Manual]) -> Config {
        let mut c = Config::default();
        for (m, i, o, cr, cw) in prices {
            c.model_prices.insert(m.to_string(), ModelPrice { input: *i, output: *o, cache_read: *cr, cache_write: *cw });
        }
        c
    }

    #[test]
    fn manual_fields_win_and_litellm_fills_the_cache() {
        let fetched = vec![
            ("litellm/deep".into(), ModelPrice { input: 0.14, output: 0.28, cache_read: Some(0.006), cache_write: Some(0.14) }),
            ("litellm/new".into(), ModelPrice { input: 1.0, output: 2.0, cache_read: None, cache_write: None }),
        ];
        // The manual entry prices input and output only; the cache comes from LiteLLM.
        let c = cfg(&[("litellm/deep", 0.14, 0.28, None, None)]);
        let p = effective(&c, &fetched);
        assert_eq!(p["litellm/deep"], ModelPrice { input: 0.14, output: 0.28, cache_read: Some(0.006), cache_write: Some(0.14) });
        // A model only LiteLLM knows is priced as received.
        assert_eq!(p["litellm/new"], fetched[1].1);
        // A manual cache price overrides LiteLLM's.
        let c = cfg(&[("litellm/deep", 0.2, 0.3, Some(0.02), None)]);
        let p = effective(&c, &fetched);
        assert_eq!(p["litellm/deep"].cache_read, Some(0.02));
        assert_eq!(p["litellm/deep"].input, 0.2);
    }

    #[test]
    fn a_model_only_manual_prices_know_is_kept() {
        let c = cfg(&[("claude-bridge/sonnet", 3.0, 15.0, Some(0.3), Some(3.75))]);
        let p = effective(&c, &[]);
        assert_eq!(p["claude-bridge/sonnet"].output, 15.0);
    }
}
