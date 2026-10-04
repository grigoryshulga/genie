//! Pull/merge requests, CI and repository facts of a git host, over its REST API.
//!
//! One [`Api`] per host; `kind` picks GitHub (REST v3) or GitLab (REST v4). Both answer
//! in the same shapes ([`ChangeRequest`], [`Ci`], [`Comment`], [`RepoInfo`]): the
//! differences in words (pull request, merge request), ids (number, iid), pipelines and
//! checks stay in the two implementations.
//!
//! Requests carry the host's token (never an agent's), time out, and are retried when they
//! can be repeated safely (reads, on rate limits and server errors). Failures are typed
//! so callers can tell a bad token from a missing repository from a refused merge.

pub mod github;
pub mod gitlab;

use std::time::Duration;

use reqwest::Method;
use serde::Serialize;
use serde_json::Value;

use super::hosts::{Host, Kind};

/// One failed check of a request: which, where to look, and what the host says.
#[derive(Debug, Clone, Serialize)]
pub struct CiFailure {
    pub name: String,
    pub url: Option<String>,
    /// The end of the log, or the host's summary of the failure.
    pub detail: String,
}

/// At most this many failures are reported, each with at most this many characters of detail.
pub(crate) const MAX_FAILURES: usize = 5;
const DETAIL_CHARS: usize = 600;

/// The last `DETAIL_CHARS` characters of `text`, without terminal colour codes and blank edges.
pub(crate) fn tail(text: &str) -> String {
    let mut clean = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            for n in chars.by_ref() {
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else if c != '\r' {
            clean.push(c);
        }
    }
    let clean = clean.trim();
    let n = clean.chars().count();
    if n <= DETAIL_CHARS { clean.to_string() } else { format!("…{}", clean.chars().skip(n - DETAIL_CHARS).collect::<String>()) }
}

/// The state of a request, as the delivery records it.
pub type CrState = genie_core::RequestState;

/// The host's answer about the checks of a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Ci {
    /// The host shows no checks for the commit.
    None,
    Pending,
    Passed,
    Failed,
}

impl From<Ci> for genie_core::CheckState {
    fn from(ci: Ci) -> Self {
        match ci {
            Ci::None => genie_core::CheckState::None,
            Ci::Pending => genie_core::CheckState::Pending,
            Ci::Passed => genie_core::CheckState::Passed,
            Ci::Failed => genie_core::CheckState::Failed,
        }
    }
}

/// A pull request (GitHub) or merge request (GitLab).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeRequest {
    /// The number the host shows (GitLab's `iid`).
    pub number: i64,
    pub url: String,
    pub title: String,
    pub state: CrState,
    pub draft: bool,
    /// The source branch.
    pub head: String,
    /// The target branch.
    pub base: String,
    pub head_sha: Option<String>,
    /// The commit the merge produced on the target branch (`merge_commit_sha`); absent when
    /// the host does not work it out (then the target branch's checks are not watched).
    pub merge_sha: Option<String>,
    /// `None` while the host is still working it out.
    pub mergeable: Option<bool>,
    pub approvals: u32,
    pub changes_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comment {
    pub author: String,
    pub body: String,
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoInfo {
    pub default_branch: String,
    /// Whether the token can push (`None`: the host did not say).
    pub can_push: Option<bool>,
    /// Protected branches (`None`: the token cannot see them).
    pub protected_branches: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Whoami {
    pub login: String,
    pub version: Option<String>,
}

pub struct OpenRequest {
    pub head: String,
    pub base: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

/// Why a call to a host failed.
#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// 401/403: the token is wrong, expired or lacks the right.
    Auth(String),
    NotFound(String),
    /// The host refused the request (400, 405, 406, 409, 422): the request is wrong or the state does not allow it.
    Rejected(String),
    /// The host's rate limit; how long it asks to wait.
    RateLimited(Option<Duration>),
    /// 5xx, or the host is unreachable.
    Unavailable(String),
    /// This host cannot do it (no API for `plain` hosts, a merge method GitLab lacks).
    Unsupported(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Auth(m) => write!(f, "the git host refused the token: {m}"),
            ApiError::NotFound(m) => write!(f, "not found on the git host: {m}"),
            ApiError::Rejected(m) => write!(f, "the git host refused: {m}"),
            ApiError::RateLimited(Some(d)) => write!(f, "the git host's rate limit is reached; try again in {} s", d.as_secs().max(1)),
            ApiError::RateLimited(None) => write!(f, "the git host's rate limit is reached; try again later"),
            ApiError::Unavailable(m) => write!(f, "the git host is unavailable: {m}"),
            ApiError::Unsupported(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for ApiError {}

pub type ApiResult<T> = Result<T, ApiError>;

pub(crate) struct Resp {
    pub body: Value,
}

/// A client of one host's API.
pub struct Api {
    pub(crate) host: Host,
    pub(crate) http: reqwest::Client,
    pub(crate) base: String,
    /// The longest a rate limit is waited out inside one call.
    pub(crate) max_wait: Duration,
}

impl Api {
    pub fn new(host: &Host) -> ApiResult<Api> {
        let Some(base) = host.api_base() else {
            return Err(ApiError::Unsupported(format!("host {} is a plain git server: it has no pull/merge request API", host.id)));
        };
        if host.token.is_none() {
            return Err(ApiError::Auth(format!("host {} has no token (set one on the repository)", host.id)));
        }
        let mut b =
            reqwest::Client::builder().timeout(Duration::from_secs(30)).connect_timeout(Duration::from_secs(10)).user_agent("genie");
        if let Some(ca) = &host.ca_cert {
            let pem = std::fs::read(ca).map_err(|e| ApiError::Unavailable(format!("ca_cert {ca}: {e}")))?;
            let cert = reqwest::Certificate::from_pem(&pem).map_err(|e| ApiError::Unavailable(format!("ca_cert {ca}: {e}")))?;
            b = b.add_root_certificate(cert);
        }
        if let Some(p) = &host.http_proxy {
            b = b.proxy(reqwest::Proxy::all(p).map_err(|e| ApiError::Unavailable(format!("http_proxy: {e}")))?);
        }
        if host.insecure_skip_verify {
            b = b.danger_accept_invalid_certs(true);
        }
        let http = b.build().map_err(|e| ApiError::Unavailable(e.to_string()))?;
        Ok(Api { host: host.clone(), http, base, max_wait: Duration::from_secs(5) })
    }

    pub fn kind(&self) -> Kind {
        self.host.kind
    }

    /// One call. Reads are repeated (up to three times) on rate limits and server errors.
    pub(crate) async fn send(&self, method: Method, path: &str, query: &[(&str, String)], body: Option<Value>) -> ApiResult<Resp> {
        let mut url = format!("{}{}", self.base, path);
        if !query.is_empty() {
            url.push('?');
            url.push_str(&query.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&"));
        }
        let token = self.host.token.clone().unwrap_or_default();
        let repeatable = method == Method::GET;
        let mut attempt = 0;
        loop {
            attempt += 1;
            let mut req = self.http.request(method.clone(), &url);
            req = match self.host.kind {
                Kind::Github => {
                    req.bearer_auth(&token).header("Accept", "application/vnd.github+json").header("X-GitHub-Api-Version", "2022-11-28")
                }
                _ => req.header("PRIVATE-TOKEN", &token),
            };
            if let Some(b) = &body {
                req = req.json(b);
            }
            let outcome = match req.send().await {
                Err(e) => Err(ApiError::Unavailable(e.without_url().to_string())),
                Ok(res) => {
                    let status = res.status().as_u16();
                    let retry_after = res
                        .headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(Duration::from_secs);
                    let exhausted = res.headers().get("x-ratelimit-remaining").and_then(|v| v.to_str().ok()) == Some("0");
                    let text = res.text().await.unwrap_or_default();
                    let body: Value =
                        serde_json::from_str(&text).unwrap_or(if text.is_empty() { Value::Null } else { Value::String(text) });
                    classify(status, body, retry_after, exhausted)
                }
            };
            match outcome {
                Ok(r) => return Ok(r),
                Err(e) if repeatable && attempt < 3 => match &e {
                    ApiError::RateLimited(wait) => {
                        let wait = wait.unwrap_or(Duration::from_millis(500));
                        if wait > self.max_wait {
                            return Err(e);
                        }
                        tokio::time::sleep(wait).await;
                    }
                    ApiError::Unavailable(_) => tokio::time::sleep(Duration::from_millis(200 * attempt)).await,
                    _ => return Err(e),
                },
                Err(e) => return Err(e),
            }
        }
    }

    // --- the operations, by host kind --------------------------------------------------

    /// Who the token is, and the host's version when it says.
    pub async fn whoami(&self) -> ApiResult<Whoami> {
        match self.host.kind {
            Kind::Github => github::whoami(self).await,
            _ => gitlab::whoami(self).await,
        }
    }

    pub async fn repo(&self, remote: &str) -> ApiResult<RepoInfo> {
        match self.host.kind {
            Kind::Github => github::repo(self, remote).await,
            _ => gitlab::repo(self, remote).await,
        }
    }

    /// Open a request; when one is already open for the branch, that one (so a retry is harmless).
    pub async fn open(&self, remote: &str, r: &OpenRequest) -> ApiResult<ChangeRequest> {
        match self.host.kind {
            Kind::Github => github::open(self, remote, r).await,
            _ => gitlab::open(self, remote, r).await,
        }
    }

    pub async fn get(&self, remote: &str, number: i64) -> ApiResult<ChangeRequest> {
        match self.host.kind {
            Kind::Github => github::get(self, remote, number).await,
            _ => gitlab::get(self, remote, number).await,
        }
    }

    pub async fn comments(&self, remote: &str, number: i64) -> ApiResult<Vec<Comment>> {
        match self.host.kind {
            Kind::Github => github::comments(self, remote, number).await,
            _ => gitlab::comments(self, remote, number).await,
        }
    }

    pub async fn comment(&self, remote: &str, number: i64, body: &str) -> ApiResult<()> {
        match self.host.kind {
            Kind::Github => github::comment(self, remote, number, body).await,
            _ => gitlab::comment(self, remote, number, body).await,
        }
    }

    /// Merge with `method` (`merge`, `squash`, `rebase`; the host's default when `None`), only if the head is still `sha`.
    pub async fn merge(&self, remote: &str, number: i64, method: Option<&str>, sha: Option<&str>) -> ApiResult<()> {
        match self.host.kind {
            Kind::Github => github::merge(self, remote, number, method, sha).await,
            _ => gitlab::merge(self, remote, number, method, sha).await,
        }
    }

    /// Which checks of `sha` failed and why (best effort: an empty list when the host says nothing).
    pub async fn ci_failures(&self, remote: &str, sha: Option<&str>) -> ApiResult<Vec<CiFailure>> {
        match self.host.kind {
            Kind::Github => github::ci_failures(self, remote, sha).await,
            _ => gitlab::ci_failures(self, remote, sha).await,
        }
    }

    /// The checks of one commit.
    pub async fn ci(&self, remote: &str, sha: Option<&str>) -> ApiResult<Ci> {
        match self.host.kind {
            Kind::Github => github::ci(self, remote, sha).await,
            _ => gitlab::ci(self, remote, sha).await,
        }
    }
}

fn classify(status: u16, body: Value, retry_after: Option<Duration>, exhausted: bool) -> ApiResult<Resp> {
    if (200..300).contains(&status) {
        return Ok(Resp { body });
    }
    let msg = message(&body);
    Err(match status {
        401 => ApiError::Auth(msg),
        403 if exhausted || msg.to_lowercase().contains("rate limit") => ApiError::RateLimited(retry_after),
        403 => ApiError::Auth(msg),
        429 => ApiError::RateLimited(retry_after),
        404 => ApiError::NotFound(msg),
        500..=599 => ApiError::Unavailable(format!("{status} {msg}")),
        _ => ApiError::Rejected(format!("{status} {msg}")),
    })
}

/// The human part of an error body: GitHub's `message` (with `errors`), GitLab's `message` (a string or a map) or `error`.
fn message(body: &Value) -> String {
    let mut parts = Vec::new();
    match body {
        Value::String(s) => parts.push(s.chars().take(300).collect::<String>()),
        Value::Object(o) => {
            for key in ["message", "error", "error_description"] {
                match o.get(key) {
                    Some(Value::String(s)) => parts.push(s.clone()),
                    Some(Value::Object(m)) => parts.push(
                        m.iter()
                            .map(|(k, v)| {
                                format!(
                                    "{k}: {}",
                                    v.as_array()
                                        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
                                        .unwrap_or_else(|| v.to_string())
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("; "),
                    ),
                    Some(Value::Array(a)) => parts.push(a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),
                    _ => {}
                }
            }
            if let Some(Value::Array(errs)) = o.get("errors") {
                for e in errs {
                    if let Some(m) = e.get("message").and_then(Value::as_str) {
                        parts.push(m.to_string());
                    } else if let Some(s) = e.as_str() {
                        parts.push(s.to_string());
                    }
                }
            }
        }
        _ => {}
    }
    if parts.is_empty() { "no details".into() } else { parts.join("; ") }
}

/// A string in a URL path segment: everything but unreserved characters is percent-encoded (`/` too).
pub(crate) fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

pub(crate) fn s(v: &Value, key: &str) -> String {
    v[key].as_str().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn errors_are_typed_by_status() {
        let e = |status, body| classify(status, body, None, false).err().unwrap();
        assert!(matches!(e(401, json!({"message": "Bad credentials"})), ApiError::Auth(_)));
        assert!(matches!(e(403, json!({"message": "Resource not accessible"})), ApiError::Auth(_)));
        assert!(matches!(e(403, json!({"message": "API rate limit exceeded"})), ApiError::RateLimited(_)));
        assert!(matches!(e(404, json!({"message": "Not Found"})), ApiError::NotFound(_)));
        assert!(
            matches!(e(422, json!({"message": "Validation Failed", "errors": [{"message": "no commits"}]})), ApiError::Rejected(m) if m.contains("no commits"))
        );
        assert!(matches!(e(502, Value::Null), ApiError::Unavailable(_)));
        assert!(
            matches!(classify(429, Value::Null, Some(Duration::from_secs(7)), false), Err(ApiError::RateLimited(Some(d))) if d.as_secs() == 7)
        );
        assert!(classify(200, json!({}), None, false).is_ok());
    }

    #[test]
    fn gitlab_style_messages_are_flattened() {
        let m = message(&json!({"message": {"base": ["Another open merge request already exists"], "x": ["y", "z"]}}));
        assert!(m.contains("Another open merge request") && m.contains("y, z"), "{m}");
        assert_eq!(message(&json!({"error": "invalid_token"})), "invalid_token");
    }

    #[test]
    fn paths_are_encoded_for_urls() {
        assert_eq!(enc("acme/shop/api"), "acme%2Fshop%2Fapi");
        assert_eq!(enc("a b"), "a%20b");
    }
}
