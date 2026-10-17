//! HTTP entrance of the observation plane (`docs/platform/observation-plane.md` §15).

use std::sync::Arc;

use axum::Router;

use crate::state::App;

pub fn routes() -> Router<Arc<App>> {
    Router::new()
}
