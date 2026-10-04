//! Fake GitHub and GitLab APIs on a real port: the pull/merge request, comment, review,
//! check and merge endpoints that genie uses, with the answers (and error shapes) of the real hosts.
//! One state per fake, so a test can change what the host says (checks, approvals, refusals).

#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct Pr {
    pub number: i64,
    pub head: String,
    pub base: String,
    pub title: String,
    pub body: String,
    /// `open`, `merged` or `closed`.
    pub state: String,
    pub sha: String,
    pub draft: bool,
}

#[derive(Debug)]
pub struct Fake {
    pub kind: &'static str,
    pub token: String,
    pub prs: Vec<Pr>,
    pub comments: Vec<(String, String)>,
    pub approvals: u32,
    pub changes_requested: bool,
    /// `none`, `pending`, `passed` or `failed`.
    pub ci: String,
    /// How many rerun calls the host accepted (a rerun flips `ci` back to `pending`).
    pub reruns: u32,
    /// The next rerun is refused with this message (403: a token without `actions:write`).
    pub refuse_rerun: Option<String>,
    /// Whether GitHub reports Actions workflow runs (`false`: the failure is a commit status or
    /// another app's check run, which has no rerun API).
    pub action_runs: bool,
    pub mergeable: bool,
    /// The next merge is refused with this message.
    pub refuse_merge: Option<String>,
    /// Answer the next reads with 429 (with `Retry-After: 0`) this many times.
    pub rate_limited: u32,
    /// Answer the next reads with 502 this many times.
    pub broken: u32,
    pub protected: Vec<String>,
    pub sha: String,
    /// The commit a merge produced on the target branch (`merge_commit_sha`); `None` is a host
    /// that does not say, so the target branch is not watched.
    pub merge_sha: Option<String>,
    pub calls: Vec<String>,
}

pub type Shared = Arc<Mutex<Fake>>;

pub struct FakeHost {
    pub url: String,
    pub state: Shared,
}

impl FakeHost {
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Fake> {
        self.state.lock().unwrap()
    }
}

/// A fake of `kind` (`github` or `gitlab`) accepting `token`, serving the repository `acme/api`.
pub async fn spawn(kind: &'static str, token: &str) -> FakeHost {
    let state: Shared = Arc::new(Mutex::new(Fake {
        kind,
        token: token.into(),
        prs: Vec::new(),
        comments: Vec::new(),
        approvals: 0,
        changes_requested: false,
        ci: "none".into(),
        reruns: 0,
        refuse_rerun: None,
        action_runs: true,
        mergeable: true,
        refuse_merge: None,
        rate_limited: 0,
        broken: 0,
        protected: vec!["main".into()],
        sha: "1111111111111111111111111111111111111111".into(),
        merge_sha: Some("2222222222222222222222222222222222222222".into()),
        calls: Vec::new(),
    }));
    let app = if kind == "github" { github(state.clone()) } else { gitlab(state.clone()) };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    FakeHost { url, state }
}

type Q = Query<std::collections::HashMap<String, String>>;

/// Authentication, the scripted failures and the call log, common to every endpoint.
#[allow(clippy::result_large_err)]
fn gate(s: &Shared, headers: &HeaderMap, what: &str, read: bool) -> Result<(), Response> {
    let mut f = s.lock().unwrap();
    f.calls.push(what.to_string());
    let given = if f.kind == "github" {
        headers.get("authorization").and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::to_string)
    } else {
        headers.get("private-token").and_then(|v| v.to_str().ok()).map(str::to_string)
    };
    if given.as_deref() != Some(f.token.as_str()) {
        return Err((StatusCode::UNAUTHORIZED, axum::Json(json!({ "message": "Bad credentials" }))).into_response());
    }
    if read && f.rate_limited > 0 {
        f.rate_limited -= 1;
        return Err(
            (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "0")], axum::Json(json!({ "message": "rate limited" }))).into_response()
        );
    }
    if read && f.broken > 0 {
        f.broken -= 1;
        return Err((StatusCode::BAD_GATEWAY, "bad gateway").into_response());
    }
    Ok(())
}

fn not_found() -> Response {
    (StatusCode::NOT_FOUND, axum::Json(json!({ "message": "Not Found" }))).into_response()
}

macro_rules! check {
    ($s:expr, $h:expr, $what:expr, $read:expr) => {
        if let Err(r) = gate(&$s, &$h, $what, $read) {
            return r;
        }
    };
}

// --- GitHub ------------------------------------------------------------------------------

fn gh_pr(f: &Fake, p: &Pr) -> Value {
    json!({
        "number": p.number, "html_url": format!("https://github.example/acme/api/pull/{}", p.number), "title": p.title,
        "state": if p.state == "open" { "open" } else { "closed" }, "merged": p.state == "merged",
        // The host only names the merge commit of a request that was merged.
        "merge_commit_sha": if p.state == "merged" { json!(f.merge_sha) } else { Value::Null },
        "draft": p.draft, "head": { "ref": p.head, "sha": p.sha }, "base": { "ref": p.base }, "mergeable": f.mergeable,
    })
}

fn github(s: Shared) -> Router {
    let api = Router::new()
        .route("/user", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "user", true);
            axum::Json(json!({ "login": "genie-bot" })).into_response()
        }))
        .route("/repos/{o}/{r}", get(|State(s): State<Shared>, h: HeaderMap, Path((o, r)): Path<(String, String)>| async move {
            check!(s, h, "repo", true);
            if (o.as_str(), r.as_str()) != ("acme", "api") {
                return not_found();
            }
            axum::Json(json!({ "default_branch": "main", "permissions": { "push": true } })).into_response()
        }))
        .route("/repos/{o}/{r}/branches", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "branches", true);
            let f = s.lock().unwrap();
            axum::Json(json!(f.protected.iter().map(|n| json!({ "name": n })).collect::<Vec<_>>())).into_response()
        }))
        .route("/repos/{o}/{r}/pulls", post(|State(s): State<Shared>, h: HeaderMap, axum::Json(b): axum::Json<Value>| async move {
            check!(s, h, "open", false);
            let mut f = s.lock().unwrap();
            let head = b["head"].as_str().unwrap_or_default().to_string();
            if f.prs.iter().any(|p| p.head == head && p.state == "open") {
                return (StatusCode::UNPROCESSABLE_ENTITY, axum::Json(json!({ "message": "Validation Failed", "errors": [{ "message": format!("A pull request already exists for acme:{head}.") }] }))).into_response();
            }
            let p = Pr {
                number: f.prs.len() as i64 + 1,
                head,
                base: b["base"].as_str().unwrap_or_default().into(),
                title: b["title"].as_str().unwrap_or_default().into(),
                body: b["body"].as_str().unwrap_or_default().into(),
                state: "open".into(),
                sha: f.sha.clone(),
                draft: b["draft"].as_bool().unwrap_or(false),
            };
            f.prs.push(p.clone());
            (StatusCode::CREATED, axum::Json(gh_pr(&f, &p))).into_response()
        }).get(|State(s): State<Shared>, h: HeaderMap, Query(q): Q| async move {
            check!(s, h, "list", true);
            let f = s.lock().unwrap();
            let head = q.get("head").and_then(|h| h.split(':').nth(1)).unwrap_or_default().to_string();
            axum::Json(json!(f.prs.iter().filter(|p| p.state == "open" && p.head == head).map(|p| gh_pr(&f, p)).collect::<Vec<_>>())).into_response()
        }))
        .route("/repos/{o}/{r}/pulls/{n}", get(|State(s): State<Shared>, h: HeaderMap, Path((_, _, n)): Path<(String, String, i64)>| async move {
            check!(s, h, "get", true);
            let f = s.lock().unwrap();
            match f.prs.iter().find(|p| p.number == n) {
                Some(p) => axum::Json(gh_pr(&f, p)).into_response(),
                None => not_found(),
            }
        }))
        .route("/repos/{o}/{r}/pulls/{n}/reviews", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "reviews", true);
            let f = s.lock().unwrap();
            let mut list: Vec<Value> = (0..f.approvals).map(|i| json!({ "user": { "login": format!("reviewer{i}") }, "state": "APPROVED", "body": "", "submitted_at": "2026-01-01T00:00:00Z" })).collect();
            if f.changes_requested {
                list.push(json!({ "user": { "login": "picky" }, "state": "CHANGES_REQUESTED", "body": "please rename", "submitted_at": "2026-01-02T00:00:00Z" }));
            }
            axum::Json(json!(list)).into_response()
        }))
        .route("/repos/{o}/{r}/pulls/{n}/comments", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "inline", true);
            axum::Json(json!([])).into_response()
        }))
        .route("/repos/{o}/{r}/issues/{n}/comments", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "comments", true);
            let f = s.lock().unwrap();
            axum::Json(json!(f.comments.iter().enumerate().map(|(i, (a, b))| json!({ "user": { "login": a }, "body": b, "created_at": format!("2026-02-01T00:00:{i:02}Z") })).collect::<Vec<_>>())).into_response()
        }).post(|State(s): State<Shared>, h: HeaderMap, axum::Json(b): axum::Json<Value>| async move {
            check!(s, h, "comment", false);
            s.lock().unwrap().comments.push(("genie-bot".into(), b["body"].as_str().unwrap_or_default().into()));
            (StatusCode::CREATED, axum::Json(json!({}))).into_response()
        }))
        .route("/repos/{o}/{r}/pulls/{n}/merge", put(|State(s): State<Shared>, h: HeaderMap, Path((_, _, n)): Path<(String, String, i64)>| async move {
            check!(s, h, "merge", false);
            let mut f = s.lock().unwrap();
            if let Some(m) = f.refuse_merge.take() {
                return (StatusCode::METHOD_NOT_ALLOWED, axum::Json(json!({ "message": m }))).into_response();
            }
            match f.prs.iter_mut().find(|p| p.number == n) {
                Some(p) => {
                    p.state = "merged".into();
                    axum::Json(json!({ "merged": true, "message": "Pull Request successfully merged" })).into_response()
                }
                None => not_found(),
            }
        }))
        .route("/repos/{o}/{r}/commits/{sha}/status", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "status", true);
            let f = s.lock().unwrap();
            let (state, total) = match f.ci.as_str() {
                "passed" => ("success", 1),
                "failed" => ("failure", 1),
                "pending" => ("pending", 1),
                _ => ("pending", 0),
            };
            axum::Json(json!({ "state": state, "total_count": total })).into_response()
        }))
        .route("/repos/{o}/{r}/commits/{sha}/check-runs", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "checks", true);
            let f = s.lock().unwrap();
            let runs = if f.ci == "failed" {
                vec![json!({
                    "name": "build", "status": "completed", "conclusion": "failure", "html_url": "https://github.example/acme/api/runs/9",
                    "output": { "title": "Build failed", "summary": "error[E0432]: unresolved import `orders`" }
                })]
            } else {
                Vec::new()
            };
            axum::Json(json!({ "check_runs": runs })).into_response()
        }))
        .route("/repos/{o}/{r}/actions/runs", get(|State(s): State<Shared>, h: HeaderMap, Query(q): Q| async move {
            check!(s, h, "runs", true);
            let f = s.lock().unwrap();
            let head = q.get("head_sha").cloned();
            let runs = if f.ci == "failed" && f.action_runs {
                vec![json!({ "id": 9, "status": "completed", "conclusion": "failure", "head_sha": head })]
            } else {
                Vec::new()
            };
            axum::Json(json!({ "total_count": runs.len(), "workflow_runs": runs })).into_response()
        }))
        .route(
            "/repos/{o}/{r}/actions/runs/{id}/rerun-failed-jobs",
            post(|State(s): State<Shared>, h: HeaderMap, Path((_, _, _)): Path<(String, String, i64)>| async move {
                check!(s, h, "rerun", false);
                let mut f = s.lock().unwrap();
                if let Some(m) = f.refuse_rerun.take() {
                    return (StatusCode::FORBIDDEN, axum::Json(json!({ "message": m }))).into_response();
                }
                f.reruns += 1;
                f.ci = "pending".into();
                (StatusCode::CREATED, axum::Json(json!({}))).into_response()
            }),
        );
    Router::new().nest("/api/v3", api).with_state(s)
}

// --- GitLab ------------------------------------------------------------------------------

fn gl_mr(f: &Fake, p: &Pr) -> Value {
    json!({
        "iid": p.number, "web_url": format!("https://gitlab.example/acme/api/-/merge_requests/{}", p.number), "title": p.title,
        "state": match p.state.as_str() { "open" => "opened", "merged" => "merged", _ => "closed" },
        "merge_commit_sha": if p.state == "merged" { json!(f.merge_sha) } else { Value::Null },
        "draft": p.draft, "source_branch": p.head, "target_branch": p.base, "sha": p.sha,
        "detailed_merge_status": if f.mergeable { "mergeable" } else { "conflict" },
    })
}

fn gitlab(s: Shared) -> Router {
    let api = Router::new()
        .route("/user", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "user", true);
            axum::Json(json!({ "username": "genie-bot" })).into_response()
        }))
        .route("/version", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "version", true);
            axum::Json(json!({ "version": "19.2.6" })).into_response()
        }))
        .route("/projects/{pid}", get(|State(s): State<Shared>, h: HeaderMap, Path(pid): Path<String>| async move {
            check!(s, h, "repo", true);
            if pid != "acme/api" {
                return not_found();
            }
            axum::Json(json!({ "default_branch": "main", "permissions": { "project_access": { "access_level": 30 }, "group_access": null } })).into_response()
        }))
        .route("/projects/{pid}/protected_branches", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "branches", true);
            let f = s.lock().unwrap();
            axum::Json(json!(f.protected.iter().map(|n| json!({ "name": n })).collect::<Vec<_>>())).into_response()
        }))
        .route("/projects/{pid}/merge_requests", post(|State(s): State<Shared>, h: HeaderMap, axum::Json(b): axum::Json<Value>| async move {
            check!(s, h, "open", false);
            let mut f = s.lock().unwrap();
            let head = b["source_branch"].as_str().unwrap_or_default().to_string();
            if f.prs.iter().any(|p| p.head == head && p.state == "open") {
                return (StatusCode::CONFLICT, axum::Json(json!({ "message": [format!("Another open merge request already exists for this source branch: !{}", f.prs.len())] }))).into_response();
            }
            let title: String = b["title"].as_str().unwrap_or_default().into();
            let p = Pr {
                number: f.prs.len() as i64 + 1,
                head,
                base: b["target_branch"].as_str().unwrap_or_default().into(),
                draft: title.starts_with("Draft:"),
                title,
                body: b["description"].as_str().unwrap_or_default().into(),
                state: "open".into(),
                sha: f.sha.clone(),
            };
            f.prs.push(p.clone());
            (StatusCode::CREATED, axum::Json(gl_mr(&f, &p))).into_response()
        }).get(|State(s): State<Shared>, h: HeaderMap, Query(q): Q| async move {
            check!(s, h, "list", true);
            let f = s.lock().unwrap();
            let head = q.get("source_branch").cloned().unwrap_or_default();
            axum::Json(json!(f.prs.iter().filter(|p| p.state == "open" && p.head == head).map(|p| gl_mr(&f, p)).collect::<Vec<_>>())).into_response()
        }))
        .route("/projects/{pid}/merge_requests/{n}", get(|State(s): State<Shared>, h: HeaderMap, Path((_, n)): Path<(String, i64)>| async move {
            check!(s, h, "get", true);
            let f = s.lock().unwrap();
            match f.prs.iter().find(|p| p.number == n) {
                Some(p) => axum::Json(gl_mr(&f, p)).into_response(),
                None => not_found(),
            }
        }))
        .route("/projects/{pid}/merge_requests/{n}/approvals", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "approvals", true);
            let f = s.lock().unwrap();
            axum::Json(json!({ "approved_by": (0..f.approvals).map(|i| json!({ "user": { "username": format!("reviewer{i}") } })).collect::<Vec<_>>() })).into_response()
        }))
        .route("/projects/{pid}/merge_requests/{n}/notes", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "comments", true);
            let f = s.lock().unwrap();
            let mut list: Vec<Value> = f.comments.iter().enumerate().map(|(i, (a, b))| json!({ "author": { "username": a }, "body": b, "system": false, "created_at": format!("2026-02-01T00:00:{i:02}Z") })).collect();
            list.push(json!({ "author": { "username": "root" }, "body": "assigned to @x", "system": true, "created_at": "2026-01-01T00:00:00Z" }));
            axum::Json(json!(list)).into_response()
        }).post(|State(s): State<Shared>, h: HeaderMap, axum::Json(b): axum::Json<Value>| async move {
            check!(s, h, "comment", false);
            s.lock().unwrap().comments.push(("genie-bot".into(), b["body"].as_str().unwrap_or_default().into()));
            (StatusCode::CREATED, axum::Json(json!({}))).into_response()
        }))
        .route("/projects/{pid}/merge_requests/{n}/merge", put(|State(s): State<Shared>, h: HeaderMap, Path((_, n)): Path<(String, i64)>| async move {
            check!(s, h, "merge", false);
            let mut f = s.lock().unwrap();
            if let Some(m) = f.refuse_merge.take() {
                return (StatusCode::METHOD_NOT_ALLOWED, axum::Json(json!({ "message": m }))).into_response();
            }
            match f.prs.iter_mut().find(|p| p.number == n) {
                Some(p) => {
                    p.state = "merged".into();
                    axum::Json(json!({ "state": "merged" })).into_response()
                }
                None => not_found(),
            }
        }))
        .route("/projects/{pid}/pipelines", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "pipelines", true);
            let f = s.lock().unwrap();
            let status = match f.ci.as_str() {
                "passed" => Some("success"),
                "failed" => Some("failed"),
                "pending" => Some("running"),
                _ => None,
            };
            axum::Json(json!(status.map(|st| vec![json!({ "id": 7, "status": st })]).unwrap_or_default())).into_response()
        }))
        .route("/projects/{pid}/pipelines/{id}/jobs", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "jobs", true);
            axum::Json(json!([{ "id": 41, "name": "test", "web_url": "https://gitlab.example/acme/api/-/jobs/41", "failure_reason": "script_failure" }]))
                .into_response()
        }))
        .route("/projects/{pid}/jobs/{id}/trace", get(|State(s): State<Shared>, h: HeaderMap| async move {
            check!(s, h, "trace", true);
            "\u{1b}[31mrunning cargo test\u{1b}[0m\r\ntest export::csv ... FAILED\nassertion failed: left == right".into_response()
        }))
        .route(
            "/projects/{pid}/pipelines/{id}/retry",
            post(|State(s): State<Shared>, h: HeaderMap, Path((_, _)): Path<(String, i64)>| async move {
                check!(s, h, "rerun", false);
                let mut f = s.lock().unwrap();
                if let Some(m) = f.refuse_rerun.take() {
                    return (StatusCode::FORBIDDEN, axum::Json(json!({ "message": m }))).into_response();
                }
                f.reruns += 1;
                f.ci = "pending".into();
                (StatusCode::CREATED, axum::Json(json!({ "id": 8, "status": "pending" }))).into_response()
            }),
        );
    Router::new().nest("/api/v4", api).with_state(s)
}
