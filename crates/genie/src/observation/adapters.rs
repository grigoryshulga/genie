//! Adapters that measure observation sources: Prometheus- and Sentry-compatible (§7.3, §7.4).
//! One async `measure` function dispatches on the source kind (D12). The adapters touch no
//! database: the scheduler stores their results. Secrets go only into the Authorization header;
//! they never reach errors, provenance or logs.

use std::fmt;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use genie_core::observation::{SourceAuth, SourceConfig, SourceKind, ts};
use reqwest::{Client, RequestBuilder, Response, Url};
use serde_json::{Value, json};

/// Largest response body an adapter reads; a bigger answer is an error (§7.3).
const MAX_BODY: usize = 1024 * 1024;
/// Characters of a successful body kept in provenance (§7.4).
const EXCERPT_CHARS: usize = 300;
/// Characters of a failed body kept in `Http` errors (§7.3).
const ERROR_BODY_CHARS: usize = 500;
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 120;
const DEFAULT_SENTRY_FIELD: &str = "count()";
const DEFAULT_SENTRY_DATASET: &str = "errors";
const REDACTED: &str = "[redacted]";

/// The observed window of a measurement: `start` to `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// One successful measurement. `samples` is set when the source counts events.
#[derive(Debug, Clone, PartialEq)]
pub struct Measurement {
    pub value: f64,
    pub samples: Option<u64>,
    /// The query sent, after placeholder substitution.
    pub query: String,
    /// Where the value came from (§7.4). Never contains a secret.
    pub provenance: Value,
}

/// Why a measurement failed. The scheduler stores `to_string()` in `Evidence.error`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// The source is configured wrongly, or it lacks a secret it needs.
    Config(String),
    /// 401 or 403: the secret is wrong, expired or lacks the right. Holds the HTTP status.
    Unauthorized(String),
    /// Any other non-2xx answer, with the first 500 characters of its body.
    Http { status: u16, body: String },
    /// The answer is not what the adapter reads.
    Parse(String),
    /// The source did not answer, or the connection failed. Holds a full sentence.
    Network(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Config(m) | SourceError::Network(m) => f.write_str(m),
            SourceError::Unauthorized(m) => write!(f, "the source refused the credentials ({m}); check the source's secret"),
            SourceError::Http { status, body } if body.is_empty() => write!(f, "the source answered HTTP {status}"),
            SourceError::Http { status, body } => write!(f, "the source answered HTTP {status}: {body}"),
            SourceError::Parse(m) => write!(f, "cannot read the source answer: {m}"),
        }
    }
}

impl std::error::Error for SourceError {}

/// Measures `query` over `span` against the source. `samples_query` (Prometheus only) counts the
/// events of the same window. `secret` is the source's stored secret, if it has one (§7.3).
pub async fn measure(
    src: &SourceConfig,
    secret: Option<&str>,
    query: &str,
    samples_query: Option<&str>,
    span: Span,
) -> Result<Measurement, SourceError> {
    let base = base_url(src)?;
    let timeout_secs = timeout_secs(src)?;
    let auth = Auth::new(src, secret)?;
    let http = Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| SourceError::Network(format!("source {} cannot start an HTTP client: {e}", src.name)))?;
    let conn = Conn { src, http, auth, timeout_secs };
    match src.kind {
        SourceKind::Prometheus => conn.prometheus(&base, query, samples_query, span).await,
        SourceKind::Sentry => conn.sentry(&base, query, span).await,
    }
}

/// Replaces every `$__range` with the span length in whole seconds, at least `1s` (§7.3).
pub fn substitute_range(query: &str, span: Span) -> String {
    let secs = (span.end - span.start).num_seconds().max(1);
    query.replace("$__range", &format!("{secs}s"))
}

/// How requests authenticate. Built before any request is sent, so a missing secret sends nothing.
enum Auth {
    Anonymous,
    Bearer(String),
    Basic { user: String, password: String },
}

impl Auth {
    fn new(src: &SourceConfig, secret: Option<&str>) -> Result<Self, SourceError> {
        let secret = secret.filter(|s| !s.is_empty());
        let missing = || SourceError::Config(format!("source {} needs a secret", src.name));
        match src.auth {
            SourceAuth::None => Ok(Auth::Anonymous),
            SourceAuth::Bearer => Ok(Auth::Bearer(secret.ok_or_else(missing)?.to_string())),
            SourceAuth::Basic => {
                let pair = secret.ok_or_else(missing)?;
                let (user, password) = pair
                    .split_once(':')
                    .ok_or_else(|| SourceError::Config(format!("source {} needs a secret as user:password for basic auth", src.name)))?;
                Ok(Auth::Basic { user: user.to_string(), password: password.to_string() })
            }
        }
    }

    fn apply(&self, req: RequestBuilder) -> RequestBuilder {
        match self {
            Auth::Anonymous => req,
            Auth::Bearer(token) => req.bearer_auth(token),
            Auth::Basic { user, password } => req.basic_auth(user, Some(password)),
        }
    }

    /// The values that must never appear in an answer shown to people.
    fn secrets(&self) -> Vec<&str> {
        match self {
            Auth::Anonymous => Vec::new(),
            Auth::Bearer(token) => vec![token.as_str()],
            Auth::Basic { password, .. } if !password.is_empty() => vec![password.as_str()],
            Auth::Basic { .. } => Vec::new(),
        }
    }
}

/// One source call: the client, the credentials and the source's settings.
struct Conn<'a> {
    src: &'a SourceConfig,
    http: Client,
    auth: Auth,
    timeout_secs: u64,
}

/// A successful answer, read in full.
struct Reply {
    url: String,
    status: u16,
    duration_ms: u64,
    body: String,
}

impl Conn<'_> {
    async fn prometheus(&self, base: &Url, query: &str, samples_query: Option<&str>, span: Span) -> Result<Measurement, SourceError> {
        let expanded = substitute_range(query, span);
        let (value, reply) = self.prometheus_value(base, &expanded, span).await?;
        let samples = match samples_query {
            Some(q) => {
                let (count, _) = self.prometheus_value(base, &substitute_range(q, span), span).await?;
                Some(round_count(count))
            }
            None => None,
        };
        Ok(Measurement { value, samples, query: expanded, provenance: provenance("prometheus", &reply) })
    }

    /// One instant query at the end of the span; returns the single sample and the answer.
    async fn prometheus_value(&self, base: &Url, query: &str, span: Span) -> Result<(f64, Reply), SourceError> {
        let mut url = endpoint(base, &["api", "v1", "query"]);
        url.query_pairs_mut().append_pair("query", query).append_pair("time", &unix_time(span.end));
        let reply = self.get(url).await?;
        let answer: Value = serde_json::from_str(&reply.body).map_err(|_| SourceError::Parse("response is not JSON".into()))?;
        Ok((prometheus_sample(&answer)?, reply))
    }

    async fn sentry(&self, base: &Url, query: &str, span: Span) -> Result<Measurement, SourceError> {
        let src = self.src;
        let organization = non_empty(&src.organization)
            .ok_or_else(|| SourceError::Config(format!("source {} needs an organization for a Sentry source", src.name)))?;
        let field = non_empty(&src.field).unwrap_or(DEFAULT_SENTRY_FIELD);
        let dataset = non_empty(&src.dataset).unwrap_or(DEFAULT_SENTRY_DATASET);
        let mut url = endpoint(base, &["api", "0", "organizations", organization, "events", ""]);
        {
            let mut params = url.query_pairs_mut();
            params
                .append_pair("field", field)
                .append_pair("query", query)
                .append_pair("start", &ts(span.start))
                .append_pair("end", &ts(span.end))
                .append_pair("dataset", dataset);
            if let Some(project) = non_empty(&src.sentry_project) {
                params.append_pair("project", project);
            }
            if let Some(environment) = non_empty(&src.environment) {
                params.append_pair("environment", environment);
            }
        }
        let reply = self.get(url).await?;
        let value = sentry_value(&reply.body, field)?;
        // Only count fields are event counts; any other aggregate has no sample size (§7.3).
        let samples = field.starts_with("count").then(|| round_count(value));
        Ok(Measurement { value, samples, query: query.to_string(), provenance: provenance("sentry", &reply) })
    }

    /// One GET with the source's credentials. 401 and 403 are `Unauthorized`; other non-2xx
    /// answers are `Http` with a truncated, redacted body.
    async fn get(&self, url: Url) -> Result<Reply, SourceError> {
        let started = Instant::now();
        let res = self.auth.apply(self.http.get(url.clone())).send().await.map_err(|e| self.network(e))?;
        let status = res.status().as_u16();
        if status == 401 || status == 403 {
            return Err(SourceError::Unauthorized(format!("HTTP {status}")));
        }
        let ok = res.status().is_success();
        let body = self.read(res, !ok).await?;
        if !ok {
            return Err(SourceError::Http { status, body: head(&body, ERROR_BODY_CHARS) });
        }
        Ok(Reply { url: url.to_string(), status, duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX), body })
    }

    /// Reads the body up to `MAX_BODY`. An error answer is truncated instead of refused, so the
    /// caller still learns its status.
    async fn read(&self, mut res: Response, truncate: bool) -> Result<String, SourceError> {
        if !truncate && res.content_length().is_some_and(|n| n > MAX_BODY as u64) {
            return Err(too_large());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = res.chunk().await.map_err(|e| self.network(e))? {
            let room = MAX_BODY - bytes.len();
            if chunk.len() > room {
                if !truncate {
                    return Err(too_large());
                }
                bytes.extend_from_slice(&chunk[..room]);
                break;
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(self.redact(String::from_utf8_lossy(&bytes).into_owned()))
    }

    fn redact(&self, text: String) -> String {
        self.auth.secrets().into_iter().fold(text, |t, secret| t.replace(secret, REDACTED))
    }

    fn network(&self, e: reqwest::Error) -> SourceError {
        if e.is_timeout() {
            return SourceError::Network(format!("source {} did not answer within {} s", self.src.name, self.timeout_secs));
        }
        SourceError::Network(format!("source {} is unreachable: {}", self.src.name, describe(e)))
    }
}

/// The source URL: http or https, without credentials (the secret travels in the header only).
fn base_url(src: &SourceConfig) -> Result<Url, SourceError> {
    let url = Url::parse(&src.url).map_err(|_| SourceError::Config(format!("source {} has an invalid url", src.name)))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(SourceError::Config(format!("source {} url must be http or https", src.name)));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SourceError::Config(format!(
            "source {} url must not contain credentials; store the secret for the source instead",
            src.name
        )));
    }
    Ok(url)
}

fn timeout_secs(src: &SourceConfig) -> Result<u64, SourceError> {
    match src.timeout_secs {
        None => Ok(DEFAULT_TIMEOUT_SECS),
        Some(secs) if (1..=MAX_TIMEOUT_SECS).contains(&secs) => Ok(secs),
        Some(_) => Err(SourceError::Config(format!("source {} timeout must be 1 to {MAX_TIMEOUT_SECS} seconds", src.name))),
    }
}

/// `base` with `segments` appended to its path. `base` is http(s), so it can take segments.
fn endpoint(base: &Url, segments: &[&str]) -> Url {
    let mut url = base.clone();
    url.path_segments_mut().expect("http(s) URLs can take path segments").pop_if_empty().extend(segments);
    url
}

fn sentry_value(body: &str, field: &str) -> Result<f64, SourceError> {
    let answer: Value = serde_json::from_str(body).map_err(|_| SourceError::Parse("response is not JSON".into()))?;
    let rows = answer["data"].as_array().ok_or_else(|| SourceError::Parse("response has no data list".into()))?;
    match rows.first() {
        // No matching events: the count is zero.
        None => Ok(0.0),
        Some(row) => row[field].as_f64().ok_or_else(|| SourceError::Parse(format!("response has no number for {field}"))),
    }
}

/// The single sample of a Prometheus instant query (§7.3).
fn prometheus_sample(answer: &Value) -> Result<f64, SourceError> {
    if answer["status"].as_str() != Some("success") {
        let message = answer["error"].as_str().unwrap_or("query failed");
        return Err(SourceError::Parse(message.to_string()));
    }
    let data = &answer["data"];
    let result = &data["result"];
    match data["resultType"].as_str().unwrap_or_default() {
        "vector" => {
            let series = result.as_array().ok_or_else(|| SourceError::Parse("response has no result list".into()))?;
            match series.len() {
                0 => Err(SourceError::Parse("query returned no series".into())),
                1 => sample(&series[0]["value"]),
                n => Err(SourceError::Parse(format!("query returned {n} series; aggregate them into one, for example with sum()"))),
            }
        }
        "scalar" => sample(result),
        other => Err(SourceError::Parse(format!("unsupported result type {other}"))),
    }
}

/// `[timestamp, "value"]` as Prometheus writes it.
fn sample(pair: &Value) -> Result<f64, SourceError> {
    let text = pair[1].as_str().ok_or_else(|| SourceError::Parse("query returned no value".into()))?;
    let value: f64 = text.parse().map_err(|_| SourceError::Parse("query returned a value that is not a number".into()))?;
    if value.is_finite() { Ok(value) } else { Err(SourceError::Parse("query returned a non-finite value".into())) }
}

/// Sample counts are whole and never negative (§7.3).
fn round_count(n: f64) -> u64 {
    n.round().max(0.0) as u64
}

/// Unix seconds with three decimals, as Prometheus takes `time` (§7.3).
fn unix_time(at: DateTime<Utc>) -> String {
    let ms = at.timestamp_millis();
    format!("{}.{:03}", ms.div_euclid(1000), ms.rem_euclid(1000))
}

/// The provenance of a successful measurement (§7.4). The URL has no credentials and the body
/// excerpt is redacted before it is kept.
fn provenance(adapter: &str, reply: &Reply) -> Value {
    json!({
        "adapter": adapter,
        "url": reply.url,
        "status": reply.status,
        "durationMs": reply.duration_ms,
        "excerpt": head(&reply.body, EXCERPT_CHARS),
    })
}

fn head(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|s| !s.is_empty())
}

fn too_large() -> SourceError {
    SourceError::Parse("response larger than 1 MiB".into())
}

/// The error and its causes, without the request URL.
fn describe(e: reqwest::Error) -> String {
    let e = e.without_url();
    let mut out = e.to_string();
    let mut cause = std::error::Error::source(&e);
    while let Some(c) = cause {
        out.push_str(": ");
        out.push_str(&c.to_string());
        cause = std::error::Error::source(c);
    }
    out
}
