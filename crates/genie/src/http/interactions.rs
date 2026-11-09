//! Personal decisions over the same authoritative subjects used by chat and channels.
use super::{ApiResult, Body, ctx::Ctx};
use crate::{interactions, state::App};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
        .route("/interactions", get(list))
        .route("/interactions/{id}", get(show))
        .route("/interactions/{id}/respond", post(respond))
}

#[derive(Default, Deserialize)]
struct Filter {
    #[serde(default)]
    all: bool,
}

async fn list(State(app): State<Arc<App>>, ctx: Ctx, Query(q): Query<Filter>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    Ok(Json(app.blocking(move |app| Ok(json!(interactions::list(app, &a, q.all)?))).await?))
}
async fn show(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    Ok(Json(app.blocking(move |app| Ok(json!(interactions::show(app, &a, &id)?))).await?))
}
async fn respond(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    Path(id): Path<String>,
    Body(b): Body<interactions::Response>,
) -> ApiResult<Json<Value>> {
    let a = ctx.access(&app, None).await?;
    Ok(Json(app.blocking(move |app| Ok(json!(interactions::respond(app, &a, &id, b, "api")?))).await?))
}
