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
    /// Reference to an authoritative interaction; never a second answer store.
    pub interaction: Option<Value>,
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

/// A bot token is not persisted in delivery payloads. Its digest separates global bot
/// generations so a message ID from another bot can never identify a previous question.
pub(crate) fn global_scope(app: &App) -> Option<String> {
    app.cfg.telegram.as_ref().map(|t| format!("global:{}", genie_core::server_db::hash_secret(&t.token)))
}

pub(crate) struct Target {
    pub address: String,
    pub scope: Option<String>,
}

pub(crate) fn target(
    db: &genie_core::server_db::ServerDb,
    app: &App,
    user: i64,
    project: Option<&str>,
    channel: &str,
    interaction: bool,
) -> genie_core::Result<Option<Target>> {
    let u = db.user(user)?;
    if u.disabled
        || project.map(|p| db.project_role(p, &u).map(|r| r.is_none_or(|r| interaction && !r.can_write()))).transpose()?.unwrap_or(false)
    {
        return Ok(None);
    }
    match channel {
        "telegram" => {
            let bot = project.map(|p| db.project_bot(p)).transpose()?.flatten();
            let (link, scope) =
                if let Some(bot) = bot { (bot.scope.clone(), Some(bot.scope)) } else { ("telegram".into(), global_scope(app)) };
            if scope.is_none() {
                return Ok(None);
            }
            let address = db.channel_address(user, &link)?;
            if let Some(address) = &address
                && db.user_by_channel(&link, address)? != Some(user)
            {
                return Ok(None);
            }
            Ok(address.map(|address| Target { address, scope }))
        }
        "email" if app.cfg.smtp.is_some() => {
            Ok(db.channel_address(user, "email")?.or(u.email).map(|address| Target { address, scope: None }))
        }
        _ => Ok(None),
    }
}

/// Notify people; a deduplicated group spans web, the preferred channel and explicit fallbacks.
pub fn send(app: &App, users: &[i64], msg: &Message, dedupe: Option<&str>) -> AppResult<usize> {
    let base = app.cfg.public_url();
    let link = msg.link.as_ref().map(|l| if l.starts_with("http") { l.clone() } else { format!("{base}{l}") });
    let wants = |c: &str| msg.channels.is_empty() || msg.channels.iter().any(|x| x == c);
    let root = dedupe.map(str::to_string).unwrap_or_else(genie_core::server_db::new_secret);
    let mut queued = 0;
    for &user in users {
        queued+=app.with_server(|db|db.tx(|| {
            let u=db.user(user)?;
            // Existing noninteractive web notices follow their domain's recipient selection
            // (including personal key/setup errors). Interactive prompts require live write
            // access; all external channels recheck current project access in `target`.
            if u.disabled || msg.interaction.is_some() && msg.project.as_deref().map(|p|db.project_role(p,&u).map(|r|!r.is_some_and(|r|r.can_write()))).transpose()?.unwrap_or(false) { return Ok(0); }
            let group=format!("{root}:{user}");
            let web_key=format!("{group}:web");
            let notification=if wants("web") {
                db.add_notification(user,msg.project.as_deref(),msg.task.as_deref(),&msg.kind,&msg.title,&msg.body,msg.link.as_deref(),Some(&web_key))?;
                use rusqlite::OptionalExtension;
                db.conn().query_row("SELECT id FROM notifications WHERE dedupe_key=?1",[&web_key],|r|r.get::<_,i64>(0)).optional()?
            } else {None};
            let policy=db.delivery_policy(user)?;
            let mut order=policy.as_ref().map(|p|p.order.clone()).unwrap_or_else(||vec!["telegram".into(),"email".into()]);
            order.retain(|c|wants(c));
            let after=policy.as_ref().map_or(genie_core::inbox::MAX_DELIVERY_ATTEMPTS,|p|p.fallback_after_attempts);
            let text=format!("{}\n\n{}{}",msg.title,msg.body,link.as_ref().map(|l|format!("\n\n{l}")).unwrap_or_default());
            let mut payload=json!({"buttons":msg.buttons,"link":link,"kind":msg.kind,"project":msg.project,"task":msg.task,"interaction":msg.interaction,"recipient":user,"notification":notification,
                "delivery":{"group":group,"order":order,"afterAttempts":after,"allowed":msg.channels}});
            // A web-only interaction still has a durable copy to update when resolved.
            if notification.is_some() && msg.interaction.is_some() {
                db.enqueue(Some(&format!("{group}:web-copy")),Some(user),"web","",&msg.title,&msg.body,&payload)?;
            }
            for channel in &order {
                let Some(t)=target(db,app,user,msg.project.as_deref(),channel,msg.interaction.is_some())? else {continue};
                if policy.is_none() {payload["delivery"]["order"]=json!([channel]);}
                payload["telegramScope"]=json!(t.scope);
                payload["forceReply"]=json!(msg.interaction.is_some());
                return Ok(db.enqueue(Some(&format!("{group}:{channel}")),Some(user),channel,&t.address,&msg.title,&text,&payload)?.is_some() as usize);
            }
            Ok(0)
        }))?;
    }
    app.wake_outbox.notify_one();
    Ok(queued)
}

/// Select only fallbacks captured for this delivery and still allowed by the person's current
/// policy. Resolving addresses again cannot expand the recipient or project authority.
pub(crate) fn fallback(app: &App, item: &genie_core::inbox::OutboxItem) -> AppResult<bool> {
    app.with_server(|db| fallback_in(db, app, item))
}

/// The failed-attempt transition and fallback enqueue belong to the same server transaction.
pub(crate) fn fallback_in(
    db: &genie_core::server_db::ServerDb,
    app: &App,
    item: &genie_core::inbox::OutboxItem,
) -> genie_core::Result<bool> {
    db.tx(|| {
        let Some(user) = item.user else { return Ok(false) };
        let delivery = &item.payload["delivery"];
        let Some(group) = delivery["group"].as_str() else { return Ok(false) };
        let order: Vec<String> = serde_json::from_value(delivery["order"].clone()).unwrap_or_default();
        let Some(index) = order.iter().position(|c| c == &item.channel) else { return Ok(false) };
        let current = db.delivery_policy(user)?;
        for channel in &order[index + 1..] {
            if current.as_ref().is_none_or(|p| !p.order.contains(channel)) {
                continue;
            }
            let Some(t) = target(db, app, user, item.payload["project"].as_str(), channel, !item.payload["interaction"].is_null())? else {
                continue;
            };
            let mut payload = item.payload.clone();
            payload["telegramScope"] = json!(t.scope);
            db.enqueue(Some(&format!("{group}:{channel}")), Some(user), channel, &t.address, &item.subject, &item.body, &payload)?;
            db.conn().execute(
                "UPDATE outbox SET status='failed',last_error=COALESCE(last_error,'preferred channel unavailable') WHERE id=?1",
                [item.id],
            )?;
            return Ok(true);
        }
        Ok(false)
    })
}

pub(crate) fn valid_target(app: &App, item: &genie_core::inbox::OutboxItem) -> AppResult<bool> {
    let Some(user) = item.user else { return Ok(false) };
    app.with_server(|db| {
        if db.delivery_policy(user)?.is_some_and(|p| !p.order.contains(&item.channel)) {
            return Ok(false);
        }
        let expected = target(db, app, user, item.payload["project"].as_str(), &item.channel, !item.payload["interaction"].is_null())?;
        Ok(expected.is_some_and(|t| {
            t.address == item.address && (item.channel != "telegram" || t.scope.as_deref() == item.payload["telegramScope"].as_str())
        }))
    })
}

/// Exhausting delivery never resolves a question. Surface the failure in the person's web
/// center without sending another external message or adding recipients.
pub(crate) fn delivery_failed_in(db: &genie_core::server_db::ServerDb, item: &genie_core::inbox::OutboxItem) -> genie_core::Result<()> {
    let Some(uid) = item.user else { return Ok(()) };
    let u = db.user(uid)?;
    let project = item.payload["project"].as_str();
    if u.disabled || project.map(|p| db.project_role(p, &u).map(|r| r.is_none())).transpose()?.unwrap_or(false) {
        return Ok(());
    }
    let group = item.payload["delivery"]["group"].as_str().map(str::to_string).unwrap_or_else(|| item.id.to_string());
    db.add_notification(uid,project,item.payload["task"].as_str(),"delivery_failed","Не удалось доставить уведомление",
            &format!("{}\n\nНастроенные способы доставки недоступны. Проверьте подключения. Вопрос или предложение остаётся открытым в Genie; ответ или одобрение не записывались.",item.subject),
            item.payload["link"].as_str(),Some(&format!("delivery-failed:{group}")))?;
    Ok(())
}
