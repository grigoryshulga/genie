//! How an operation reaches the server's API: over HTTP with a token (the
//! command line of an agent or a person), or inside the server's process —
//! with the caller's token (the MCP server) or as the operator (the command line
//! on the server's machine, without a token: whoever reads the data directory
//! runs the server anyway).

use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, header};
use futures_util::future::BoxFuture;
use serde_json::Value;
use tower::ServiceExt;

/// With the project header: that project or an error, never another project of the caller.
pub const STRICT: &str = "x-genie-project-strict";

/// The body of a request.
pub enum Payload {
    None,
    Json(Value),
    /// A file's bytes as they are (the files of a skill).
    Bytes(Vec<u8>),
}

/// A request as the handlers of the server see it; the answer is its JSON, a
/// failure the server's error message.
pub trait Api: Send + Sync {
    fn request<'a>(&'a self, method: &'a str, path: &'a str, body: Payload) -> BoxFuture<'a, Result<Value, String>>;
    /// Where the calls go, for messages.
    fn place(&self) -> String;
    /// Whether a call changed something in the data directory of a server this process does not
    /// run (the command line without a token): that server has to be told ([`wake_server`]).
    fn wrote_locally(&self) -> bool {
        false
    }

    fn call<'a>(&'a self, method: &'a str, path: &'a str, body: Option<Value>) -> BoxFuture<'a, Result<Value, String>> {
        self.request(method, path, body.map_or(Payload::None, Payload::Json))
    }
}

/// The answer's JSON, or the server's error message.
fn answer(status: axum::http::StatusCode, v: Value) -> Result<Value, String> {
    if !status.is_success() {
        return Err(v["error"].as_str().map(str::to_string).unwrap_or_else(|| format!("HTTP {status}")));
    }
    Ok(v)
}

/// The server over HTTP with a bearer token.
pub struct Remote {
    http: reqwest::Client,
    base: String,
    token: String,
    /// The project to act in; a person's token reaches every project of theirs.
    project: Option<String>,
}

impl Remote {
    pub fn new(base: &str, token: &str, project: Option<String>) -> Remote {
        Remote { http: reqwest::Client::new(), base: base.trim_end_matches('/').to_string(), token: token.to_string(), project }
    }
}

impl Api for Remote {
    fn request<'a>(&'a self, method: &'a str, path: &'a str, body: Payload) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            let url = format!("{}/api{path}", self.base);
            let host = self.base.split("://").nth(1).unwrap_or("127.0.0.1:7420").split('/').next().unwrap_or_default().to_string();
            let mut req = self
                .http
                .request(method.parse().map_err(|_| "bad method".to_string())?, &url)
                .bearer_auth(&self.token)
                .header("host", host)
                .header("x-genie", "1");
            if let Some(p) = &self.project {
                req = req.header("x-genie-project", p).header(STRICT, "1");
            }
            req = match body {
                Payload::None => req,
                Payload::Json(b) => req.json(&b),
                Payload::Bytes(b) => req.header(header::CONTENT_TYPE, "application/octet-stream").body(b),
            };
            let res = req.send().await.map_err(|e| format!("cannot reach genie at {}: {e}", self.base))?;
            let status = res.status();
            answer(status, res.json().await.unwrap_or(Value::Null))
        })
    }

    fn place(&self) -> String {
        self.base.clone()
    }
}

/// Who calls inside the server's process.
#[derive(Clone)]
pub enum Auth {
    /// A person's or an agent's token, as over HTTP.
    Bearer(String),
    /// The operator of the server's machine: a server admin.
    Operator,
}

/// Marks a request made inside the server's process by the operator. Requests
/// from the network never carry it: extensions exist only inside the process.
#[derive(Clone, Copy, Debug)]
pub struct OperatorAccess;

/// The server's router, called without the network.
pub struct InProcess {
    router: Router,
    port: u16,
    auth: Auth,
    /// The project to act in (`X-Genie-Project`).
    project: Option<String>,
    /// A call other than a read went through.
    wrote: AtomicBool,
}

impl InProcess {
    pub fn new(router: Router, port: u16, auth: Auth, project: Option<String>) -> InProcess {
        InProcess { router, port, auth, project, wrote: AtomicBool::new(false) }
    }
}

/// Tell a server running on this machine that the data directory changed under it: the command line
/// without a token writes the databases itself, and the server's workers sleep until woken. Best
/// effort: no server, no harm.
pub async fn wake_server(port: u16) {
    let Ok(http) = reqwest::Client::builder().timeout(std::time::Duration::from_millis(500)).build() else { return };
    let _ = http
        .post(format!("http://127.0.0.1:{port}/api/wake"))
        .header("host", format!("127.0.0.1:{port}"))
        .header("x-genie", "1")
        .send()
        .await;
}

impl Api for InProcess {
    fn request<'a>(&'a self, method: &'a str, path: &'a str, body: Payload) -> BoxFuture<'a, Result<Value, String>> {
        Box::pin(async move {
            if method != "GET" {
                self.wrote.store(true, Ordering::Relaxed);
            }
            let mut req = Request::builder()
                .method(method)
                .uri(format!("/api{path}"))
                .header(header::HOST, format!("127.0.0.1:{}", self.port))
                .header("x-genie", "1");
            if let Some(p) = &self.project {
                req = req.header("x-genie-project", p).header(STRICT, "1");
            }
            if let Auth::Bearer(token) = &self.auth {
                req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
            }
            let mut req = match body {
                Payload::None => req.body(Body::empty()),
                Payload::Json(b) => req.header(header::CONTENT_TYPE, "application/json").body(Body::from(b.to_string())),
                Payload::Bytes(b) => req.header(header::CONTENT_TYPE, "application/octet-stream").body(Body::from(b)),
            }
            .map_err(|e| e.to_string())?;
            req.extensions_mut().insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 0))));
            if matches!(self.auth, Auth::Operator) {
                req.extensions_mut().insert(OperatorAccess);
            }
            let res = self.router.clone().oneshot(req).await.map_err(|e| e.to_string())?;
            let status = res.status();
            let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024 * 1024).await.map_err(|e| e.to_string())?;
            answer(status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
        })
    }

    fn place(&self) -> String {
        "the local server data".into()
    }

    fn wrote_locally(&self) -> bool {
        self.wrote.load(Ordering::Relaxed)
    }
}
