//! Test harness: a full app on a temp data directory, driven through the router.

#![allow(dead_code)]

pub mod fakehost;
pub mod githost;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use genie::config::Config;
use genie::state::App;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

pub struct Harness {
    pub dir: tempfile::TempDir,
    pub app: Arc<App>,
    pub router: Router,
    pub remote: Router,
}

impl Harness {
    pub fn new() -> Harness {
        Harness::with_config(|_| {})
    }

    pub fn with_config(f: impl FnOnce(&mut Config)) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::load(dir.path()).unwrap();
        // Test files live in the data directory, which a sandboxed agent does not see.
        cfg.runtime.sandbox.mode = "off".into();
        cfg.runtime.enabled = false;
        f(&mut cfg);
        let app = App::open(dir.path(), cfg, PathBuf::from("/nonexistent-web")).unwrap();
        let router = genie::http::router(app.clone());
        let local = router.clone().layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))));
        let remote = router.layer(MockConnectInfo(SocketAddr::from(([10, 0, 0, 7], 50000))));
        Harness { dir, app, router: local, remote }
    }

    pub fn project(&self, slug: &str) {
        self.app.create_project(slug, "", None, None, None).unwrap();
    }
}

pub struct Call<'a> {
    router: &'a Router,
    method: Method,
    uri: String,
    body: Option<Value>,
    raw: Option<Vec<u8>>,
    headers: Vec<(String, String)>,
    csrf: bool,
}

pub fn call<'a>(router: &'a Router, method: &str, uri: &str) -> Call<'a> {
    Call { router, method: method.parse().unwrap(), uri: uri.to_string(), body: None, raw: None, headers: Vec::new(), csrf: true }
}

impl<'a> Call<'a> {
    pub fn json(mut self, v: Value) -> Self {
        self.body = Some(v);
        self
    }
    /// A body sent as it is (`application/octet-stream`).
    pub fn bytes(mut self, b: &[u8]) -> Self {
        self.raw = Some(b.to_vec());
        self
    }
    pub fn header(mut self, k: &str, v: &str) -> Self {
        self.headers.push((k.to_string(), v.to_string()));
        self
    }
    pub fn bearer(self, token: &str) -> Self {
        self.header("authorization", &format!("Bearer {token}"))
    }
    pub fn cookie(self, c: &str) -> Self {
        self.header("cookie", c)
    }
    pub fn no_csrf(mut self) -> Self {
        self.csrf = false;
        self
    }
    pub async fn send(self) -> (StatusCode, Value, Vec<String>) {
        let (status, headers, bytes) = self.send_raw().await;
        let cookies = headers
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap().to_string())
            .collect();
        let body = serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
        (status, body, cookies)
    }
    /// The response as it came: status, headers and body bytes.
    pub async fn send_raw(self) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut req = Request::builder().method(self.method).uri(&self.uri);
        if !self.headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("host")) {
            req = req.header(header::HOST, "127.0.0.1:7420");
        }
        if self.csrf {
            req = req.header("x-genie", "1");
        }
        for (k, v) in &self.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        let req = match (self.body, self.raw) {
            (Some(b), _) => req.header(header::CONTENT_TYPE, "application/json").body(Body::from(b.to_string())).unwrap(),
            (None, Some(raw)) => req.header(header::CONTENT_TYPE, "application/octet-stream").body(Body::from(raw)).unwrap(),
            (None, None) => req.body(Body::empty()).unwrap(),
        };
        let res = self.router.clone().oneshot(req).await.unwrap();
        let (status, headers) = (res.status(), res.headers().clone());
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        (status, headers, bytes.to_vec())
    }
}
