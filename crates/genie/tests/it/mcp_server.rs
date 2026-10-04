//! The genie MCP server: the catalog's operations as tools, one per group, for
//! a person's own agent and for genie's agents. The token decides which actions
//! a caller sees, and every call goes through the API with it.

use crate::common;

use std::collections::HashSet;

use axum::http::StatusCode;
use common::{Harness, call};
use genie_core::Role;
use genie_core::server_db::ProjectRole;
use genie_core::work::NewJob;
use serde_json::{Value, json};

fn person(h: &Harness, login: &str, admin: bool, role: Option<ProjectRole>) -> String {
    let u = h.app.with_server(|db| db.create_user(login, login, None, Some("password-1"), admin)).unwrap();
    if let Some(r) = role {
        h.app.with_server(|db| db.set_membership("shop", u.id, r)).unwrap();
    }
    h.app.with_server(|db| db.create_user_token(u.id, "mcp")).unwrap()
}

fn agent(h: &Harness, role: Role, name: &str, team: Option<&str>, job: Option<i64>) -> String {
    h.app.with_server(|db| db.create_agent_token("shop", role, name, team, job, chrono::Duration::hours(1))).unwrap()
}

/// A JSON-RPC request to `/mcp` with a token, from another machine as MCP clients connect.
async fn rpc(h: &Harness, token: &str, method: &str, params: Value) -> Value {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let (s, v, _) = call(&h.remote, "POST", "/mcp").bearer(token).no_csrf().json(body).send().await;
    assert_eq!(s, StatusCode::OK, "{v}");
    v
}

/// A tool call: its text and whether it is an error.
async fn tool(h: &Harness, token: &str, name: &str, args: Value) -> (String, bool) {
    let v = rpc(h, token, "tools/call", json!({ "name": name, "arguments": args })).await;
    let r = &v["result"];
    assert!(r.is_object(), "a tool answers with a result: {v}");
    (r["content"][0]["text"].as_str().unwrap_or_default().to_string(), r["isError"] == json!(true))
}

/// The actions of a tool in a `tools/list` answer (empty without the tool).
fn actions(list: &Value, name: &str) -> Vec<String> {
    list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["name"] == name)
        .map(|t| {
            t["inputSchema"]["properties"]["action"]["enum"].as_array().unwrap().iter().map(|a| a.as_str().unwrap().to_string()).collect()
        })
        .unwrap_or_default()
}

fn tool_def<'a>(list: &'a Value, name: &str) -> &'a Value {
    list["result"]["tools"].as_array().unwrap().iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("no tool {name}"))
}

#[tokio::test]
async fn a_person_drives_genie_from_their_own_agent() {
    let h = Harness::new();
    h.project("shop");
    h.project("other");
    let anna = person(&h, "anna", false, Some(ProjectRole::Member));

    let init =
        rpc(&h, &anna, "initialize", json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test" } }))
            .await;
    assert_eq!(init["result"]["serverInfo"]["name"], "genie");
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(init["result"]["capabilities"]["tools"].is_object());
    assert!(init["result"]["instructions"].as_str().unwrap().contains("the person anna"));
    let newest = rpc(&h, &anna, "initialize", json!({ "protocolVersion": "1999-01-01" })).await;
    assert_eq!(newest["result"]["protocolVersion"], "2025-06-18", "an unknown version gets the newest");

    let list = rpc(&h, &anna, "tools/list", json!({})).await;
    let task = actions(&list, "genie_task");
    assert!(task.iter().any(|a| a == "create") && task.iter().any(|a| a == "split"), "{task:?}");
    let team = actions(&list, "genie_team");
    assert!(team.iter().any(|a| a == "spawn") && !team.iter().any(|a| a == "set-status"), "a member's own line is an agent's: {team:?}");
    assert!(!actions(&list, "genie_mail").iter().any(|a| a == "ask"), "asks are for agents");
    assert!(actions(&list, "genie_user").is_empty(), "people's accounts are for admins");
    assert_eq!(actions(&list, "genie_agents"), ["show", "preview", "mcp-calls"]);
    assert!(actions(&list, "genie_me").iter().any(|a| a == "answer"));
    assert!(tool_def(&list, "genie_task")["inputSchema"]["properties"]["project"].is_object(), "a person picks the project");
    assert!(tool_def(&list, "genie_task")["description"].as_str().unwrap().contains("- show(task?, history?): Show a task in full"));

    let (created, err) = tool(&h, &anna, "genie_task", json!({ "action": "create", "title": "CSV export", "project": "shop" })).await;
    assert!(!err && created.starts_with("created G-1 — CSV export"), "{created}");
    let (shown, _) = tool(&h, &anna, "genie_task", json!({ "action": "show", "task": "G-1" })).await;
    assert!(shown.starts_with("# G-1 — CSV export"), "{shown}");
    let (_, err) = tool(&h, &anna, "genie_task", json!({ "action": "artifact_read", "task": "G-1", "n": 1 })).await;
    assert!(err, "an action named the older way reaches the operation, which reports there is no artifact 1");

    let (refused, err) = tool(&h, &anna, "genie_team", json!({ "action": "set-status", "text": "x" })).await;
    assert!(err && refused.contains("genie_team has no action `set-status` for you; yours: show"), "{refused}");
    let (elsewhere, err) = tool(&h, &anna, "genie_task", json!({ "action": "list", "project": "other" })).await;
    assert!(err && elsewhere.contains("no access to project other"), "the named project or nothing: {elsewhere}");
    let (bad, err) = tool(&h, &anna, "genie_task", json!({ "action": "show", "task": 7 })).await;
    assert!(err && bad.starts_with("invalid arguments"), "{bad}");
    let unknown = rpc(&h, &anna, "tools/call", json!({ "name": "genie_nope", "arguments": {} })).await;
    assert_eq!(unknown["error"]["code"], -32602);

    // A server admin also sees the administration.
    let root = person(&h, "root", true, None);
    let list = rpc(&h, &root, "tools/list", json!({})).await;
    assert!(actions(&list, "genie_user").iter().any(|a| a == "add"));
    assert!(actions(&list, "genie_agents").iter().any(|a| a == "save"));
    let (users, err) = tool(&h, &root, "genie_user", json!({ "action": "list" })).await;
    assert!(!err && users.contains("anna") && users.contains("root"), "{users}");
}

#[tokio::test]
async fn an_agent_sees_what_its_class_and_role_may_do() {
    let h = Harness::new();
    h.project("shop");
    let owner = person(&h, "owner", true, None);
    tool(&h, &owner, "genie_task", json!({ "action": "create", "title": "CSV export", "project": "shop" })).await;
    let bender = agent(&h, Role::Executor, "bender", Some("G-1"), None);

    let init = rpc(&h, &bender, "initialize", json!({})).await;
    assert!(init["result"]["instructions"].as_str().unwrap().contains("the agent bender of project shop"));
    let list = rpc(&h, &bender, "tools/list", json!({})).await;
    let task = actions(&list, "genie_task");
    for a in ["show", "status", "comment", "artifact-read"] {
        assert!(task.iter().any(|x| x == a), "{a}: {task:?}");
    }
    for a in ["create", "check", "split", "accept"] {
        assert!(!task.iter().any(|x| x == a), "an executor has no {a}: {task:?}");
    }
    let team = actions(&list, "genie_team");
    assert!(team.iter().any(|a| a == "set-status") && !team.iter().any(|a| a == "spawn"), "{team:?}");
    assert!(actions(&list, "genie_mail").iter().any(|a| a == "ask"));
    assert!(actions(&list, "genie_me").is_empty() && actions(&list, "genie_automation").is_empty(), "the commands of its prompt only");
    assert!(actions(&list, "genie_job").is_empty(), "a member reports no job result");
    let schema = &tool_def(&list, "genie_task")["inputSchema"];
    assert!(schema["properties"]["project"].is_null(), "an agent's token is bound to its project");
    assert!(tool_def(&list, "genie_task")["description"].as_str().unwrap().contains("(your role may set: in_progress, review)"));

    // Its own task by default, as on the command line.
    let (shown, err) = tool(&h, &bender, "genie_task", json!({ "action": "show" })).await;
    assert!(!err && shown.starts_with("# G-1 — CSV export"), "{shown}");
    let (refused, err) = tool(&h, &bender, "genie_task", json!({ "action": "create", "title": "more" })).await;
    assert!(err && refused.contains("no action `create` for you"), "{refused}");

    // A job sees its output and no team.
    let job = h
        .app
        .with_server(|db| {
            db.create_job(NewJob {
                project: "shop".into(),
                task: Some("G-1".into()),
                run_step: None,
                role: "documenter".into(),
                model: None,
                goal: "describe".into(),
                inputs: json!({}),
                output_schema: None,
                workspace: "none".into(),
                initiator: None,
            })
        })
        .unwrap();
    let writer = agent(&h, Role::Documenter, "job-1", None, Some(job.id));
    let list = rpc(&h, &writer, "tools/list", json!({})).await;
    assert_eq!(actions(&list, "genie_job").iter().filter(|a| *a == "output").count(), 1);
    assert!(actions(&list, "genie_team").is_empty() && actions(&list, "genie_mail").is_empty(), "a job works alone");
    let (shown, _) = tool(&h, &writer, "genie_task", json!({ "action": "show" })).await;
    assert!(shown.starts_with("# G-1"), "a job's task is its default: {shown}");
}

#[tokio::test]
async fn the_endpoint_speaks_streamable_http_with_json_answers() {
    let h = Harness::new();
    h.project("shop");
    let anna = person(&h, "anna", false, Some(ProjectRole::Member));

    let (s, _, _) = call(&h.remote, "POST", "/mcp").no_csrf().json(json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" })).send().await;
    assert_eq!(s, StatusCode::FORBIDDEN, "without a token a browser's request is refused");
    let (s, _, _) = call(&h.remote, "POST", "/mcp").json(json!({ "jsonrpc": "2.0", "id": 1, "method": "ping" })).send().await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    let (s, _, _) = call(&h.remote, "GET", "/mcp").bearer(&anna).send().await;
    assert_eq!(s, StatusCode::METHOD_NOT_ALLOWED, "no stream from the server");

    let (s, _, _) = call(&h.remote, "POST", "/mcp")
        .bearer(&anna)
        .no_csrf()
        .json(json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .send()
        .await;
    assert_eq!(s, StatusCode::ACCEPTED, "a notification needs no answer");
    let batch = json!([
        { "jsonrpc": "2.0", "id": 1, "method": "ping" },
        { "jsonrpc": "2.0", "method": "notifications/initialized" },
        { "jsonrpc": "2.0", "id": 2, "method": "resources/list" },
    ]);
    let (s, v, _) = call(&h.remote, "POST", "/mcp").bearer(&anna).no_csrf().json(batch).send().await;
    assert_eq!(s, StatusCode::OK);
    let replies = v.as_array().unwrap();
    assert_eq!(replies.len(), 2, "{v}");
    assert_eq!(replies[0]["result"], json!({}));
    assert_eq!(replies[1]["error"]["code"], -32601);

    // On the server's machine a server without users needs no token.
    let local = Harness::new();
    local.project("shop");
    let (s, v, _) = call(&local.router, "POST", "/mcp")
        .json(json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "genie_task", "arguments": { "action": "list" } } }))
        .send()
        .await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["result"]["content"][0]["text"], "(no tasks)");
}

/// Both entrances come from the catalog: every operation is a command and, for
/// someone, an action of a tool.
#[tokio::test]
async fn every_operation_is_a_command_and_an_action_of_a_tool() {
    let h = Harness::new();
    h.project("shop");
    let admin = person(&h, "root", true, None);
    tool(&h, &admin, "genie_task", json!({ "action": "create", "title": "t", "project": "shop" })).await;
    let job = h
        .app
        .with_server(|db| {
            db.create_job(NewJob {
                project: "shop".into(),
                task: None,
                run_step: None,
                role: "documenter".into(),
                model: None,
                goal: "g".into(),
                inputs: json!({}),
                output_schema: None,
                workspace: "none".into(),
                initiator: None,
            })
        })
        .unwrap();
    let callers = [
        admin,
        agent(&h, Role::Orchestrator, "orchestrator", None, None),
        agent(&h, Role::Executor, "bender", Some("G-1"), None),
        agent(&h, Role::Documenter, "job-1", None, Some(job.id)),
    ];
    let mut reachable = HashSet::new();
    for token in &callers {
        let list = rpc(&h, token, "tools/list", json!({})).await;
        for t in list["result"]["tools"].as_array().unwrap() {
            for a in t["inputSchema"]["properties"]["action"]["enum"].as_array().unwrap() {
                reachable.insert(format!("{} {}", t["name"].as_str().unwrap(), a.as_str().unwrap()));
            }
        }
    }
    let cli = genie::cli::command();
    for e in genie::ops::catalog() {
        assert!(reachable.contains(&format!("genie_{} {}", e.group, e.name)), "{} {} is no tool action", e.group, e.name);
        let group = cli.find_subcommand(e.group).unwrap_or_else(|| panic!("no command genie {}", e.group));
        assert!(group.find_subcommand(e.name).is_some(), "no command genie {} {}", e.group, e.name);
    }
}
