//! Project tool choices restrict inherited grants without changing other projects.
use crate::common::{Harness, call};
use axum::http::StatusCode;
use genie_core::server_db::ProjectRole;
use serde_json::json;

#[tokio::test]
async fn project_admins_select_tools_without_editing_server_connections() {
    let h = Harness::new();
    h.project("shop");
    h.project("other");
    std::fs::write(
        h.app.data.join("mcp.json"),
        r#"{"mcpServers":{"docs":{"command":"echo"},"private":{"command":"echo","projects":["other"]}}}"#,
    )
    .unwrap();
    h.app.reload_agents();
    let (pm, member, root) = h
        .app
        .with_server(|db| {
            let pm = db.create_user("pm", "PM", None, None, false)?;
            let member = db.create_user("member", "Member", None, None, false)?;
            let root = db.create_user("root", "Root", None, None, true)?;
            db.set_membership("shop", pm.id, ProjectRole::Admin)?;
            db.set_membership("shop", member.id, ProjectRole::Member)?;
            Ok((db.create_user_token(pm.id, "test")?, db.create_user_token(member.id, "test")?, db.create_user_token(root.id, "test")?))
        })
        .unwrap();
    let path = "/api/projects/shop/tools";
    {
        let token = &member;
        let (s, _, _) = call(&h.router, "GET", path).bearer(token).send().await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (s, _, _) = call(&h.router, "PUT", path).bearer(token).json(json!({"settings":{"mcp":[]},"baseHash":""})).send().await;
        assert_eq!(s, StatusCode::FORBIDDEN);
    }
    let (s, view, _) = call(&h.router, "GET", path).bearer(&pm).send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(view["hash"], "");
    assert!(view["mcp"].as_array().unwrap().iter().all(|s| s["id"] != "private"));
    for invalid in [json!({"mcp":["private"]}), json!({"skills":["missing"]})] {
        let (s, _, _) = call(&h.router, "PUT", path).bearer(&pm).json(json!({"settings":invalid,"baseHash":""})).send().await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }
    let (s, _, _) = call(&h.router, "PUT", path).bearer(&pm).json(json!({"settings":{"skills":[],"mcp":[]},"baseHash":""})).send().await;
    assert_eq!(s, StatusCode::OK);
    let agents = h.app.agents();
    let mut role = agents.roles["executor"].clone();
    role.mcp = vec!["*".into()];
    assert!(agents.mcp_for("shop", &role).is_empty());
    assert_eq!(agents.mcp_for("other", &role).len(), 2);
    assert_eq!(agents.project_tools["shop"].skills, Some(vec![]));
    let (s, _, _) = call(&h.router, "PUT", path).bearer(&root).json(json!({"settings":{"mcp":["docs"]},"baseHash":""})).send().await;
    assert_eq!(s, StatusCode::CONFLICT, "stale editors cannot overwrite the selection");
    let (s, _, _) = call(&h.router, "GET", "/api/projects/other/tools").bearer(&pm).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, _) = call(&h.router, "PUT", "/api/mcp").bearer(&pm).json(json!({"content":"{}"})).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (_, view, _) = call(&h.router, "GET", path).bearer(&root).send().await;
    let (s, _, _) =
        call(&h.router, "PUT", path).bearer(&root).json(json!({"settings":{"mcp":["docs"]},"baseHash":view["hash"]})).send().await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(h.app.agents().mcp_for("shop", &role).len(), 1);
    std::fs::write(h.app.data.join("project-tools/shop.json"), "broken").unwrap();
    h.app.reload_agents();
    assert_eq!(h.app.agents().mcp_for("shop", &role).len(), 1, "bad files retain the previous restriction");
    let cold = genie::agent_config::AgentConfig::load(&h.app.data, &h.app.cfg, None);
    assert!(cold.mcp_for("shop", &role).is_empty(), "a cold load of a broken restriction fails closed");
}

#[tokio::test]
async fn deleting_accounts_is_admin_only_and_revokes_tokens() {
    let h = Harness::new();
    h.project("shop");
    let (root_id, root, user_id, user) = h
        .app
        .with_server(|db| {
            let root = db.create_user("root", "Root", None, None, true)?;
            let user = db.create_user("user", "User", None, None, false)?;
            db.set_membership("shop", user.id, ProjectRole::Admin)?;
            Ok((root.id, db.create_user_token(root.id, "t")?, user.id, db.create_user_token(user.id, "t")?))
        })
        .unwrap();
    let path = format!("/api/users/{user_id}");
    let (s, _, _) = call(&h.router, "DELETE", &path).bearer(&user).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    let (s, _, _) = call(&h.router, "DELETE", &format!("/api/users/{root_id}")).bearer(&root).send().await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _, _) = call(&h.router, "DELETE", &path).bearer(&root).send().await;
    assert_eq!(s, StatusCode::OK);
    let (s, _, _) = call(&h.router, "GET", "/api/auth/me").bearer(&user).send().await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert!(h.app.with_server(|db| db.members_of("shop")).unwrap().is_empty());
    assert!(h.app.with_server(|db| db.delete_user(root_id)).is_err());
}
