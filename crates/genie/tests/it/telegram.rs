use crate::common::{Harness, call};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use genie::config::TelegramConfig;
use genie_core::server_db::ProjectRole;
use genie_core::{Actor, CommentKind, CreateInput, Role, Status, StatusOptions};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

type Messages = Arc<Mutex<Vec<Value>>>;
async fn fake() -> (String, Messages, Messages) {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let updates: Messages = Arc::new(Mutex::new(Vec::new()));
    async fn api(
        State((sent, updates)): State<(Messages, Messages)>,
        Path((token, method)): Path<(String, String)>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        let result = match method.as_str() {
            "getMe" => {
                if token.starts_with("bot999:") {
                    return Json(json!({"ok": false, "description": token}));
                } else {
                    json!({"id": 123, "username": "project_bot", "is_bot": true})
                }
            }
            "getWebhookInfo" => json!({"url": if token.starts_with("bot888:") { "https://example.com/hook" } else { "" }}),
            "getUpdates" => {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                json!(std::mem::take(&mut *updates.lock().unwrap()))
            }
            "sendMessage" => {
                sent.lock().unwrap().push(json!({"token": token, "body": body}));
                json!({"message_id": 42})
            }
            _ => json!([]),
        };
        Json(json!({"ok": true, "result": result}))
    }
    let router = Router::new().route("/{token}/{method}", post(api)).with_state((sent.clone(), updates.clone()));
    let sock = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", sock.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(sock, router).await.unwrap();
    });
    (base, sent, updates)
}

#[tokio::test]
async fn admin_configures_bot_member_links_privately_and_disconnect_cancels_codes() {
    let (base, sent, _) = fake().await;
    let h = Harness::with_config(|cfg| cfg.telegram = Some(TelegramConfig { token: "LEGACY".into(), api_base: Some(base) }));
    h.project("shop");
    h.project("other");
    let (owner, member) = h
        .app
        .with_server(|db| {
            let owner = db.create_user("owner", "", None, None, true)?;
            let member = db.create_user("member", "", None, None, false)?;
            db.set_membership("shop", member.id, ProjectRole::Viewer)?;
            Ok((db.create_user_token(owner.id, "test")?, db.create_user_token(member.id, "test")?))
        })
        .unwrap();
    let url = "/api/project/telegram?project=shop";
    let token = "123:abcdefghijklmnopqrstuvwxyz";
    assert_eq!(call(&h.router, "PUT", url).bearer(&member).json(json!({"token": token})).send().await.0, 403);
    assert_eq!(call(&h.router, "PUT", url).bearer(&owner).json(json!({"token": token})).send().await.0, 200);
    let (_, status, _) = call(&h.router, "GET", url).bearer(&member).send().await;
    assert_eq!(status["username"], "project_bot");
    assert!(!status.to_string().contains(token));
    let (_, error, _) = call(&h.router, "PUT", url).bearer(&owner).json(json!({"token": "999:abcdefghijklmnopqrstuvwxyz"})).send().await;
    assert!(!error.to_string().contains("abcdefghijklmnopqrstuvwxyz"));
    assert_eq!(call(&h.router, "PUT", url).bearer(&owner).json(json!({"token": "888:abcdefghijklmnopqrstuvwxyz"})).send().await.0, 400);
    let (_, link, _) = call(&h.router, "POST", "/api/project/telegram/link?project=shop").bearer(&member).send().await;
    let code = link["url"].as_str().unwrap().split("start=").nth(1).unwrap();
    let bot = h.app.with_server(|db| db.project_bot("shop")).unwrap().unwrap();
    genie::telegram::handle_update(
        &h.app,
        &bot,
        &json!({"message": {"chat": {"id": -42, "type": "group"}, "text": format!("/start {code}")}}),
    )
    .await
    .unwrap();
    assert!(sent.lock().unwrap().is_empty());
    genie::telegram::handle_update(
        &h.app,
        &bot,
        &json!({"message": {"chat": {"id": 42, "type": "private"}, "text": format!("/start {code}")}}),
    )
    .await
    .unwrap();
    let (_, status, _) = call(&h.router, "GET", url).bearer(&member).send().await;
    assert_eq!(status["connected"], true);
    assert_eq!(call(&h.router, "POST", "/api/project/telegram/test?project=shop").bearer(&member).send().await.0, 200);
    assert_eq!(call(&h.router, "DELETE", "/api/project/telegram/link?project=shop").bearer(&member).send().await.0, 200);
    let (_, status, _) = call(&h.router, "GET", url).bearer(&member).send().await;
    assert_eq!(status["connected"], false);
    assert_ne!(call(&h.router, "GET", "/api/project/telegram?project=other").bearer(&member).send().await.0, 200);
    assert_eq!(call(&h.router, "DELETE", url).bearer(&member).send().await.0, 403);
}

#[tokio::test]
async fn watches_notify_author_and_followers_route_to_project_bot_and_respect_access() {
    let (base, sent, _) = fake().await;
    let h = Harness::with_config(|cfg| cfg.telegram = Some(TelegramConfig { token: "LEGACY".into(), api_base: Some(base) }));
    h.project("shop");
    let (author, watcher, token) = h
        .app
        .with_server(|db| {
            let a = db.create_user("author", "", None, None, false)?;
            let w = db.create_user("watcher", "", None, None, false)?;
            for u in [&a, &w] {
                db.set_membership("shop", u.id, ProjectRole::Member)?;
            }
            db.set_project_bot("shop", 123, "project_bot", "123:abcdefghijklmnopqrstuvwxyz")?;
            let bot = db.project_bot("shop")?.unwrap();
            db.link_channel(a.id, &bot.scope, "42")?;
            db.link_channel(w.id, &bot.scope, "43")?;
            Ok((a.id, w.id, db.create_user_token(w.id, "test")?))
        })
        .unwrap();
    let task = h
        .app
        .with_tracker("shop", |tr| {
            tr.create(
                &Actor::new("author", Role::Human),
                CreateInput { title: "Watch me".into(), status: Some(Status::Inbox), ..Default::default() },
            )
        })
        .unwrap();
    let url = format!("/api/tasks/{}/watch?project=shop", task.id);
    assert_eq!(call(&h.router, "PUT", &url).bearer(&token).send().await.1["watching"], true);
    h.app.with_tracker("shop", |tr| tr.comment(&Actor::new("third", Role::Human), &task.id, "A comment", CommentKind::Note)).unwrap();
    genie::engine::tick(&h.app).unwrap();
    genie::engine::tick(&h.app).unwrap();
    genie::channels::dispatch(&h.app).await;
    assert_eq!(sent.lock().unwrap().len(), 2, "author and follower, once each");
    assert!(sent.lock().unwrap().iter().all(|s| s["token"] == "bot123:abcdefghijklmnopqrstuvwxyz"));
    assert_eq!(h.app.with_server(|db| db.notifications(author, true, 50)).unwrap().len(), 1);
    h.app.with_tracker("shop", |tr| tr.comment(&Actor::new("watcher", Role::Human), &task.id, "My comment", CommentKind::Note)).unwrap();
    genie::engine::tick(&h.app).unwrap();
    genie::channels::dispatch(&h.app).await;
    assert_eq!(sent.lock().unwrap().len(), 3, "no notification about own comment");
    genie::tasks::comment(
        &h.app,
        &genie::tasks::Caller::person("shop", "author"),
        &task.id,
        genie::tasks::CommentBody { text: "@watcher please check".into(), kind: None },
    )
    .unwrap();
    genie::engine::tick(&h.app).unwrap();
    genie::channels::dispatch(&h.app).await;
    assert_eq!(sent.lock().unwrap().len(), 4, "a mention and follow produce one message for the same comment");
    h.app.with_server(|db| db.remove_membership("shop", watcher)).unwrap();
    h.app
        .with_tracker("shop", |tr| tr.set_status(&Actor::new("third", Role::Human), &task.id, Status::Cancelled, StatusOptions::default()))
        .unwrap();
    genie::engine::tick(&h.app).unwrap();
    genie::channels::dispatch(&h.app).await;
    assert_eq!(sent.lock().unwrap().len(), 5, "removed follower receives no message");
    assert_ne!(call(&h.router, "GET", &url).bearer(&token).send().await.0, 200);
}

#[tokio::test]
async fn bots_added_after_start_are_polled_and_offsets_are_persisted() {
    let (base, _, updates) = fake().await;
    let h = Harness::with_config(|cfg| cfg.telegram = Some(TelegramConfig { token: "LEGACY".into(), api_base: Some(base) }));
    h.project("shop");
    let mut poller = genie::telegram::Poller::new(h.app.clone());
    poller.reconcile().await.unwrap();
    let (scope, user, code) = h
        .app
        .with_server(|db| {
            let user = db.create_user("member", "", None, None, false)?;
            db.set_membership("shop", user.id, ProjectRole::Member)?;
            db.set_project_bot("shop", 123, "project_bot", "123:abcdefghijklmnopqrstuvwxyz")?;
            let bot = db.project_bot("shop")?.unwrap();
            Ok((bot.scope.clone(), user.id, db.bot_link_code(&bot, user.id)?))
        })
        .unwrap();
    updates
        .lock()
        .unwrap()
        .push(json!({"update_id": 20, "message": {"chat": {"id": 42, "type": "private"}, "text": format!("/start {code}")}}));
    poller.reconcile().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let bot = h.app.with_server(|db| db.project_bot("shop")).unwrap().unwrap();
            if bot.offset == 21 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(h.app.with_server(|db| db.channel_address(user, &scope)).unwrap().as_deref(), Some("42"));
}
