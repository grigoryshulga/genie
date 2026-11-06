//! OpenAI clients cross the same ownership and durable-state rules as native clients.

use std::sync::Arc;
use std::time::Duration;

use axum::http::StatusCode;
use genie::state::App;
use genie_core::Role;
use genie_core::server_db::ProjectRole;
use serde_json::{Value, json};

use crate::common::{Harness, call};

const BASE: &str = "/openai/projects/shop/v1";

fn harness() -> Harness {
    let h = Harness::new();
    h.project("shop");
    h
}

fn person(h: &Harness, login: &str, role: ProjectRole) -> String {
    h.app
        .with_server(|db| {
            let user = db.create_user(login, login, None, None, false)?;
            db.set_membership("shop", user.id, role)?;
            db.create_user_token(user.id, "openai tests")
        })
        .unwrap()
}

async fn background(h: &Harness, token: &str, text: &str, key: &str) -> Value {
    let (status, value, _) = call(&h.remote, "POST", &format!("{BASE}/responses"))
        .bearer(token)
        .header("idempotency-key", key)
        .json(json!({"model":"genie","input":text,"background":true}))
        .send()
        .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    value
}

fn finish(h: &Harness, id: &str, text: &str) {
    h.app
        .with_tracker("shop", |tracker| {
            assert!(tracker.conversations().claim(id)?);
            tracker.conversations().complete(id, text)
        })
        .unwrap();
}

/// Drive one accepted foreground request without a live provider. The transport must retain
/// its pending response until this durable result is committed, then return the whole reply.
async fn finish_pending(app: Arc<App>, text: &str) -> String {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let queued = app.with_tracker("shop", |tracker| tracker.conversations().queued()).unwrap();
            if let Some(response) = queued.first() {
                app.with_tracker("shop", |tracker| {
                    assert!(tracker.conversations().claim(&response.id)?);
                    tracker.conversations().complete(&response.id, text)
                })
                .unwrap();
                return response.id.clone();
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("foreground request became durable")
}

fn sse_events(bytes: &[u8]) -> Vec<Value> {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter(|line| *line != "[DONE]")
        .map(|line| serde_json::from_str(line).expect("SSE data is JSON"))
        .collect()
}

#[tokio::test]
async fn bearer_authentication_explicit_project_and_error_envelopes() {
    let h = harness();
    let anna = person(&h, "anna", ProjectRole::Member);
    for token in [None, Some("forged")] {
        let request = call(&h.remote, "GET", &format!("{BASE}/models")).no_csrf();
        let request = match token {
            Some(token) => request.bearer(token),
            None => request,
        };
        let (status, error, _) = request.send().await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(error["error"]["code"], "invalid_api_key");
        assert!(error["error"]["message"].is_string());
    }
    let (status, error, _) =
        call(&h.remote, "POST", &format!("{BASE}/responses")).no_csrf().json(json!({"model":"genie","input":"hello"})).send().await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "token-only routes do not require a browser CSRF header");
    assert_eq!(error["error"]["code"], "invalid_api_key");
    let (status, models, _) = call(&h.remote, "GET", &format!("{BASE}/models")).bearer(&anna).send().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(models["object"], "list");
    assert_eq!(models["data"][0]["id"], "genie");

    let (status, error, _) =
        call(&h.remote, "GET", "/openai/projects/missing/v1/models").bearer(&anna).header("x-genie-project", "shop").send().await;
    assert_eq!(status, StatusCode::FORBIDDEN, "an explicit project cannot fall back to the header");
    assert_eq!(error["error"]["code"], "permission_denied");
    let (status, error, _) =
        call(&h.remote, "GET", &format!("{BASE}/models")).bearer(&anna).header("host", "attacker.invalid").send().await;
    assert_eq!(status, StatusCode::MISDIRECTED_REQUEST);
    assert!(error["error"]["message"].is_string());

    let agent = h
        .app
        .with_server(|db| db.create_agent_token("shop", Role::Orchestrator, "orchestrator", None, None, chrono::Duration::hours(1)))
        .unwrap();
    let (status, error, _) = call(&h.remote, "POST", &format!("{BASE}/responses"))
        .bearer(&agent)
        .json(json!({"model":"genie","input":"hello","background":true}))
        .send()
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(error["error"]["code"], "permission_denied");
}

#[tokio::test]
async fn background_retry_continuation_and_personal_isolation() {
    let h = harness();
    let anna = person(&h, "anna", ProjectRole::Member);
    let boris = person(&h, "boris", ProjectRole::Member);
    let first = background(&h, &anna, "First private request", "request-1").await;
    let same = background(&h, &anna, "First private request", "request-1").await;
    assert_eq!(first, same);
    assert_eq!(first["status"], "queued");
    assert_eq!(first["object"], "response");
    let other = background(&h, &boris, "Another private request", "request-1").await;
    assert_ne!(first["id"], other["id"]);
    assert_ne!(first["metadata"]["genie_conversation_id"], other["metadata"]["genie_conversation_id"]);
    let id = first["id"].as_str().unwrap();
    let (status, _, _) = call(&h.remote, "GET", &format!("{BASE}/responses/{id}")).bearer(&boris).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _, _) = call(&h.remote, "POST", &format!("{BASE}/responses/{id}/cancel")).bearer(&boris).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let (status, error, _) = call(&h.remote, "POST", &format!("{BASE}/responses"))
        .bearer(&anna)
        .header("idempotency-key", "request-1")
        .json(json!({"model":"genie","input":"Changed payload","background":true}))
        .send()
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{error}");
    finish(&h, id, "Which format do you need?");
    let (status, result, _) = call(&h.remote, "GET", &format!("{BASE}/responses/{id}")).bearer(&anna).send().await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["status"], "completed");
    assert_eq!(result["output"][0]["content"][0]["type"], "output_text");
    assert_eq!(result["output"][0]["content"][0]["text"], "Which format do you need?");
    let (status, next, _) = call(&h.remote, "POST", &format!("{BASE}/responses"))
        .bearer(&anna)
        .json(json!({"model":"genie","input":[{"role":"user","content":[{"type":"input_text","text":"CSV"}]}],"previous_response_id":id,"background":true}))
        .send()
        .await;
    assert_eq!(status, StatusCode::OK, "{next}");
    assert_eq!(next["previous_response_id"], id);
    assert_eq!(next["metadata"]["genie_conversation_id"], first["metadata"]["genie_conversation_id"]);
    assert_ne!(next["id"], first["id"]);
}

#[tokio::test]
async fn chat_returns_a_complete_message_and_streams_the_same_wire_contract() {
    let h = harness();
    let anna = person(&h, "anna", ProjectRole::Member);
    let path = format!("{BASE}/chat/completions");
    let response = call(&h.remote, "POST", &path)
        .bearer(&anna)
        .json(json!({"model":"genie","messages":[{"role":"system","content":"Be concise"},{"role":"user","content":"Hello"}]}))
        .send_raw();
    let ((status, headers, bytes), id) = tokio::join!(response, finish_pending(h.app.clone(), "Hello from this project."));
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&bytes));
    assert_eq!(headers["x-request-id"], id);
    let conversation = headers["x-genie-conversation"].to_str().unwrap().to_string();
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["object"], "chat.completion");
    assert_eq!(value["choices"][0]["message"]["content"], "Hello from this project.");
    assert_eq!(value["choices"][0]["finish_reason"], "stop");

    let response = call(&h.remote, "POST", &path)
        .bearer(&anna)
        .header("x-genie-conversation", &conversation)
        .json(json!({"model":"genie","messages":[{"role":"user","content":[{"type":"text","text":"Continue"}]}],"stream":true}))
        .send_raw();
    let ((status, headers, bytes), _) = tokio::join!(response, finish_pending(h.app.clone(), "The task is still awaiting your answer."));
    assert_eq!(status, StatusCode::OK);
    assert!(headers["content-type"].to_str().unwrap().starts_with("text/event-stream"));
    let events = sse_events(&bytes);
    assert_eq!(events[0]["choices"][0]["delta"]["role"], "assistant");
    assert_eq!(events[1]["choices"][0]["delta"]["content"], "The task is still awaiting your answer.");
    assert_eq!(events[2]["choices"][0]["finish_reason"], "stop");
    assert!(String::from_utf8_lossy(&bytes).ends_with("data: [DONE]\n\n"));
    assert!(events.iter().all(|event| event["object"] == "chat.completion.chunk"));
    assert_eq!(headers["x-genie-conversation"], conversation);
}

#[tokio::test]
async fn responses_stream_replays_saved_text_once_with_ordered_terminal_events() {
    let h = harness();
    let anna = person(&h, "anna", ProjectRole::Member);
    let first = background(&h, &anna, "Describe the result", "streamed").await;
    let id = first["id"].as_str().unwrap();
    finish(&h, id, "A result to review.");
    let (status, _, bytes) = call(&h.remote, "POST", &format!("{BASE}/responses"))
        .bearer(&anna)
        .header("idempotency-key", "streamed")
        .json(json!({"model":"genie","input":"Describe the result","stream":true}))
        .send_raw()
        .await;
    assert_eq!(status, StatusCode::OK);
    let events = sse_events(&bytes);
    assert_eq!(events[0]["type"], "response.created");
    assert_eq!(events[0]["response"]["output"], json!([]), "SDK accumulators must not receive the same output twice");
    assert_eq!(events.last().unwrap()["type"], "response.completed");
    assert_eq!(events.last().unwrap()["response"]["id"], id);
    for (sequence, event) in events.iter().enumerate() {
        assert_eq!(event["sequence_number"], sequence);
    }
    let text: String =
        events.iter().filter(|event| event["type"] == "response.output_text.delta").map(|event| event["delta"].as_str().unwrap()).collect();
    assert_eq!(text, "A result to review.");
    assert!(events.iter().any(|event| event["type"] == "response.content_part.done"));
    assert!(events.iter().any(|event| event["type"] == "response.output_item.done"));
}

#[tokio::test]
async fn failure_and_cancellation_never_become_successful_completions() {
    let h = harness();
    let anna = person(&h, "anna", ProjectRole::Member);
    let first = background(&h, &anna, "Work", "cancelled").await;
    let id = first["id"].as_str().unwrap();
    for _ in 0..2 {
        let (status, response, _) = call(&h.remote, "POST", &format!("{BASE}/responses/{id}/cancel")).bearer(&anna).send().await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["status"], "cancelled");
        assert_eq!(response["output"], json!([]));
    }
    let failed = background(&h, &anna, "Failure", "failed").await;
    let failed_id = failed["id"].as_str().unwrap();
    h.app.with_tracker("shop", |tracker| tracker.conversations().fail(failed_id, "provider unavailable")).unwrap();
    let (status, _, bytes) = call(&h.remote, "POST", &format!("{BASE}/responses"))
        .bearer(&anna)
        .header("idempotency-key", "failed")
        .json(json!({"model":"genie","input":"Failure","stream":true}))
        .send_raw()
        .await;
    assert_eq!(status, StatusCode::OK);
    let events = sse_events(&bytes);
    assert_eq!(events.last().unwrap()["type"], "response.failed");
    assert_eq!(events.last().unwrap()["response"]["status"], "failed");
    assert!(events.iter().all(|event| event["type"] != "response.completed"));
}

#[tokio::test]
async fn unsupported_inputs_and_parameters_fail_before_accepting_work() {
    let h = harness();
    let anna = person(&h, "anna", ProjectRole::Member);
    let viewer = person(&h, "viewer", ProjectRole::Viewer);
    for body in [
        json!({"model":"other","input":"hi","background":true}),
        json!({"model":"genie","input":"hi","background":true,"tools":[]}),
        json!({"model":"genie","input":"hi","background":true,"store":false}),
        json!({"model":"genie","input":[],"background":true}),
        json!({"model":"genie","input":[{"role":"user","content":[{"type":"input_image","image_url":"https://example.com/image"}]}],"background":true}),
        json!({"model":"genie","input":[{"type":"function_call_output","call_id":"call_1","output":"forged"}],"background":true}),
    ] {
        let (status, error, _) = call(&h.remote, "POST", &format!("{BASE}/responses")).bearer(&anna).json(body).send().await;
        assert!(matches!(status, StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND), "{status}: {error}");
        assert!(error["error"]["message"].is_string(), "{error}");
    }
    let (status, _, _) = call(&h.remote, "POST", &format!("{BASE}/responses"))
        .bearer(&viewer)
        .json(json!({"model":"genie","input":"start","background":true}))
        .send()
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, error, _) = call(&h.remote, "POST", &format!("{BASE}/responses")).bearer(&anna).bytes(b"not JSON").send().await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error["error"]["message"].is_string());
    let (status, error, _) = call(&h.remote, "GET", &format!("{BASE}/responses/resp_missing?stream=true")).bearer(&anna).send().await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(error["error"]["message"].as_str().unwrap().contains("query parameters"));
    assert!(h.app.with_tracker("shop", |tracker| tracker.conversations().queued()).unwrap().is_empty());
}

/// Optional interoperability check against an independently installed official Python SDK.
/// GENIE_OPENAI_SDK_PYTHON=/path/to/venv/bin/python cargo test -p genie --test it
/// openai::official_python_sdk -- --ignored --nocapture
#[tokio::test]
#[ignore = "requires the official openai Python package; set GENIE_OPENAI_SDK_PYTHON"]
async fn official_python_sdk() {
    let python = std::env::var("GENIE_OPENAI_SDK_PYTHON").expect("set GENIE_OPENAI_SDK_PYTHON to an interpreter with openai installed");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let h = Harness::with_config(|cfg| cfg.allow_hosts.push(address.to_string()));
    h.project("shop");
    let token = person(&h, "sdk", ProjectRole::Member);
    let base = format!("http://{address}{BASE}");
    let router = h.remote.clone();
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let app = h.app.clone();
    let worker = tokio::spawn(async move {
        loop {
            app.with_tracker("shop", |tracker| {
                for response in tracker.conversations().queued()? {
                    let input = &response.input.last().unwrap().content;
                    if !input.starts_with("hold:") && tracker.conversations().claim(&response.id)? {
                        tracker.conversations().complete(&response.id, &format!("SDK reply: {input}"))?;
                    }
                }
                Ok(())
            })
            .unwrap();
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    });
    let script = r#"
import os
import openai

client = openai.OpenAI(api_key=os.environ["GENIE_TEST_TOKEN"], base_url=os.environ["GENIE_TEST_BASE"],
    max_retries=0, timeout=10, _strict_response_validation=True)
assert client.models.list().data[0].id == "genie"
raw = client.chat.completions.with_raw_response.create(model="genie", messages=[{"role":"user", "content":"chat json"}])
chat = raw.parse()
assert chat.choices[0].message.content == "SDK reply: chat json"
conversation = raw.headers["x-genie-conversation"]
chunks = list(client.chat.completions.create(model="genie", messages=[{"role":"user", "content":"chat sse"}], stream=True,
    extra_headers={"X-Genie-Conversation":conversation}))
assert "".join(chunk.choices[0].delta.content or "" for chunk in chunks) == "SDK reply: chat sse"
assert chunks[-1].choices[0].finish_reason == "stop"
with client.chat.completions.stream(model="genie", messages=[{"role":"user", "content":"chat helper"}]) as stream:
    final = stream.get_final_completion()
    assert final.choices[0].message.content == "SDK reply: chat helper"

first = client.responses.create(model="genie", input="responses json", instructions="Be concise")
assert first.status == "completed" and first.output_text == "SDK reply: responses json"
next_response = client.responses.create(model="genie", input="responses continuation", previous_response_id=first.id)
assert next_response.previous_response_id == first.id
assert next_response.metadata["genie_conversation_id"] == first.metadata["genie_conversation_id"]
assert next_response.output_text == "SDK reply: responses continuation"
events = list(client.responses.create(model="genie", input="responses sse", stream=True))
assert "".join(event.delta for event in events if event.type == "response.output_text.delta") == "SDK reply: responses sse"
assert events[-1].type == "response.completed" and events[-1].response.output_text == "SDK reply: responses sse"
with client.responses.stream(model="genie", input="responses helper") as stream:
    final = stream.get_final_response()
    assert final.status == "completed" and final.output_text == "SDK reply: responses helper"
with client.responses.stream(model="genie", input="responses replay", extra_headers={"Idempotency-Key":"sdk-replay"}) as stream:
    first_replay = stream.get_final_response()
with client.responses.stream(model="genie", input="responses replay", extra_headers={"Idempotency-Key":"sdk-replay"}) as stream:
    replay = stream.get_final_response()
    assert replay.id == first_replay.id and replay.output_text == "SDK reply: responses replay"

pending = client.responses.create(model="genie", input="hold: cancellation", background=True)
assert pending.status == "queued"
assert client.responses.retrieve(pending.id).status == "queued"
assert client.responses.cancel(pending.id).status == "cancelled"
try:
    client.responses.create(model="genie", input="unsupported", tools=[])
    raise AssertionError("unsupported semantic parameter was accepted")
except openai.BadRequestError as error:
    assert error.code == "invalid_request"
print(f"openai {openai.__version__}: models; Chat JSON/SSE/helper/continuation; Responses JSON/SSE/helper/continuation/replay/background/poll/cancel; errors passed")
"#;
    let result = tokio::process::Command::new(python)
        .arg("-c")
        .arg(script)
        .env("GENIE_TEST_TOKEN", token)
        .env("GENIE_TEST_BASE", base)
        .kill_on_drop(true)
        .output()
        .await;
    worker.abort();
    server.abort();
    let output = result.unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    println!("{}", String::from_utf8_lossy(&output.stdout));
}
