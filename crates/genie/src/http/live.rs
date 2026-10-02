//! Live stream for the web UI, per project.
//!
//! - `event: journal` carries each journal event with `id:` = event id, so a
//!   reconnecting client resumes with `Last-Event-ID`;
//! - `event: change` is what the SPA listens to: it fires when events arrive.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::routing::get;
use futures_util::stream::{self, Stream};

use super::ApiResult;
use super::ctx::Ctx;
use super::tasks::tracker;
use crate::state::App;

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/events", get(live))
}

struct Cursor {
    app: Arc<App>,
    project: String,
    last_event: i64,
    pending: VecDeque<SseEvent>,
}

async fn live(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    headers: HeaderMap,
) -> ApiResult<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>> {
    let access = ctx.access(&app, None).await?;
    let resume = headers.get("last-event-id").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<i64>().ok());
    let last_event = tracker(&app, &access, move |t| Ok(resume.unwrap_or(t.last_event_id()?))).await?;
    let cursor = Cursor { app, project: access.project, last_event, pending: VecDeque::new() };
    let events = stream::unfold(cursor, |mut c| async move {
        loop {
            if let Some(ev) = c.pending.pop_front() {
                return Some((Ok(ev), c));
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            let (after, slug) = (c.last_event, c.project.clone());
            let polled = c.app.blocking(move |app| app.with_tracker(&slug, |t| t.events_after(after, 200))).await;
            let Ok(events) = polled else { continue };
            let changed = !events.is_empty();
            for e in events {
                c.last_event = e.id;
                if let Ok(ev) = SseEvent::default().event("journal").id(e.id.to_string()).json_data(&e) {
                    c.pending.push_back(ev);
                }
            }
            if changed {
                c.pending.push_back(SseEvent::default().event("change").data(c.last_event.to_string()));
            }
        }
    });
    let head = stream::once(async { Ok(SseEvent::default().retry(Duration::from_secs(2)).comment("genie")) });
    Ok(Sse::new(futures_util::StreamExt::chain(head, events)).keep_alive(KeepAlive::default()))
}
