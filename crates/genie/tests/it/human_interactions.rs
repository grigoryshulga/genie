//! Human answers commit once across transports and survive the server/tracker commit gap.

use crate::common::{Harness, call};
use genie_core::inbox::NewQuestion;
use genie_core::server_db::ProjectRole;
use genie_core::{Actor, CreateInput, Role};
use serde_json::{Value, json};

fn person(h: &Harness, name: &str) -> (i64, String) {
    h.app
        .with_server(|db| {
            let u = db.create_user(name, "", Some(&format!("{name}@example.test")), None, false)?;
            db.set_membership("shop", u.id, ProjectRole::Member)?;
            db.link_channel(u.id, "telegram", if name == "anna" { "42" } else { "43" })?;
            Ok((u.id, db.create_user_token(u.id, "interactions")?))
        })
        .unwrap()
}

fn questionnaire(h: &Harness, user: i64, questions: &[NewQuestion]) -> (genie_core::inbox::Questionnaire, String) {
    h.app.with_server(|db| db.create_questionnaire("shop", None, "analyst", user, "web", questions, None, None, None)).unwrap()
}

#[tokio::test]
async fn personal_interaction_contract_is_versioned_and_opencode_commands_do_not_start_workers() {
    let h = Harness::new();
    h.project("shop");
    let (uid, token) = person(&h, "anna");
    let (_, other) = person(&h, "bob");
    let (qn, link) = questionnaire(
        &h,
        uid,
        &[NewQuestion { text: "Format?".into(), ..Default::default() }, NewQuestion { text: "Recipient?".into(), ..Default::default() }],
    );
    let path = format!("/api/interactions/shop:questionnaire:{}:1?project=shop", qn.id);
    let (s, items, _) = call(&h.remote, "GET", "/api/interactions?project=shop").bearer(&token).send().await;
    assert_eq!(s, 200, "{items}");
    assert_eq!(items.as_array().unwrap().len(), 2);
    assert_eq!(call(&h.remote, "GET", "/api/interactions?project=shop").bearer(&other).send().await.1, json!([]));
    assert_eq!(call(&h.remote, "GET", &path).bearer(&other).send().await.0, 403);
    let first = &items[0];
    let respond = format!("/api/interactions/{}/respond?project=shop", first["id"].as_str().unwrap());
    assert_eq!(
        call(&h.remote, "POST", &respond).bearer(&token).json(json!({"version":"old","action":"answer","text":"CSV"})).send().await.0,
        409
    );
    let batch = format!("/api/questions/{}/answer", qn.id);
    assert_eq!(call(&h.remote, "POST", &batch).bearer(&token).json(json!({"answers":{"1":"CSV","99":"bad"}})).send().await.0, 422);
    assert!(h.app.with_server(|db| db.questionnaire(qn.id)).unwrap().questions[0].answer.is_none());
    let body = json!({"version":first["version"],"action":"answer","text":"CSV"});
    assert_eq!(call(&h.remote, "POST", &respond).bearer(&token).json(body.clone()).send().await.0, 200);
    assert_eq!(call(&h.remote, "POST", &respond).bearer(&token).json(body).send().await.0, 200);
    let (_,mcp,_)=call(&h.remote,"POST","/mcp").bearer(&token).no_csrf().json(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"genie_interaction","arguments":{"action":"respond","decision":"answer","id":first["id"],"version":first["version"],"text":"CSV","project":"shop"}}})).send().await;
    assert_ne!(mcp["result"]["isError"], true, "{mcp}");
    assert!(mcp["result"]["content"][0]["text"].as_str().unwrap().contains("CSV"), "{mcp}");
    assert_eq!(call(&h.remote, "POST", &batch).bearer(&token).json(json!({"answers":{"1":"XLSX","2":"Bob"}})).send().await.0, 422);
    assert!(h.app.with_server(|db| db.questionnaire(qn.id)).unwrap().questions[1].answer.is_none());
    let second = &items[1];
    let input = format!("/respond {} {} answer Accounting", second["id"].as_str().unwrap(), second["version"].as_str().unwrap());
    let (s, v, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/responses")
        .bearer(&token)
        .json(json!({"model":"genie","input":input}))
        .send()
        .await;
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["status"], "completed");
    assert!(v.to_string().contains("Accounting"));
    let (_, v, _) = call(&h.remote, "POST", "/openai/projects/shop/v1/chat/completions")
        .bearer(&token)
        .json(json!({"model":"genie","messages":[{"role":"user","content":"/interactions"}]}))
        .send()
        .await;
    assert!(v["choices"][0]["message"]["content"].as_str().unwrap().contains("No pending interactions"));
    assert!(h.app.with_server(|db| db.turns("shop", None, 10)).unwrap().is_empty());
    h.app.with_server(|db| db.remove_membership("shop", uid)).unwrap();
    assert_eq!(
        call(&h.remote, "GET", &format!("/api/answer/{link}")).send().await.0,
        403,
        "old links do not bypass revoked project access"
    );
}

#[tokio::test]
async fn old_pause_reference_cannot_resume_a_later_pause() {
    let h = Harness::new();
    h.project("shop");
    let (_, token) = person(&h, "anna");
    let task = h
        .app
        .with_tracker("shop", |t| t.create(&Actor::new("anna", Role::Human), CreateInput { title: "Export".into(), ..Default::default() }))
        .unwrap();
    let stop = format!("/api/tasks/{}/stop?project=shop", task.id);
    assert_eq!(call(&h.remote, "POST", &stop).bearer(&token).json(json!({"note":"hold"})).send().await.0, 200);
    let (_, items, _) = call(&h.remote, "GET", "/api/interactions?project=shop").bearer(&token).send().await;
    let pause = items.as_array().unwrap().iter().find(|i| i["kind"] == "execution_pause").unwrap();
    let respond = format!("/api/interactions/{}/respond?project=shop", pause["id"].as_str().unwrap());
    let body = json!({"version":pause["version"],"action":"resume"});
    assert_eq!(call(&h.remote, "POST", &respond).bearer(&token).json(body.clone()).send().await.0, 200);
    assert_eq!(call(&h.remote, "POST", &respond).bearer(&token).json(body.clone()).send().await.0, 200);
    assert_eq!(call(&h.remote, "POST", &stop).bearer(&token).json(json!({"note":"new hold"})).send().await.0, 200);
    assert_eq!(call(&h.remote, "POST", &respond).bearer(&token).json(body).send().await.0, 422);
    assert!(h.app.with_tracker("shop", |t| t.execution().state(&task.id)).unwrap().unwrap().paused);
}

#[derive(Default)]
struct Telegram {
    fail: std::sync::atomic::AtomicBool,
    block: std::sync::atomic::AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    requests: std::sync::Mutex<Vec<(String, Value)>>,
}
async fn telegram() -> (String, std::sync::Arc<Telegram>) {
    use axum::{
        Json, Router,
        extract::{Path, State},
        routing::post,
    };
    async fn handler(
        State(state): State<std::sync::Arc<Telegram>>,
        Path((_, method)): Path<(String, String)>,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        state.requests.lock().unwrap().push((method.clone(), body));
        if method == "sendMessage" {
            if state.fail.load(std::sync::atomic::Ordering::SeqCst) {
                return Json(json!({"ok":false,"description":"simulated unavailable recipient"}));
            }
            if state.block.load(std::sync::atomic::Ordering::SeqCst) {
                state.entered.notify_one();
                state.release.notified().await;
            }
        }
        Json(json!({"ok":true,"result":{"message_id":501}}))
    }
    let state = std::sync::Arc::new(Telegram::default());
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", socket.local_addr().unwrap());
    let router = Router::new().route("/{bot}/{method}", post(handler)).with_state(state.clone());
    tokio::spawn(async move {
        axum::serve(socket, router).await.unwrap();
    });
    (url, state)
}
fn ask(h: &Harness, user: i64) -> i64 {
    genie::questions::ask(
        &h.app,
        genie::questions::Ask {
            project: "shop",
            task: None,
            asked_by: "analyst",
            recipient: user,
            questions: vec![NewQuestion { text: "Format?".into(), options: vec!["CSV".into(), "XLSX".into()], ..Default::default() }],
            run_step: None,
            remind_after: None,
            timeout: None,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn fallback_requires_explicit_preference_and_rechecks_recipient_rights() {
    let (url, tg) = telegram().await;
    tg.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let (port, mail) = super::channels::fake_smtp().await;
    let h = Harness::with_config(|cfg| {
        cfg.telegram = Some(genie::config::TelegramConfig { token: "TEST".into(), api_base: Some(url) });
        cfg.smtp = Some(genie::config::SmtpConfig {
            host: "127.0.0.1".into(),
            port,
            from: "genie@example.test".into(),
            security: "none".into(),
            ..Default::default()
        });
    });
    h.project("shop");
    let (uid, token) = person(&h, "anna");
    let first = ask(&h, uid);
    genie::channels::dispatch(&h.app).await;
    assert!(mail.lock().unwrap().is_empty());
    assert_eq!(h.app.with_server(|db| db.questionnaire(first)).unwrap().status, "open");
    let prefs = json!({"policy":{"order":["telegram","email"],"fallbackAfterAttempts":1}});
    assert_eq!(call(&h.remote, "PUT", "/api/me/delivery").bearer(&token).json(prefs).send().await.0, 200);
    assert_eq!(
        call(&h.remote, "PUT", "/api/me/delivery").bearer(&token).json(json!({})).send().await.0,
        400,
        "reset must be explicit, not a missing field"
    );
    let second = ask(&h, uid);
    genie::channels::dispatch(&h.app).await;
    genie::channels::dispatch(&h.app).await;
    assert_eq!(mail.lock().unwrap().len(), 1, "one configured fallback goes to the same person");
    assert_eq!(h.app.with_server(|db| db.questionnaire(second)).unwrap().status, "open", "delivery is not an answer or approval");
    ask(&h, uid);
    genie::channels::dispatch(&h.app).await;
    h.app.with_server(|db| db.remove_membership("shop", uid)).unwrap();
    genie::channels::dispatch(&h.app).await;
    assert_eq!(mail.lock().unwrap().len(), 1, "queued fallback does not survive revoked authority");
    assert_eq!(
        call(&h.remote, "PUT", "/api/me/delivery")
            .bearer(&token)
            .json(json!({"policy":{"order":["email","email"],"fallbackAfterAttempts":1}}))
            .send()
            .await
            .0,
        422
    );
}

#[tokio::test]
async fn resetting_preferences_cancels_a_queued_fallback_after_restart_and_keeps_legacy_routing() {
    let (url, tg) = telegram().await;
    tg.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let (port, mail) = super::channels::fake_smtp().await;
    let h = Harness::with_config(|cfg| {
        cfg.telegram = Some(genie::config::TelegramConfig { token: "TEST".into(), api_base: Some(url) });
        cfg.smtp = Some(genie::config::SmtpConfig {
            host: "127.0.0.1".into(),
            port,
            from: "genie@example.test".into(),
            security: "none".into(),
            ..Default::default()
        });
    });
    h.project("shop");
    let (uid, token) = person(&h, "anna");
    assert_eq!(
        call(&h.remote, "PUT", "/api/me/delivery")
            .bearer(&token)
            .json(json!({
                "policy":{"order":["telegram","email"],"fallbackAfterAttempts":1}
            }))
            .send()
            .await
            .0,
        200
    );
    let qn = ask(&h, uid);
    genie::channels::dispatch(&h.app).await;
    let email = h
        .app
        .with_server(|db| {
            let items = db.due_outbox(50)?;
            Ok(items.into_iter().find(|item| item.channel == "email").unwrap())
        })
        .unwrap();
    assert!(mail.lock().unwrap().is_empty(), "the fallback is queued, not yet dispatched");
    assert_eq!(call(&h.remote, "PUT", "/api/me/delivery").bearer(&token).json(json!({"policy":null})).send().await.0, 200);

    let recovered = genie::state::App::open(h.dir.path(), h.app.cfg.clone(), std::path::PathBuf::from("/nonexistent")).unwrap();
    assert!(recovered.with_server(|db| db.delivery_policy(uid)).unwrap().is_none());
    genie::channels::dispatch(&recovered).await;
    assert!(mail.lock().unwrap().is_empty(), "reset withdraws email fallback while Telegram remains linked");
    assert_eq!(recovered.with_server(|db| db.outbox_item(email.id)).unwrap().status, "cancelled");
    assert_eq!(recovered.with_server(|db| db.questionnaire(qn)).unwrap().status, "open");
    assert_eq!(tg.requests.lock().unwrap().iter().filter(|(method, _)| method == "sendMessage").count(), 1);

    // Reset restores Telegram-otherwise-email rather than disabling delivery.
    h.app.with_server(|db| db.unlink_channel(uid, "telegram")).unwrap();
    let legacy = ask(&h, uid);
    genie::channels::dispatch(&recovered).await;
    assert_eq!(mail.lock().unwrap().len(), 1, "email is still the default when Telegram is disconnected");
    assert_eq!(recovered.with_server(|db| db.questionnaire(legacy)).unwrap().status, "open");

    // A message's original channel restriction remains part of its authority.
    h.app.with_server(|db| db.link_channel(uid, "telegram", "42")).unwrap();
    genie::notify::send(
        &h.app,
        &[uid],
        &genie::notify::Message {
            kind: "test".into(),
            title: "Explicit email".into(),
            body: "Only this allowed channel".into(),
            project: Some("shop".into()),
            channels: vec!["email".into()],
            ..Default::default()
        },
        Some("email-only-after-reset"),
    )
    .unwrap();
    genie::channels::dispatch(&recovered).await;
    assert_eq!(mail.lock().unwrap().len(), 2, "explicit email-only messages retain their original restriction");
    assert_eq!(tg.requests.lock().unwrap().iter().filter(|(method, _)| method == "sendMessage").count(), 1);
}

#[tokio::test]
async fn undeliverable_interaction_stays_open_and_failure_is_visible_only_to_its_person() {
    let (url, tg) = telegram().await;
    tg.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let h = Harness::with_config(|cfg| {
        cfg.telegram = Some(genie::config::TelegramConfig { token: "DUMMY-PRIVATE-TOKEN".into(), api_base: Some(url) })
    });
    h.project("shop");
    let (uid, token) = person(&h, "anna");
    let qn = ask(&h, uid);
    for _ in 0..genie_core::inbox::MAX_DELIVERY_ATTEMPTS {
        h.app
            .with_server(|db| {
                db.conn().execute("UPDATE outbox SET next_at='2000-01-01' WHERE channel='telegram' AND status='pending'", [])?;
                Ok(())
            })
            .unwrap();
        genie::channels::dispatch(&h.app).await;
    }
    let notifications = h.app.with_server(|db| db.notifications(uid, false, 20)).unwrap();
    assert_eq!(notifications.iter().filter(|n| n.kind == "delivery_failed").count(), 1);
    let (_, item, _) =
        call(&h.remote, "GET", &format!("/api/interactions/shop:questionnaire:{qn}:1?project=shop")).bearer(&token).send().await;
    assert_eq!(item["state"], "open");
    assert!(item["deliveries"].as_array().unwrap().iter().any(|d| d["channel"] == "telegram" && d["status"] == "failed"));
    assert!(!item.to_string().contains("DUMMY-PRIVATE-TOKEN"));
    h.app
        .with_server(|db| {
            db.set_delivery_policy(uid, Some(&genie_core::human_delivery::DeliveryPolicy { order: vec![], fallback_after_attempts: 1 }))
        })
        .unwrap();
    ask(&h, uid);
    genie::channels::dispatch(&h.app).await;
    assert_eq!(
        tg.requests.lock().unwrap().iter().filter(|(m, _)| m == "sendMessage").count() as i64,
        genie_core::inbox::MAX_DELIVERY_ATTEMPTS
    );
}

#[tokio::test]
async fn failed_attempt_and_fallback_projection_roll_back_together_on_storage_failure() {
    let (url, tg) = telegram().await;
    tg.fail.store(true, std::sync::atomic::Ordering::SeqCst);
    let (port, mail) = super::channels::fake_smtp().await;
    let h = Harness::with_config(|cfg| {
        cfg.telegram = Some(genie::config::TelegramConfig { token: "TEST".into(), api_base: Some(url) });
        cfg.smtp = Some(genie::config::SmtpConfig {
            host: "127.0.0.1".into(),
            port,
            from: "genie@example.test".into(),
            security: "none".into(),
            ..Default::default()
        });
    });
    h.project("shop");
    let (uid, _) = person(&h, "anna");
    h.app
        .with_server(|db| {
            db.set_delivery_policy(
                uid,
                Some(&genie_core::human_delivery::DeliveryPolicy {
                    order: vec!["telegram".into(), "email".into()],
                    fallback_after_attempts: 1,
                }),
            )
        })
        .unwrap();
    ask(&h, uid);
    h.app.with_server(|db| {db.conn().execute_batch("CREATE TRIGGER fail_fallback BEFORE INSERT ON outbox WHEN NEW.channel='email' BEGIN SELECT RAISE(ABORT,'simulated storage failure'); END;")?;Ok(())}).unwrap();
    genie::channels::dispatch(&h.app).await;
    let (status, attempts) = h
        .app
        .with_server(|db| {
            Ok(db.conn().query_row("SELECT status,attempts FROM outbox WHERE channel='telegram'", [], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })?)
        })
        .unwrap();
    assert_eq!((status.as_str(), attempts), ("delivering", 0), "do not commit a final failure before its fallback exists");
    h.app
        .with_server(|db| {
            db.conn().execute_batch("DROP TRIGGER fail_fallback; UPDATE outbox SET next_at='2000-01-01' WHERE channel='telegram';")?;
            Ok(())
        })
        .unwrap();
    genie::channels::dispatch(&h.app).await;
    genie::channels::dispatch(&h.app).await;
    assert_eq!(mail.lock().unwrap().len(), 1, "recovery creates one fallback after the interrupted transaction");
}

#[tokio::test]
async fn an_answer_during_send_updates_the_eventual_telegram_copy_without_sending_twice() {
    let (url, tg) = telegram().await;
    tg.block.store(true, std::sync::atomic::Ordering::SeqCst);
    let h = Harness::with_config(|cfg| cfg.telegram = Some(genie::config::TelegramConfig { token: "TEST".into(), api_base: Some(url) }));
    h.project("shop");
    let (uid, token) = person(&h, "anna");
    let qn = ask(&h, uid);
    let app = h.app.clone();
    let sending = tokio::spawn(async move { genie::channels::dispatch(&app).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), tg.entered.notified()).await.unwrap();
    assert_eq!(
        call(&h.remote, "POST", &format!("/api/questions/{qn}/answer")).bearer(&token).json(json!({"answers":{"1":"CSV"}})).send().await.0,
        200
    );
    genie::interactions::reconcile(&h.app, "shop").unwrap();
    genie::channels::dispatch(&h.app).await; // Sync sees the in-flight send and waits for its reference.
    tg.release.notify_one();
    sending.await.unwrap();
    genie::channels::dispatch(&h.app).await;
    let requests = tg.requests.lock().unwrap().clone();
    assert_eq!(requests.iter().filter(|(m, _)| m == "sendMessage").count(), 1);
    let edited = requests.iter().find(|(m, _)| m == "editMessageText").expect("a late send still loses its obsolete buttons");
    assert_eq!(edited.1["message_id"], 501);
    assert_eq!(edited.1["reply_markup"]["inline_keyboard"], json!([]));
    tg.block.store(false, std::sync::atomic::Ordering::SeqCst);
    let update = json!({"message":{"chat":{"id":42,"type":"private"},"text":"XLSX","reply_to_message":{"message_id":501}}});
    genie::channels::handle_update(&h.app, &update).await.unwrap();
    assert_eq!(h.app.with_server(|db| db.questionnaire(qn)).unwrap().questions[0].answer.as_deref(), Some("CSV"));
    let web = h.app.with_server(|db| db.notifications(uid, false, 10)).unwrap();
    assert_eq!(web[0].kind, "interaction_closed");
    assert!(web[0].read_at.is_none(), "resolution does not impersonate reading");
}

#[test]
fn questionnaire_answers_require_current_recipient_authority_and_replay_task_effect_once() {
    let h = Harness::new();
    h.project("shop");
    let user = h.app.with_server(|db| db.create_user("anna", "", None, None, false)).unwrap();
    h.app.with_server(|db| db.set_membership("shop", user.id, ProjectRole::Member)).unwrap();
    let task = h
        .app
        .with_tracker("shop", |t| t.create(&Actor::new("anna", Role::Human), CreateInput { title: "Export".into(), ..Default::default() }))
        .unwrap();
    let (qn, _) = h
        .app
        .with_server(|db| {
            db.create_questionnaire(
                "shop",
                Some(&task.id),
                "analyst",
                user.id,
                "web",
                &[NewQuestion { text: "Format?".into(), ..Default::default() }],
                None,
                None,
                None,
            )
        })
        .unwrap();
    h.app.with_server(|db| db.remove_membership("shop", user.id)).unwrap();
    assert!(genie::questions::answer(&h.app, user.id, qn.id, 1, "CSV", "web").is_err());
    assert!(h.app.with_server(|db| db.questionnaire(qn.id)).unwrap().questions[0].answer.is_none());
    h.app.with_server(|db| db.set_membership("shop", user.id, ProjectRole::Member)).unwrap();

    // Crash point 1: the accepted answer and continuation exist, but the tracker has not received it.
    h.app
        .with_server(|db| {
            db.tx(|| {
                db.answer_question(qn.id, 1, "CSV", "telegram")?;
                db.queue_answer_record(qn.id, "anna", "Ответы на вопросы: CSV")
            })
        })
        .unwrap();
    let pending = h.app.with_server(|db| db.due_answer_records()).unwrap().remove(0);
    // Crash point 2: task comment/mail committed, while the server still sees the record as pending.
    h.app.with_tracker("shop", |t| t.comment_once(&pending.id, &Actor::new("anna", Role::Human), &task.id, &pending.body)).unwrap();
    genie::questions::flush_records(&h.app).unwrap();
    genie::questions::flush_records(&h.app).unwrap();
    assert!(h.app.with_server(|db| db.due_answer_records()).unwrap().is_empty());
    let task = h.app.with_tracker("shop", |t| t.get(&task.id)).unwrap();
    assert_eq!(task.comments.iter().filter(|c| c.text == pending.body).count(), 1);
    let (_, done) = genie::questions::answer(&h.app, user.id, qn.id, 1, "CSV", "api").unwrap();
    assert!(done);
    assert!(genie::questions::answer(&h.app, user.id, qn.id, 1, "XLSX", "api").is_err());
    assert!(h.app.with_tracker("shop", |t| t.comment_once(&pending.id, &Actor::new("anna", Role::Human), &task.id, "rebound")).is_err());
}

#[test]
fn questionnaire_creation_and_reminder_delivery_replay_without_invalidating_previous_links() {
    let h = Harness::new();
    h.project("shop");
    let (uid, _) = person(&h, "anna");
    let (qn, original) = h
        .app
        .with_server(|db| {
            db.tx(|| {
                let result = db.create_questionnaire(
                    "shop",
                    None,
                    "analyst",
                    uid,
                    "web",
                    &[NewQuestion { text: "When?".into(), ..Default::default() }],
                    None,
                    None,
                    None,
                )?;
                db.queue_questionnaire_send(result.0.id, false)?;
                Ok(result)
            })
        })
        .unwrap();
    assert!(h.app.with_server(|db| db.notifications(uid, false, 10)).unwrap().is_empty());
    genie::questions::flush_sends(&h.app).unwrap();
    // Rehearse a crash after outbox projection but before acknowledging the send intent.
    h.app
        .with_server(|db| {
            db.conn().execute("UPDATE questionnaire_sends SET status='pending' WHERE questionnaire=?1", [qn.id])?;
            Ok(())
        })
        .unwrap();
    genie::questions::flush_sends(&h.app).unwrap();
    assert_eq!(h.app.with_server(|db| db.notifications(uid, false, 10)).unwrap().len(), 1);
    h.app.with_server(|db| db.queue_questionnaire_send(qn.id, true)).unwrap();
    genie::questions::flush_sends(&h.app).unwrap();
    assert_eq!(h.app.with_server(|db| db.notifications(uid, false, 10)).unwrap().len(), 2);
    assert_eq!(h.app.with_server(|db| db.questionnaire_by_secret(&original)).unwrap().unwrap().id, qn.id);
    assert!(h.app.with_server(|db| db.due_questionnaire_sends()).unwrap().is_empty());
}
