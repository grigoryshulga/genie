//! The models' tariffs from LiteLLM: they arrive at boot (when the server is
//! configured for it), an administrator's action pulls them again, `modelPrices`
//! of config.json overrides them per field, and a LiteLLM that is down leaves
//! the prices that work plus a warning. What cache tokens cost is settled in
//! `genie_core::usage`'s tests: without a price they are unpriced, never input.

use crate::common;

use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use common::{Harness, call};
use genie::config::ModelPrice;
use serde_json::{Value, json};

static TOKEN: Once = Once::new();

fn info_token() {
    TOKEN.call_once(|| unsafe { std::env::set_var("LITELLM_INFO_TOKEN", "info-token") });
}

type Models = Arc<Mutex<Value>>;

/// A LiteLLM that answers `/model/info` with whatever `models` holds right now,
/// to the service token `info-token` only.
async fn fake_litellm(models: Models) -> String {
    let route = get(|State(models): State<Models>, headers: HeaderMap| async move {
        if headers.get("authorization").and_then(|v| v.to_str().ok()) != Some("Bearer info-token") {
            return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "Bad credentials" })));
        }
        (StatusCode::OK, Json(json!({ "data": models.lock().unwrap().clone() })))
    });
    let app: Router = Router::new().route("/v1/model/info", route).with_state(models);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

fn models_before_change() -> Value {
    json!([
        { "model_name": "gpt-6-sol", "model_info": {
            "mode": "chat",
            "input_cost_per_token": 0.000002, "output_cost_per_token": 0.00001,
            "cache_read_input_token_cost": 2e-7, "cache_creation_input_token_cost": 0.0000025 } },
        { "model_name": "deep-vision", "model_info": {
            "mode": "chat",
            "input_cost_per_token": 1.4e-7, "output_cost_per_token": 2.8e-7,
            "cache_creation_input_token_cost": 0 } },
        { "model_name": "embed-text", "model_info": { "mode": "embedding", "input_cost_per_token": 1e-7 } }
    ])
}

fn price_of(v: &Value, model: &str) -> Value {
    v["models"].as_array().unwrap().iter().find(|m| m["model"] == json!(model)).unwrap()["price"].clone()
}

#[tokio::test]
async fn prices_arrive_at_boot_merge_with_modelprices_and_refresh_by_an_administrator() {
    info_token();
    let models: Models = Arc::new(Mutex::new(models_before_change()));
    let base = fake_litellm(models.clone()).await;
    let h = Harness::with_config(|cfg| {
        cfg.litellm.base_url = Some(base);
        // A manual price for the input and output only: the cache must come from LiteLLM,
        // and the manual cacheRead (0.5) must override what LiteLLM says (0.2).
        cfg.model_prices
            .insert("litellm/gpt-6-sol".into(), ModelPrice { input: 2.0, output: 10.0, cache_read: Some(0.5), cache_write: None });
    });
    // Boot: serve_on loads the tariffs in the background; wait for them to land.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    tokio::spawn(genie::serve_on(h.app.clone(), listener, std::future::pending()));
    for _ in 0..250 {
        if h.app.with_server(|db| db.model_prices()).unwrap().len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (st, v, _) = call(&h.router, "GET", "/api/model-prices").send().await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert!(v["fetchedAt"].as_str().is_some_and(|a| !a.is_empty()), "the fetch time is shown: {v}");
    assert_eq!(v["models"].as_array().unwrap().len(), 2, "the embedding model is not priced: {v}");
    let sol = price_of(&v, "litellm/gpt-6-sol");
    assert_eq!(sol["cacheRead"], json!(0.5), "the manual cache price wins: {sol}");
    assert_eq!(sol["cacheWrite"], json!(2.5), "the cache write LiteLLM knows fills the gap: {sol}");
    let deep = price_of(&v, "litellm/deep-vision");
    assert_eq!(deep["cacheRead"], Value::Null, "no cache price is invented: {deep}");
    assert_eq!(deep["cacheWrite"], json!(0.0), "a zero tariff is a price, not a gap: {deep}");

    // The prices in effect price the spend: the merged set is what the app holds.
    let eff = h.app.prices().unwrap();
    assert_eq!(eff["litellm/gpt-6-sol"].cache_read, Some(0.5));
    assert_eq!(eff["litellm/deep-vision"].cache_read, None);

    // The administrator pulls the config again: LiteLLM now prices deep-vision's
    // cache (a second deployment of the same model — the last one wins) and a
    // new model arrives.
    {
        let mut m = models.lock().unwrap();
        let arr = m.as_array_mut().unwrap();
        arr.push(json!({
            "model_name": "deep-vision", "model_info": { "mode": "chat",
                "input_cost_per_token": 1.4e-7, "output_cost_per_token": 2.8e-7,
                "cache_read_input_token_cost": 6e-9, "cache_creation_input_token_cost": 1.4e-7 }
        }));
        arr.push(json!({
            "model_name": "kimi-k3", "model_info": { "mode": "chat",
                "input_cost_per_token": 5e-7, "output_cost_per_token": 2e-6 }
        }));
    }
    let (st, v, _) = call(&h.router, "POST", "/api/model-prices").send().await;
    assert_eq!(st, StatusCode::OK, "{v}");
    assert_eq!(v["received"], json!(3), "three chat models came back: {v}");
    let (_, v, _) = call(&h.router, "GET", "/api/model-prices").send().await;
    assert_eq!(price_of(&v, "litellm/deep-vision")["cacheRead"], json!(0.006), "refreshed, the later deployment wins: {v}");
    assert_eq!(price_of(&v, "litellm/kimi-k3")["input"], json!(0.5), "the new model is priced: {v}");
}

#[tokio::test]
async fn a_refusing_token_is_an_error_and_a_dead_litellm_keeps_the_manual_prices() {
    info_token();
    // Port 9 (discard): nothing listens there, the connection is refused at once.
    let h = Harness::with_config(|cfg| {
        cfg.litellm.base_url = Some("http://127.0.0.1:9".into());
        cfg.model_prices.insert("litellm/gpt-6-sol".into(), ModelPrice { input: 2.0, output: 10.0, cache_read: None, cache_write: None });
    });
    let (st, v, _) = call(&h.router, "POST", "/api/model-prices").send().await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert!(v["error"].as_str().is_some_and(|e| e.contains("LiteLLM")), "{v}");
    // What already works keeps working: the manual prices price the spend, the
    // failure is visible, and doctor warns about it.
    let (_, v, _) = call(&h.router, "GET", "/api/model-prices").send().await;
    assert!(v["lastError"].as_str().is_some_and(|e| e.contains("LiteLLM")), "{v}");
    assert_eq!(price_of(&v, "litellm/gpt-6-sol")["input"], json!(2.0), "the manual price stays: {v}");
    let (st, v, _) = call(&h.router, "GET", "/api/doctor").send().await;
    assert_eq!(st, StatusCode::OK);
    let check = v["checks"].as_array().unwrap().iter().find(|c| c["area"] == json!("models")).unwrap();
    assert_eq!(check["level"], json!("warn"), "doctor warns about the unfetched prices: {check}");
    assert!(check["text"].as_str().is_some_and(|t| t.contains("failed")), "{check}");
}

#[tokio::test]
async fn a_wrong_service_token_is_explained() {
    info_token();
    let models: Models = Arc::new(Mutex::new(models_before_change()));
    let base = fake_litellm(models).await;
    let h = Harness::with_config(|cfg| cfg.litellm.base_url = Some(base));
    // The harness-wide token is right; a wrong one comes from a server started
    // elsewhere — here the fetch is made directly, with the wrong key.
    let err = genie::model_prices::fetch(h.app.cfg.litellm.base_url.as_deref().unwrap(), "wrong").await.unwrap_err();
    assert!(err.contains("401"), "{err}");
}
