//! Notifying people: resolve who (`task.author`, `project.admins`, `@anna`…),
//! write to the web notification center and queue a message for the person's
//! preferred channel (Telegram when linked, otherwise e-mail). Delivery itself
//! (with retries) is done by the channel dispatcher.

use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::state::{App, AppResult};

#[derive(Debug, Clone, Default)]
pub struct Message {
    pub kind: String,
    pub title: String,
    pub body: String,
    pub project: Option<String>,
    pub task: Option<String>,
    /// Path in the web UI (`/inbox?task=G-7`); made absolute with the public URL.
    pub link: Option<String>,
    /// Telegram inline buttons: `[(label, callback data)]`.
    pub buttons: Vec<(String, String)>,
    /// Only these channels (`web`, `telegram`, `email`); empty = web + preferred channel.
    pub channels: Vec<String>,
}

/// Resolve recipient specs to user ids.
///
/// `event.actor` (the person who caused the event), `task.author` (who created
/// the task), `task.assignee` (the person responsible), `task.assignees`, `project.owners`, `project.admins`,
/// `project.members`, `@login` or a plain login.
pub fn resolve(app: &App, project: &str, specs: &[String], ctx: &Value) -> AppResult<Vec<i64>> {
    let mut logins: BTreeSet<String> = BTreeSet::new();
    let mut ids: BTreeSet<i64> = BTreeSet::new();
    let task_id = ctx["event"]["task"]["id"].as_str().or_else(|| ctx["task"]["id"].as_str()).map(str::to_string);
    for spec in specs {
        let spec = spec.trim();
        match spec {
            "event.actor" => {
                if let Some(a) = ctx["event"]["actor"].as_str() {
                    logins.insert(a.to_string());
                }
            }
            "task.author" | "task.owner" => {
                if let Some(t) = &task_id
                    && let Ok(task) = app.with_tracker(project, |tr| tr.get(t))
                    && let Some(first) = task.history.iter().find(|h| h.role == genie_core::Role::Human)
                {
                    logins.insert(first.actor.clone());
                }
            }
            "task.assignee" => {
                if let Some(t) = &task_id
                    && let Ok(task) = app.with_tracker(project, |tr| tr.get(t))
                    && let Some(a) = task.assignee
                {
                    logins.insert(a);
                }
            }
            "task.watchers" => {
                if let Some(t) = &task_id {
                    ids.extend(app.with_server(|db| db.task_watchers(project, t))?);
                }
            }
            "task.assignees" => {
                if let Some(t) = &task_id
                    && let Ok(task) = app.with_tracker(project, |tr| tr.get(t))
                {
                    logins.extend(task.assignees);
                }
            }
            "project.owners" | "project.admins" | "project.members" | "epic.owner" => {
                let min = match spec {
                    "project.owners" => genie_core::server_db::ProjectRole::Owner,
                    "project.members" => genie_core::server_db::ProjectRole::Member,
                    _ => genie_core::server_db::ProjectRole::Admin,
                };
                let members = app.with_server(|db| db.members_of(project))?;
                ids.extend(members.into_iter().filter(|(u, r)| *r >= min && !u.disabled).map(|(u, _)| u.id));
                if spec != "project.members" {
                    // Server admins own every project.
                    ids.extend(app.with_server(|db| db.users())?.into_iter().filter(|u| u.is_admin && !u.disabled).map(|u| u.id));
                }
            }
            other => {
                logins.insert(other.trim_start_matches('@').to_string());
            }
        }
    }
    for login in logins {
        if let Some(u) = app.with_server(|db| db.user_by_login(&login))?
            && !u.disabled
        {
            ids.insert(u.id);
        }
    }
    Ok(ids.into_iter().collect())
}

/// Notify people; `dedupe` makes repeated calls for the same thing idempotent.
pub fn send(app: &App, users: &[i64], msg: &Message, dedupe: Option<&str>) -> AppResult<usize> {
    let base = app.cfg.public_url();
    let link = msg.link.as_ref().map(|l| if l.starts_with("http") { l.clone() } else { format!("{base}{l}") });
    let wants = |c: &str| msg.channels.is_empty() || msg.channels.iter().any(|x| x == c);
    let mut queued = 0;
    for &user in users {
        app.with_server(|db| {
            if wants("web") {
                let key = dedupe.map(|d| format!("{d}:{user}:web"));
                db.add_notification(
                    user,
                    msg.project.as_deref(),
                    msg.task.as_deref(),
                    &msg.kind,
                    &msg.title,
                    &msg.body,
                    msg.link.as_deref(),
                    key.as_deref(),
                )?;
            }
            let u = db.user(user)?;
            let project_bot = msg.project.as_deref().map(|p| db.project_bot(p)).transpose()?.flatten();
            let telegram_scope = project_bot.as_ref().map(|b| b.scope.as_str());
            let authorized = !u.disabled && match msg.project.as_deref() { Some(p) => db.project_role(p, &u)?.is_some(), None => true };
            let telegram = if !authorized || !wants("telegram") { None }
                else if let Some(bot) = &project_bot { db.channel_address(user, &bot.scope)?.filter(|_| !bot.token.0.is_empty()) }
                else { db.channel_address(user, "telegram")?.filter(|_| app.cfg.telegram.is_some()) };
            let email = db.channel_address(user, "email")?.or(u.email.clone()).filter(|_| app.cfg.smtp.is_some() && wants("email"));
            let text = format!("{}\n\n{}{}", msg.title, msg.body, link.as_ref().map(|l| format!("\n\n{l}")).unwrap_or_default());
            let payload = json!({ "buttons": msg.buttons, "link": link, "kind": msg.kind, "project": msg.project, "task": msg.task, "telegramScope": telegram_scope });
            let key = |ch: &str| dedupe.map(|d| format!("{d}:{user}:{ch}"));
            // One channel per person: Telegram when linked, otherwise e-mail.
            if let Some(chat) = telegram {
                if db.enqueue(key("telegram").as_deref(), Some(user), "telegram", &chat, &msg.title, &text, &payload)?.is_some() {
                    queued += 1;
                }
            } else if let Some(addr) = email
                && db.enqueue(key("email").as_deref(), Some(user), "email", &addr, &msg.title, &text, &payload)?.is_some()
            {
                queued += 1;
            }
            Ok(())
        })?;
    }
    app.wake_outbox.notify_one();
    Ok(queued)
}
