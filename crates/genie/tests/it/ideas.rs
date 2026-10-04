use crate::common;

use axum::http::StatusCode;
use common::{Harness, call};
use serde_json::{Value, json};

/// Mail waiting for the orchestrator, as texts.
fn orchestrator_mail(h: &Harness) -> Vec<String> {
    h.app.with_tracker("shop", |t| t.bus().pending(None, "orchestrator")).unwrap().into_iter().map(|m| m.text).collect()
}

async fn start(h: &Harness, text: &str) -> Value {
    let (s, v, _) = call(&h.router, "POST", "/api/ideas").json(json!({ "text": text })).send().await;
    assert_eq!(s, StatusCode::CREATED, "{v}");
    v
}

#[tokio::test]
async fn an_idea_gets_a_planner_and_stays_away_from_the_orchestrator() {
    let h = Harness::new();
    h.project("shop");
    let r = &h.router;
    let (s, e, _) = call(r, "POST", "/api/ideas").json(json!({ "text": "  " })).send().await;
    assert_eq!((s, e["error"].as_str()), (StatusCode::BAD_REQUEST, Some("describe the idea")));

    let v = start(&h, "Остатки по ячейкам с телефона.\nКладовщики бегают к компьютеру.").await;
    let id = v["task"].as_str().unwrap();
    let (_, task, _) = call(r, "GET", &format!("/api/tasks/{id}")).send().await;
    assert_eq!(task["title"], "Остатки по ячейкам с телефона");
    assert_eq!(task["status"], "refining");
    assert_eq!(task["labels"], json!(["идея"]));
    assert!(task["description"].as_str().unwrap().contains("бегают"));

    let (_, team, _) = call(r, "GET", &format!("/api/teams/{}", v["team"].as_str().unwrap())).send().await;
    assert_eq!(team["template"], "idea");
    assert_eq!(team["members"][0]["role"], "planner");
    assert_eq!(team["members"][0]["name"], v["member"]);
    let kickoff = team["mail"][0]["text"].as_str().unwrap();
    assert!(kickoff.contains("ask the owner your first question") && kickoff.contains("plan.json"), "{kickoff}");
    assert!(!kickoff.contains("Report your result to the orchestrator"), "{kickoff}");
    assert!(orchestrator_mail(&h).is_empty(), "the orchestrator is not told about an idea being shaped");
}

#[tokio::test]
async fn applying_a_plan_with_an_epic_files_the_tasks_in_one_go() {
    let h = Harness::new();
    h.project("shop");
    let r = &h.router;
    let v = start(&h, "Пополнение ячеек с телефона").await;
    let id = v["task"].as_str().unwrap().to_string();
    let team = v["team"].as_str().unwrap().to_string();

    let plan = json!({
        "summary": "…",
        "epic": { "title": "Пополнение с телефона", "goal": "Кладовщик заказывает пополнение сам", "criteria": ["заявка не теряется"], "roadmap": "1. экран 2. заявки" },
        "tasks": [
            { "key": "screen", "title": "Экран ячейки", "description": "Остатки", "criteria": ["видно остатки"] },
            { "key": "order", "title": "Заявка", "criteria": ["можно заказать"], "deps": ["screen"] },
            { "key": "q", "title": "Нужны ли этикетки?", "type": "spike" }
        ]
    });
    let (s, e, _) = call(r, "POST", &format!("/api/ideas/{id}/apply"))
        .json(json!({ "plan": { "tasks": [{ "title": "a", "deps": ["zzz"] }] } }))
        .send()
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{e}");
    let (s, out, _) = call(r, "POST", &format!("/api/ideas/{id}/apply")).json(json!({ "plan": plan })).send().await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["epic"], true);
    let created: Vec<String> = out["created"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
    assert_eq!(created.len(), 3);

    let (_, epic, _) = call(r, "GET", &format!("/api/tasks/{id}")).send().await;
    assert_eq!((epic["type"].as_str(), epic["status"].as_str()), (Some("epic"), Some("inbox")));
    assert_eq!(epic["title"], "Пополнение с телефона");
    assert_eq!(epic["acceptance"][0]["text"], "заявка не теряется");
    assert_eq!(epic["labels"], json!([]));
    assert!(epic["notes"].as_str().unwrap().contains("> Пополнение ячеек с телефона"), "the idea is kept");
    let (_, order, _) = call(r, "GET", &format!("/api/tasks/{}", created[1])).send().await;
    assert_eq!((order["parent"].as_str(), order["status"].as_str()), (Some(id.as_str()), Some("inbox")));
    assert_eq!(order["deps"], json!([created[0]]));
    let (_, spike, _) = call(r, "GET", &format!("/api/tasks/{}", created[2])).send().await;
    assert_eq!(spike["type"], "spike");

    let (_, t, _) = call(r, "GET", &format!("/api/teams/{team}")).send().await;
    assert_eq!(t["state"], "stopped", "the planner is done");
    let mail = orchestrator_mail(&h);
    assert_eq!(mail.len(), 1, "one word for the whole batch: {mail:?}");
    assert!(mail[0].contains(&created.join(", ")), "{}", mail[0]);

    let (s, _, _) = call(r, "POST", &format!("/api/ideas/{id}/apply")).json(json!({ "plan": plan })).send().await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "an applied idea is no longer an idea");
}

#[tokio::test]
async fn without_an_epic_the_idea_becomes_the_first_task() {
    let h = Harness::new();
    h.project("shop");
    let r = &h.router;
    let id = start(&h, "Кнопка экспорта").await["task"].as_str().unwrap().to_string();
    let plan = json!({ "epic": null, "tasks": [
        { "key": "a", "title": "Экспорт в CSV", "criteria": ["скачивается"], "deps": ["b"] },
        { "key": "b", "title": "Формат колонок", "type": "spike" }
    ]});
    let (s, out, _) = call(r, "POST", &format!("/api/ideas/{id}/apply")).json(json!({ "plan": plan })).send().await;
    assert_eq!(s, StatusCode::OK, "{out}");
    assert_eq!(out["epic"], false);
    let other = out["created"][0].as_str().unwrap();
    let (_, t, _) = call(r, "GET", &format!("/api/tasks/{id}")).send().await;
    assert_eq!((t["type"].as_str(), t["title"].as_str(), t["status"].as_str()), (Some("task"), Some("Экспорт в CSV"), Some("inbox")));
    assert_eq!(t["deps"], json!([other]), "a dependency on a task filed after it");
    assert_eq!(t["parent"], Value::Null);
}
