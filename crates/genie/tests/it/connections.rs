//! Connecting a GitHub account through the deployment's GitHub App, picking repositories and
//! binding them to a project, against a fake GitHub that speaks the App's endpoints.

use crate::common;

use axum::http::{StatusCode, header};
use common::fakeapp::{self, AppShared};
use common::fakehost::{self, FakeHost};
use common::githost::upstream;
use common::*;
use genie::config::GithubApp;
use genie_core::server_db::ProjectRole;
use serde_json::{Value, json};

struct Rig {
    h: Harness,
    fake: FakeHost,
    app: AppShared,
    ann: String,
    bob: String,
    admin: String,
    ann_id: i64,
    port: u16,
}

const MINTED: &str = "ghs_minted";

/// A PKCS#1 PEM key of the form GitHub hands out, written where the config points.
fn private_key(dir: &std::path::Path) -> String {
    use aws_lc_rs::encoding::AsDer;
    use base64::Engine;
    let key = aws_lc_rs::signature::RsaKeyPair::generate(aws_lc_rs::rsa::KeySize::Rsa2048).unwrap();
    let pkcs8 = AsDer::<aws_lc_rs::encoding::Pkcs8V1Der>::as_der(&key).unwrap();
    let b64 = base64::engine::general_purpose::STANDARD.encode(&pkcs8.as_ref()[26..]);
    let lines: Vec<String> = b64.as_bytes().chunks(64).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
    let path = dir.join("app.pem");
    std::fs::write(&path, format!("-----BEGIN RSA PRIVATE KEY-----\n{}\n-----END RSA PRIVATE KEY-----\n", lines.join("\n"))).unwrap();
    path.to_string_lossy().into_owned()
}

async fn rig() -> Rig {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // The secret the config names; set before the server reads it.
    unsafe { std::env::set_var("GENIE_TEST_GH_APP_SECRET", "shh") };
    let tmp = tempfile::tempdir().unwrap();
    let key = private_key(tmp.path());
    let h = Harness::with_config(|c| {
        c.port = port;
        c.public_url = Some(format!("http://127.0.0.1:{port}/"));
        c.git_apps.github = Some(GithubApp {
            host: "h".into(),
            app_id: 4242,
            slug: "genie-test".into(),
            client_id: "cid".into(),
            client_secret_env: "GENIE_TEST_GH_APP_SECRET".into(),
            private_key_file: key,
        });
    });
    let hosts = h.dir.path().join("hosts");
    std::fs::create_dir_all(&hosts).unwrap();
    upstream(&hosts, "acme/api", &[("README.md", "api\n")]);
    let app = fakeapp::state(MINTED);
    let fake = fakehost::spawn_with("github", MINTED, Some(fakeapp::routes(app.clone()))).await;
    let cfg = json!({ "hosts": { "h": {
        "kind": "github", "url": fake.url,
        "clone_urls": { "https": format!("file://{}/{{remote}}.git", hosts.display()) }
    } } });
    std::fs::write(h.dir.path().join("git.json"), cfg.to_string()).unwrap();
    h.project("shop");
    tokio::spawn(genie::serve_on(h.app.clone(), listener, std::future::pending()));
    let (ann_id, ann, bob, admin) = h
        .app
        .with_server(|db| {
            let user = |login: &str, admin: bool, role: Option<ProjectRole>| {
                let u = db.create_user(login, login, None, Some("a-long-enough-password"), admin).unwrap();
                if let Some(r) = role {
                    db.set_membership("shop", u.id, r).unwrap();
                }
                (u.id, db.create_user_token(u.id, "test").unwrap())
            };
            let (ann_id, ann) = user("ann", false, Some(ProjectRole::Admin));
            let (_, bob) = user("bob", false, Some(ProjectRole::Member));
            let (_, admin) = user("root", true, None);
            Ok((ann_id, ann, bob, admin))
        })
        .unwrap();
    // The directory with the key must outlive the test.
    std::mem::forget(tmp);
    Rig { h, fake, app, ann, bob, admin, ann_id, port }
}

fn host_header(r: &Rig) -> String {
    format!("127.0.0.1:{}", r.port)
}

/// A request as a person at the web UI makes it.
fn api<'a>(r: &'a Rig, method: &str, path: &str, who: &str) -> Call<'a> {
    call(&r.h.remote, method, path).bearer(who).header("x-genie-project", "shop").header("host", &host_header(r))
}

/// The two legs of the browser flow. Returns where the browser ends up.
async fn browser_flow(r: &Rig, installation: i64) -> String {
    let (status, body, cookies) = api(r, "POST", "/api/git/github/connect", &r.ann).json(json!({})).send().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let url = body["url"].as_str().unwrap().to_string();
    assert!(url.starts_with(&format!("{}/apps/genie-test/installations/new?state=", r.fake.url)), "{url}");
    let state = url.split("state=").nth(1).unwrap().to_string();
    let cookie = cookies.iter().find(|c| c.starts_with("genie_git_flow=")).unwrap().clone();

    // Setup leg: GitHub brings the person back with a claim about the installation.
    let (status, headers, _) =
        call(&r.h.remote, "GET", &format!("/api/git/github/callback?state={state}&installation_id={installation}&setup_action=install"))
            .no_csrf()
            .cookie(&cookie)
            .header("host", &host_header(r))
            .send_raw()
            .await;
    assert!(status.is_redirection(), "{status}");
    let location = headers.get(header::LOCATION).unwrap().to_str().unwrap().to_string();
    assert!(location.starts_with(&format!("{}/login/oauth/authorize?", r.fake.url)), "{location}");
    assert!(location.contains("code_challenge_method=S256") && location.contains(&format!("state={state}")), "{location}");
    let challenge = location.split("code_challenge=").nth(1).unwrap().split('&').next().unwrap().to_string();
    r.app.lock().unwrap().challenge = Some(challenge);

    // Authorization leg: the code is exchanged and the installation is checked against the person's own.
    let (status, headers, _) = call(&r.h.remote, "GET", &format!("/api/git/github/callback?state={state}&code=the-code"))
        .no_csrf()
        .cookie(&cookie)
        .header("host", &host_header(r))
        .send_raw()
        .await;
    assert!(status.is_redirection(), "{status}");
    headers.get(header::LOCATION).unwrap().to_str().unwrap().to_string()
}

fn connection_id(location: &str) -> String {
    location.split("connection=").nth(1).unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_connects_picks_a_repository_and_the_project_reaches_it_with_a_narrow_token() {
    let r = rig().await;

    // Only an administrator of a project connects.
    let (status, _, _) = api(&r, "POST", "/api/git/github/connect", &r.bob).json(json!({})).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let location = browser_flow(&r, 42).await;
    assert!(location.starts_with("/settings/project/repositories?connection=gc_"), "{location}");
    let id = connection_id(&location);

    let (_, list, _) = api(&r, "GET", "/api/git/connections", &r.ann).send().await;
    assert_eq!(list["github"]["configured"], true);
    let c = &list["connections"][0];
    assert_eq!(
        (c["id"].as_str(), c["health"].as_str(), c["accountLogin"].as_str(), c["ownerLogin"].as_str()),
        (Some(id.as_str()), Some("ok"), Some("acme"), Some("ann"))
    );
    assert!(c.get("manageUrl").is_some());
    // Somebody else does not see it; a server administrator sees all.
    let (_, other, _) = api(&r, "GET", "/api/git/connections", &r.bob).send().await;
    assert_eq!(other["connections"].as_array().unwrap().len(), 0);
    let (_, all, _) = api(&r, "GET", "/api/git/connections", &r.admin).send().await;
    assert_eq!(all["connections"].as_array().unwrap().len(), 1);

    // The picker offers what the person and the installation both reach.
    let (status, picker, _) = api(&r, "GET", &format!("/api/git/connections/{id}/repositories"), &r.ann).send().await;
    assert_eq!(status, StatusCode::OK, "{picker}");
    let names: Vec<&str> = picker["repositories"].as_array().unwrap().iter().map(|x| x["fullName"].as_str().unwrap()).collect();
    assert_eq!(names, ["acme/api", "acme/web"]);
    let (status, _, _) = api(&r, "GET", &format!("/api/git/connections/{id}/repositories"), &r.bob).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    // A repository outside the offer cannot be bound; one inside binds with the provider's own facts.
    let (status, body, _) =
        api(&r, "POST", "/api/repos", &r.ann).json(json!({ "name": "evil", "connection": id, "providerRepoId": 999 })).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let (status, body, _) = api(&r, "POST", "/api/repos", &r.ann)
        .json(json!({ "name": "api", "connection": id, "providerRepoId": 501, "host": "other", "remote": "other/repo", "mount": "." }))
        .send()
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert_eq!(
        (body["remote"].as_str(), body["host"]["id"].as_str(), body["connection"].as_str()),
        (Some("acme/api"), Some("h"), Some(id.as_str()))
    );
    assert!(body["warning"].is_null(), "{body}");
    // The token was minted for that one repository, from an app JWT of this app.
    assert_eq!(r.app.lock().unwrap().mints.last(), Some(&vec![501]));
    assert!(r.app.lock().unwrap().jwt_issuers.iter().all(|i| *i == 4242));

    // The repository now reaches the host with the minted token (the fake accepts nothing else).
    let (status, check, _) = api(&r, "POST", "/api/repos/api/check", &r.ann).json(json!({})).send().await;
    assert_eq!(status, StatusCode::OK, "{check}");
    assert!(r.fake.lock().calls.iter().any(|c| c == "repo"), "{:?}", r.fake.lock().calls);

    // A connected repository takes no token of its own and never allows force push or deleting branches.
    let (status, _, _) = api(&r, "PATCH", "/api/repos/api", &r.ann).json(json!({ "token": "pat" })).send().await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _, _) = api(&r, "PATCH", "/api/repos/api", &r.ann).json(json!({ "policy": { "force_push": true } })).send().await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    // The grant is in the audit log and in the project's journal.
    let changes = r.h.app.with_server(|db| db.config_changes(Some(&format!("git.connection:{id}")), 10)).unwrap();
    let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
    assert!(paths.contains(&"connected") && paths.contains(&"granted"), "{paths:?}");
    let journal = r.h.app.with_tracker("shop", |t| genie_core::events::latest_of(t.conn(), "git.connection", 50)).unwrap();
    assert_eq!(journal.len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_forged_installation_is_not_accepted() {
    let r = rig().await;
    // GitHub's setup address can be hit by anyone with any installation id: the person's own list decides.
    let location = browser_flow(&r, 777).await;
    assert_eq!(location, "/settings/project/repositories?gitError=1");
    let (_, list, _) = api(&r, "GET", "/api/git/connections", &r.ann).send().await;
    assert_eq!(list["connections"].as_array().unwrap().len(), 0);
    // A wrong browser (no flow cookie) gets nothing either.
    let (status, headers, _) = call(&r.h.remote, "GET", "/api/git/github/callback?state=whatever&code=the-code")
        .no_csrf()
        .header("host", &host_header(&r))
        .send_raw()
        .await;
    assert!(status.is_redirection());
    assert_eq!(headers.get(header::LOCATION).unwrap(), "/settings/project/repositories?gitError=1");
}

async fn connected(r: &Rig) -> String {
    let id = connection_id(&browser_flow(r, 42).await);
    let (status, body, _) = api(r, "POST", "/api/repos", &r.ann)
        .json(json!({ "name": "api", "connection": id, "providerRepoId": 501, "mount": "." }))
        .send()
        .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    id
}

fn mints(r: &Rig) -> usize {
    r.app.lock().unwrap().mints.len()
}

#[tokio::test(flavor = "multi_thread")]
async fn disconnecting_stops_every_operation_and_connecting_again_revives_it() {
    let r = rig().await;
    let id = connected(&r).await;
    let before = mints(&r);

    // Only the owner (or a server administrator) disconnects.
    let (status, _, _) = api(&r, "DELETE", &format!("/api/git/connections/{id}"), &r.bob).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body, _) = api(&r, "DELETE", &format!("/api/git/connections/{id}"), &r.ann).send().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["health"], "revoked");

    let (status, body, _) = api(&r, "POST", "/api/repos/api/sync", &r.ann).json(json!({})).send().await;
    assert!(!status.is_success() && body.to_string().contains("disconnected by ann"), "{status} {body}");
    assert_eq!(mints(&r), before, "no token is minted for a disconnected connection");
    // The repository and its history stay.
    let (_, repos, _) = api(&r, "GET", "/api/repos", &r.ann).send().await;
    assert_eq!(repos.as_array().unwrap().len(), 1);

    // Connecting the same installation again brings the same connection back.
    let again = connection_id(&browser_flow(&r, 42).await);
    assert_eq!(again, id);
    let (status, body, _) = api(&r, "POST", "/api/repos/api/sync", &r.ann).json(json!({})).send().await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn removing_the_owner_blocks_operations_until_an_administrator_reassigns_the_connection() {
    let r = rig().await;
    let id = connected(&r).await;
    let before = mints(&r);

    r.h.app.with_server(|db| db.delete_user(r.ann_id)).unwrap();
    let (status, body, _) = api(&r, "POST", "/api/repos/api/sync", &r.admin).json(json!({})).send().await;
    assert!(!status.is_success() && body.to_string().contains("reassign"), "{status} {body}");
    assert_eq!(mints(&r), before);
    let (_, all, _) = api(&r, "GET", "/api/git/connections", &r.admin).send().await;
    assert_eq!(all["connections"][0]["health"], "owner_removed");

    // Only a server administrator reassigns, to an active person.
    let (status, _, _) =
        api(&r, "POST", &format!("/api/git/connections/{id}/reassign"), &r.bob).json(json!({ "login": "bob" })).send().await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, body, _) =
        api(&r, "POST", &format!("/api/git/connections/{id}/reassign"), &r.admin).json(json!({ "login": "bob" })).send().await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!((body["health"].as_str(), body["ownerLogin"].as_str()), (Some("ok"), Some("bob")));
    let (status, body, _) = api(&r, "POST", "/api/repos/api/sync", &r.admin).json(json!({})).send().await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_provider_removing_or_suspending_the_installation_is_noticed() {
    let r = rig().await;
    let id = connected(&r).await;

    r.app.lock().unwrap().suspended = true;
    let (_, body, _) = api(&r, "POST", &format!("/api/git/connections/{id}/check"), &r.ann).json(json!({})).send().await;
    assert_eq!(body["health"], "suspended", "{body}");
    let (status, body, _) = api(&r, "POST", "/api/repos/api/sync", &r.ann).json(json!({})).send().await;
    assert!(!status.is_success() && body.to_string().contains("suspended"), "{status} {body}");

    r.app.lock().unwrap().suspended = false;
    let (_, body, _) = api(&r, "POST", &format!("/api/git/connections/{id}/check"), &r.ann).json(json!({})).send().await;
    assert_eq!(body["health"], "ok", "{body}");

    r.app.lock().unwrap().removed = true;
    let (_, body, _) = api(&r, "POST", &format!("/api/git/connections/{id}/check"), &r.ann).json(json!({})).send().await;
    assert_eq!(body["health"], "revoked", "{body}");
    let v: Value = body;
    assert!(v["statusNote"].as_str().unwrap().contains("no longer exists"), "{v}");
}
