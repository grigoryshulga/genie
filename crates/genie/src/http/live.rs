//! Live stream for the web UI, per project.
//!
//! - `event: journal` carries each journal event with `id:` = event id, so a
//!   reconnecting client resumes with `Last-Event-ID`;
//! - `event: change` is what the SPA listens to: it fires when events arrive.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::routing::get;
use futures_util::stream::{self, Stream};
use genie_core::events::Event;
use tokio::sync::broadcast;

use super::ApiResult;
use super::ctx::Ctx;
use super::tasks::tracker;
use crate::state::App;

/// How often the journal of a watched project is read.
const POLL: Duration = Duration::from_millis(500);

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/events", get(live))
}

/// One poller per project with at least one open stream; it reads the journal and hands every batch
/// to all streams of the project, so the cost does not grow with the number of tabs.
#[derive(Default)]
pub struct Hub {
    senders: Mutex<HashMap<String, broadcast::Sender<Arc<Vec<Event>>>>>,
}

impl Hub {
    fn subscribe(&self, app: &Arc<App>, project: &str, from: i64) -> broadcast::Receiver<Arc<Vec<Event>>> {
        let mut senders = self.senders.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(tx) = senders.get(project) {
            return tx.subscribe();
        }
        let (tx, rx) = broadcast::channel(64);
        senders.insert(project.to_string(), tx.clone());
        tokio::spawn(poll(app.clone(), project.to_string(), tx, from));
        rx
    }
}

async fn poll(app: Arc<App>, project: String, tx: broadcast::Sender<Arc<Vec<Event>>>, mut last: i64) {
    loop {
        tokio::time::sleep(POLL).await;
        {
            // Under the lock, so a stream subscribing right now either keeps this poller alive or starts a new one.
            let mut senders = app.live.senders.lock().unwrap_or_else(|e| e.into_inner());
            if tx.receiver_count() == 0 {
                senders.remove(&project);
                return;
            }
        }
        let (after, slug) = (last, project.clone());
        let Ok(events) = app.blocking(move |app| app.with_tracker(&slug, |t| t.events_after(after, 200))).await else { continue };
        if let Some(e) = events.last() {
            last = e.id;
            let _ = tx.send(Arc::new(events));
        }
    }
}

struct Cursor {
    app: Arc<App>,
    project: String,
    last_event: i64,
    rx: broadcast::Receiver<Arc<Vec<Event>>>,
    /// Read the journal itself (at the start, and after the stream fell behind).
    catch_up: bool,
    pending: VecDeque<SseEvent>,
}

impl Cursor {
    fn take(&mut self, events: Vec<Event>) {
        let mut changed = false;
        let seen = self.last_event;
        for e in events.into_iter().filter(|e| e.id > seen) {
            self.last_event = e.id;
            changed = true;
            if let Ok(ev) = SseEvent::default().event("journal").id(e.id.to_string()).json_data(&e) {
                self.pending.push_back(ev);
            }
        }
        if changed {
            self.pending.push_back(SseEvent::default().event("change").data(self.last_event.to_string()));
        }
    }
}

async fn live(
    State(app): State<Arc<App>>,
    ctx: Ctx,
    headers: HeaderMap,
) -> ApiResult<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>> {
    let access = ctx.access(&app, None).await?;
    let resume = headers.get("last-event-id").and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<i64>().ok());
    let last_event = tracker(&app, &access, move |t| Ok(resume.unwrap_or(t.last_event_id()?))).await?;
    // Subscribed before the first read, so nothing falls between the two.
    let rx = app.live.subscribe(&app, &access.project, last_event);
    let cursor = Cursor { app, project: access.project, last_event, rx, catch_up: true, pending: VecDeque::new() };
    let events = stream::unfold(cursor, |mut c| async move {
        loop {
            if let Some(ev) = c.pending.pop_front() {
                return Some((Ok(ev), c));
            }
            if c.catch_up {
                let (after, slug) = (c.last_event, c.project.clone());
                match c.app.blocking(move |app| app.with_tracker(&slug, |t| t.events_after(after, 200))).await {
                    Ok(events) => {
                        c.catch_up = events.len() >= 200;
                        c.take(events);
                    }
                    Err(_) => tokio::time::sleep(POLL).await,
                }
                continue;
            }
            match c.rx.recv().await {
                Ok(batch) => c.take(batch.as_ref().clone()),
                Err(broadcast::error::RecvError::Lagged(_)) => c.catch_up = true,
                Err(broadcast::error::RecvError::Closed) => {
                    let from = c.last_event;
                    c.rx = c.app.live.subscribe(&c.app, &c.project, from);
                    c.catch_up = true;
                }
            }
        }
    });
    let head = stream::once(async { Ok(SseEvent::default().retry(Duration::from_secs(2)).comment("genie")) });
    Ok(Sse::new(futures_util::StreamExt::chain(head, events)).keep_alive(KeepAlive::default()))
}
