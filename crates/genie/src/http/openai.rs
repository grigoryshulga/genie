//! Text-only OpenAI wire adapters over durable, independently authorized conversations.
//!
//! The HTTP response describes one agent turn, not completion of its linked tasks. SSE keeps
//! the connection alive while the turn runs and emits the persisted final reply as one delta;
//! private tool calls, intermediate thoughts and other conversations never enter this stream.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, FromRequest, FromRequestParts, Path, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response as HttpResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::stream;
use genie_core::GenieError;
use genie_core::conversations::{Message, Response, ResponseStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::ApiError;
use super::ctx::Ctx;
use crate::conversations::{self, Submit};
use crate::state::{App, AppError};

const MODEL: &str = "genie";
const POLL: Duration = Duration::from_millis(250);
const MAX_MESSAGES: usize = 100;
const MAX_INPUT_BYTES: usize = 128 * 1024;

pub fn routes() -> Router<Arc<App>> {
    let routes = Router::new()
        .route("/models", get(models))
        .route("/chat/completions", post(chat))
        .route("/responses", post(respond))
        .route("/responses/{id}", get(retrieve))
        .route("/responses/{id}/cancel", post(cancel))
        .fallback(|| async { Error::new(StatusCode::NOT_FOUND, "unknown OpenAI endpoint") })
        .method_not_allowed_fallback(|| async { Error::new(StatusCode::METHOD_NOT_ALLOWED, "method not allowed") })
        .layer(DefaultBodyLimit::max(256 * 1024));
    Router::new().nest("/openai/projects/{project}/v1", routes)
}

#[derive(Debug)]
struct Error {
    status: StatusCode,
    message: String,
    code: &'static str,
}

impl Error {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        let code = match status {
            StatusCode::UNAUTHORIZED => "invalid_api_key",
            StatusCode::FORBIDDEN => "permission_denied",
            StatusCode::NOT_FOUND => "not_found",
            StatusCode::CONFLICT => "conflict",
            StatusCode::PAYLOAD_TOO_LARGE => "request_too_large",
            _ if status.is_server_error() => "server_error",
            _ => "invalid_request",
        };
        Self { status, message: message.into(), code }
    }

    fn bad(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    fn body(&self) -> Value {
        json!({"error": {"message": self.message, "type": if self.status.is_server_error() { "server_error" } else { "invalid_request_error" }, "param": null, "code": self.code}})
    }
}

impl From<ApiError> for Error {
    fn from(e: ApiError) -> Self {
        Self::new(e.status, e.message)
    }
}

impl From<AppError> for Error {
    fn from(e: AppError) -> Self {
        match e {
            AppError::Genie(GenieError::Denied(message)) => Self::new(StatusCode::FORBIDDEN, message),
            AppError::Genie(GenieError::NotFound(message)) => Self::new(StatusCode::NOT_FOUND, message),
            AppError::Genie(GenieError::Invalid(message)) => Self::bad(message),
            e => ApiError::from(e).into(),
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> HttpResponse {
        (self.status, Json(self.body())).into_response()
    }
}

/// The outer Host/CSRF guard also uses this envelope for the OpenAI namespace.
pub(super) fn error_response(error: ApiError) -> HttpResponse {
    Error::from(error).into_response()
}

struct Auth(Ctx);

impl FromRequestParts<Arc<App>> for Auth {
    type Rejection = Error;

    async fn from_request_parts(parts: &mut Parts, app: &Arc<App>) -> Result<Self, Error> {
        let bearer = parts.headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok());
        if !bearer.is_some_and(|v| v.starts_with("Bearer ") && !v[7..].trim().is_empty()) {
            return Err(Error::new(StatusCode::UNAUTHORIZED, "a Genie personal bearer token is required"));
        }
        let ctx = Ctx::from_request_parts(parts, app).await?;
        ctx.user()?;
        if parts.uri.query().is_some_and(|query| !query.is_empty()) {
            return Err(Error::bad("query parameters are unsupported; retrieve returns a response snapshot"));
        }
        Ok(Self(ctx))
    }
}

struct Body<T>(T);

impl<T: serde::de::DeserializeOwned> FromRequest<Arc<App>> for Body<T> {
    type Rejection = Error;

    async fn from_request(req: Request, app: &Arc<App>) -> Result<Self, Error> {
        let Json(value) = Json::<Value>::from_request(req, app).await.map_err(|e| {
            let status = if e.status() == StatusCode::PAYLOAD_TOO_LARGE { e.status() } else { StatusCode::BAD_REQUEST };
            Error::new(status, e.body_text())
        })?;
        serde_json::from_value(value).map(Self).map_err(|e| Error::bad(e.to_string()))
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InputMessage {
    role: String,
    content: Value,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TextPart {
    #[serde(rename = "type")]
    kind: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChatBody {
    max_completion_tokens: Option<usize>,
    max_tokens: Option<usize>,
    tools: Option<Vec<Value>>,
    tool_choice: Option<String>,
    model: String,
    messages: Vec<InputMessage>,
    #[serde(default)]
    stream: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponsesBody {
    model: String,
    input: Value,
    #[serde(default)]
    instructions: Option<String>,
    #[serde(default)]
    previous_response_id: Option<String>,
    #[serde(default)]
    conversation: Option<String>,
    #[serde(default)]
    background: bool,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    store: Option<bool>,
}

fn check_model(model: &str) -> Result<(), Error> {
    if model == MODEL {
        Ok(())
    } else {
        let mut error = Error::new(StatusCode::NOT_FOUND, "unknown model; this project exposes the model 'genie'");
        error.code = "model_not_found";
        Err(error)
    }
}

fn messages(input: Vec<InputMessage>, responses: bool) -> Result<Vec<Message>, Error> {
    if input.is_empty() || input.len() > MAX_MESSAGES {
        return Err(Error::bad(format!("provide between 1 and {MAX_MESSAGES} text messages")));
    }
    let mut total = 0;
    let mut output = Vec::new();
    for message in input {
        if !matches!(message.role.as_str(), "system" | "developer" | "user" | "assistant") {
            return Err(Error::bad("only system, developer, user and assistant text messages are supported"));
        }
        if message.kind.as_deref().is_some_and(|kind| !responses || kind != "message") {
            return Err(Error::bad("only input items of type 'message' are supported"));
        }
        let content = match message.content {
            Value::String(text) => text,
            Value::Array(parts) if !parts.is_empty() => {
                let mut text = String::new();
                for part in parts {
                    let part: TextPart = serde_json::from_value(part).map_err(|e| Error::bad(format!("text content: {e}")))?;
                    let allowed = if responses { matches!(part.kind.as_str(), "input_text" | "output_text") } else { part.kind == "text" };
                    if !allowed {
                        return Err(Error::bad("only text content is supported; images, audio and tool calls are unavailable"));
                    }
                    text.push_str(&part.text);
                }
                text
            }
            _ => return Err(Error::bad("message content must be a string or a nonempty array of text parts")),
        };
        if content.trim().is_empty() {
            return Err(Error::bad("message content must not be empty"));
        }
        total += content.len();
        if total > MAX_INPUT_BYTES {
            return Err(Error::new(StatusCode::PAYLOAD_TOO_LARGE, "input text exceeds 128 KiB"));
        }
        output.push(Message { role: message.role, content });
    }
    if output.last().is_none_or(|message| message.role != "user") {
        return Err(Error::bad("the final message must be the current user's request"));
    }
    Ok(output)
}

fn optional_header(headers: &HeaderMap, name: &str) -> Result<Option<String>, Error> {
    let Some(value) = headers.get(name) else { return Ok(None) };
    let value = value.to_str().map_err(|_| Error::bad(format!("invalid {name} header")))?.trim();
    if value.is_empty() || value.len() > 200 || headers.get_all(name).iter().count() != 1 {
        return Err(Error::bad(format!("{name} must be a single nonempty value of at most 200 bytes")));
    }
    Ok(Some(value.to_string()))
}

async fn models(State(app): State<Arc<App>>, Auth(ctx): Auth, Path(project): Path<String>) -> Result<Json<Value>, Error> {
    ctx.access(&app, Some(&project)).await?;
    Ok(Json(json!({"object":"list","data":[{"id":MODEL,"object":"model","created":0,"owned_by":"genie"}]})))
}

fn submission(
    headers: &HeaderMap,
    input: Vec<Message>,
    previous: Option<String>,
    conversation: Option<String>,
    protocol: &str,
) -> Result<Submit, Error> {
    let conversation_header = optional_header(headers, "x-genie-conversation")?;
    if conversation.is_some() && conversation_header.is_some() && conversation != conversation_header {
        return Err(Error::bad("conversation and X-Genie-Conversation disagree"));
    }
    let conversation = conversation.or(conversation_header);
    let client_session = optional_header(headers, "x-opencode-session-id")?.map(|id| format!("opencode:{id}"));
    let canonical = json!({"protocol":protocol,"input":input,"conversation":conversation,"previous_response_id":previous,"client_session":client_session});
    let fingerprint = Sha256::digest(canonical.to_string().as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(Submit {
        client_session,
        messages: input,
        conversation_id: conversation,
        previous_response_id: previous,
        idempotency_key: optional_header(headers, "idempotency-key")?,
        fingerprint,
    })
}

async fn submit(app: &Arc<App>, ctx: &Ctx, project: &str, input: Submit) -> Result<Response, Error> {
    let access = ctx.access(app, Some(project)).await?;
    app.blocking(move |app| conversations::submit(app, &access, input)).await.map_err(Into::into)
}

async fn chat(
    State(app): State<Arc<App>>,
    Auth(ctx): Auth,
    Path(project): Path<String>,
    headers: HeaderMap,
    Body(body): Body<ChatBody>,
) -> Result<HttpResponse, Error> {
    check_model(&body.model)?;
    if body.tools.as_ref().is_some_and(|tools| !tools.is_empty()) || body.tool_choice.as_deref().is_some_and(|v| v != "none") {
        return Err(Error::bad("Genie executes tools on the server; disable client tools and use tool_choice=none"));
    }
    if body.max_completion_tokens.is_some() && body.max_tokens.is_some() && body.max_completion_tokens != body.max_tokens {
        return Err(Error::bad("max_tokens and max_completion_tokens disagree"));
    }
    let cap = body.max_completion_tokens.or(body.max_tokens);
    if cap.is_some_and(|v| v == 0 || v > 1_048_576) {
        return Err(Error::bad("completion token limit must be between 1 and 1048576"));
    }
    let mut input = messages(body.messages, false)?;
    // OpenCode 2.2.5 JSON-quotes text. Unwrap only its current user message, never history.
    if optional_header(&headers, "x-opencode-session-id")?.is_some()
        && let Some(last) = input.last_mut()
        && let Ok(text) = serde_json::from_str::<String>(&last.content)
        && !text.trim().is_empty()
    {
        last.content = text;
    }
    let response = submit(&app, &ctx, &project, submission(&headers, input, None, None, &format!("chat:{cap:?}"))?).await?;
    let receipt = receipt(&response)?;
    if body.stream {
        return Ok((receipt, streaming(app, ctx, project, response, Protocol::Chat(cap))).into_response());
    }
    let response = wait(&app, &ctx, &project, response).await?;
    if response.status != ResponseStatus::Completed {
        return Err(turn_error(&response));
    }
    Ok((receipt, Json(chat_body(&response, cap)?)).into_response())
}

async fn respond(
    State(app): State<Arc<App>>,
    Auth(ctx): Auth,
    Path(project): Path<String>,
    headers: HeaderMap,
    Body(body): Body<ResponsesBody>,
) -> Result<HttpResponse, Error> {
    check_model(&body.model)?;
    if body.store == Some(false) {
        return Err(Error::bad("store=false is unsupported: Genie persists accepted work and conversation state"));
    }
    let mut input: Vec<InputMessage> = match body.input {
        Value::String(content) => vec![InputMessage { role: "user".into(), content: Value::String(content), kind: None }],
        Value::Array(items) => serde_json::from_value(Value::Array(items)).map_err(|e| Error::bad(format!("input: {e}")))?,
        _ => return Err(Error::bad("input must be text or an array of text messages")),
    };
    if let Some(instructions) = body.instructions {
        input.insert(0, InputMessage { role: "developer".into(), content: Value::String(instructions), kind: None });
    }
    let input = messages(input, true)?;
    let input = submission(&headers, input, body.previous_response_id, body.conversation, "responses")?;
    let response = submit(&app, &ctx, &project, input).await?;
    let receipt = receipt(&response)?;
    if body.stream {
        return Ok((receipt, streaming(app, ctx, project, response, Protocol::Responses)).into_response());
    }
    let response = if body.background { response } else { wait(&app, &ctx, &project, response).await? };
    Ok((receipt, Json(response_body(&response))).into_response())
}

async fn retrieve(
    State(app): State<Arc<App>>,
    Auth(ctx): Auth,
    Path((project, id)): Path<(String, String)>,
) -> Result<HttpResponse, Error> {
    let response = read(&app, &ctx, &project, &id).await?;
    Ok((receipt(&response)?, Json(response_body(&response))).into_response())
}

async fn cancel(State(app): State<Arc<App>>, Auth(ctx): Auth, Path((project, id)): Path<(String, String)>) -> Result<HttpResponse, Error> {
    let access = ctx.access(&app, Some(&project)).await?;
    let response = app.blocking(move |app| conversations::cancel_response(app, &access, &id)).await?;
    Ok((receipt(&response)?, Json(response_body(&response))).into_response())
}

fn receipt(response: &Response) -> Result<HeaderMap, Error> {
    let mut headers = HeaderMap::new();
    for (name, value) in [("x-genie-conversation", &response.conversation_id), ("x-request-id", &response.id)] {
        headers.insert(
            name,
            HeaderValue::from_str(value).map_err(|_| Error::new(StatusCode::INTERNAL_SERVER_ERROR, "invalid response identifier"))?,
        );
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(headers)
}

fn terminal(response: &Response) -> bool {
    response.status.is_terminal()
}

async fn read(app: &Arc<App>, ctx: &Ctx, project: &str, id: &str) -> Result<Response, Error> {
    let access = ctx.access(app, Some(project)).await?;
    let id = id.to_string();
    app.blocking(move |app| conversations::get_response(app, &access, &id)).await.map_err(Into::into)
}

async fn wait(app: &Arc<App>, ctx: &Ctx, project: &str, mut response: Response) -> Result<Response, Error> {
    while !terminal(&response) {
        tokio::time::sleep(POLL).await;
        response = read(app, ctx, project, &response.id).await?;
    }
    Ok(response)
}

fn turn_error(response: &Response) -> Error {
    if response.status == ResponseStatus::Cancelled {
        let mut error = Error::new(StatusCode::CONFLICT, "response was cancelled; linked tasks are unchanged");
        error.code = "response_cancelled";
        error
    } else {
        Error::new(StatusCode::INTERNAL_SERVER_ERROR, response.error.as_deref().unwrap_or("agent turn failed"))
    }
}

fn chat_id(response: &Response) -> String {
    format!("chatcmpl_{}", response.id.trim_start_matches("resp_"))
}

/// Genie defines its wire output limit in cl100k_base tokens, independently of
/// worker model usage. Full output remains in the durable response receipt.
fn limited_output(text: &str, cap: Option<usize>) -> Result<(String, &'static str), Error> {
    let Some(cap) = cap else { return Ok((text.into(), "stop")) };
    static BPE: std::sync::OnceLock<Result<tiktoken_rs::CoreBPE, String>> = std::sync::OnceLock::new();
    let bpe = BPE
        .get_or_init(|| tiktoken_rs::cl100k_base().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|_| Error::new(StatusCode::INTERNAL_SERVER_ERROR, "cannot initialize output tokenizer"))?;
    let tokens = bpe.encode_ordinary(text);
    if tokens.len() <= cap {
        return Ok((text.into(), "stop"));
    }
    let bytes =
        bpe.decode_bytes(&tokens[..cap]).map_err(|_| Error::new(StatusCode::INTERNAL_SERVER_ERROR, "cannot decode output tokens"))?;
    // A BPE boundary can split a UTF-8 scalar. Return only the valid prefix.
    let end = std::str::from_utf8(&bytes).err().map_or(bytes.len(), |e| e.valid_up_to());
    Ok((text[..end].into(), "length"))
}

fn chat_body(response: &Response, cap: Option<usize>) -> Result<Value, Error> {
    let (text, finish) = limited_output(&response.output, cap)?;
    Ok(json!({"id":chat_id(response),"object":"chat.completion","created":response.created_at,"model":MODEL,
        "choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":finish,"logprobs":null}],"usage":null}))
}

fn message_item(response: &Response) -> Value {
    json!({"id":format!("msg_{}",response.id.trim_start_matches("resp_")),"type":"message","status":"completed","role":"assistant",
        "content":[text_part(&response.output)]})
}

fn text_part(text: &str) -> Value {
    json!({"type":"output_text","text":text,"annotations":[],"logprobs":[]})
}

fn response_body(response: &Response) -> Value {
    let output = if response.status == ResponseStatus::Completed { vec![message_item(response)] } else { Vec::new() };
    json!({"id":response.id,"object":"response","created_at":response.created_at,"completed_at":response.completed_at,
        "status":response.status,"error":response.error.as_ref().map(|message|json!({"code":"server_error","message":message})),
        "incomplete_details":null,"instructions":null,"model":MODEL,"output":output,"previous_response_id":response.previous_response_id,
        "parallel_tool_calls":false,"tool_choice":"none","tools":[],"store":true,"text":{"format":{"type":"text"}},
        "truncation":"disabled","usage":null,"metadata":{"genie_conversation_id":response.conversation_id}})
}

#[derive(Clone, Copy)]
enum Protocol {
    Chat(Option<usize>),
    Responses,
}

struct StreamState {
    app: Arc<App>,
    ctx: Ctx,
    project: String,
    response: Response,
    protocol: Protocol,
    pending: VecDeque<Event>,
    sequence: u64,
    finished: bool,
    started: bool,
}

impl StreamState {
    fn event(&mut self, kind: &str, mut data: Value) {
        data["type"] = json!(kind);
        data["sequence_number"] = json!(self.sequence);
        self.sequence += 1;
        self.pending.push_back(Event::default().event(kind).data(data.to_string()));
    }

    fn chat_chunk(&mut self, delta: Value, finish: Option<&str>) {
        let chunk = json!({"id":chat_id(&self.response),"object":"chat.completion.chunk","created":self.response.created_at,
            "model":MODEL,"choices":[{"index":0,"delta":delta,"finish_reason":finish,"logprobs":null}]});
        self.pending.push_back(Event::default().data(chunk.to_string()));
    }

    fn fail(&mut self, error: Error) {
        match self.protocol {
            Protocol::Chat(_) => {
                self.pending.push_back(Event::default().data(error.body().to_string()));
                self.pending.push_back(Event::default().data("[DONE]"));
            }
            Protocol::Responses => self.event("error", json!({"code":error.code,"message":error.message,"param":null})),
        }
        self.finished = true;
    }

    fn complete(&mut self) {
        self.finished = true;
        match self.protocol {
            Protocol::Chat(cap) if self.response.status == ResponseStatus::Completed => {
                match limited_output(&self.response.output, cap) {
                    Ok((text, finish)) => {
                        self.chat_chunk(json!({"content":text}), None);
                        self.chat_chunk(json!({}), Some(finish));
                    }
                    Err(error) => {
                        self.fail(error);
                        return;
                    }
                }
                self.pending.push_back(Event::default().data("[DONE]"));
            }
            Protocol::Chat(_) => self.fail(turn_error(&self.response)),
            Protocol::Responses if self.response.status == ResponseStatus::Completed => {
                let item = message_item(&self.response);
                let id = item["id"].clone();
                self.event(
                    "response.output_item.added",
                    json!({"output_index":0,"item":{"id":id,"type":"message","status":"in_progress","role":"assistant","content":[]}}),
                );
                self.event("response.content_part.added", json!({"item_id":id,"output_index":0,"content_index":0,"part":text_part("")}));
                self.event(
                    "response.output_text.delta",
                    json!({"item_id":id,"output_index":0,"content_index":0,"delta":self.response.output,"logprobs":[]}),
                );
                self.event(
                    "response.output_text.done",
                    json!({"item_id":id,"output_index":0,"content_index":0,"text":self.response.output,"logprobs":[]}),
                );
                self.event(
                    "response.content_part.done",
                    json!({"item_id":id,"output_index":0,"content_index":0,"part":text_part(&self.response.output)}),
                );
                self.event("response.output_item.done", json!({"output_index":0,"item":item}));
                self.event("response.completed", json!({"response":response_body(&self.response)}));
            }
            Protocol::Responses if self.response.status == ResponseStatus::Failed => {
                self.event("response.failed", json!({"response":response_body(&self.response)}));
            }
            Protocol::Responses => self.fail(turn_error(&self.response)),
        }
    }

    fn initial_body(&self) -> Value {
        // An idempotent replay streams the saved reply through the same item events. Starting
        // with that reply already in `output` would duplicate it in SDK stream accumulators.
        let mut body = response_body(&self.response);
        body["output"] = json!([]);
        if terminal(&self.response) {
            body["status"] = json!("in_progress");
            body["completed_at"] = Value::Null;
            body["error"] = Value::Null;
        }
        body
    }
}

fn streaming(app: Arc<App>, ctx: Ctx, project: String, response: Response, protocol: Protocol) -> HttpResponse {
    let mut state =
        StreamState { app, ctx, project, response, protocol, pending: VecDeque::new(), sequence: 0, finished: false, started: false };
    match protocol {
        Protocol::Chat(_) => state.chat_chunk(json!({"role":"assistant","content":""}), None),
        Protocol::Responses => state.event("response.created", json!({"response":state.initial_body()})),
    }
    let events = stream::unfold(state, |mut state| async move {
        loop {
            if let Some(event) = state.pending.pop_front() {
                return Some((Ok::<_, Infallible>(event), state));
            }
            if state.finished {
                return None;
            }
            if matches!(state.protocol, Protocol::Responses)
                && !state.started
                && matches!(state.response.status, ResponseStatus::InProgress | ResponseStatus::Completed)
            {
                state.started = true;
                state.event("response.in_progress", json!({"response":state.initial_body()}));
                continue;
            }
            if terminal(&state.response) {
                state.complete();
                continue;
            }
            tokio::time::sleep(POLL).await;
            match read(&state.app, &state.ctx, &state.project, &state.response.id).await {
                Ok(response) => state.response = response,
                Err(error) => state.fail(error),
            }
        }
    });
    Sse::new(events).keep_alive(KeepAlive::new().interval(Duration::from_secs(10))).into_response()
}

#[cfg(test)]
mod output_tests {
    use super::*;
    #[test]
    fn token_caps_preserve_unicode_and_treat_special_tokens_as_text() {
        let bpe = tiktoken_rs::cl100k_base().unwrap();
        for text in ["🙂 Привет, мир!", "hello <|endoftext|> world"] {
            for cap in 1..10 {
                let (prefix, reason) = limited_output(text, Some(cap)).unwrap();
                assert!(text.starts_with(&prefix));
                assert!(bpe.encode_ordinary(&prefix).len() <= cap);
                assert_eq!(reason, if prefix == text { "stop" } else { "length" });
            }
        }
    }
}
