//! Observation adapters: Prometheus- and Sentry-compatible sources against a fake server.

use chrono::{DateTime, Utc};
use genie::observation::adapters::{Measurement, SourceError, Span, measure, substitute_range};
use genie_core::observation::{SourceAuth, SourceConfig, SourceKind, ts};
use reqwest::Url;
use serde_json::{Value, json};

use crate::common::fakemetrics::{self, Call, FakeMetrics};

const CREATED: &str = "2026-10-01T00:00:00.000Z";

fn at(s: &str) -> DateTime<Utc> {
    s.parse().unwrap()
}

/// One day, ending at 2026-10-17T12:00:00Z.
fn span() -> Span {
    Span { start: at("2026-10-16T12:00:00Z"), end: at("2026-10-17T12:00:00Z") }
}

fn source(fake: &FakeMetrics, kind: SourceKind, auth: SourceAuth) -> SourceConfig {
    source_at(&fake.url, kind, auth)
}

fn source_at(url: &str, kind: SourceKind, auth: SourceAuth) -> SourceConfig {
    SourceConfig {
        name: "metrics".into(),
        kind,
        url: url.into(),
        auth,
        organization: None,
        sentry_project: None,
        environment: None,
        field: None,
        dataset: None,
        timeout_secs: None,
        created: CREATED.into(),
        updated: CREATED.into(),
    }
}

fn prometheus(fake: &FakeMetrics) -> SourceConfig {
    source(fake, SourceKind::Prometheus, SourceAuth::None)
}

fn sentry(fake: &FakeMetrics) -> SourceConfig {
    SourceConfig { organization: Some("acme".into()), ..source(fake, SourceKind::Sentry, SourceAuth::None) }
}

/// A vector answer with one series.
fn vector(sample: &str) -> Value {
    json!({ "status": "success", "data": { "resultType": "vector", "result": [{ "metric": { "job": "api" }, "value": [1760702400, sample] }] } })
}

fn scalar(sample: &str) -> Value {
    json!({ "status": "success", "data": { "resultType": "scalar", "result": [1760702400, sample] } })
}

fn calls(fake: &FakeMetrics) -> Vec<Call> {
    fake.state.lock().calls.clone()
}

/// The decoded query parameters of a request the fake received.
fn params(call: &Call) -> Vec<(String, String)> {
    Url::parse(&call.url).unwrap().query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
}

fn param(call: &Call, key: &str) -> Option<String> {
    params(call).into_iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn config_error(err: SourceError) -> String {
    match err {
        SourceError::Config(m) => m,
        other => panic!("expected a configuration error, got {other:?}"),
    }
}

fn parse_error(err: SourceError) -> String {
    match err {
        SourceError::Parse(m) => m,
        other => panic!("expected a parse error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prometheus_vector_with_one_series_is_the_value() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("api_calls", vector("0.25"));
    let q = "sum(rate(api_calls[5m]))";
    let m = measure(&prometheus(&fake), None, q, None, span()).await.unwrap();
    assert_eq!(m.value, 0.25);
    assert_eq!(m.samples, None);
    assert_eq!(m.query, q);
    assert_eq!(m.provenance["adapter"], "prometheus");
    assert_eq!(m.provenance["status"], 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prometheus_scalar_is_the_value() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("scalar_probe", scalar("42.5"));
    let m = measure(&prometheus(&fake), None, "scalar_probe", None, span()).await.unwrap();
    assert_eq!(m.value, 42.5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prometheus_empty_result_is_a_parse_error() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("empty_probe", json!({ "status": "success", "data": { "resultType": "vector", "result": [] } }));
    let err = measure(&prometheus(&fake), None, "empty_probe", None, span()).await.unwrap_err();
    assert_eq!(parse_error(err), "query returned no series");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prometheus_two_series_ask_for_an_aggregate() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond(
        "two_probe",
        json!({ "status": "success", "data": { "resultType": "vector", "result": [
            { "metric": { "job": "a" }, "value": [1760702400, "1"] },
            { "metric": { "job": "b" }, "value": [1760702400, "2"] }
        ] } }),
    );
    let err = measure(&prometheus(&fake), None, "two_probe", None, span()).await.unwrap_err();
    assert_eq!(parse_error(err), "query returned 2 series; aggregate them into one, for example with sum()");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prometheus_error_status_carries_the_source_message() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("bad_probe", json!({ "status": "error", "errorType": "bad_data", "error": "parse error at char 5" }));
    let err = measure(&prometheus(&fake), None, "bad_probe", None, span()).await.unwrap_err();
    assert_eq!(parse_error(err), "parse error at char 5");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prometheus_nan_and_infinity_are_refused() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("nan_probe", vector("NaN"));
    fake.state.lock().respond("inf_probe", vector("+Inf"));
    let err = measure(&prometheus(&fake), None, "nan_probe", None, span()).await.unwrap_err();
    assert_eq!(parse_error(err), "query returned a non-finite value");
    let err = measure(&prometheus(&fake), None, "inf_probe", None, span()).await.unwrap_err();
    assert_eq!(parse_error(err), "query returned a non-finite value");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prometheus_non_success_status_is_http_with_a_short_body() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("broken_probe", json!({ "error": "x".repeat(600) }));
    fake.state.lock().status = Some(500);
    match measure(&prometheus(&fake), None, "broken_probe", None, span()).await.unwrap_err() {
        SourceError::Http { status, body } => {
            assert_eq!(status, 500);
            assert_eq!(body.chars().count(), 500, "the body is cut to 500 characters");
            assert!(body.starts_with("{\"error\":\"xxx"), "{body}");
        }
        other => panic!("expected an HTTP error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_secret_is_unauthorized() {
    let fake = fakemetrics::spawn(Some("right-token")).await;
    fake.state.lock().respond("auth_probe", vector("1"));
    let src = source(&fake, SourceKind::Prometheus, SourceAuth::Bearer);
    match measure(&src, Some("wrong-token"), "auth_probe", None, span()).await.unwrap_err() {
        SourceError::Unauthorized(m) => assert_eq!(m, "HTTP 401"),
        other => panic!("expected unauthorized, got {other:?}"),
    }
    let err = measure(&src, Some("wrong-token"), "auth_probe", None, span()).await.unwrap_err();
    assert_eq!(err.to_string(), "the source refused the credentials (HTTP 401); check the source's secret");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn range_placeholders_are_replaced_by_the_span_seconds() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("rate(http_requests_total", vector("3"));
    let m = measure(&prometheus(&fake), None, "sum(rate(http_requests_total[$__range]))", None, span()).await.unwrap();
    assert_eq!(m.query, "sum(rate(http_requests_total[86400s]))");
    let sent = &calls(&fake)[0];
    assert_eq!(param(sent, "query").as_deref(), Some("sum(rate(http_requests_total[86400s]))"));
}

#[test]
fn substitute_range_uses_whole_seconds_with_a_one_second_minimum() {
    assert_eq!(substitute_range("a[$__range] b[$__range]", span()), "a[86400s] b[86400s]");
    let instant = Span { start: at("2026-10-17T12:00:00Z"), end: at("2026-10-17T12:00:00.400Z") };
    assert_eq!(substitute_range("a[$__range]", instant), "a[1s]");
    assert_eq!(substitute_range("no placeholder", span()), "no placeholder");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_time_parameter_is_the_span_end_in_unix_seconds() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("time_probe", vector("1"));
    measure(&prometheus(&fake), None, "time_probe", None, span()).await.unwrap();
    let sent = &calls(&fake)[0];
    assert!(sent.url.starts_with(&format!("{}/api/v1/query?", fake.url)), "{}", sent.url);
    assert_eq!(param(sent, "time").as_deref(), Some(format!("{}.000", span().end.timestamp()).as_str()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_samples_query_is_rounded_to_a_whole_count() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("rate(api_calls", vector("0.5"));
    fake.state.lock().respond("increase(api_calls", vector("10000.4"));
    let main = "sum(rate(api_calls[$__range]))";
    let samples = "sum(increase(api_calls[$__range]))";
    let m: Measurement = measure(&prometheus(&fake), None, main, Some(samples), span()).await.unwrap();
    assert_eq!(m.samples, Some(10000));

    fake.state.lock().respond("increase(api_calls", vector("9999.6"));
    let m = measure(&prometheus(&fake), None, main, Some(samples), span()).await.unwrap();
    assert_eq!(m.samples, Some(10000));

    fake.state.lock().respond("increase(api_calls", vector("-3.2"));
    let m = measure(&prometheus(&fake), None, main, Some(samples), span()).await.unwrap();
    assert_eq!(m.samples, Some(0), "a sample count is never negative");
    assert_eq!(m.value, 0.5);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sentry_count_sends_project_environment_and_the_window() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("level:error", json!({ "data": [{ "count()": 37 }] }));
    let src = SourceConfig { sentry_project: Some("12".into()), environment: Some("production".into()), ..sentry(&fake) };
    let m = measure(&src, None, "transaction:/checkout level:error", None, span()).await.unwrap();
    assert_eq!(m.value, 37.0);
    assert_eq!(m.samples, Some(37));
    assert_eq!(m.query, "transaction:/checkout level:error");
    assert_eq!(m.provenance["adapter"], "sentry");

    let sent = &calls(&fake)[0];
    assert_eq!(Url::parse(&sent.url).unwrap().path(), "/api/0/organizations/acme/events/");
    assert_eq!(param(sent, "field").as_deref(), Some("count()"), "field defaults to count()");
    assert_eq!(param(sent, "dataset").as_deref(), Some("errors"), "dataset defaults to errors");
    assert_eq!(param(sent, "query").as_deref(), Some("transaction:/checkout level:error"));
    assert_eq!(param(sent, "start"), Some(ts(span().start)));
    assert_eq!(param(sent, "end"), Some(ts(span().end)));
    assert_eq!(param(sent, "project").as_deref(), Some("12"));
    assert_eq!(param(sent, "environment").as_deref(), Some("production"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sentry_without_data_counts_zero() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("nothing_matches", json!({ "data": [] }));
    let m = measure(&sentry(&fake), None, "nothing_matches", None, span()).await.unwrap();
    assert_eq!(m.value, 0.0);
    assert_eq!(m.samples, Some(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sentry_custom_field_has_no_sample_count() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("p95_probe", json!({ "data": [{ "p95(transaction.duration)": 412.5 }] }));
    let src = SourceConfig { field: Some("p95(transaction.duration)".into()), dataset: Some("transactions".into()), ..sentry(&fake) };
    let m = measure(&src, None, "p95_probe", None, span()).await.unwrap();
    assert_eq!(m.value, 412.5);
    assert_eq!(m.samples, None);
    let sent = &calls(&fake)[0];
    assert_eq!(param(sent, "field").as_deref(), Some("p95(transaction.duration)"));
    assert_eq!(param(sent, "dataset").as_deref(), Some("transactions"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sentry_refused_secret_is_unauthorized() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("anything", json!({ "data": [] }));
    fake.state.lock().status = Some(403);
    match measure(&sentry(&fake), None, "anything", None, span()).await.unwrap_err() {
        SourceError::Unauthorized(m) => assert_eq!(m, "HTTP 403"),
        other => panic!("expected unauthorized, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sentry_without_an_organization_is_a_configuration_error() {
    let fake = fakemetrics::spawn(None).await;
    let src = SourceConfig { organization: None, ..source(&fake, SourceKind::Sentry, SourceAuth::None) };
    let msg = config_error(measure(&src, None, "level:error", None, span()).await.unwrap_err());
    assert_eq!(msg, "source metrics needs an organization for a Sentry source");
    assert!(calls(&fake).is_empty(), "no request is sent");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bearer_secret_is_sent_as_the_authorization_header() {
    let fake = fakemetrics::spawn(Some("tok-123")).await;
    fake.state.lock().respond("bearer_probe", vector("1"));
    let src = source(&fake, SourceKind::Prometheus, SourceAuth::Bearer);
    measure(&src, Some("tok-123"), "bearer_probe", None, span()).await.unwrap();
    assert_eq!(calls(&fake)[0].authorization.as_deref(), Some("Bearer tok-123"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_basic_secret_is_sent_as_basic_auth() {
    let fake = fakemetrics::spawn(Some("ops:pa55")).await;
    fake.state.lock().respond("basic_probe", json!({ "data": [{ "count()": 2 }] }));
    let src = source(&fake, SourceKind::Sentry, SourceAuth::Basic);
    let src = SourceConfig { organization: Some("acme".into()), ..src };
    measure(&src, Some("ops:pa55"), "basic_probe", None, span()).await.unwrap();
    let expected = format!("Basic {}", base64_of("ops:pa55"));
    assert_eq!(calls(&fake)[0].authorization.as_deref(), Some(expected.as_str()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_auth_sends_no_authorization_header() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("open_probe", vector("1"));
    measure(&prometheus(&fake), Some("ignored"), "open_probe", None, span()).await.unwrap();
    assert_eq!(calls(&fake)[0].authorization, None);
}

fn base64_of(s: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(s)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_secret_is_a_configuration_error_and_sends_nothing() {
    let fake = fakemetrics::spawn(Some("tok")).await;
    let bearer = source(&fake, SourceKind::Prometheus, SourceAuth::Bearer);
    let msg = config_error(measure(&bearer, None, "q", None, span()).await.unwrap_err());
    assert_eq!(msg, "source metrics needs a secret");
    let msg = config_error(measure(&bearer, Some(""), "q", None, span()).await.unwrap_err());
    assert_eq!(msg, "source metrics needs a secret");

    let basic = source(&fake, SourceKind::Prometheus, SourceAuth::Basic);
    let msg = config_error(measure(&basic, Some("no-colon"), "q", None, span()).await.unwrap_err());
    assert_eq!(msg, "source metrics needs a secret as user:password for basic auth");
    assert!(calls(&fake).is_empty(), "no request is sent without a usable secret");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_url_with_credentials_or_another_scheme_is_refused() {
    let fake = fakemetrics::spawn(None).await;
    let with_userinfo = SourceConfig { url: fake.url.replacen("http://", "http://ops:pa55@", 1), ..prometheus(&fake) };
    let msg = config_error(measure(&with_userinfo, None, "q", None, span()).await.unwrap_err());
    assert_eq!(msg, "source metrics url must not contain credentials; store the secret for the source instead");
    let file = SourceConfig { url: "file:///etc/passwd".into(), ..prometheus(&fake) };
    let msg = config_error(measure(&file, None, "q", None, span()).await.unwrap_err());
    assert_eq!(msg, "source metrics url must be http or https");
    assert!(calls(&fake).is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeouts_outside_one_to_120_seconds_are_refused() {
    let fake = fakemetrics::spawn(None).await;
    for bad in [0, 121] {
        let src = SourceConfig { timeout_secs: Some(bad), ..prometheus(&fake) };
        let msg = config_error(measure(&src, None, "q", None, span()).await.unwrap_err());
        assert_eq!(msg, "source metrics timeout must be 1 to 120 seconds");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_oversize_body_is_rejected_while_it_streams() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().oversize = true;
    let msg = parse_error(measure(&prometheus(&fake), None, "big_probe", None, span()).await.unwrap_err());
    assert_eq!(msg, "response larger than 1 MiB");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_oversize_body_is_rejected_by_its_length() {
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("big_probe", json!({ "status": "success", "pad": "x".repeat(1024 * 1024 + 16) }));
    let msg = parse_error(measure(&prometheus(&fake), None, "big_probe", None, span()).await.unwrap_err());
    assert_eq!(msg, "response larger than 1 MiB");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn provenance_has_no_secret_and_no_authorization_value() {
    let secret = "s3cret-token";
    let fake = fakemetrics::spawn(Some(secret)).await;
    // The source echoes the token back in its answer.
    let mut answer = scalar("7");
    answer["note"] = json!(secret);
    fake.state.lock().respond("echo_probe", answer);
    let src = source(&fake, SourceKind::Prometheus, SourceAuth::Bearer);
    let m = measure(&src, Some(secret), "echo_probe", None, span()).await.unwrap();
    assert_eq!(m.value, 7.0);

    let text = m.provenance.to_string();
    assert!(!text.contains(secret), "{text}");
    assert!(!text.contains("Authorization") && !text.contains("Bearer"), "{text}");
    assert!(m.provenance["excerpt"].as_str().unwrap().contains("[redacted]"), "{text}");
    let url = m.provenance["url"].as_str().unwrap();
    assert!(url.starts_with(&format!("{}/api/v1/query?", fake.url)), "{url}");
    assert!(m.provenance["durationMs"].is_u64());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn secrets_echoed_in_an_error_are_redacted() {
    let secret = "ops:pa55";
    let fake = fakemetrics::spawn(None).await;
    fake.state.lock().respond("err_probe", json!({ "error": format!("bad credentials {secret}") }));
    fake.state.lock().status = Some(500);
    let src = source(&fake, SourceKind::Prometheus, SourceAuth::Basic);
    match measure(&src, Some(secret), "err_probe", None, span()).await.unwrap_err() {
        SourceError::Http { body, .. } => {
            assert!(!body.contains("pa55"), "{body}");
            assert!(body.contains("[redacted]"), "{body}");
        }
        other => panic!("expected an HTTP error, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_source_is_a_network_error_that_names_it() {
    // A port nothing listens on: bind, note the port, and let the listener go.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let src = source_at(&format!("http://127.0.0.1:{port}"), SourceKind::Prometheus, SourceAuth::None);
    match measure(&src, None, "q", None, span()).await.unwrap_err() {
        SourceError::Network(m) => assert!(m.starts_with("source metrics is unreachable: "), "{m}"),
        other => panic!("expected a network error, got {other:?}"),
    }
}

#[test]
fn source_errors_read_as_sentences() {
    assert_eq!(SourceError::Config("source a needs a secret".into()).to_string(), "source a needs a secret");
    assert_eq!(SourceError::Http { status: 500, body: String::new() }.to_string(), "the source answered HTTP 500");
    assert_eq!(SourceError::Http { status: 502, body: "down".into() }.to_string(), "the source answered HTTP 502: down");
    assert_eq!(
        SourceError::Parse("query returned no series".into()).to_string(),
        "cannot read the source answer: query returned no series"
    );
    assert_eq!(SourceError::Network("source a did not answer within 30 s".into()).to_string(), "source a did not answer within 30 s");
}
