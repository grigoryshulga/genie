//! E-mail delivery through a minimal in-process SMTP server, and retries when
//! the server is down.

use crate::common;

use std::sync::{Arc, Mutex};

use common::Harness;
use genie::config::SmtpConfig;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Accepts one session at a time and records the DATA of each message.
async fn fake_smtp() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let got = Arc::new(Mutex::new(Vec::new()));
    let store = got.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { return };
            let store = store.clone();
            tokio::spawn(async move {
                let (r, mut w) = sock.into_split();
                let mut lines = BufReader::new(r).lines();
                w.write_all(b"220 fake ESMTP\r\n").await.unwrap();
                let mut data: Option<String> = None;
                while let Ok(Some(line)) = lines.next_line().await {
                    if let Some(buf) = data.as_mut() {
                        if line == "." {
                            store.lock().unwrap().push(data.take().unwrap());
                            w.write_all(b"250 queued as 42\r\n").await.unwrap();
                        } else {
                            buf.push_str(&line);
                            buf.push('\n');
                        }
                        continue;
                    }
                    let upper = line.to_uppercase();
                    let reply: &[u8] = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                        b"250 fake\r\n"
                    } else if upper.starts_with("DATA") {
                        data = Some(String::new());
                        b"354 go\r\n"
                    } else if upper.starts_with("QUIT") {
                        w.write_all(b"221 bye\r\n").await.unwrap();
                        break;
                    } else {
                        b"250 ok\r\n"
                    };
                    w.write_all(reply).await.unwrap();
                }
            });
        }
    });
    (port, got)
}

#[tokio::test]
async fn email_is_delivered_and_failures_are_retried() {
    let (port, got) = fake_smtp().await;
    let h = Harness::with_config(|c| {
        c.smtp = Some(SmtpConfig {
            host: "127.0.0.1".into(),
            port,
            from: "genie <genie@example.com>".into(),
            security: "none".into(),
            ..Default::default()
        });
    });
    h.project("shop");
    let anna = h.app.with_server(|db| db.create_user("anna", "Анна", Some("anna@example.com"), None, true)).unwrap();
    let msg = genie::notify::Message {
        kind: "done".into(),
        title: "SHOP-1 готова".into(),
        body: "Экспорт работает".into(),
        link: Some("/done?task=SHOP-1".into()),
        ..Default::default()
    };
    assert_eq!(genie::notify::send(&h.app, &[anna.id], &msg, Some("t1")).unwrap(), 1);
    assert_eq!(genie::notify::send(&h.app, &[anna.id], &msg, Some("t1")).unwrap(), 0, "the same notification is queued once");
    genie::channels::dispatch(&h.app).await;
    let mails = got.lock().unwrap().clone();
    assert_eq!(mails.len(), 1);
    assert!(mails[0].contains("Subject: [genie]") && mails[0].contains("anna@example.com"), "{}", mails[0]);
    let web = h.app.with_server(|db| db.notifications(anna.id, true, 10)).unwrap();
    assert_eq!(web.len(), 1, "the web notification center gets it too");

    // Server down: the message stays queued for a retry with backoff.
    let h2 = Harness::with_config(|c| {
        c.smtp = Some(SmtpConfig {
            host: "127.0.0.1".into(),
            port: 1,
            from: "genie@example.com".into(),
            security: "none".into(),
            ..Default::default()
        });
    });
    let bob = h2.app.with_server(|db| db.create_user("bob", "", Some("bob@example.com"), None, false)).unwrap();
    genie::notify::send(&h2.app, &[bob.id], &msg, None).unwrap();
    genie::channels::dispatch(&h2.app).await;
    let (status, attempts, err): (String, i64, Option<String>) = h2
        .app
        .with_server(|db| {
            Ok(db.conn().query_row("SELECT status, attempts, last_error FROM outbox", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?)
        })
        .unwrap();
    assert_eq!((status.as_str(), attempts), ("pending", 1));
    assert!(err.unwrap().contains("smtp"));
    let _ = json!({});
}
