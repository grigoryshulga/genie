//! Test harness: a full app on a temp data directory, driven through the router.

#![allow(dead_code)]

pub mod fakehost;
pub mod githost;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use genie::config::Config;
use genie::state::App;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

#[derive(Debug)]
pub struct ManualClock {
    now: Mutex<Instant>,
    changed: tokio::sync::Notify,
    sleep_started: tokio::sync::Notify,
    sleepers: Mutex<std::collections::BTreeMap<u64, Instant>>,
    sequence: std::sync::atomic::AtomicU64,
}

impl Default for ManualClock {
    fn default() -> Self {
        Self {
            now: Mutex::new(Instant::now()),
            changed: Default::default(),
            sleep_started: Default::default(),
            sleepers: Default::default(),
            sequence: Default::default(),
        }
    }
}

impl ManualClock {
    pub fn advance(&self, by: Duration) {
        *self.now.lock().unwrap() += by;
        self.changed.notify_waiters();
    }

    /// Run real I/O, advancing only when the operation requests a retry delay.
    pub async fn run<F: std::future::Future>(&self, future: F) -> F::Output {
        tokio::pin!(future);
        loop {
            tokio::select! {
                biased;
                result = &mut future => return result,
                deadline = self.next_sleep() => {
                    let now = *self.now.lock().unwrap();
                    self.advance(deadline.saturating_duration_since(now));
                }
            }
        }
    }

    async fn next_sleep(&self) -> Instant {
        loop {
            let started = self.sleep_started.notified();
            tokio::pin!(started);
            started.as_mut().enable();
            let now = *self.now.lock().unwrap();
            let next = self.sleepers.lock().unwrap().values().copied().filter(|deadline| *deadline > now).min();
            if let Some(deadline) = next {
                return deadline;
            }
            started.await;
        }
    }
}

struct ClockSleep<'a> {
    clock: &'a ManualClock,
    id: u64,
}

impl Drop for ClockSleep<'_> {
    fn drop(&mut self) {
        self.clock.sleepers.lock().unwrap().remove(&self.id);
    }
}

impl genie::clock::Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }

    fn sleep_until(&self, deadline: Instant) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            let id = self.sequence.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            self.sleepers.lock().unwrap().insert(id, deadline);
            let _sleep = ClockSleep { clock: self, id };
            self.sleep_started.notify_one();
            loop {
                let changed = self.changed.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.now() >= deadline {
                    return;
                }
                changed.await;
            }
        })
    }
}

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
        Self::with_clock(f, Arc::new(genie::clock::SystemClock))
    }

    pub fn with_clock(f: impl FnOnce(&mut Config), clock: Arc<dyn genie::clock::Clock>) -> Harness {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::load(dir.path()).unwrap();
        // PD24: no default models anywhere — every role carries its own, like a real installation.
        std::fs::create_dir_all(dir.path().join("agents")).unwrap();
        for id in ["analyst", "executor", "reviewer", "tester", "documenter", "orchestrator"] {
            std::fs::write(dir.path().join(format!("agents/{id}.md")), format!("---\nmodel: fake/{id}\n---\n")).unwrap();
        }
        // Test files live in the data directory, which a sandboxed agent does not see.
        cfg.runtime.sandbox.mode = "off".into();
        cfg.runtime.enabled = false;
        // Route tests need the real indexed vault, not a Git history for every fixture.
        // The documentation publication and vault-sync tests enable history explicitly.
        cfg.vault.commit = Some(false);
        f(&mut cfg);
        let app = App::open_with_clock(dir.path(), cfg, PathBuf::from("/nonexistent-web"), clock).unwrap();
        let router = genie::http::router(app.clone());
        let local = router.clone().layer(MockConnectInfo(SocketAddr::from(([127, 0, 0, 1], 50000))));
        let remote = router.layer(MockConnectInfo(SocketAddr::from(([10, 0, 0, 7], 50000))));
        Harness { dir, app, router: local, remote }
    }

    pub fn project(&self, slug: &str) {
        self.app.create_project(slug, "", None, None, None).unwrap();
    }
}

/// PD24: no default models — role files per id, so spawned agents always have one.
/// (The Harness itself writes only the six team roles: its tests rely on planner
/// and researcher staying file-less built-ins.)
pub fn give_roles_models(data: &std::path::Path) {
    std::fs::create_dir_all(data.join("agents")).unwrap();
    for id in ["analyst", "executor", "reviewer", "tester", "documenter", "orchestrator", "planner", "researcher"] {
        std::fs::write(data.join(format!("agents/{id}.md")), format!("---\nmodel: fake/{id}\n---\n")).unwrap();
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
