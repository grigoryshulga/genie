//! GitLab (gitlab.com and self-hosted), REST API v4. Merge requests are addressed by their `iid`.

use reqwest::Method;
use serde_json::{Value, json};

use super::{Api, ApiError, ApiResult, ChangeRequest, Ci, Comment, CrState, OpenRequest, RepoInfo, Whoami, enc, s};

/// The project path in a URL: `group%2Fsub%2Frepo`.
fn project(remote: &str) -> String {
    format!("/projects/{}", enc(remote))
}

pub(super) async fn whoami(api: &Api) -> ApiResult<Whoami> {
    let u = api.send(Method::GET, "/user", &[], None).await?.body;
    let version = api.send(Method::GET, "/version", &[], None).await.ok().and_then(|v| v.body["version"].as_str().map(str::to_string));
    Ok(Whoami { login: s(&u, "username"), version })
}

pub(super) async fn repo(api: &Api, remote: &str) -> ApiResult<RepoInfo> {
    let base = project(remote);
    let r = api.send(Method::GET, &base, &[], None).await?.body;
    let level = |k: &str| r["permissions"][k]["access_level"].as_i64();
    let can_push = match (level("project_access"), level("group_access")) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0).max(b.unwrap_or(0)) >= 30),
    };
    let protected = match api.send(Method::GET, &format!("{base}/protected_branches"), &[("per_page", "100".into())], None).await {
        Ok(b) => Some(b.body.as_array().map(|a| a.iter().map(|x| s(x, "name")).collect()).unwrap_or_default()),
        Err(ApiError::Auth(_) | ApiError::NotFound(_)) => None,
        Err(e) => return Err(e),
    };
    Ok(RepoInfo { default_branch: s(&r, "default_branch"), can_push, protected_branches: protected })
}

fn parse(mr: &Value) -> ChangeRequest {
    let state = match mr["state"].as_str() {
        Some("merged") => CrState::Merged,
        Some("opened") => CrState::Open,
        _ => CrState::Closed,
    };
    let title = s(mr, "title");
    let mergeable = match (mr["detailed_merge_status"].as_str(), mr["merge_status"].as_str()) {
        (Some("mergeable"), _) | (None, Some("can_be_merged")) => Some(true),
        (Some("checking" | "unchecked" | "preparing" | "ci_still_running"), _) | (None, Some("checking" | "unchecked")) => None,
        (Some(_), _) | (None, Some(_)) => Some(false),
        (None, None) => None,
    };
    ChangeRequest {
        number: mr["iid"].as_i64().unwrap_or_default(),
        url: s(mr, "web_url"),
        draft: mr["draft"].as_bool().unwrap_or_else(|| title.starts_with("Draft:")),
        title,
        state,
        head: s(mr, "source_branch"),
        base: s(mr, "target_branch"),
        head_sha: mr["sha"].as_str().map(str::to_string),
        merge_sha: mr["merge_commit_sha"].as_str().map(str::to_string),
        mergeable,
        approvals: 0,
        changes_requested: false,
    }
}

pub(super) async fn open(api: &Api, remote: &str, r: &OpenRequest) -> ApiResult<ChangeRequest> {
    let base = project(remote);
    let title = if r.draft && !r.title.starts_with("Draft:") { format!("Draft: {}", r.title) } else { r.title.clone() };
    let body = json!({ "source_branch": r.head, "target_branch": r.base, "title": title, "description": r.body });
    match api.send(Method::POST, &format!("{base}/merge_requests"), &[], Some(body)).await {
        Ok(res) => Ok(parse(&res.body)),
        Err(ApiError::Rejected(m)) if m.contains("already exists") => match find_open(api, remote, &r.head).await? {
            Some(cr) => Ok(cr),
            None => Err(ApiError::Rejected(m)),
        },
        Err(e) => Err(e),
    }
}

async fn find_open(api: &Api, remote: &str, head: &str) -> ApiResult<Option<ChangeRequest>> {
    let base = project(remote);
    let list = api
        .send(Method::GET, &format!("{base}/merge_requests"), &[("state", "opened".into()), ("source_branch", head.to_string())], None)
        .await?
        .body;
    Ok(list.as_array().and_then(|a| a.first()).map(parse))
}

pub(super) async fn get(api: &Api, remote: &str, number: i64) -> ApiResult<ChangeRequest> {
    let base = project(remote);
    let mr = api.send(Method::GET, &format!("{base}/merge_requests/{number}"), &[], None).await?.body;
    let mut cr = parse(&mr);
    // Approvals are not everywhere (older or free editions answer 404/403): then none are shown.
    cr.approvals = match api.send(Method::GET, &format!("{base}/merge_requests/{number}/approvals"), &[], None).await {
        Ok(a) => a.body["approved_by"].as_array().map(|x| x.len() as u32).unwrap_or(0),
        Err(ApiError::NotFound(_) | ApiError::Auth(_)) => 0,
        Err(e) => return Err(e),
    };
    Ok(cr)
}

pub(super) async fn comments(api: &Api, remote: &str, number: i64) -> ApiResult<Vec<Comment>> {
    let base = project(remote);
    let notes = api
        .send(Method::GET, &format!("{base}/merge_requests/{number}/notes"), &[("sort", "asc".into()), ("per_page", "100".into())], None)
        .await?
        .body;
    Ok(notes
        .as_array()
        .into_iter()
        .flatten()
        .filter(|n| n["system"].as_bool() != Some(true))
        .map(|n| Comment { author: s(&n["author"], "username"), body: s(n, "body"), at: s(n, "created_at") })
        .collect())
}

pub(super) async fn comment(api: &Api, remote: &str, number: i64, body: &str) -> ApiResult<()> {
    let base = project(remote);
    api.send(Method::POST, &format!("{base}/merge_requests/{number}/notes"), &[], Some(json!({ "body": body }))).await.map(|_| ())
}

pub(super) async fn merge(api: &Api, remote: &str, number: i64, method: Option<&str>, sha: Option<&str>) -> ApiResult<()> {
    if method == Some("rebase") {
        return Err(ApiError::Unsupported(
            "GitLab merges with `merge` or `squash`; a rebase (fast-forward) merge is a project setting on the host".into(),
        ));
    }
    let base = project(remote);
    let mut body = json!({ "squash": method == Some("squash") });
    if let Some(sha) = sha {
        body["sha"] = json!(sha);
    }
    api.send(Method::PUT, &format!("{base}/merge_requests/{number}/merge"), &[], Some(body)).await.map(|_| ())
}

/// The latest pipeline of one commit.
pub(super) async fn ci(api: &Api, remote: &str, sha: Option<&str>) -> ApiResult<Ci> {
    let Some(sha) = sha else { return Ok(Ci::None) };
    let base = project(remote);
    let list = api
        .send(
            Method::GET,
            &format!("{base}/pipelines"),
            &[("sha", sha.to_string()), ("order_by", "id".into()), ("sort", "desc".into()), ("per_page", "1".into())],
            None,
        )
        .await?
        .body;
    Ok(match list.as_array().and_then(|a| a.first()).and_then(|p| p["status"].as_str()) {
        Some("success" | "success_with_warnings") => Ci::Passed,
        Some("failed" | "canceled" | "canceling") => Ci::Failed,
        Some("running" | "pending" | "created" | "waiting_for_resource" | "preparing" | "scheduled" | "waiting_for_callback") => {
            Ci::Pending
        }
        _ => Ci::None,
    })
}

/// Rerun the failed jobs of one commit's latest pipeline (GitLab's pipeline retry).
pub(super) async fn rerun_failed(api: &Api, remote: &str, sha: Option<&str>) -> ApiResult<u32> {
    let Some(sha) = sha else {
        return Err(ApiError::Unsupported("no commit is watched yet: push the branch first".into()));
    };
    let base = project(remote);
    let list = api
        .send(
            Method::GET,
            &format!("{base}/pipelines"),
            &[("sha", sha.to_string()), ("order_by", "id".into()), ("sort", "desc".into()), ("per_page", "1".into())],
            None,
        )
        .await?
        .body;
    let Some(id) = list.as_array().and_then(|a| a.first()).and_then(|p| p["id"].as_i64()) else {
        return Err(ApiError::Unsupported(format!(
            "no GitLab pipeline of {sha} was found: the failed checks are not a pipeline — rerun them on the host"
        )));
    };
    api.send(Method::POST, &format!("{base}/pipelines/{id}/retry"), &[], None).await?;
    Ok(1)
}

/// The failed jobs of one commit's latest pipeline, each with the end of its log.
pub(super) async fn ci_failures(api: &Api, remote: &str, sha: Option<&str>) -> ApiResult<Vec<super::CiFailure>> {
    let Some(sha) = sha else { return Ok(Vec::new()) };
    let base = project(remote);
    let list = api
        .send(
            Method::GET,
            &format!("{base}/pipelines"),
            &[("sha", sha.to_string()), ("order_by", "id".into()), ("sort", "desc".into()), ("per_page", "1".into())],
            None,
        )
        .await?
        .body;
    let Some(id) = list.as_array().and_then(|a| a.first()).and_then(|p| p["id"].as_i64()) else { return Ok(Vec::new()) };
    let jobs = api.send(Method::GET, &format!("{base}/pipelines/{id}/jobs"), &[("scope", "failed".into())], None).await?.body;
    let mut out = Vec::new();
    for j in jobs.as_array().into_iter().flatten().take(super::MAX_FAILURES) {
        let trace = match j["id"].as_i64() {
            Some(job) => match api.send(Method::GET, &format!("{base}/jobs/{job}/trace"), &[], None).await {
                Ok(r) => r.body.as_str().map(str::to_string).unwrap_or_else(|| r.body.to_string()),
                Err(_) => String::new(),
            },
            None => String::new(),
        };
        let detail = if trace.trim().is_empty() { j["failure_reason"].as_str().unwrap_or_default().to_string() } else { trace };
        out.push(super::CiFailure {
            name: j["name"].as_str().unwrap_or("job").to_string(),
            url: j["web_url"].as_str().map(str::to_string),
            detail: super::tail(&detail),
        });
    }
    Ok(out)
}
