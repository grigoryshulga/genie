//! Observation operations: the `genie observe` catalog, command lines and MCP schemas. A command
//! line and JSON arguments (as MCP passes them) reach the API as the caller; the command tables of
//! agents list the operations their role may use (`docs/platform/observation-plane.md` §16).

use crate::common::Harness;
use genie::ops::{self, AgentKind, Auth, Cx, InProcess, Out};
use genie_core::{Capability, Role};
use serde_json::{Value, json};

fn cx(h: &Harness, auth: Auth, local: bool) -> Cx {
    let api = InProcess::new(genie::http::router(h.app.clone()), h.app.cfg.port, auth, Some("shop".into()));
    Cx { api: Box::new(api), project: Some("shop".into()), task: None, team: None, local }
}

/// A command line, as `genie …` runs it. Files are read only where the caller is local.
async fn cli(cx: &Cx, line: &[&str]) -> Result<Out, String> {
    let m = genie::cli::command().try_get_matches_from(std::iter::once("genie").chain(line.iter().copied())).map_err(|e| e.to_string())?;
    let (entry, args) = ops::chosen(&m).expect("an operation of the catalog");
    entry.run_cli(args, cx).await
}

/// An operation with JSON arguments, as the MCP server runs it.
async fn op(cx: &Cx, group: &str, name: &str, args: Value) -> Result<Out, String> {
    ops::find(group, name).unwrap_or_else(|| panic!("no operation {group} {name}")).run_json(args, cx).await
}

/// The text of an answer is the pretty JSON of its data, which is what the command line prints.
fn printed(out: &Out) -> Value {
    serde_json::from_str(&out.text).unwrap_or_else(|e| panic!("not JSON ({e}): {}", out.text))
}

/// A project with one task, `G-1`, that the cases observe.
async fn project() -> Harness {
    let h = Harness::new();
    h.project("shop");
    let admin = cx(&h, Auth::Operator, true);
    cli(&admin, &["task", "create", "CSV export"]).await.unwrap();
    h
}

/// A manual guardrail: `checkout_errors` may grow by 0.1pp at most. Manual sources need no secret.
fn guardrail_contract() -> Value {
    json!({
        "guardrails": [{ "id": "checkout_errors", "source": "manual", "maxChange": "+0.1pp" }],
        "observation": { "minimumWindow": "24h" }
    })
}

fn caps(role: Role) -> impl Fn(Capability) -> bool {
    let granted = genie_core::class_capabilities(role);
    move |c| granted.contains(&c)
}

#[tokio::test]
async fn a_case_runs_from_the_command_line_to_an_incident() {
    let h = project().await;
    let admin = cx(&h, Auth::Operator, true);

    // The contract as JSON on the command line.
    let contract = guardrail_contract().to_string();
    let created = cli(
        &admin,
        &["observe", "create", "--title", "Checkout errors", "--policy", "guardrails", "--task", "G-1", "--contract", &contract],
    )
    .await
    .unwrap();
    assert_eq!(created.data["case"]["key"], "O-1", "{}", created.text);
    assert_eq!(created.data["case"]["status"], "proposed");
    assert_eq!(created.data["case"]["contractVersion"], 1);
    assert_eq!(printed(&created), created.data, "the output is the API's JSON");

    // The contract from a file, read on the command line.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("contract.json");
    std::fs::write(&file, guardrail_contract().to_string()).unwrap();
    let from_file = cli(
        &admin,
        &[
            "observe",
            "create",
            "--title",
            "Checkout errors, again",
            "--policy",
            "guardrails",
            "--task",
            "G-1",
            "--contract-file",
            file.to_str().unwrap(),
        ],
    )
    .await
    .unwrap();
    assert_eq!(from_file.data["case"]["key"], "O-2");
    assert_eq!(from_file.data["case"]["contractVersion"], 1);

    // Ids are accepted as keys or numbers; a case without a contract cannot be armed.
    let shown = cli(&admin, &["observe", "show", "O-1"]).await.unwrap();
    assert_eq!(shown.data["case"]["title"], "Checkout errors");
    assert_eq!(shown.data["armProblems"], json!([]));
    assert_eq!(cli(&admin, &["observe", "show", "1"]).await.unwrap().data["case"]["key"], "O-1");
    cli(&admin, &["observe", "create", "--title", "No contract", "--policy", "guardrails", "--task", "G-1"]).await.unwrap();
    let refused = cli(&admin, &["observe", "arm", "O-3"]).await.unwrap_err();
    assert!(refused.contains("no contract yet"), "{refused}");

    let armed = cli(&admin, &["observe", "arm", "O-1"]).await.unwrap();
    assert_eq!(armed.data["case"]["status"], "waiting", "{}", armed.text);
    assert!(armed.data["case"]["armedAt"].is_string());
    let started = cli(&admin, &["observe", "start", "O-1"]).await.unwrap();
    assert_eq!(started.data["case"]["status"], "observing");

    // A reported violation of the guardrail opens an incident at once (§8, §10.1).
    let reported = cli(
        &admin,
        &[
            "observe",
            "report",
            "O-1",
            "--criterion",
            "checkout_errors",
            "--value",
            "1.5",
            "--baseline",
            "1.0",
            "--samples",
            "9000",
            "--note",
            "dashboard",
        ],
    )
    .await
    .unwrap();
    assert_eq!(reported.data["evidence"][0]["trust"], "reported", "{}", reported.text);
    assert_eq!(reported.data["evidence"][0]["deltaAbs"], 0.5);
    assert_eq!(reported.data["incidents"][0]["key"], "I-1", "{}", reported.text);

    let open = cli(&admin, &["observe", "incidents", "--status", "open"]).await.unwrap();
    assert_eq!(open.data.as_array().map(Vec::len), Some(1), "{}", open.text);
    assert_eq!((open.data[0]["key"].as_str(), open.data[0]["status"].as_str()), (Some("I-1"), Some("open")));
    let unknown = cli(&admin, &["observe", "incidents", "--status", "sleeping"]).await.unwrap_err();
    assert_eq!(unknown, "unknown incident status sleeping");

    let resolved = cli(&admin, &["observe", "resolve", "I-1", "--note", "rolled back"]).await.unwrap();
    assert_eq!(resolved.data["status"], "resolved", "{}", resolved.text);
    let dismissed = cli(&admin, &["observe", "dismiss", "I-1", "--note", "false alarm"]).await.unwrap();
    assert_eq!(dismissed.data["status"], "dismissed", "a person dismisses: {}", dismissed.text);
}

#[tokio::test]
async fn mcp_callers_send_contracts_as_objects_and_never_files() {
    let h = project().await;
    let admin = cx(&h, Auth::Operator, false);

    let created = op(
        &admin,
        "observe",
        "create",
        json!({ "title": "Checkout errors", "policy": "guardrails", "task": ["G-1"], "contract": guardrail_contract() }),
    )
    .await
    .unwrap();
    assert_eq!((created.data["case"]["key"].as_str(), created.data["case"]["contractVersion"].as_i64()), (Some("O-1"), Some(1)));
    // A number is an id too.
    assert_eq!(op(&admin, "observe", "show", json!({ "id": 1 })).await.unwrap().data["case"]["key"], "O-1");

    let revised =
        op(&admin, "observe", "contract", json!({ "id": "O-1", "contract": guardrail_contract(), "reason": "tighter" })).await.unwrap();
    assert_eq!(revised.data["case"]["contractVersion"], 2, "{}", revised.text);
    assert_eq!(revised.data["contracts"].as_array().map(Vec::len), Some(2));

    let refused = op(
        &admin,
        "observe",
        "create",
        json!({ "title": "x", "policy": "guardrails", "task": ["G-1"], "contractFile": "/etc/contract.json" }),
    )
    .await
    .unwrap_err();
    assert_eq!(refused, "files are read only on the command line; pass the text itself");
    let refused = op(&admin, "observe", "contract", json!({ "id": "O-1", "file": "/etc/contract.json" })).await.unwrap_err();
    assert_eq!(refused, "files are read only on the command line; pass the text itself");
    let broken = op(&admin, "observe", "contract", json!({ "id": "O-1", "contract": "{not json" })).await.unwrap_err();
    assert!(broken.starts_with("the contract must be JSON: "), "{broken}");
}

#[tokio::test]
async fn sources_and_their_secrets_are_set_by_people_and_show_only_hints() {
    let h = project().await;
    let admin = cx(&h, Auth::Operator, true);

    let added = cli(&admin, &["observe", "source-add", "prom", "--kind", "prometheus", "--url", "http://127.0.0.1:9", "--auth", "bearer"])
        .await
        .unwrap();
    assert_eq!((added.data["name"].as_str(), added.data["auth"].as_str()), (Some("prom"), Some("bearer")), "{}", added.text);
    assert!(added.data.get("secret").is_none(), "no secret is stored yet: {}", added.text);

    let set = cli(&admin, &["observe", "source-secret", "prom", "--value", "tok-s3cret"]).await.unwrap();
    assert!(set.data["secret"].is_object(), "the secret has a hint: {}", set.text);
    assert!(!set.text.contains("tok-s3cret") && !set.data.to_string().contains("tok-s3cret"), "the secret is never printed");
    let cleared = cli(&admin, &["observe", "source-secret", "prom", "--clear"]).await.unwrap();
    assert!(cleared.data.get("secret").is_none(), "{}", cleared.text);
    let neither = cli(&admin, &["observe", "source-secret", "prom"]).await.unwrap_err();
    assert_eq!(neither, "pass the secret as --value (`-` reads stdin), or --clear to remove the stored one");

    let unknown =
        cli(&admin, &["observe", "source-add", "graphite", "--kind", "graphite", "--url", "http://127.0.0.1:9"]).await.unwrap_err();
    assert_eq!(unknown, "unknown source kind graphite");

    let updated =
        cli(&admin, &["observe", "source-update", "prom", "--kind", "prometheus", "--url", "http://127.0.0.1:19090"]).await.unwrap();
    assert_eq!(updated.data["url"], "http://127.0.0.1:19090");
    assert_eq!(cli(&admin, &["observe", "sources"]).await.unwrap().data.as_array().map(Vec::len), Some(1));
    cli(&admin, &["observe", "source-remove", "prom"]).await.unwrap();
    assert_eq!(cli(&admin, &["observe", "sources"]).await.unwrap().data, json!([]));
}

#[test]
fn command_tables_list_what_each_role_may_use() {
    let orchestrator = ops::command_table(AgentKind::Orchestrator, &|_| true);
    for name in [
        "list",
        "show",
        "create",
        "update",
        "contract",
        "arm",
        "start",
        "evaluate",
        "cancel",
        "deploy",
        "evidence",
        "report",
        "incidents",
        "resolve",
        "followup",
    ] {
        assert!(orchestrator.contains(&format!("| `genie_observe` {name} |")), "the project manager lists {name}:\n{orchestrator}");
    }
    assert!(!orchestrator.contains("`genie_observe` dismiss"), "only a person dismisses an incident:\n{orchestrator}");
    assert!(!orchestrator.contains("`genie_observe` source"), "only people manage sources:\n{orchestrator}");

    // Team members and jobs get no observation commands in their tables.
    for (kind, role) in [(AgentKind::Member, Role::Executor), (AgentKind::Member, Role::Analyst), (AgentKind::Job, Role::Documenter)] {
        let table = ops::command_table(kind, &caps(role));
        assert!(!table.contains("genie_observe"), "{role:?} lists observation commands:\n{table}");
    }
}
