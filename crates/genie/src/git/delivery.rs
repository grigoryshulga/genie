//! A task's delivery: the pull/merge request of its branch, the checks, the merge.
//!
//! The state lives in `task_repos` (one row per task and repository). Agents open and
//! merge requests through the server (`genie pr …`), which checks the effective
//! policy first and calls the host with the host's token. A watcher polls open
//! requests, records merges, closures and CI results, tells the orchestrator (a merge
//! is owner activity there) and the team (a failed check), and merges by itself where
//! the policy says `auto`. Webhooks would replace the polling behind the same functions.

use std::sync::Arc;
use std::time::Duration;

use genie_core::db::now;
use genie_core::events;
use genie_core::repos::{Delivery, ProjectRepo, TaskRepo};
use genie_core::team::SendMail;
use genie_core::tracker::Tracker;
use genie_core::{Actor, CommentKind, Role, Status};
use serde_json::{Value, json};

use super::hosts::Host;
use super::policy::{Effective, Merge, RoleGit, effective};
use super::provider::{Api, ApiError, ChangeRequest, Ci, CiFailure, Comment, CrState, OpenRequest};
use super::service::{self, AgentId};
use super::store;
use crate::state::{App, AppError};

/// Why a delivery operation failed.
#[derive(Debug)]
pub enum DeliveryError {
    /// The policy or the caller's rights forbid it.
    Denied(String),
    /// The request cannot be done in this state.
    Invalid(String),
    NotFound(String),
    /// The host's answer.
    Host(ApiError),
    Internal(String),
}

impl std::fmt::Display for DeliveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeliveryError::Denied(m) | DeliveryError::Invalid(m) | DeliveryError::NotFound(m) | DeliveryError::Internal(m) => {
                write!(f, "{m}")
            }
            DeliveryError::Host(e) => write!(f, "{e}"),
        }
    }
}

impl From<ApiError> for DeliveryError {
    fn from(e: ApiError) -> Self {
        DeliveryError::Host(e)
    }
}

impl From<AppError> for DeliveryError {
    fn from(e: AppError) -> Self {
        match e {
            AppError::Genie(genie_core::GenieError::NotFound(m)) => DeliveryError::NotFound(m),
            AppError::Genie(genie_core::GenieError::Invalid(m) | genie_core::GenieError::Denied(m)) => DeliveryError::Invalid(m),
            other => DeliveryError::Internal(other.to_string()),
        }
    }
}

impl From<genie_core::GenieError> for DeliveryError {
    fn from(e: genie_core::GenieError) -> Self {
        AppError::Genie(e).into()
    }
}

pub type DResult<T> = Result<T, DeliveryError>;

/// The agent making a call (people make them too, with `None`).
#[derive(Debug, Clone)]
pub struct Caller {
    pub name: String,
    pub role: Role,
    pub agent: Option<AgentId>,
}

/// Everything an operation on one repository of one task needs.
struct Target {
    record: ProjectRepo,
    host: Host,
    row: TaskRepo,
    eff: Effective,
}

fn load(app: &App, project: &str, task: &str, repo: &str, caller: &Caller) -> DResult<Target> {
    let record = app.with_server(|db| db.repo(project, repo))?;
    let record = store::resolved(app, &record);
    let host = store::host_of(app, &record).map_err(DeliveryError::Internal)?;
    let Some(row) = app.with_server(|db| db.task_repo(project, task, repo))? else {
        return Err(DeliveryError::NotFound(format!("{task} does not use the repository {repo}: name it first (`genie repos use`)")));
    };
    // People act with the write rights of the task's row; agents with their role's.
    let eff = match &caller.agent {
        Some(a) => {
            let role = service::role_git(app, a);
            effective(&record, Some(task), role, Some(&row.access))
        }
        None => effective(&record, Some(task), RoleGit::Write, Some(&row.access)),
    }
    .map_err(DeliveryError::Invalid)?;
    Ok(Target { record, host, row, eff })
}

fn api(t: &Target) -> DResult<Api> {
    Ok(Api::new(&t.host)?)
}

fn cr_json(row: &TaskRepo, cr: &ChangeRequest, ci: Ci) -> Value {
    json!({ "repo": row.repo, "branch": row.branch, "request": cr, "ci": ci, "delivery": row })
}

/// The task's title and status.
fn task_info(app: &App, project: &str, task: &str) -> DResult<(String, Status)> {
    Ok(app.with_tracker(project, |t| t.get(task).map(|t| (t.title, t.status)))?)
}

/// Blocking work whose failures keep their kind (a refusal by policy stays a refusal).
async fn blocking<T: Send + 'static>(app: &Arc<App>, f: impl FnOnce(&App) -> DResult<T> + Send + 'static) -> DResult<T> {
    app.blocking(move |app| Ok(f(app))).await?
}

pub struct OpenArgs {
    pub title: Option<String>,
    pub body: String,
    pub base: Option<String>,
    pub draft: bool,
}

/// Open the request of the task's branch (or return the one already open).
pub async fn open_request(app: &Arc<App>, project: &str, task: &str, repo: &str, caller: &Caller, args: OpenArgs) -> DResult<Value> {
    let (p, tk, rp, c) = (project.to_string(), task.to_string(), repo.to_string(), caller.clone());
    let (t, head, base, title, body) = blocking(app, move |app| {
        let t = load(app, &p, &tk, &rp, &c)?;
        let base = args.base.clone().filter(|b| !b.trim().is_empty()).unwrap_or_else(|| t.eff.default_branch.clone());
        if base.is_empty() {
            return Err(DeliveryError::Internal(
                "the repository's default branch is not known yet: sync it first (POST /api/repos/<name>/sync)".into(),
            ));
        }
        t.eff.check_open_request(&base).map_err(DeliveryError::Denied)?;
        let head = if t.row.branch.is_empty() { t.eff.task_branch().unwrap_or_default() } else { t.row.branch.clone() };
        // The branch must be on the host, with something in it.
        let mirror = store::refresh(app, &t.host, &t.record.remote, Duration::from_secs(5)).map_err(DeliveryError::Internal)?.path;
        if store::ref_sha(&mirror, &format!("refs/heads/{head}")).is_none() {
            return Err(DeliveryError::Invalid(format!("the branch {head} is not on the git host: `git push origin {head}` first")));
        }
        let ahead =
            store::run(Some(&mirror), &[], &["rev-list", "--count", &format!("refs/heads/{base}..refs/heads/{head}")]).unwrap_or_default();
        if ahead == "0" {
            return Err(DeliveryError::Invalid(format!("{head} has no commits beyond {base}: nothing to deliver")));
        }
        let (task_title, _) = task_info(app, &p, &tk)?;
        let title = args.title.clone().filter(|t| !t.trim().is_empty()).unwrap_or(task_title);
        let title = if title.contains(&tk) { title } else { format!("[{tk}] {title}") };
        let mut body = args.body.trim().to_string();
        if !body.is_empty() {
            body.push_str("\n\n");
        }
        body.push_str(&format!(
            "---\nTask {tk} · opened by {} ({}) through genie{}",
            c.name,
            c.role.as_str(),
            app.cfg.public_url.as_deref().map(|u| format!(" · {}", u.trim_end_matches('/'))).unwrap_or_default()
        ));
        Ok((t, head, base, title, body))
    })
    .await?;
    let api = api(&t)?;
    let cr = api.open(&t.record.remote, &OpenRequest { head: head.clone(), base, title, body, draft: args.draft }).await?;
    let ci = api.ci(&t.record.remote, cr.head_sha.as_deref()).await.unwrap_or(Ci::None);
    // What is on the request already is not news later.
    let seen = api.comments(&t.record.remote, cr.number).await.ok().and_then(|c| c.into_iter().map(|x| x.at).max());
    let (p, tk, rp, c, first) =
        (project.to_string(), task.to_string(), repo.to_string(), caller.clone(), t.row.cr_number != Some(cr.number));
    let cr2 = cr.clone();
    let row = app
        .blocking(move |app| {
            let row = app.with_server(|db| {
                let mut d = Delivery {
                    branch: Some(head.clone()),
                    state: Some("published".into()),
                    cr_number: Some(cr2.number),
                    cr_url: Some(cr2.url.clone()),
                    cr_state: Some(cr2.state.as_str().into()),
                    ci_state: Some(ci.as_str().into()),
                    head_sha: cr2.head_sha.clone(),
                    seen_at: first.then_some(seen).flatten(),
                    ..Default::default()
                };
                // The request's branch is watched from here: a check that fails after it was
                // opened must reach the team, and the review gate reads it.
                if let Some(sha) = cr2.head_sha.clone() {
                    watching(&mut d, &head, &sha);
                    d.ci_since = Some(if ci == Ci::Pending { now() } else { String::new() });
                }
                db.update_delivery(&p, &tk, &rp, d)
            })?;
            if first {
                let actor = Actor::new(c.name.clone(), c.role);
                let payload = json!({ "task": tk, "repo": rp, "number": cr2.number, "url": cr2.url, "by": c.name });
                app.with_tracker(&p, |t| {
                    events::append(t.conn(), events::CR_OPENED, Some(&tk), &actor.name, actor.role.as_str(), payload)?;
                    t.comment(
                        &actor,
                        &tk,
                        &format!(
                            "Opened {} #{} in {rp}: {}",
                            if cr2.url.contains("/pull/") { "pull request" } else { "merge request" },
                            cr2.number,
                            cr2.url
                        ),
                        CommentKind::Note,
                    )?;
                    Ok(())
                })?;
            }
            Ok(row)
        })
        .await?;
    Ok(cr_json(&row, &cr, ci))
}

fn to_app(e: DeliveryError) -> AppError {
    match e {
        DeliveryError::Denied(m) => AppError::Genie(genie_core::GenieError::Denied(m)),
        DeliveryError::Invalid(m) => AppError::Genie(genie_core::GenieError::Invalid(m)),
        DeliveryError::NotFound(m) => AppError::Genie(genie_core::GenieError::NotFound(m)),
        DeliveryError::Internal(m) => AppError::Internal(m),
        DeliveryError::Host(h) => AppError::Internal(h.to_string()),
    }
}

/// The delivery of one repository, brought up to date from the host.
pub async fn show(app: &Arc<App>, project: &str, task: &str, repo: &str, caller: &Caller) -> DResult<Value> {
    let (p, tk, rp, c) = (project.to_string(), task.to_string(), repo.to_string(), caller.clone());
    let t = app.blocking(move |app| load(app, &p, &tk, &rp, &c).map_err(to_app)).await?;
    if t.row.cr_number.is_none() {
        return Ok(json!({ "repo": t.row.repo, "branch": t.row.branch, "request": null, "delivery": t.row }));
    }
    let (cr, ci, row) = sync_row(app, &t.row).await?;
    Ok(cr_json(&row, &cr, ci))
}

pub async fn comments(app: &Arc<App>, project: &str, task: &str, repo: &str, caller: &Caller) -> DResult<Value> {
    let (p, tk, rp, c) = (project.to_string(), task.to_string(), repo.to_string(), caller.clone());
    let t = app.blocking(move |app| load(app, &p, &tk, &rp, &c).map_err(to_app)).await?;
    let number = t.row.cr_number.ok_or_else(|| DeliveryError::Invalid("no request is open for this task yet".into()))?;
    let list = api(&t)?.comments(&t.record.remote, number).await?;
    Ok(json!(list))
}

/// What the reviewer and the caller read of the three rerun limits.
fn rerun_limits(app: &App) -> (u32, u32) {
    (app.cfg.runtime.ci_reruns_per_request, app.cfg.runtime.ci_reruns_per_task)
}

/// The human name of the watched ref of a delivery.
fn watched_name(row: &TaskRepo) -> String {
    if row.ci_ref.is_empty() { row.branch.clone() } else { row.ci_ref.clone() }
}

/// Ask the host to rerun the **failed** checks of the task's watched commit (GitHub Actions runs,
/// a GitLab pipeline), bounded by three limits: one rerun per commit (the owner's rule),
/// `runtime.ciRerunsPerRequest` for this delivery and `runtime.ciRerunsPerTask` over all of the
/// task's repositories. Only a commit whose checks have actually failed can be rerun; on success the
/// watch is re-armed so the restarted run is followed as if it had just been pushed.
///
/// The limit is reserved before the host is called (one transaction, so two concurrent calls cannot
/// both pass) and released when the host refuses, so a broken token does not burn a rerun.
pub async fn rerun(app: &Arc<App>, project: &str, task: &str, repo: &str, caller: &Caller) -> DResult<Value> {
    let (p, tk, rp, c) = (project.to_string(), task.to_string(), repo.to_string(), caller.clone());
    let t = app.blocking(move |app| load(app, &p, &tk, &rp, &c).map_err(to_app)).await?;
    if !t.eff.write {
        return Err(DeliveryError::Denied(format!("{repo}: no access")));
    }
    // The watched commit — the one the review gate and the watcher read. Without one nothing was
    // pushed yet (a row from before the watch may still name the request's head).
    let sha = t
        .row
        .ci_sha
        .clone()
        .or_else(|| t.row.head_sha.clone())
        .ok_or_else(|| DeliveryError::Invalid("no commit is watched yet: push the branch first".into()))?;
    // The host's live answer decides: restarting a running pipeline would be wrong, and a `stalled`
    // watch keeps its own letter (nothing may restart a run nobody is waiting for).
    let api = api(&t)?;
    let ci = api.ci(&t.record.remote, Some(&sha)).await?;
    if ci != Ci::Failed {
        return Err(DeliveryError::Invalid(format!(
            "the checks of `{}` in {repo} are {}, not failed: a rerun is possible only after they fail",
            watched_name(&t.row),
            ci.as_str()
        )));
    }
    // Reserve the limit and count the rerun in one transaction.
    let (per_request, per_task) = rerun_limits(app);
    let (p, tk, rp, sha2) = (project.to_string(), task.to_string(), repo.to_string(), sha.clone());
    let (_reserved, previous_sha, previous_count) = app
        .blocking(move |app| {
            app.with_server(|db| {
                let row =
                    db.task_repo(&p, &tk, &rp)?.ok_or_else(|| genie_core::GenieError::not_found(format!("{tk} has no repository {rp}")))?;
                if per_request == 0 || per_task == 0 {
                    let key = if per_request == 0 { "ciRerunsPerRequest" } else { "ciRerunsPerTask" };
                    return Err(genie_core::GenieError::invalid(format!("reruns are switched off (`runtime.{key}: 0`)")));
                }
                if row.ci_rerun_sha == sha2 {
                    return Err(genie_core::GenieError::invalid(format!(
                        "the checks of `{}` in {rp} were already rerun once: fix and push a new commit instead",
                        watched_name(&row)
                    )));
                }
                if row.ci_reruns as u32 >= per_request {
                    return Err(genie_core::GenieError::invalid(format!(
                        "{rp} already used {} of {per_request} reruns for this request (`runtime.ciRerunsPerRequest`)",
                        row.ci_reruns
                    )));
                }
                let total: i64 = db.task_repos(&p, &tk)?.iter().map(|r| r.ci_reruns).sum();
                if total as u32 >= per_task {
                    return Err(genie_core::GenieError::invalid(format!(
                        "the task used {total} of {per_task} reruns (`runtime.ciRerunsPerTask`)"
                    )));
                }
                let updated = db.update_delivery(
                    &p,
                    &tk,
                    &rp,
                    Delivery { ci_rerun_sha: Some(sha2.clone()), ci_reruns: Some(row.ci_reruns + 1), ..Default::default() },
                )?;
                Ok((updated, row.ci_rerun_sha.clone(), row.ci_reruns))
            })
        })
        .await?;
    // The host is asked to restart the failed runs.
    let runs = match api.rerun_failed(&t.record.remote, Some(&sha)).await {
        Ok(n) => n,
        Err(e) => {
            // The refusal must not eat a rerun: give the reservation back.
            let (p, tk, rp) = (project.to_string(), task.to_string(), repo.to_string());
            let _ = app
                .blocking(move |app| {
                    app.with_server(|db| {
                        db.update_delivery(
                            &p,
                            &tk,
                            &rp,
                            Delivery { ci_rerun_sha: Some(previous_sha), ci_reruns: Some(previous_count), ..Default::default() },
                        )?;
                        Ok(())
                    })
                })
                .await;
            return Err(e.into());
        }
    };
    // Re-arm the watch to `pending` so the watcher follows the restarted run: a `failed` row is not
    // watched, and `ciPendingSecs` starts over rather than making the rerun look stalled at once.
    let (p, tk, rp, sha2, ref2, actor) =
        (project.to_string(), task.to_string(), repo.to_string(), sha.clone(), watched_name(&t.row), (caller.name.clone(), caller.role));
    let row = app
        .blocking(move |app| {
            let row = app.with_server(|db| {
                db.update_delivery(
                    &p,
                    &tk,
                    &rp,
                    Delivery {
                        ci_state: Some("pending".into()),
                        ci_ref: Some(ref2.clone()),
                        ci_sha: Some(sha2.clone()),
                        ci_since: Some(now()),
                        reset_ci: true,
                        ..Default::default()
                    },
                )
            })?;
            let payload = json!({
                "task": tk, "repo": rp, "ref": ref2, "sha": sha2, "runs": runs,
                "reruns": row.ci_reruns, "by": actor.0,
            });
            app.with_tracker(&p, |tr| {
                events::append(tr.conn(), events::CI_RERUN, Some(&tk), &actor.0, actor.1.as_str(), payload)?;
                Ok(())
            })?;
            Ok(row)
        })
        .await?;
    let total: i64 = app.with_server(|db| db.task_repos(project, task))?.iter().map(|r| r.ci_reruns).sum();
    Ok(json!({
        "repo": repo,
        "ref": row.ci_ref,
        "sha": row.ci_sha,
        "runs": runs,
        "request": row.cr_number,
        "used": { "perCommit": 1, "request": row.ci_reruns, "task": total },
        "limits": { "perCommit": 1, "request": per_request, "task": per_task },
    }))
}

pub async fn comment(app: &Arc<App>, project: &str, task: &str, repo: &str, caller: &Caller, text: &str) -> DResult<()> {
    let (p, tk, rp, c) = (project.to_string(), task.to_string(), repo.to_string(), caller.clone());
    let t = app.blocking(move |app| load(app, &p, &tk, &rp, &c).map_err(to_app)).await?;
    if !t.eff.read {
        return Err(DeliveryError::Denied(format!("{repo}: no access")));
    }
    let number = t.row.cr_number.ok_or_else(|| DeliveryError::Invalid("no request is open for this task yet".into()))?;
    let text = format!("{}\n\n— {} ({}) through genie", text.trim(), caller.name, caller.role.as_str());
    api(&t)?.comment(&t.record.remote, number, &text).await?;
    Ok(())
}

/// Merge the task's request, if the policy and the host's conditions allow it.
/// How closely a merge follows the repository's policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strictness {
    /// Agents and the server: everything the policy asks (checks, approvals, no requested changes).
    Agent,
    /// A person from the merge button of an agent's request: the same checks, the person
    /// pressing it counting as one approval.
    Owner,
    /// A person merging as they wish; the host's own rules still apply.
    Free,
}

pub async fn merge(app: &Arc<App>, project: &str, task: &str, repo: &str, caller: &Caller, by_policy: bool) -> DResult<Value> {
    let (p, tk, rp, c) = (project.to_string(), task.to_string(), repo.to_string(), caller.clone());
    let (t, status) = app
        .blocking(move |app| {
            let t = load(app, &p, &tk, &rp, &c).map_err(to_app)?;
            let (_, status) = task_info(app, &p, &tk).map_err(to_app)?;
            Ok((t, status))
        })
        .await?;
    // People merge as they wish, or by the policy when they ask for it; agents (and the
    // server's own auto-merge) always follow the policy.
    let strictness = if caller.agent.is_some() {
        t.eff.check_merge(status == Status::Approved).map_err(DeliveryError::Denied)?;
        Strictness::Agent
    } else if by_policy {
        Strictness::Owner
    } else {
        Strictness::Free
    };
    do_merge(app, project, task, &t, strictness).await
}

async fn do_merge(app: &Arc<App>, project: &str, task: &str, t: &Target, strictness: Strictness) -> DResult<Value> {
    let number = t.row.cr_number.ok_or_else(|| DeliveryError::Invalid("no request is open for this task yet".into()))?;
    let api = api(t)?;
    let cr = api.get(&t.record.remote, number).await?;
    if cr.state != CrState::Open {
        return Err(DeliveryError::Invalid(format!("#{number} is already {}", cr.state.as_str())));
    }
    if cr.draft {
        return Err(DeliveryError::Invalid(format!("#{number} is a draft")));
    }
    let policy = &t.eff.policy.change_request;
    if strictness != Strictness::Free {
        if cr.mergeable == Some(false) {
            return Err(DeliveryError::Invalid(format!(
                "the host says #{number} cannot be merged now (conflicts or unmet rules): fix that, then merge"
            )));
        }
        if cr.changes_requested {
            return Err(DeliveryError::Invalid(format!("a reviewer asked for changes on #{number}")));
        }
        let approvals = cr.approvals + u32::from(strictness == Strictness::Owner);
        if approvals < policy.approvals {
            return Err(DeliveryError::Invalid(format!("#{number} has {} of {} approvals on the host", cr.approvals, policy.approvals)));
        }
        if policy.require_ci {
            match api.ci(&t.record.remote, cr.head_sha.as_deref()).await? {
                Ci::Failed => return Err(DeliveryError::Invalid(format!("the checks of #{number} failed"))),
                Ci::Pending => return Err(DeliveryError::Invalid(format!("the checks of #{number} are still running"))),
                Ci::Passed | Ci::None | Ci::Stalled => {}
            }
        }
    }
    api.merge(&t.record.remote, number, policy.method.as_deref(), cr.head_sha.as_deref()).await?;
    let (cr, ci, row) = sync_row(app, &t.row).await?;
    let _ = (project, task);
    Ok(cr_json(&row, &cr, ci))
}

/// Read the request and its checks from the host, record what changed, and announce it.
pub async fn sync_row(app: &Arc<App>, row: &TaskRepo) -> DResult<(ChangeRequest, Ci, TaskRepo)> {
    let number = row.cr_number.ok_or_else(|| DeliveryError::Invalid("no request yet".into()))?;
    let (p, r) = (row.project.clone(), row.repo.clone());
    let (record, host) = app
        .blocking(move |app| {
            let record = app.with_server(|db| db.repo(&p, &r))?;
            let host = store::host_of(app, &record).map_err(AppError::Internal)?;
            Ok((record, host))
        })
        .await?;
    let api = Api::new(&host)?;
    let cr = api.get(&record.remote, number).await?;
    let ci = if cr.state == CrState::Open {
        api.ci(&record.remote, cr.head_sha.as_deref()).await?
    } else {
        row.ci_state.as_deref().map(parse_ci).unwrap_or(Ci::None)
    };
    // What people wrote on an open request (a failure to read it must not hide the rest).
    let comments = if cr.state == CrState::Open { api.comments(&record.remote, number).await.unwrap_or_default() } else { Vec::new() };
    // A check that just failed: which one and why, for the team (a host that says nothing leaves it empty).
    let failures = if ci == Ci::Failed && row.ci_state.as_deref() != Some("failed") {
        api.ci_failures(&record.remote, cr.head_sha.as_deref()).await.unwrap_or_default()
    } else {
        Vec::new()
    };
    let (row0, cr2) = (row.clone(), cr.clone());
    let row = app.blocking(move |app| record_changes(app, &row0, &cr2, ci, &comments, &failures)).await?;
    Ok((cr, ci, row))
}

fn parse_ci(s: &str) -> Ci {
    match s {
        "pending" => Ci::Pending,
        "passed" => Ci::Passed,
        "failed" => Ci::Failed,
        "stalled" => Ci::Stalled,
        _ => Ci::None,
    }
}

/// Arm the watch of one ref on a delivery: its checks are looked at from now on. The waiting clock
/// starts with the first look that finds them running.
pub(crate) fn watching(d: &mut Delivery, r#ref: &str, sha: &str) {
    d.ci_ref = Some(r#ref.to_string());
    d.ci_sha = Some(sha.to_string());
    d.ci_since = Some(String::new());
    d.reset_ci = true;
}

/// Stop watching the ref of a delivery (nothing to look at, or nothing left to watch: the checks
/// are recorded once more, then the row leaves the watch set).
fn stop_watching(ci_state: Option<&str>) -> Delivery {
    Delivery { ci_state: ci_state.map(str::to_string), reset_ci: true, ..Default::default() }
}

/// Store the host's state and, for what changed, the event, the note on the task and the message to
/// whoever acts on it. The checks are recorded by [`record_ci`] — of the request's branch while the
/// request is open, of the target branch once it was merged.
fn record_changes(
    app: &App,
    row: &TaskRepo,
    cr: &ChangeRequest,
    ci: Ci,
    comments: &[Comment],
    failures: &[CiFailure],
) -> Result<TaskRepo, AppError> {
    let (p, task, repo) = (row.project.as_str(), row.task.as_str(), row.repo.as_str());
    let host_actor = Actor::new("git-host", Role::Human);
    let state_now = cr.state.as_str();
    let state_changed = row.cr_state.as_deref() != Some(state_now);
    let merged_now = state_changed && cr.state == CrState::Merged;
    let abandoned_now = state_changed && cr.state == CrState::Closed;
    let delivery = match cr.state {
        CrState::Merged => Some("merged"),
        CrState::Closed => Some("abandoned"),
        CrState::Open => None,
    };
    // Comments of people that are new since the last look (ours carry a signature and are not echoed).
    let fresh: Vec<&Comment> =
        comments.iter().filter(|c| c.at > row.seen_at && !c.body.trim().is_empty() && !c.body.contains(" through genie")).collect();
    let seen_at = fresh.iter().map(|c| c.at.clone()).max();
    let mut d = Delivery {
        state: delivery.map(str::to_string),
        cr_state: Some(state_now.into()),
        head_sha: cr.head_sha.clone(),
        seen_at,
        ..Default::default()
    };
    // A request that ended watches something else: a merge hands the watch to the commit that landed
    // on the target branch (a person may have merged past the checks), a closure watches nothing.
    if merged_now || abandoned_now {
        d.reset_ci = true;
        if merged_now && let Some(sha) = cr.merge_sha.clone() {
            watching(&mut d, &cr.base, &sha);
        }
    } else if cr.state == CrState::Open
        && let Some(sha) = cr.head_sha.clone()
        && row.ci_sha.as_deref() != Some(sha.as_str())
    {
        // While the request is open, the checks that matter are those of its head: either the row was
        // never armed (it predates the watch, or its host named no commit when the request was opened)
        // or the branch moved under us (a push that did not go through the proxy). The recorded state
        // stays — it is what the host said last — and the clock starts again for the new commit.
        d.ci_ref = Some(cr.head.clone());
        d.ci_sha = Some(sha);
        if row.ci_sha.is_some() {
            d.ci_since = Some(String::new());
        }
    }
    let updated = app.with_server(|db| db.update_delivery(p, task, repo, d))?;
    if !fresh.is_empty() {
        let text: String = fresh
            .iter()
            .map(|c| format!("[{} on #{} in {repo}] {}", c.author, cr.number, c.body.trim().chars().take(1500).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n\n");
        app.with_tracker(p, |t| {
            // A person's review on the host becomes a `review` comment of the task (it wakes the orchestrator too) and reaches the team.
            t.comment(&host_actor, task, &text, CommentKind::Review)?;
            if let Some(team) = active_team(t, task) {
                let mail = format!("Comments on the request #{} in {repo} ({}):\n\n{text}", cr.number, cr.url);
                let _ = t.bus().send(SendMail {
                    team: &team,
                    from: "git-host",
                    from_role: "system",
                    to: "all",
                    text: &mail,
                    level: Some("normal"),
                    intent: Some("question"),
                    kind: "message",
                    ..Default::default()
                });
            }
            Ok(())
        })?;
    }
    if state_changed {
        let payload = json!({ "task": task, "repo": repo, "number": cr.number, "url": cr.url, "ci": ci });
        app.with_tracker(p, |t| {
            if merged_now {
                events::append(t.conn(), events::CR_MERGED, Some(task), "git-host", "human", payload.clone())?;
                // A merge is news for the orchestrator (owner activity wakes it).
                t.comment(&host_actor, task, &format!("The request #{} in {repo} was merged: {}", cr.number, cr.url), CommentKind::Note)?;
            }
            if abandoned_now {
                events::append(t.conn(), events::CR_CLOSED, Some(task), "git-host", "human", payload.clone())?;
                t.comment(
                    &host_actor,
                    task,
                    &format!("The request #{} in {repo} was closed without merging: {}", cr.number, cr.url),
                    CommentKind::Note,
                )?;
            }
            Ok(())
        })?;
    }
    // The checks of a request that ended were said above; a merge starts a new watch next look.
    if merged_now || abandoned_now {
        return Ok(updated);
    }
    record_ci(app, &updated, ci, failures, Some(cr))
}

/// The team of a task when that team is active: a letter means something only then.
fn active_team(t: &Tracker, task: &str) -> Option<String> {
    let team = t.get(task).ok()?.team?;
    t.bus().get(&team).ok().filter(|x| x.state == "active").map(|_| team)
}

/// What to store for a look: the host's answer, unless the checks have stayed `pending` for longer
/// than `runtime.ciPendingSecs` — then `stalled` once (terminal for the watcher).
fn settle(app: &App, row: &TaskRepo, ci: Ci) -> Ci {
    // `stalled` stands until the host says something else: a later look that still sees `pending`
    // neither repeats the letter nor pretends the checks are running.
    if row.ci_state.as_deref() == Some("stalled") && ci == Ci::Pending {
        return Ci::Stalled;
    }
    let limit = app.cfg.runtime.ci_pending_secs;
    if ci != Ci::Pending || limit == 0 {
        return ci;
    }
    let Ok(since) = chrono::DateTime::parse_from_rfc3339(&row.ci_since) else { return ci };
    if chrono::Utc::now().signed_duration_since(since.with_timezone(&chrono::Utc)).num_seconds() >= limit as i64 { Ci::Stalled } else { ci }
}

/// The checks of one watched ref: store them and, for what changed, the event, the letter or the note.
/// `cr` names the request whose branch the checks belong to; without it the watched ref is named.
fn record_ci(app: &App, row: &TaskRepo, ci: Ci, failures: &[CiFailure], cr: Option<&ChangeRequest>) -> Result<TaskRepo, AppError> {
    let (p, task, repo) = (row.project.as_str(), row.task.as_str(), row.repo.as_str());
    let ci = settle(app, row, ci);
    let changed = row.ci_state.as_deref() != Some(ci.as_str());
    // A second look that still finds no checks ends the watch: a repository without CI, or one whose
    // checks never start, must not be polled for the life of the row.
    let none_settled = ci == Ci::None && row.ci_state.as_deref() == Some("none");
    // Waiting begins with the first look that finds the checks running (a `none` in between does not
    // start the clock) and ends when they settle.
    let began = ci == Ci::Pending && (row.ci_since.is_empty() || row.ci_state.as_deref() != Some("pending"));
    if !changed && !began && !none_settled {
        return Ok(row.clone());
    }
    let (ci_since, reset) = match ci {
        Ci::Pending if began => (Some(now()), false),
        Ci::Pending => (None, false),
        // `stalled` keeps the moment it began: the record is what the team was told about.
        Ci::Stalled => (None, false),
        // The first `none` gets one more look (checks may start a moment after the push), the second
        // one stops the watch.
        Ci::None if none_settled => (Some(String::new()), true),
        Ci::None => (Some(now()), false),
        Ci::Passed | Ci::Failed => (Some(String::new()), false),
    };
    let updated = app.with_server(|db| {
        db.update_delivery(p, task, repo, Delivery { ci_state: Some(ci.as_str().into()), ci_since, reset_ci: reset, ..Default::default() })
    })?;
    if !changed {
        return Ok(updated);
    }
    let payload = json!({ "task": task, "repo": repo, "ref": row.ci_ref, "sha": row.ci_sha, "number": cr.map(|c| c.number), "ci": ci });
    app.with_tracker(p, |t| {
        match ci {
            Ci::Failed => {
                let mut payload = payload.clone();
                payload["failures"] = json!(failures);
                events::append(t.conn(), events::CI_FAILED, Some(task), "git-host", "human", payload)?;
                if let Some(team) = active_team(t, task) {
                    let mut text = match cr {
                        Some(cr) => format!(
                            "The checks of the request #{} in {repo} failed: {}. Open it, find out why, fix and push.",
                            cr.number, cr.url
                        ),
                        None => format!("The checks of the branch `{}` in {repo} failed: fix them and push, then carry on.", row.ci_ref),
                    };
                    for f in failures {
                        text.push_str(&format!("\n\n- {}{}", f.name, f.url.as_deref().map(|u| format!(" ({u})")).unwrap_or_default()));
                        if !f.detail.is_empty() {
                            text.push_str(&format!(":\n{}", f.detail));
                        }
                    }
                    let _ = t.bus().send(SendMail {
                        team: &team,
                        from: "git-host",
                        from_role: "system",
                        to: "all",
                        text: &text,
                        level: Some("high"),
                        intent: Some("blocker"),
                        kind: "message",
                        ..Default::default()
                    });
                }
            }
            Ci::Passed => {
                events::append(t.conn(), events::CI_PASSED, Some(task), "git-host", "human", payload)?;
            }
            Ci::Stalled => {
                events::append(t.conn(), events::CI_STALLED, Some(task), "git-host", "human", payload)?;
                let subject = match cr {
                    Some(cr) => format!("the request #{} in {repo} ({})", cr.number, cr.url),
                    None => format!("the branch `{}` in {repo}", row.ci_ref),
                };
                // How long they have been stuck, in the unit a person reads best.
                let secs = app.cfg.runtime.ci_pending_secs;
                let waited = if secs < 120 { format!("{secs} s") } else { format!("{} min", secs / 60) };
                let text = format!(
                    "The checks of {subject} have been running for more than {waited} and nothing has come of them: \
                     settle them on the host by hand — `genie pr rerun` restarts failed checks, not one that is still running."
                );
                match active_team(t, task) {
                    Some(team) => {
                        let _ = t.bus().send(SendMail {
                            team: &team,
                            from: "git-host",
                            from_role: "system",
                            to: "all",
                            text: &text,
                            level: Some("normal"),
                            intent: Some("question"),
                            kind: "message",
                            ..Default::default()
                        });
                    }
                    None => {
                        let _ = t.bus().notify_orchestrator("git-host", "system", "system", &text, Some(task));
                    }
                }
            }
            Ci::Pending | Ci::None => {}
        }
        Ok(())
    })?;
    Ok(updated)
}

// --- the gates of the workflow ---------------------------------------------------------------

/// A status change an agent asks for, checked against the task's delivery. `review` needs a
/// request for every pushed branch whose policy calls for one and needs the watched checks not to
/// have failed; `done` needs none left open. (People are not held to it: their moves are authoritative.)
pub async fn gate(app: &Arc<App>, project: &str, task: &str, to: Status) -> Result<(), String> {
    if !matches!(to, Status::Review | Status::Done) {
        return Ok(());
    }
    let task = app.with_tracker(project, |t| t.normalize_id(task)).map_err(|e| e.to_string())?;
    let rows = app.with_server(|db| db.task_repos(project, &task)).map_err(|e| e.to_string())?;
    for row in rows.iter().filter(|r| r.access == "write") {
        if to == Status::Done {
            if row.cr_state.as_deref() == Some("open") {
                return Err(format!(
                    "the request #{} of {1} is not merged yet: it is merged by a person (move the task to needs_owner with `--action ask-for-merge-pr --repo {1}`) or by you (`genie pr merge --repo {1}`) when the policy allows",
                    row.cr_number.unwrap_or_default(),
                    row.repo,
                ));
            }
            continue;
        }
        // `review`: a pushed branch whose policy calls for a request needs one.
        if row.state == "published" && row.cr_number.is_none() {
            let wants_request = app
                .with_server(|db| db.repo(project, &row.repo))
                .ok()
                .and_then(|r| effective(&store::resolved(app, &r), Some(&task), RoleGit::Write, Some("write")).ok())
                .is_some_and(|e| e.policy.push == super::policy::Push::PrOnly && e.policy.change_request.open);
            if wants_request {
                return Err(format!(
                    "the branch {} of {} is pushed but has no pull/merge request: `genie pr open --repo {}`, then move the task to review",
                    row.branch, row.repo, row.repo
                ));
            }
        }
        // A failed check is in the way: the agent fixes and pushes, and a push restarts the watch.
        // `pending` deliberately does not block (nothing wakes a session when checks turn green),
        // and `stalled` is not a failure.
        if let Some(reason) = failed_checks(app, project, row).await {
            return Err(reason);
        }
    }
    Ok(())
}

/// The checks in the way of a review, if any: the host is asked live about the watched commit — or,
/// for a delivery that was never armed, about the request's head — and the recorded state decides
/// when there is no commit to ask about and when the host cannot be reached: an outage must not block
/// work, and a delivery that predates the watch must not slip past a red CI either.
async fn failed_checks(app: &Arc<App>, project: &str, row: &TaskRepo) -> Option<String> {
    let stored = || row.ci_state.as_deref().map(parse_ci).unwrap_or(Ci::None);
    let ci = match row.ci_sha.as_deref().or(row.head_sha.as_deref()) {
        Some(sha) => match repo_and_host(app, project, &row.repo).await {
            Ok((record, host)) => match Api::new(&host) {
                Ok(api) => api.ci(&record.remote, Some(sha)).await.unwrap_or_else(|_| stored()),
                Err(_) => stored(),
            },
            Err(_) => stored(),
        },
        None => stored(),
    };
    (ci == Ci::Failed).then(|| {
        let what = if row.ci_ref.is_empty() { row.branch.clone() } else { row.ci_ref.clone() };
        format!("the checks of `{what}` in {} failed: fix them and push, then move the task to review", row.repo)
    })
}

/// A project repository and the host it lives on.
async fn repo_and_host(app: &Arc<App>, project: &str, repo: &str) -> Result<(ProjectRepo, Host), String> {
    let (p, r) = (project.to_string(), repo.to_string());
    app.blocking(move |app| {
        let record = app.with_server(|db| db.repo(&p, &r))?;
        let host = store::host_of(app, &record).map_err(AppError::Internal)?;
        Ok((store::resolved(app, &record), host))
    })
    .await
    .map_err(|e| e.to_string())
}

// --- the watcher --------------------------------------------------------------------------------

/// Watch the delivery of every task — open requests and the branches of policies that have none —
/// until the server stops. Each row is looked at every `poll_secs` of its host.
pub fn spawn_poller(app: Arc<App>) {
    tokio::spawn(async move {
        let mut last: std::collections::HashMap<String, std::time::Instant> = Default::default();
        let mut complained: std::collections::HashMap<String, std::time::Instant> = Default::default();
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let rows = match app.blocking(|app| app.with_server(|db| db.watched_deliveries())).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            for row in rows {
                let key = format!("{}/{}/{}", row.project, row.task, row.repo);
                let every = app
                    .blocking({
                        let (p, r) = (row.project.clone(), row.repo.clone());
                        move |app| {
                            let rec = app.with_server(|db| db.repo(&p, &r))?;
                            Ok(store::host_of(app, &rec).map(|h| h.poll_secs).unwrap_or(60))
                        }
                    })
                    .await
                    .unwrap_or(60);
                if last.get(&key).is_some_and(|t| t.elapsed() < Duration::from_secs(every)) {
                    continue;
                }
                last.insert(key.clone(), std::time::Instant::now());
                if let Err(e) = watch_one(&app, &row).await
                    && complained.get(&key).is_none_or(|t| t.elapsed() > Duration::from_secs(600))
                {
                    complained.insert(key.clone(), std::time::Instant::now());
                    eprintln!("genie git: {key}: {e}");
                }
            }
        }
    });
}

/// One look at one watched delivery: an open request (its state, its checks, and a merge when the
/// policy says `auto`) or a branch watched on its own (the checks of its commit).
pub async fn watch_one(app: &Arc<App>, row: &TaskRepo) -> DResult<()> {
    if row.cr_state.as_deref() != Some("open") {
        return sync_ref_ci(app, row).await;
    }
    let (cr, ci, row) = sync_row(app, row).await?;
    if cr.state != CrState::Open {
        crate::http::tasks::changed(app);
        return Ok(());
    }
    let (p, tk, rp) = (row.project.clone(), row.task.clone(), row.repo.clone());
    let auto = app
        .blocking(move |app| {
            let caller = Caller { name: "genie".into(), role: Role::Orchestrator, agent: None };
            let t = load(app, &p, &tk, &rp, &caller).map_err(to_app)?;
            let (_, status) = task_info(app, &p, &tk).map_err(to_app)?;
            Ok((t.eff.policy.change_request.merge == Merge::Auto && status == Status::Approved).then_some(t))
        })
        .await?;
    if let Some(t) = auto
        && ci != Ci::Failed
    {
        match do_merge(app, &row.project, &row.task, &t, Strictness::Agent).await {
            Ok(_) => crate::http::tasks::changed(app),
            // Not yet mergeable (approvals, checks still running): try again next time.
            Err(DeliveryError::Invalid(_)) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// One look at a watched ref of a task whose delivery has no open request: the branch of a policy
/// with no requests, or the target branch a request was merged into. Nothing else is read.
pub async fn sync_ref_ci(app: &Arc<App>, row: &TaskRepo) -> DResult<()> {
    let Some(sha) = row.ci_sha.clone() else { return Ok(()) };
    let (record, host) = repo_and_host(app, &row.project, &row.repo).await.map_err(DeliveryError::Internal)?;
    let api = match Api::new(&host) {
        Ok(api) => api,
        // A plain git server has no API to ask: stop watching instead of asking forever.
        Err(ApiError::Unsupported(_)) => {
            let (p, task, repo) = (row.project.clone(), row.task.clone(), row.repo.clone());
            app.blocking(move |app| app.with_server(|db| db.update_delivery(&p, &task, &repo, stop_watching(None))).map(|_| ())).await?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };
    let ci = api.ci(&record.remote, Some(&sha)).await?;
    let failures = if ci == Ci::Failed && row.ci_state.as_deref() != Some("failed") {
        api.ci_failures(&record.remote, Some(&sha)).await.unwrap_or_default()
    } else {
        Vec::new()
    };
    let row0 = row.clone();
    app.blocking(move |app| Ok(record_ci(app, &row0, ci, &failures, None))).await??;
    crate::http::tasks::changed(app);
    Ok(())
}
