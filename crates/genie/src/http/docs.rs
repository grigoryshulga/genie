//! Knowledge routes: reads, edits in changesets, publication, merges, history and the
//! changelog under `/api/docs`, serving the SPA's Docs page from the vault.
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use genie_core::changesets::{ChangesetFilter, ChangesetStatus};
use genie_core::doc_ops::DocOp;
use genie_core::vault::{AuthorKind, SearchOptions, Space};
use genie_core::{Capability, GenieError};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ctx::{Access, Ctx};
use super::{ApiError, ApiResult};
use crate::changesets::{self, Editor, Published};
use crate::knowledge::{self, Author, DocWrite};
use crate::state::{App, AppError, AppResult};

fn docs_error(error: AppError) -> ApiError {
    match error {
        AppError::Genie(GenieError::NotFound(message)) => ApiError::new(StatusCode::NOT_FOUND, message),
        AppError::Genie(GenieError::Denied(message)) | AppError::Forbidden(message) => ApiError::new(StatusCode::FORBIDDEN, message),
        AppError::Conflict(message) => ApiError::new(StatusCode::CONFLICT, message),
        AppError::Genie(GenieError::Invalid(message)) | AppError::Bad(message) => invalid(message),
        other => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
}

/// Invalid input is a 400; a page whose frontmatter breaks the documentation contract is a 422
/// carrying the diagnostics, as the direct write reported them.
fn invalid(message: String) -> ApiError {
    const CONTRACT: &str = "invalid documentation metadata: ";
    if let Some(list) = message.strip_prefix(CONTRACT) {
        let diagnostics: Vec<String> = list.split("; ").map(str::to_string).collect();
        return ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, message).with_details(json!({ "diagnostics": diagnostics }));
    }
    // The changeset service reports a changeset or merge in the wrong state as invalid input; the
    // contract calls that a conflict.
    if wrong_state(&message) {
        return ApiError::new(StatusCode::CONFLICT, message);
    }
    ApiError::bad(message)
}

fn wrong_state(message: &str) -> bool {
    const STATES: &[&str] = &[
        "is published",
        "is discarded",
        "is merging",
        "is resolved",
        "is already",
        "only an open",
        "only a published",
        "is not open",
        "still open",
        "wrong state",
    ];
    (message.starts_with("changeset ") || message.starts_with("merge ")) && STATES.iter().any(|s| message.contains(s))
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/docs/version", get(version))
        .route("/docs/tree", get(tree))
        .route("/docs/required", get(required))
        .route("/docs/search", get(search))
        .route("/docs/page", get(read).post(write).delete(remove_page))
        .route("/docs/edit", post(edit))
        .route("/docs/pages/move", post(move_page))
        .route("/docs/changesets", get(feed))
        .route("/docs/changesets/{id}", get(detail))
        .route("/docs/changesets/{id}/publish", post(publish))
        .route("/docs/changesets/{id}/discard", post(discard))
        .route("/docs/changesets/{id}/review", post(review))
        .route("/docs/changesets/{id}/revert", post(revert))
        .route("/docs/history", get(history))
        .route("/docs/diff", get(diff))
        .route("/docs/restore", post(restore))
        .route("/docs/merges/{id}", get(merge_show))
        .route("/docs/merges/{id}/resolve", post(merge_resolve))
        .route("/docs/note", post(note))
        .route("/docs/section", delete(remove_section))
        .route("/docs/spaces", get(spaces))
        .route("/docs/spaces/{name}", put(set_space))
        .route("/docs/changelog", get(changelog))
        .route("/docs/changelog/release", post(release))
}

/// Who edits, publishes or reviews, from the caller's access. Agents act under their role and
/// their team's task; people under their own account and project administration.
async fn editor(app: &Arc<App>, access: &Access, task: Option<String>) -> ApiResult<Editor> {
    let role = access.agent.then(|| access.agent_role_id.clone().unwrap_or_else(|| access.actor.role.to_string()));
    let mut task = task.map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    if task.is_none()
        && let Some(team) = access.agent_team.clone()
    {
        let slug = access.project.clone();
        if let Ok(found) = app.blocking(move |app| app.with_tracker(&slug, |t| Ok(t.bus().get(&team)?.task))).await {
            task = Some(found);
        }
    }
    let (name, login, kind) = match &access.user {
        Some(u) => (u.name.clone(), u.login.clone(), AuthorKind::Human),
        None => (access.actor.name.clone(), access.actor.name.clone(), AuthorKind::Agent),
    };
    Ok(Editor { name, login, kind, task, project: access.project.clone(), admin: !access.agent && access.role.can_admin(), role })
}

fn published(out: Published) -> Value {
    match out {
        Published::Published { changeset, commit } => json!({ "status": "published", "commit": commit, "changeset": changeset }),
        Published::Merging { changeset, merge } => json!({ "status": "merging", "merge": merge, "changeset": changeset }),
    }
}

fn path_in_project(app: &App, project: &str, path: &str) -> AppResult<String> {
    app.with_vault(|v| Ok(v.resolve_for_project(project, path)?.0))
}

async fn version(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let sig = app.blocking(move |app| app.with_vault(|v| Ok(v.project_signature(&access.project)))).await.map_err(docs_error)?;
    Ok(Json(json!({ "version": sig })))
}

async fn tree(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let (sig, roots, pages) = app
        .blocking(move |app| {
            app.with_vault(|v| {
                v.refresh()?;
                Ok((v.project_signature(&access.project), v.spaces_of(&access.project), v.project_tree(&access.project)?))
            })
        })
        .await
        .map_err(docs_error)?;
    let diagnostics: Vec<Value> =
        pages.iter().filter(|p| !p.diagnostics.is_empty()).map(|p| json!({ "path": p.path, "diagnostics": p.diagnostics })).collect();
    Ok(Json(json!({ "version": sig, "roots": roots, "pages": pages, "diagnostics": diagnostics })))
}

#[derive(Deserialize, Default)]
struct RequiredQuery {
    role: Option<String>,
}

/// The pages the project requires a role to read, whole, with their versions.
/// Without `?role`, an agent's own role is used; people must name one.
async fn required(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<RequiredQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let role = q
        .role
        .map(|r| r.trim().to_string())
        .filter(|r| !r.is_empty())
        .or_else(|| {
            if access.actor.role == genie_core::Role::Orchestrator {
                Some("orchestrator".to_string())
            } else {
                access.agent_role_id.clone()
            }
        })
        .ok_or_else(|| ApiError::bad("which role? pass ?role=executor"))?;
    let project = access.project.clone();
    let wanted = role.clone();
    let reading = app.blocking(move |app| Ok(crate::context::required_reading(app, &project, &wanted))).await.map_err(docs_error)?;
    Ok(match reading {
        Some(r) => Json(json!({ "role": r.role, "pages": r.pages, "missing": r.missing })),
        None => Json(json!({ "role": role, "pages": [], "missing": [] })),
    })
}

#[derive(Deserialize, Default)]
struct SearchQuery {
    q: Option<String>,
    #[serde(rename = "type")]
    doc_type: Option<String>,
    status: Option<String>,
    limit: Option<usize>,
}

async fn search(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<SearchQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let query = q.q.unwrap_or_default();
    if let Some(t) = &q.doc_type
        && !genie_core::vault::DOC_TYPES.contains(&t.as_str())
    {
        return Err(ApiError::bad(format!("unknown docs type {t}")));
    }
    if let Some(s) = &q.status
        && !genie_core::vault::DOC_STATUSES.contains(&s.as_str())
    {
        return Err(ApiError::bad(format!("unknown docs status {s}")));
    }
    let project = access.project.clone();
    let task = access.agent_team.clone();
    let query2 = query.clone();
    let results = app
        .blocking(move |app| {
            app.with_vault(|v| {
                let opts = SearchOptions {
                    limit: q.limit.unwrap_or(20),
                    doc_type: q.doc_type,
                    status: q.status,
                    spaces: Vec::new(),
                    project: Some(project),
                    related: task.into_iter().collect(),
                };
                v.search(&query2, &opts)
            })
        })
        .await
        .map_err(docs_error)?;
    Ok(Json(json!({ "query": query, "results": results })))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ReadQuery {
    path: Option<String>,
    heading: Option<String>,
    max_chars: Option<usize>,
    changeset: Option<String>,
    at: Option<String>,
}

/// Read a page: the caller's changeset version when one touches it, otherwise main.
async fn read(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<ReadQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let path = q.path.filter(|p| !p.trim().is_empty()).ok_or_else(|| ApiError::bad("missing docs path"))?;
    let ed = editor(&app, &access, None).await?;
    let page = app
        .blocking(move |app| {
            changesets::read_for(app, &ed, &path, q.changeset.as_deref(), q.at.as_deref(), q.heading.as_deref(), q.max_chars)
        })
        .await
        .map_err(docs_error)?;
    Ok(Json(page))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WriteBody {
    path: String,
    content: Option<String>,
    text: Option<String>,
    mode: Option<String>,
    #[serde(default)]
    note: String,
    task: Option<String>,
    changeset: Option<i64>,
}

/// Compatibility write: one `write-page` edit into the caller's changeset.
async fn write(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<WriteBody>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    access.can(Capability::DocsWrite)?;
    let content = b.content.or(b.text).ok_or_else(|| ApiError::bad("missing docs content"))?;
    if content.trim().is_empty() {
        return Err(ApiError::bad("docs content must not be empty"));
    }
    let mode = b.mode.unwrap_or_else(|| "upsert".into());
    if !matches!(mode.as_str(), "create" | "update" | "upsert") {
        return Err(ApiError::bad(format!("unknown docs mode {mode}")));
    }
    let ed = editor(&app, &access, b.task).await?;
    let (project, path, note, changeset) = (access.project.clone(), b.path.clone(), b.note, b.changeset);
    let out = app
        .blocking(move |app| {
            let rel = path_in_project(app, &project, &path)?;
            let exists = match changesets::read_for(app, &ed, &rel, None, None, None, None) {
                Ok(_) => true,
                Err(AppError::Genie(GenieError::NotFound(_))) => false,
                Err(e) => return Err(e),
            };
            if mode == "create" && exists {
                return Ok(Err(ApiError::new(StatusCode::CONFLICT, "documentation page already exists")));
            }
            if mode == "update" && !exists {
                return Ok(Err(ApiError::new(StatusCode::NOT_FOUND, "documentation page not found")));
            }
            let ops = [DocOp::WritePage { content }];
            let edited = changesets::edit(app, &ed, &rel, &ops, &note, changeset)?;
            let page = changesets::read_for(app, &ed, &rel, None, None, None, None)?;
            Ok(Ok(json!({ "page": page, "changeset": edited.changeset, "commit": edited.commit, "diagnostics": [] })))
        })
        .await
        .map_err(docs_error)??;
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EditBody {
    path: String,
    ops: Vec<DocOp>,
    #[serde(default)]
    note: String,
    task: Option<String>,
    changeset: Option<i64>,
}

/// Apply section operations to the changeset version of a page.
async fn edit(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<EditBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    access.can(Capability::DocsWrite)?;
    if b.ops.is_empty() {
        return Err(ApiError::bad("pass at least one operation"));
    }
    let ed = editor(&app, &access, b.task).await?;
    let (project, path, note, changeset) = (access.project.clone(), b.path.clone(), b.note, b.changeset);
    let out = app
        .blocking(move |app| {
            let rel = path_in_project(app, &project, &path)?;
            let edited = changesets::edit(app, &ed, &rel, &b.ops, &note, changeset)?;
            let page = changesets::read_for(app, &ed, &rel, None, None, None, None)?;
            Ok(json!({ "changeset": edited.changeset, "commit": edited.commit, "page": page }))
        })
        .await
        .map_err(docs_error)?;
    Ok(Json(out))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RemoveQuery {
    path: Option<String>,
    note: Option<String>,
    task: Option<String>,
    changeset: Option<i64>,
}

/// Remove one page inside the caller's changeset.
async fn remove_page(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<RemoveQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    access.can(Capability::DocsWrite)?;
    let path = q.path.filter(|p| !p.trim().is_empty()).ok_or_else(|| ApiError::bad("missing docs path"))?;
    let ed = editor(&app, &access, q.task).await?;
    let (project, note, changeset) = (access.project.clone(), q.note.unwrap_or_default(), q.changeset);
    let (rel, out) = app
        .blocking(move |app| {
            let rel = path_in_project(app, &project, &path)?;
            let edited = changesets::remove_page(app, &ed, &rel, &note, changeset)?;
            Ok((rel, edited))
        })
        .await
        .map_err(docs_error)?;
    Ok(Json(json!({ "removed": rel, "changeset": out.changeset, "commit": out.commit })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoveBody {
    from: String,
    to: String,
    #[serde(default)]
    note: String,
    task: Option<String>,
    changeset: Option<i64>,
}

/// Move a page inside the caller's changeset; its wiki-links follow in the same changeset.
async fn move_page(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<MoveBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    access.can(Capability::DocsWrite)?;
    let ed = editor(&app, &access, b.task).await?;
    let (project, from, to, note, changeset) = (access.project.clone(), b.from.clone(), b.to.clone(), b.note, b.changeset);
    let out = app
        .blocking(move |app| {
            let from = path_in_project(app, &project, &from)?;
            let to = path_in_project(app, &project, &to)?;
            changesets::move_page(app, &ed, &from, &to, &note, changeset)
        })
        .await
        .map_err(docs_error)?;
    Ok(Json(json!({ "from": b.from, "to": b.to, "changeset": out.changeset, "commit": out.commit })))
}

#[derive(Deserialize, Default)]
struct ChangesQuery {
    status: Option<String>,
    task: Option<String>,
    author: Option<String>,
    unreviewed: Option<String>,
    path: Option<String>,
    limit: Option<i64>,
    /// Only the caller's own changesets: the command line's default feed.
    mine: Option<String>,
}

fn flag(value: Option<&str>) -> bool {
    matches!(value.map(str::trim), Some("1" | "true" | "yes"))
}

fn text(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// The Changes feed, scoped to the project's documentation roots.
async fn feed(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<ChangesQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let statuses = match text(q.status) {
        Some(list) => list
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| ChangesetStatus::parse(s).map_err(|e| ApiError::bad(e.to_string())))
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };
    let mine = flag(q.mine.as_deref());
    let ed = editor(&app, &access, None).await?;
    let project = access.project.clone();
    let roots = app.blocking(move |app| app.with_vault(|v| Ok(v.spaces_of(&project)))).await.map_err(docs_error)?;
    let filter = ChangesetFilter {
        project_roots: Some(roots),
        statuses,
        task: text(q.task),
        author: if mine { Some(ed.login.clone()) } else { text(q.author) },
        unreviewed: flag(q.unreviewed.as_deref()),
        path: text(q.path),
        limit: q.limit.unwrap_or(100),
    };
    let list = app.blocking(move |app| app.with_server(|db| db.changesets(&filter))).await.map_err(docs_error)?;
    Ok(Json(json!(list)))
}

/// A changeset with its commits and full diff.
async fn detail(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let project = access.project.clone();
    let out = app.blocking(move |app| changesets::changeset_detail(app, &project, id)).await.map_err(docs_error)?;
    Ok(Json(out))
}

async fn publish(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    access.can(Capability::DocsWrite)?;
    let ed = editor(&app, &access, None).await?;
    let out = app.blocking(move |app| changesets::publish(app, id, &ed)).await.map_err(docs_error)?;
    Ok(Json(published(out)))
}

async fn discard(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    access.can(Capability::DocsWrite)?;
    let ed = editor(&app, &access, None).await?;
    let out = app.blocking(move |app| changesets::discard(app, id, &ed)).await.map_err(docs_error)?;
    Ok(Json(json!({ "changeset": out })))
}

#[derive(Deserialize, Default)]
struct ReviewBody {
    decision: Option<String>,
}

/// Mark a published changeset reviewed; the touched sections' owners only.
async fn review(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>, body: Option<Json<ReviewBody>>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    access.can(Capability::DocsWrite)?;
    if let Some(decision) = body.and_then(|Json(b)| b.decision).map(|d| d.trim().to_string()).filter(|d| !d.is_empty())
        && decision != "reviewed"
    {
        return Err(ApiError::bad("decision must be reviewed; use revert to undo a published changeset"));
    }
    let ed = editor(&app, &access, None).await?;
    let out = app.blocking(move |app| changesets::review(app, id, &ed)).await.map_err(docs_error)?;
    Ok(Json(json!({ "changeset": out })))
}

async fn revert(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    access.can(Capability::DocsWrite)?;
    let ed = editor(&app, &access, None).await?;
    let out = app.blocking(move |app| changesets::revert(app, id, &ed)).await.map_err(docs_error)?;
    Ok(Json(published(out)))
}

#[derive(Deserialize, Default)]
struct HistoryQuery {
    path: Option<String>,
    limit: Option<usize>,
}

async fn history(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<HistoryQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let path = q.path.filter(|p| !p.trim().is_empty()).ok_or_else(|| ApiError::bad("missing docs path"))?;
    let project = access.project.clone();
    let commits = app.blocking(move |app| changesets::history(app, &project, &path, q.limit.unwrap_or(20))).await.map_err(docs_error)?;
    Ok(Json(json!(commits)))
}

#[derive(Deserialize, Default)]
struct DiffQuery {
    path: Option<String>,
    from: Option<String>,
    to: Option<String>,
}

async fn diff(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<DiffQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let path = q.path.filter(|p| !p.trim().is_empty()).ok_or_else(|| ApiError::bad("missing docs path"))?;
    let from = q.from.filter(|f| !f.trim().is_empty()).ok_or_else(|| ApiError::bad("missing --from version"))?;
    let to = q.to.filter(|t| !t.trim().is_empty()).ok_or_else(|| ApiError::bad("missing --to version"))?;
    let project = access.project.clone();
    let (p, f, t) = (path.clone(), from.clone(), to.clone());
    let text = app.blocking(move |app| changesets::diff(app, &project, &p, &f, &t)).await.map_err(docs_error)?;
    Ok(Json(json!({ "path": path, "from": from, "to": to, "diff": text })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RestoreBody {
    path: String,
    at: String,
}

/// Publish a page's content at a version as a new changeset.
async fn restore(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<RestoreBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    access.can(Capability::DocsWrite)?;
    if b.at.trim().is_empty() {
        return Err(ApiError::bad("missing --at version"));
    }
    let ed = editor(&app, &access, None).await?;
    let out = app.blocking(move |app| changesets::restore(app, &ed, &b.path, &b.at)).await.map_err(docs_error)?;
    Ok(Json(published(out)))
}

async fn merge_show(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let project = access.project.clone();
    let out = app.blocking(move |app| changesets::merge_detail(app, &project, id)).await.map_err(docs_error)?;
    Ok(Json(out))
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ResolveBody {
    #[serde(default)]
    pages: Vec<ResolvePage>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    fail: bool,
    #[serde(default)]
    contradiction: bool,
    #[serde(default)]
    note: String,
}

#[derive(Deserialize)]
struct ResolvePage {
    path: String,
    content: String,
}

async fn merge_resolve(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>, Json(b): Json<ResolveBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    access.can(Capability::DocsWrite)?;
    let ed = editor(&app, &access, None).await?;
    let pages: Vec<(String, Option<String>)> = b.pages.into_iter().map(|p| (p.path, Some(p.content))).collect();
    let out = app
        .blocking(move |app| changesets::resolve_merge(app, id, &ed, &pages, b.done, b.fail, b.contradiction, &b.note))
        .await
        .map_err(docs_error)?;
    Ok(Json(out))
}

/// Remove a section and everything under it. Project admins only: it is the one destructive docs route.
async fn remove_section(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<PathQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    access.can(Capability::DocsWrite)?;
    let path = q.path.filter(|p| !p.trim().is_empty()).ok_or_else(|| ApiError::bad("missing section path"))?;
    let (name, login, kind) = match &access.user {
        Some(u) => (u.name.clone(), u.login.clone(), AuthorKind::Human),
        None => (access.actor.name.clone(), access.actor.name.clone(), AuthorKind::Agent),
    };
    let project = access.project.clone();
    let pages = app
        .blocking(move |app| knowledge::remove_section(app, &project, &path, Author { name: &name, login: &login, kind }))
        .await
        .map_err(docs_error)?;
    Ok(Json(json!({ "removed": pages.len(), "pages": pages })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NoteBody {
    title: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    related: Vec<String>,
    task: Option<String>,
}

/// A draft note in the inbox of the caller's project space; never replaces a page.
async fn note(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<NoteBody>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    access.can(Capability::DocsWrite)?;
    let title = b.title.trim().to_string();
    if title.is_empty() {
        return Err(ApiError::bad("a note needs a title"));
    }
    let (name, login, kind) = match &access.user {
        Some(u) => (u.name.clone(), u.login.clone(), AuthorKind::Human),
        None => (access.actor.name.clone(), access.actor.name.clone(), AuthorKind::Agent),
    };
    let project = access.project.clone();
    let task = b.task.filter(|t| !t.is_empty());
    let content = knowledge::note_page(&title, &b.body, &b.tags, &b.related);
    let (status, v) = app
        .blocking(move |app| {
            let path = app.with_vault(|v| {
                let mut spaces = v.spaces_of(&project);
                spaces.sort();
                let root = spaces.first().ok_or_else(|| GenieError::not_found("the project has no vault space"))?;
                let dir = format!("{root}/inbox");
                let base = format!("{dir}/{}-{}", chrono::Utc::now().format("%Y-%m-%d"), knowledge::slug(&title));
                let mut path = format!("{base}.md");
                let mut n = 2;
                while v.current_hash(&path)?.is_some() {
                    path = format!("{base}-{n}.md");
                    n += 1;
                }
                Ok(path)
            })?;
            let author = Author { name: &name, login: &login, kind };
            let out = knowledge::write_doc(app, &path, &content, author, Some(""), &format!("note: {title}"), task.as_deref())?;
            Ok(match out {
                DocWrite::Saved { .. } => (
                    StatusCode::CREATED,
                    json!({ "path": path, "title": title, "type": "note", "status": "draft", "tags": b.tags, "related": b.related }),
                ),
                DocWrite::Proposed(p) => (
                    StatusCode::ACCEPTED,
                    json!({ "path": path, "title": title, "proposal": p.id, "proposalUrl": format!("{}/docs?proposal={}", app.cfg.public_url(), p.id) }),
                ),
            })
        })
        .await
        .map_err(docs_error)?;
    Ok((status, Json(v)))
}

#[derive(Deserialize, Default)]
struct PathQuery {
    path: Option<String>,
}

async fn spaces(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let cfg = app
        .blocking(move |app| {
            app.with_vault(|v| {
                let mut cfg = v.config.clone();
                cfg.spaces.retain(|_, s| s.project.as_deref() == Some(&access.project));
                Ok(cfg)
            })
        })
        .await
        .map_err(docs_error)?;
    Ok(Json(json!(cfg)))
}

async fn set_space(State(app): State<Arc<App>>, ctx: Ctx, Path(name): Path<String>, Json(space): Json<Space>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    if name.is_empty() || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err(ApiError::bad("invalid space name"));
    }
    let cfg = app
        .blocking(move |app| {
            app.with_vault(|v| {
                if space.project.as_deref() != Some(&access.project)
                    || v.config.spaces.get(&name).is_some_and(|s| s.project.as_deref() != Some(&access.project))
                {
                    return Err(GenieError::invalid("a documentation root must belong to the selected project"));
                }
                v.config.spaces.insert(name, space);
                v.save_config()?;
                let mut cfg = v.config.clone();
                cfg.spaces.retain(|_, s| s.project.as_deref() == Some(&access.project));
                Ok(cfg)
            })
        })
        .await
        .map_err(docs_error)?;
    Ok(Json(json!(cfg)))
}

async fn changelog(State(app): State<Arc<App>>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    let project = access.project.clone();
    let (path, content) = app
        .blocking(move |app| {
            app.with_vault(|v| {
                let space =
                    v.spaces_of(&project).into_iter().next().ok_or_else(|| GenieError::not_found("the project has no vault space"))?;
                let rel = format!("{space}/changelog.md");
                let (_, abs) = v.resolve(&rel)?;
                Ok((rel, std::fs::read_to_string(abs).unwrap_or_default()))
            })
        })
        .await
        .map_err(docs_error)?;
    Ok(Json(json!({ "path": path, "content": content })))
}

#[derive(Deserialize)]
struct ReleaseBody {
    version: String,
}

async fn release(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<ReleaseBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    if b.version.trim().is_empty() || b.version.contains(char::is_whitespace) {
        return Err(ApiError::bad("version must be a single word such as 1.2.0"));
    }
    let (project, by) = (access.project.clone(), access.actor.name.clone());
    let notes = app.blocking(move |app| crate::knowledge::release(app, &project, &b.version, &by)).await.map_err(docs_error)?;
    Ok(Json(json!({ "notes": notes })))
}
