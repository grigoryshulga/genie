use crate::common::{Harness, call};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Json, Router, routing::post};
use genie_core::model_calls::{CallState, Scope, Worker};
use genie_core::server_db::{ProjectRole, hash_secret, new_secret};
use genie_core::{Actor, CreateInput, Role};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};
use tower::ServiceExt;

#[derive(Clone)]
struct Wire {
    hits: Arc<AtomicUsize>,
    seen: Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    arrived: Arc<Notify>,
    release: Arc<Semaphore>,
    hold_first: bool,
    mode: &'static str,
}
struct Provider {
    url: String,
    wire: Wire,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Provider {
    async fn new(mode: &'static str, hold_first: bool) -> Self {
        let wire = Wire {
            hits: Default::default(),
            seen: Default::default(),
            arrived: Default::default(),
            release: Arc::new(Semaphore::new(0)),
            hold_first,
            mode,
        };
        let router = Router::new().route("/v1/chat/completions", post(provider)).with_state(wire.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self { url, wire, server }
    }
    async fn hits(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let arrived = self.wire.arrived.notified();
                if self.wire.hits.load(Ordering::SeqCst) >= count {
                    break;
                }
                arrived.await;
            }
        })
        .await
        .unwrap();
    }
}
async fn provider(State(wire): State<Wire>, headers: HeaderMap, Json(body): Json<Value>) -> Response {
    let index = wire.hits.fetch_add(1, Ordering::SeqCst);
    wire.seen.lock().unwrap().push((headers["authorization"].to_str().unwrap().into(), body.clone()));
    wire.arrived.notify_waiters();
    if wire.hold_first && index == 0 {
        wire.release.acquire().await.unwrap().forget();
    }
    if wire.mode == "error" {
        return (StatusCode::TOO_MANY_REQUESTS, Json(json!({"error":"retry later"}))).into_response();
    }
    let usage = if wire.mode == "full" {
        json!({"prompt_tokens":100,"completion_tokens":10,"total_tokens":110})
    } else if wire.mode == "overrun" {
        json!({"prompt_tokens":101,"completion_tokens":11,"total_tokens":112})
    } else {
        json!({"prompt_tokens":45,"prompt_cache_hit_tokens":5,"prompt_cache_miss_tokens":40,"completion_tokens":5,"total_tokens":50,"completion_tokens_details":{"reasoning_tokens":3}})
    };
    if body["stream"] == true {
        if wire.mode == "delayed-stream" {
            let stream = futures_util::stream::unfold((0, wire.release, usage), |(stage, release, usage)| async move {
                match stage {
                    0 => Some((
                        Ok::<_, std::io::Error>(format!("data: {}\n\n", json!({"choices":[{"delta":{"content":"Local answer"}}]}))),
                        (1, release, usage),
                    )),
                    1 => {
                        release.acquire().await.unwrap().forget();
                        Some((Ok(format!("data: {}\n\ndata: [DONE]\n\n", json!({"choices":[],"usage":usage}))), (2, release, usage)))
                    }
                    _ => None,
                }
            });
            return ([("content-type", "text/event-stream")], axum::body::Body::from_stream(stream)).into_response();
        }
        let mut stream = format!(
            "data: {}\r\n\r\n",
            json!({"id":"local","choices":[{"index":0,"delta":{"content":"Local answer"},"finish_reason":null}]})
        );
        stream += &format!("data: {}\r\n\r\n", json!({"id":"local","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}));
        if wire.mode != "missing" {
            stream += &format!("data: {}\r\n\r\n", json!({"choices":[],"usage":usage}));
        }
        if wire.mode != "truncated" {
            stream += "data: [DONE]\r\n\r\n";
        }
        return ([("content-type", "text/event-stream")], stream).into_response();
    }
    let mut value =
        json!({"id":"local","choices":[{"index":0,"message":{"role":"assistant","content":"Local answer"},"finish_reason":"stop"}]});
    if wire.mode != "missing" {
        value["usage"] = usage;
    }
    Json(value).into_response()
}
fn harness(limit: f64) -> Harness {
    let h = Harness::with_config(|cfg| {
        cfg.budgets.per_task = limit;
        cfg.runtime.turn_timeout_secs = 5;
        cfg.model_prices.insert(
            "fake/model".into(),
            genie_core::usage::ModelPrice { input: 1.0, output: 2.0, cache_read: Some(0.5), cache_write: Some(0.0) },
        );
    });
    h.project("shop");
    h
}
fn enroll(h: &Harness, provider: &Provider, initiator: Option<i64>) -> (String, String, String) {
    enroll_under(h, provider, initiator, None)
}
fn enroll_under(h: &Harness, provider: &Provider, initiator: Option<i64>, parent: Option<&str>) -> (String, String, String) {
    let token = h
        .app
        .with_server(|db| db.create_agent_token("shop", Role::Orchestrator, "worker", None, None, chrono::Duration::hours(1)))
        .unwrap();
    let source = h.dir.path().join("provider");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("models.json"), r#"{"providers":{"fake":{"apiKey":"upstream-test-key"}}}"#).unwrap();
    let (task, worker) = h.app.with_tracker("shop", |t| {
        let task = t.create(&Actor::new("owner", Role::Human), CreateInput { title: "Budgeted task".into(), parent: parent.map(str::to_string), ..Default::default() })?.id;
        let worker = Worker { id:format!("mw_{}",new_secret()),token_hash:hash_secret(&token),agent:"worker".into(),task:Some(task.clone()),model:"fake/model".into(),input_limit:100,output_limit:10,profile_fingerprint:"fake-profile".into(),profile:json!({"model":{"provider":"fake","id":"model","api":"openai-completions","baseUrl":provider.url,"contextWindow":100,"maxTokens":10},"source":source}),initiator,execution_stamp:Some(t.execution().stamp(&task)?),closed:false };
        t.model_calls().enroll(&worker)?;
        Ok((task, worker.id))
    }).unwrap();
    (task, worker, token)
}
fn body(stream: bool) -> Value {
    json!({"model":"model","messages":[{"role":"user","content":"A bounded local request"}],"max_tokens":10,"stream":stream})
}
fn endpoint(worker: &str) -> String {
    format!("/api/agent/model-gateway/{worker}/v1/chat/completions?project=shop")
}
async fn settled(h: &Harness, task: &str, state: CallState) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let calls = h.app.with_tracker("shop", |t| t.model_calls().list(Some(task), 50)).unwrap();
            if !calls.is_empty() && calls.iter().all(|call| call.state == state) {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

async fn streaming_response(h: &Harness, worker: &str, token: &str) -> Response {
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(endpoint(worker))
        .header("host", format!("127.0.0.1:{}", h.app.cfg.port))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body(true).to_string()))
        .unwrap();
    h.router.clone().oneshot(request).await.unwrap()
}

#[tokio::test]
async fn server_pins_credentials_caps_and_accounts_once_without_worker_reports() {
    let upstream = Provider::new("normal", false).await;
    let h = harness(1.0);
    let (task, worker, token) = enroll(&h, &upstream, None);
    let mut input = body(true);
    input["max_tokens"] = json!(1000);
    let (status, _, bytes) = call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(input).send_raw().await;
    assert_eq!(status, StatusCode::OK);
    assert!(String::from_utf8(bytes).unwrap().contains("[DONE]"));
    settled(&h, &task, CallState::Settled).await;
    let seen = upstream.wire.seen.lock().unwrap().clone();
    assert_eq!(seen[0].0, "Bearer upstream-test-key");
    assert_eq!(seen[0].1["max_tokens"], 10);
    assert_eq!(seen[0].1["stream_options"]["include_usage"], true);
    let receipt = h.app.with_tracker("shop", |t| t.model_calls().list(Some(&task), 5)).unwrap().remove(0);
    assert_eq!(receipt.cost_nanos.unwrap().value(), 52_500, "reasoning is already included in completion_tokens");
    assert_eq!(h.app.with_tracker("shop", |t| t.usage_of_task(&task)).unwrap()[0].calls, 1);
    assert!(genie::budget::check(&h.app, "shop", Some(&task)).unwrap().is_none(), "gateway usage must not also count as legacy spend");
    let (s, _, _) = call(&h.router, "POST", "/api/agent/usage?project=shop")
        .bearer(&token)
        .json(json!({"reports":[{"model":"fake/model","input":40,"output":5,"cacheRead":5}]}))
        .send()
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(h.app.with_tracker("shop", |t| t.usage_of_task(&task)).unwrap()[0].calls, 1);
}

#[tokio::test]
async fn waiting_reservations_do_not_pause_an_admitted_call_or_double_spend_allowance() {
    let upstream = Provider::new("normal", true).await;
    let h = harness(0.00018);
    let (task, worker, token) = enroll(&h, &upstream, None);
    let first = call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(body(false)).send();
    let other = call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(body(false)).send();
    tokio::pin!(first);
    tokio::pin!(other);
    tokio::select! { _ = &mut first => panic!("first request should be held"), _ = upstream.hits(1) => {} }
    assert!(genie::budget::check(&h.app, "shop", Some(&task)).unwrap().is_none());
    assert!(tokio::time::timeout(Duration::from_millis(100), &mut other).await.is_err());
    assert_eq!(upstream.wire.hits.load(Ordering::SeqCst), 1, "second caller must wait before provider I/O");
    assert!(h.app.with_tracker("shop", |t| t.execution().blocking(&task)).unwrap().is_none());
    upstream.wire.release.add_permits(1);
    assert_eq!(first.await.0, StatusCode::OK);
    assert_eq!(other.await.0, StatusCode::OK);
    settled(&h, &task, CallState::Settled).await;
    assert_eq!(h.app.with_tracker("shop", |t| t.model_calls().exposure(&Scope::Task(task.clone()))).unwrap().value(), 105_000);
    let (status, _, _) = call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(body(false)).send().await;
    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(upstream.wire.hits.load(Ordering::SeqCst), 2);
    assert_eq!(h.app.with_tracker("shop", |t| t.execution().blocking(&task)).unwrap().unwrap().reason, "budget");
}

#[tokio::test]
async fn malformed_or_revoked_requests_never_reach_the_provider() {
    let upstream = Provider::new("normal", false).await;
    let h = harness(1.0);
    let (_, worker, token) = enroll(&h, &upstream, None);
    for input in [
        json!({"model":"another","messages":[],"max_tokens":10}),
        json!({"model":"model","messages":[{"role":"user","content":"x"}]}),
        json!({"model":"model","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/image"}}]}],"max_tokens":10}),
        json!({"model":"model","messages":[{"role":"user","content":"x"}],"max_tokens":10,"web_search_options":{}}),
    ] {
        assert_eq!(call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(input).send().await.0, StatusCode::BAD_REQUEST);
    }
    let wrong = h
        .app
        .with_server(|db| db.create_agent_token("shop", Role::Orchestrator, "worker", None, None, chrono::Duration::hours(1)))
        .unwrap();
    assert_eq!(call(&h.router, "POST", &endpoint(&worker)).bearer(&wrong).json(body(false)).send().await.0, StatusCode::FORBIDDEN);
    h.app.with_server(|db| db.revoke_token(&token)).unwrap();
    assert_eq!(call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(body(false)).send().await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(upstream.wire.hits.load(Ordering::SeqCst), 0);
    assert!(h.app.with_tracker("shop", |t| t.model_calls().list(None, 10)).unwrap().is_empty());
}

#[tokio::test]
async fn uncertain_usage_retains_allowance_and_only_an_admin_reconciles_it() {
    let upstream = Provider::new("missing", false).await;
    let h = harness(0.00018);
    let (owner, member, user) = h
        .app
        .with_server(|db| {
            let owner = db.create_user("owner", "", None, None, true)?;
            let member = db.create_user("member", "", None, None, false)?;
            db.set_membership("shop", member.id, ProjectRole::Member)?;
            Ok((db.create_user_token(owner.id, "test")?, db.create_user_token(member.id, "test")?, owner.id))
        })
        .unwrap();
    let (task, worker, token) = enroll(&h, &upstream, Some(user));
    assert_eq!(call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(body(false)).send().await.0, StatusCode::BAD_GATEWAY);
    settled(&h, &task, CallState::Uncertain).await;
    let receipt = h.app.with_tracker("shop", |t| t.model_calls().list(None, 10)).unwrap().remove(0);
    assert_eq!(h.app.with_tracker("shop", |t| t.model_calls().committed(&Scope::Task(task.clone()))).unwrap().value(), 120_000);
    let path = format!("/api/budget/calls/{}/reconcile?project=shop", receipt.id);
    let evidence = json!({"costNanos":40_000,"note":"Checked the local provider's completed request receipt"});
    for denied in [&member, &token] {
        assert_eq!(call(&h.router, "POST", &path).bearer(denied).json(evidence.clone()).send().await.0, StatusCode::FORBIDDEN);
    }
    assert_eq!(call(&h.router, "POST", &path).bearer(&owner).json(evidence.clone()).send().await.0, StatusCode::OK);
    let event = h.app.with_tracker("shop", |t| t.last_event_id()).unwrap();
    assert_eq!(call(&h.router, "POST", &path).bearer(&owner).json(evidence).send().await.0, StatusCode::OK);
    assert_eq!(h.app.with_tracker("shop", |t| t.last_event_id()).unwrap(), event);
    assert!(
        !call(&h.router, "POST", &path).bearer(&owner).json(json!({"costNanos":0,"note":"Conflicting amount"})).send().await.0.is_success()
    );
    let (status, value, _) = call(&h.router, "GET", "/api/budget/calls?project=shop").bearer(&owner).send().await;
    assert_eq!(status, StatusCode::OK);
    for key in ["agent", "worker", "tokenHash", "profile", "initiator"] {
        assert!(value["calls"][0].get(key).is_none());
    }
    assert_eq!(h.app.with_tracker("shop", |t| t.model_calls().exposure(&Scope::Day)).unwrap().value(), 40_000);
}

#[tokio::test]
async fn access_is_checked_again_after_a_request_waits_for_budget_credit() {
    let upstream = Provider::new("normal", true).await;
    let h = harness(0.00018);
    let user = h
        .app
        .with_server(|db| {
            let user = db.create_user("member", "", None, None, false)?;
            db.set_membership("shop", user.id, ProjectRole::Member)?;
            Ok(user.id)
        })
        .unwrap();
    let (_, worker, token) = enroll(&h, &upstream, Some(user));
    let first = call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(body(false)).send();
    let second = call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(body(false)).send();
    tokio::pin!(first);
    tokio::pin!(second);
    tokio::select! { _=&mut first=>panic!("first call should wait"), _=upstream.hits(1)=>{} }
    assert!(tokio::time::timeout(Duration::from_millis(100), &mut second).await.is_err());
    h.app.with_server(|db| db.set_membership("shop", user, ProjectRole::Viewer)).unwrap();
    upstream.wire.release.add_permits(1);
    assert_eq!(first.await.0, StatusCode::OK, "an already accepted expense must still settle after authority is revoked");
    assert!(!second.await.0.is_success());
    assert_eq!(upstream.wire.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn incomplete_streams_fail_without_a_success_terminator_and_keep_their_reserve() {
    for mode in ["missing", "truncated"] {
        let upstream = Provider::new(mode, false).await;
        let h = harness(1.0);
        let (task, worker, token) = enroll(&h, &upstream, None);
        let response = streaming_response(&h, &worker, &token).await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut bytes = Vec::new();
        let mut failed = false;
        let mut body = response.into_body();
        while let Some(frame) = body.frame().await {
            match frame {
                Ok(frame) => {
                    if let Ok(data) = frame.into_data() {
                        bytes.extend_from_slice(&data);
                    }
                }
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed, "{mode} must abort the client stream");
        assert!(!String::from_utf8(bytes).unwrap().contains("[DONE]"));
        settled(&h, &task, CallState::Uncertain).await;
        assert_eq!(h.app.with_tracker("shop", |t| t.model_calls().exposure(&Scope::Task(task.clone()))).unwrap().value(), 120_000);
        assert!(h.app.with_tracker("shop", |t| t.usage_of_task(&task)).unwrap().is_empty());
    }
}

#[tokio::test]
async fn client_disconnect_and_revocation_cannot_discard_an_accepted_provider_expense() {
    let upstream = Provider::new("delayed-stream", false).await;
    let h = harness(1.0);
    let (task, worker, token) = enroll(&h, &upstream, None);
    let response = streaming_response(&h, &worker, &token).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(h.app.with_tracker("shop", |t| t.model_calls().pending(&Scope::Task(task.clone()))).unwrap().value(), 120_000);
    h.app.with_server(|db| db.revoke_token(&token)).unwrap();
    drop(response);
    upstream.wire.release.add_permits(1);
    settled(&h, &task, CallState::Settled).await;
    assert_eq!(h.app.with_tracker("shop", |t| t.model_calls().exposure(&Scope::Task(task.clone()))).unwrap().value(), 52_500);
    assert_eq!(upstream.wire.hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_upstream_attempts_keep_exposure_and_bound_violations_keep_the_actual_cost() {
    for (mode, state, cost) in [("error", CallState::Uncertain, 120_000), ("overrun", CallState::Settled, 123_000)] {
        let upstream = Provider::new(mode, false).await;
        let h = harness(1.0);
        let (task, worker, token) = enroll(&h, &upstream, None);
        let (status, _, _) = call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(body(false)).send().await;
        assert_eq!(status, if mode == "error" { StatusCode::BAD_GATEWAY } else { StatusCode::OK });
        settled(&h, &task, state).await;
        assert_eq!(h.app.with_tracker("shop", |t| t.model_calls().exposure(&Scope::Task(task.clone()))).unwrap().value(), cost);
        assert_eq!(upstream.wire.hits.load(Ordering::SeqCst), 1, "the transport must not retry invisibly");
        if mode == "overrun" {
            assert!(h.app.with_tracker("shop", |t| t.execution().blocking(&task)).unwrap().is_some());
            assert!(h.app.with_tracker("shop", |t| t.model_calls().profile_fault("fake-profile")).unwrap().is_some());
        }
    }
}

#[tokio::test]
async fn unknown_or_incomplete_tariffs_and_insufficient_credit_do_not_send_a_request() {
    for mode in ["unknown", "incomplete", "credit"] {
        let upstream = Provider::new("normal", false).await;
        let h = Harness::with_config(|cfg| {
            cfg.budgets.per_task = if mode == "credit" { 0.00001 } else { 1.0 };
            cfg.model_prices.insert(
                "another-provider/model".into(),
                genie_core::usage::ModelPrice { input: 0.0, output: 0.0, cache_read: Some(0.0), cache_write: Some(0.0) },
            );
            if mode != "unknown" {
                cfg.model_prices.insert(
                    "fake/model".into(),
                    genie_core::usage::ModelPrice {
                        input: 1.0,
                        output: 2.0,
                        cache_read: if mode == "incomplete" { None } else { Some(0.5) },
                        cache_write: Some(0.0),
                    },
                );
            }
        });
        h.project("shop");
        let (_, worker, token) = enroll(&h, &upstream, None);
        assert!(!call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(body(false)).send().await.0.is_success(), "{mode}");
        assert_eq!(upstream.wire.hits.load(Ordering::SeqCst), 0, "{mode}");
        assert!(h.app.with_tracker("shop", |t| t.model_calls().list(None, 10)).unwrap().is_empty());
    }
}

#[tokio::test]
async fn settlement_enforces_the_original_epic_after_a_task_moves_during_provider_io() {
    let upstream = Provider::new("full", true).await;
    let h = Harness::with_config(|cfg| {
        cfg.budgets.per_epic = 0.00012;
        cfg.model_prices.insert(
            "fake/model".into(),
            genie_core::usage::ModelPrice { input: 1.0, output: 2.0, cache_read: Some(0.5), cache_write: Some(0.0) },
        );
    });
    h.project("shop");
    let (old, new) = h
        .app
        .with_tracker("shop", |t| {
            let actor = Actor::new("owner", Role::Human);
            let old = t
                .create(
                    &actor,
                    CreateInput { title: "Original epic".into(), task_type: Some(genie_core::TaskType::Epic), ..Default::default() },
                )?
                .id;
            let new = t
                .create(
                    &actor,
                    CreateInput { title: "New epic".into(), task_type: Some(genie_core::TaskType::Epic), ..Default::default() },
                )?
                .id;
            Ok((old, new))
        })
        .unwrap();
    let (task, worker, token) = enroll_under(&h, &upstream, None, Some(&old));
    let pending = call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(body(false)).send();
    tokio::pin!(pending);
    tokio::select! { _=&mut pending=>panic!("provider should be held"), _=upstream.hits(1)=>{} }
    h.app
        .with_tracker("shop", |t| {
            t.update(
                &Actor::new("owner", Role::Human),
                &task,
                genie_core::UpdateInput { parent: Some(Some(new.clone())), ..Default::default() },
            )
        })
        .unwrap();
    upstream.wire.release.add_permits(1);
    assert_eq!(pending.await.0, StatusCode::OK);
    let pause = h.app.with_tracker("shop", |t| t.execution().blocking(&old)).unwrap().unwrap();
    assert_eq!(pause.reason, "budget");
    assert!(h.app.with_tracker("shop", |t| t.execution().blocking(&new)).unwrap().is_none());
    assert!(h.app.with_tracker("shop", |t| t.execution().blocking(&task)).unwrap().is_none());
    assert_eq!(h.app.with_tracker("shop", |t| t.model_calls().exposure(&Scope::Epic(old))).unwrap().value(), 120_000);
    assert_eq!(h.app.with_tracker("shop", |t| t.model_calls().exposure(&Scope::Epic(new))).unwrap().value(), 0);
}

#[tokio::test]
async fn a_human_reconciliation_that_exhausts_credit_pauses_its_frozen_scope() {
    let upstream = Provider::new("missing", false).await;
    let h = harness(0.00018);
    let (task, worker, token) = enroll(&h, &upstream, None);
    assert_eq!(call(&h.router, "POST", &endpoint(&worker)).bearer(&token).json(body(false)).send().await.0, StatusCode::BAD_GATEWAY);
    let receipt = h.app.with_tracker("shop", |t| t.model_calls().list(Some(&task), 1)).unwrap().remove(0);
    let (status, _, _) = call(&h.router, "POST", &format!("/api/budget/calls/{}/reconcile?project=shop", receipt.id))
        .json(json!({"costNanos":180000,"note":"Verified the final provider invoice"}))
        .send()
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h.app.with_tracker("shop", |t| t.execution().blocking(&task)).unwrap().unwrap().reason, "budget");
}
