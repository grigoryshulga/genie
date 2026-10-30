//! Knowledge routes: `/api/docs/{version,tree,search,page}` serve the SPA's Docs
//! page from the vault, next to proposals, spaces and the changelog.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use genie_core::vault::{AuthorKind, SearchOptions, Space};
use genie_core::{Capability, GenieError};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ctx::{Access, Ctx};
use super::{ApiError, ApiResult};
use crate::knowledge::{self, Author, DocWrite};
use crate::state::{App, AppError};

fn docs_error(error: AppError) -> ApiError {
    match error {
        AppError::Genie(GenieError::NotFound(message)) => ApiError::new(StatusCode::NOT_FOUND, message),
        other => other.into(),
    }
}

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/docs/version", get(version))
        .route("/docs/tree", get(tree))
        .route("/docs/search", get(search))
        .route("/docs/page", get(read).post(write).delete(remove_page))
        .route("/docs/note", post(note))
        .route("/docs/section", delete(remove_section))
        .route("/docs/proposals", get(proposals))
        .route("/docs/proposals/{id}", get(proposal))
        .route("/docs/proposals/{id}/approve", post(approve))
        .route("/docs/proposals/{id}/reject", post(reject))
        .route("/docs/spaces", get(spaces))
        .route("/docs/spaces/{name}", put(set_space))
        .route("/docs/changelog", get(changelog))
        .route("/docs/changelog/release", post(release))
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
}

async fn read(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<ReadQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let path = q.path.filter(|p| !p.trim().is_empty()).ok_or_else(|| ApiError::bad("missing docs path"))?;
    let page = app
        .blocking(move |app| app.with_vault(|v| v.project_read(&access.project, &path, q.heading.as_deref(), q.max_chars)))
        .await
        .map_err(docs_error)?;
    Ok(Json(json!(page)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WriteBody {
    path: String,
    content: Option<String>,
    text: Option<String>,
    mode: Option<String>,
    base_hash: Option<String>,
    #[serde(default)]
    note: String,
    task: Option<String>,
}

fn author_of(access: &Access) -> (String, String, AuthorKind) {
    match &access.user {
        Some(u) => (u.name.clone(), u.login.clone(), AuthorKind::Human),
        None => (access.actor.name.clone(), access.actor.name.clone(), AuthorKind::Agent),
    }
}

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
    let (name, login, kind) = author_of(&access);
    let task = b.task.filter(|t| !t.is_empty());
    let project = access.project.clone();
    let path = b.path.clone();
    let result = app
        .blocking(move |app| {
            let path = app.with_vault(|v| Ok(v.resolve_for_project(&project, &path)?.0))?;
            let exists = app.with_vault(|v| Ok(v.current_hash(&path)?.is_some()))?;
            if mode == "create" && exists {
                return Ok(Err(ApiError::new(StatusCode::CONFLICT, "documentation page already exists")));
            }
            if mode == "update" && !exists {
                return Ok(Err(ApiError::new(StatusCode::NOT_FOUND, "documentation page not found")));
            }
            let base = b.base_hash.clone().or_else(|| (mode == "create").then(String::new));
            let out = knowledge::write_doc(
                app,
                &path,
                &content,
                Author { name: &name, login: &login, kind },
                base.as_deref(),
                &b.note,
                task.as_deref(),
            )?;
            Ok(Ok(match out {
                DocWrite::Saved { created, .. } => {
                    let page = app.with_vault(|v| v.project_read(&project, &path, None, None))?;
                    (if created { StatusCode::CREATED } else { StatusCode::OK }, json!({ "page": page, "diagnostics": [] }))
                }
                DocWrite::Changeset(cs) => {
                    let ed = crate::changesets::Editor {
                        name: name.clone(),
                        login: login.clone(),
                        kind,
                        task: task.clone(),
                        project: project.clone(),
                        admin: false,
                        role: None,
                    };
                    let page = crate::changesets::read_for(app, &ed, &path, None, None, None, None)?;
                    (StatusCode::OK, json!({ "changeset": cs, "page": page }))
                }
            }))
        })
        .await
        .map_err(|e| {
            let e = docs_error(e);
            if e.message.starts_with("conflict") {
                ApiError::new(StatusCode::CONFLICT, e.message)
            } else if e.message.contains("is locked") {
                ApiError::new(StatusCode::FORBIDDEN, e.message)
            } else {
                e
            }
        })??;
    Ok((result.0, Json(result.1)))
}

#[derive(Deserialize, Default)]
struct PathQuery {
    path: Option<String>,
}

/// Remove one page; the same write permission and publishing policy as an edit.
async fn remove_page(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<PathQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.write()?;
    access.can(Capability::DocsWrite)?;
    let path = q.path.filter(|p| !p.trim().is_empty()).ok_or_else(|| ApiError::bad("missing docs path"))?;
    let (name, login, kind) = author_of(&access);
    let project = access.project.clone();
    let removed = app
        .blocking(move |app| knowledge::remove_page(app, &project, &path, Author { name: &name, login: &login, kind }))
        .await
        .map_err(docs_error)?;
    Ok(Json(json!({ "removed": removed })))
}

/// Remove a section and everything under it. Project admins only: it is the one destructive docs route.
async fn remove_section(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<PathQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.admin()?;
    access.can(Capability::DocsWrite)?;
    let path = q.path.filter(|p| !p.trim().is_empty()).ok_or_else(|| ApiError::bad("missing section path"))?;
    let (name, login, kind) = author_of(&access);
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
    let (name, login, kind) = author_of(&access);
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
            let ed = crate::changesets::Editor {
                name: name.clone(),
                login: login.clone(),
                kind,
                task: task.clone(),
                project: project.clone(),
                admin: false,
                role: None,
            };
            Ok(match out {
                DocWrite::Saved { .. } => (
                    StatusCode::CREATED,
                    json!({ "path": path, "title": title, "type": "note", "status": "draft", "tags": b.tags, "related": b.related }),
                ),
                DocWrite::Changeset(cs) => {
                    let page = crate::changesets::read_for(app, &ed, &path, None, None, None, None)?;
                    (
                        StatusCode::OK,
                        json!({ "path": path, "title": title, "changeset": cs, "page": page, "tags": b.tags, "related": b.related }),
                    )
                }
            })
        })
        .await
        .map_err(docs_error)?;
    Ok((status, Json(v)))
}

#[derive(Deserialize, Default)]
struct ProposalsQuery {
    status: Option<String>,
}

async fn proposals(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<ProposalsQuery>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let status = q.status.unwrap_or_else(|| "open".into());
    let list = app
        .blocking(move |app| {
            let roots = app.with_vault(|v| Ok(v.spaces_of(&access.project)))?;
            app.with_server(|db| db.proposals_in_roots(if status == "all" { None } else { Some(&status) }, &roots, 200))
        })
        .await
        .map_err(docs_error)?;
    let out: Vec<Value> = list
        .into_iter()
        .map(|p| {
            let mut v = json!(p);
            v.as_object_mut().map(|o| o.remove("content"));
            v
        })
        .collect();
    Ok(Json(json!(out)))
}

async fn proposal(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    access.can(Capability::DocsRead)?;
    let (p, current, owners) = app
        .blocking(move |app| {
            let p = app.with_server(|db| db.proposal(id))?;
            let (current, owners) = app.with_vault(|v| {
                let (_, abs) = v.resolve_for_project(&access.project, &p.path)?;
                Ok((std::fs::read_to_string(abs).ok(), v.owners_for(&p.path)))
            })?;
            Ok((p, current, owners))
        })
        .await
        .map_err(docs_error)?;
    Ok(Json(json!({ "proposal": p, "current": current, "owners": owners })))
}

/// Server admins, the page's section/space owners and admins of the space's project decide.
async fn can_decide(app: &Arc<App>, ctx: &Ctx, id: i64) -> ApiResult<String> {
    let access = ctx.access(app, None).await?;
    let user = ctx.user()?.clone();
    let login = user.login.clone();
    let ok = app
        .blocking(move |app| {
            let p = app.with_server(|db| db.proposal(id))?;
            app.with_vault(|v| v.resolve_for_project(&access.project, &p.path))?;
            if user.is_admin {
                return Ok(true);
            }
            let (owners, project) = app.with_vault(|v| Ok((v.owners_for(&p.path), v.project_of(&p.path))))?;
            if owners.iter().any(|o| o == &user.login) {
                return Ok(true);
            }
            match project {
                Some(proj) => Ok(app.with_server(|db| db.project_role(&proj, &user))?.is_some_and(|r| r.can_admin())),
                None => Ok(false),
            }
        })
        .await
        .map_err(docs_error)?;
    if ok { Ok(login) } else { Err(ApiError::new(StatusCode::FORBIDDEN, "only the section owners or project admins decide on proposals")) }
}

#[derive(Deserialize, Default)]
struct DecideBody {
    note: Option<String>,
    #[serde(default)]
    force: bool,
}

async fn approve(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>, body: Option<Json<DecideBody>>) -> ApiResult<Json<Value>> {
    let by = can_decide(&app, &ctx, id).await?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    let p = app.blocking(move |app| knowledge::decide(app, id, true, &by, b.note.as_deref(), b.force)).await.map_err(|e| {
        let e = docs_error(e);
        if e.message.starts_with("conflict") {
            ApiError::new(StatusCode::CONFLICT, format!("{} (approve with force to overwrite)", e.message))
        } else {
            e
        }
    })?;
    Ok(Json(json!(p)))
}

async fn reject(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<i64>, body: Option<Json<DecideBody>>) -> ApiResult<Json<Value>> {
    let by = can_decide(&app, &ctx, id).await?;
    let note = body.and_then(|Json(b)| b.note);
    let p = app.blocking(move |app| knowledge::decide(app, id, false, &by, note.as_deref(), false)).await.map_err(docs_error)?;
    Ok(Json(json!(p)))
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
