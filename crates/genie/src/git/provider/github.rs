//! GitHub (github.com and GitHub Enterprise Server), REST API v3.

use reqwest::Method;
use serde_json::{Value, json};

use super::{Api, ApiError, ApiResult, ChangeRequest, Ci, Comment, CrState, OpenRequest, RepoInfo, Whoami, enc, s};

/// `owner/repo`: GitHub has no nested groups.
fn slug(remote: &str) -> ApiResult<(String, String)> {
    let mut it = remote.split('/');
    match (it.next(), it.next(), it.next()) {
        (Some(o), Some(r), None) if !o.is_empty() && !r.is_empty() => Ok((enc(o), enc(r))),
        _ => Err(ApiError::Rejected(format!("{remote}: a GitHub repository is owner/repo"))),
    }
}

fn repo_path(remote: &str) -> ApiResult<String> {
    let (o, r) = slug(remote)?;
    Ok(format!("/repos/{o}/{r}"))
}

pub(super) async fn whoami(api: &Api) -> ApiResult<Whoami> {
    let u = api.send(Method::GET, "/user", &[], None).await?.body;
    Ok(Whoami { login: s(&u, "login"), version: None })
}

pub(super) async fn repo(api: &Api, remote: &str) -> ApiResult<RepoInfo> {
    let base = repo_path(remote)?;
    let r = api.send(Method::GET, &base, &[], None).await?.body;
    let protected =
        match api.send(Method::GET, &format!("{base}/branches"), &[("protected", "true".into()), ("per_page", "100".into())], None).await {
            Ok(b) => Some(b.body.as_array().map(|a| a.iter().map(|x| s(x, "name")).collect()).unwrap_or_default()),
            Err(ApiError::Auth(_) | ApiError::NotFound(_)) => None,
            Err(e) => return Err(e),
        };
    Ok(RepoInfo { default_branch: s(&r, "default_branch"), can_push: r["permissions"]["push"].as_bool(), protected_branches: protected })
}

fn parse(pr: &Value) -> ChangeRequest {
    let merged = pr["merged"].as_bool().unwrap_or(false) || !pr["merged_at"].is_null();
    let state = if merged {
        CrState::Merged
    } else if pr["state"] == "open" {
        CrState::Open
    } else {
        CrState::Closed
    };
    ChangeRequest {
        number: pr["number"].as_i64().unwrap_or_default(),
        url: s(pr, "html_url"),
        title: s(pr, "title"),
        state,
        draft: pr["draft"].as_bool().unwrap_or(false),
        head: s(&pr["head"], "ref"),
        base: s(&pr["base"], "ref"),
        head_sha: pr["head"]["sha"].as_str().map(str::to_string),
        merge_sha: pr["merge_commit_sha"].as_str().map(str::to_string),
        mergeable: pr["mergeable"].as_bool(),
        approvals: 0,
        changes_requested: false,
    }
}

/// The latest review of each reviewer decides: how many approve, whether anyone asks for changes.
async fn reviews(api: &Api, base: &str, number: i64) -> ApiResult<(u32, bool, Vec<Value>)> {
    let list = api.send(Method::GET, &format!("{base}/pulls/{number}/reviews"), &[("per_page", "100".into())], None).await?.body;
    let list = list.as_array().cloned().unwrap_or_default();
    let mut latest: std::collections::BTreeMap<String, String> = Default::default();
    for r in &list {
        let state = s(r, "state");
        if matches!(state.as_str(), "APPROVED" | "CHANGES_REQUESTED" | "DISMISSED") {
            latest.insert(s(&r["user"], "login"), state);
        }
    }
    let approvals = latest.values().filter(|v| *v == "APPROVED").count() as u32;
    let changes = latest.values().any(|v| v == "CHANGES_REQUESTED");
    Ok((approvals, changes, list))
}

pub(super) async fn open(api: &Api, remote: &str, r: &OpenRequest) -> ApiResult<ChangeRequest> {
    let base = repo_path(remote)?;
    let body = json!({ "title": r.title, "head": r.head, "base": r.base, "body": r.body, "draft": r.draft });
    match api.send(Method::POST, &format!("{base}/pulls"), &[], Some(body)).await {
        Ok(res) => Ok(parse(&res.body)),
        Err(ApiError::Rejected(m)) if m.contains("already exists") => match find_open(api, remote, &r.head).await? {
            Some(cr) => Ok(cr),
            None => Err(ApiError::Rejected(m)),
        },
        Err(e) => Err(e),
    }
}

async fn find_open(api: &Api, remote: &str, head: &str) -> ApiResult<Option<ChangeRequest>> {
    let base = repo_path(remote)?;
    let owner = remote.split('/').next().unwrap_or_default();
    let list =
        api.send(Method::GET, &format!("{base}/pulls"), &[("state", "open".into()), ("head", format!("{owner}:{head}"))], None).await?.body;
    Ok(list.as_array().and_then(|a| a.first()).map(parse))
}

pub(super) async fn get(api: &Api, remote: &str, number: i64) -> ApiResult<ChangeRequest> {
    let base = repo_path(remote)?;
    let pr = api.send(Method::GET, &format!("{base}/pulls/{number}"), &[], None).await?.body;
    let mut cr = parse(&pr);
    let (approvals, changes, _) = reviews(api, &base, number).await?;
    cr.approvals = approvals;
    cr.changes_requested = changes;
    Ok(cr)
}

pub(super) async fn comments(api: &Api, remote: &str, number: i64) -> ApiResult<Vec<Comment>> {
    let base = repo_path(remote)?;
    let mut out: Vec<Comment> = Vec::new();
    let page = [("per_page", "100".to_string())];
    let issue = api.send(Method::GET, &format!("{base}/issues/{number}/comments"), &page, None).await?.body;
    let inline = api.send(Method::GET, &format!("{base}/pulls/{number}/comments"), &page, None).await?.body;
    let (_, _, reviews) = reviews(api, &base, number).await?;
    for c in issue.as_array().into_iter().flatten().chain(inline.as_array().into_iter().flatten()) {
        out.push(Comment { author: s(&c["user"], "login"), body: s(c, "body"), at: s(c, "created_at") });
    }
    for r in &reviews {
        let body = s(r, "body");
        if !body.trim().is_empty() {
            out.push(Comment {
                author: s(&r["user"], "login"),
                body: format!("[{}] {body}", s(r, "state").to_lowercase()),
                at: s(r, "submitted_at"),
            });
        }
    }
    out.sort_by(|a, b| a.at.cmp(&b.at));
    Ok(out)
}

pub(super) async fn comment(api: &Api, remote: &str, number: i64, body: &str) -> ApiResult<()> {
    let base = repo_path(remote)?;
    api.send(Method::POST, &format!("{base}/issues/{number}/comments"), &[], Some(json!({ "body": body }))).await.map(|_| ())
}

pub(super) async fn merge(api: &Api, remote: &str, number: i64, method: Option<&str>, sha: Option<&str>) -> ApiResult<()> {
    let base = repo_path(remote)?;
    let mut body = json!({});
    if let Some(m) = method {
        body["merge_method"] = json!(m);
    }
    if let Some(sha) = sha {
        body["sha"] = json!(sha);
    }
    let res = api.send(Method::PUT, &format!("{base}/pulls/{number}/merge"), &[], Some(body)).await?.body;
    if res["merged"].as_bool() == Some(false) {
        return Err(ApiError::Rejected(s(&res, "message")));
    }
    Ok(())
}

/// Commit statuses and check runs together: any failure fails, anything unfinished is pending.
pub(super) async fn ci(api: &Api, remote: &str, sha: Option<&str>) -> ApiResult<Ci> {
    let Some(sha) = sha else { return Ok(Ci::None) };
    let base = repo_path(remote)?;
    let status = api.send(Method::GET, &format!("{base}/commits/{sha}/status"), &[], None).await?.body;
    let runs = api.send(Method::GET, &format!("{base}/commits/{sha}/check-runs"), &[("per_page", "100".into())], None).await?.body;
    let (mut failed, mut pending, mut passed) = (false, false, false);
    if status["total_count"].as_i64().unwrap_or(0) > 0 {
        match status["state"].as_str() {
            Some("failure" | "error") => failed = true,
            Some("pending") => pending = true,
            Some("success") => passed = true,
            _ => {}
        }
    }
    for r in runs["check_runs"].as_array().into_iter().flatten() {
        if r["status"] != "completed" {
            pending = true;
            continue;
        }
        match r["conclusion"].as_str() {
            Some("failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure") => failed = true,
            Some("success" | "neutral" | "skipped") => passed = true,
            _ => {}
        }
    }
    Ok(match (failed, pending, passed) {
        (true, _, _) => Ci::Failed,
        (_, true, _) => Ci::Pending,
        (_, _, true) => Ci::Passed,
        _ => Ci::None,
    })
}

/// The failed check runs (with the host's title and summary) and failed commit statuses.
pub(super) async fn ci_failures(api: &Api, remote: &str, sha: Option<&str>) -> ApiResult<Vec<super::CiFailure>> {
    let Some(sha) = sha else { return Ok(Vec::new()) };
    let base = repo_path(remote)?;
    let mut out = Vec::new();
    let runs = api.send(Method::GET, &format!("{base}/commits/{sha}/check-runs"), &[("per_page", "100".into())], None).await?.body;
    for r in runs["check_runs"].as_array().into_iter().flatten() {
        if !matches!(r["conclusion"].as_str(), Some("failure" | "timed_out" | "cancelled" | "action_required" | "startup_failure")) {
            continue;
        }
        let detail = [r["output"]["title"].as_str(), r["output"]["summary"].as_str(), r["output"]["text"].as_str()]
            .into_iter()
            .flatten()
            .filter(|t| !t.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        out.push(super::CiFailure {
            name: r["name"].as_str().unwrap_or("check").to_string(),
            url: r["html_url"].as_str().map(str::to_string),
            detail: super::tail(&detail),
        });
    }
    let status = api.send(Method::GET, &format!("{base}/commits/{sha}/status"), &[], None).await?.body;
    for st in status["statuses"].as_array().into_iter().flatten() {
        if matches!(st["state"].as_str(), Some("failure" | "error")) {
            out.push(super::CiFailure {
                name: st["context"].as_str().unwrap_or("status").to_string(),
                url: st["target_url"].as_str().map(str::to_string),
                detail: super::tail(st["description"].as_str().unwrap_or_default()),
            });
        }
    }
    out.truncate(super::MAX_FAILURES);
    Ok(out)
}
