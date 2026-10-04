//! Ideas: a person describes an idea in their own words and shapes it into
//! tasks with a planner agent before anything reaches the orchestrator.
//!
//! `POST /ideas` files the idea as a draft labelled «идея» (the orchestrator is
//! not told) and assembles the `idea` team: one planner who talks with the
//! person in the agent chat and keeps its proposal as a `plan.json` artifact on
//! the idea. `POST /ideas/{id}/apply` takes that proposal (as the person edited
//! it): the planner stops, the idea becomes the epic (or the first task), the
//! other tasks are created with their dependencies, and the orchestrator hears
//! about the whole batch once, through the idea moving to the inbox.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use genie_core::{Actor, CLOSED, CreateInput, GenieError, Role, Status, StatusOptions, TaskType, UpdateInput};
use serde::Deserialize;
use serde_json::{Value, json};

use super::ctx::{Access, Ctx};
use super::tasks::changed;
use super::{ApiError, ApiResult};
use crate::runtime::{self, SpawnRequest};
use crate::state::{App, AppResult};

/// The label that marks a task as an idea being shaped.
pub const IDEA_LABEL: &str = "идея";
/// The team template of the planner.
pub const IDEA_TEMPLATE: &str = "idea";
/// The artifact the planner keeps its proposal in.
pub const PLAN_ARTIFACT: &str = "plan.json";

pub fn routes() -> Router<Arc<App>> {
    Router::new().route("/ideas", post(start)).route("/ideas/{id}/apply", post(apply))
}

fn people_only(access: &Access) -> ApiResult<()> {
    access.write()?;
    if access.agent {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "ideas are shaped by people"));
    }
    Ok(())
}

#[derive(Deserialize)]
struct StartBody {
    text: String,
    title: Option<String>,
}

/// A short title from the idea's first line.
fn title_of(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    let line = line.trim_end_matches(['.', ':', ';', ',']);
    if line.chars().count() <= 80 {
        return line.to_string();
    }
    let cut: String = line.chars().take(80).collect();
    let cut = cut.rsplit_once(' ').map(|(a, _)| a).filter(|a| a.chars().count() > 40).unwrap_or(&cut);
    format!("{}…", cut.trim_end_matches([',', ' ']))
}

async fn start(State(app): State<Arc<App>>, ctx: Ctx, Json(b): Json<StartBody>) -> ApiResult<impl IntoResponse> {
    let access = ctx.access(&app, None).await?;
    people_only(&access)?;
    let text = b.text.trim().to_string();
    if text.is_empty() {
        return Err(ApiError::bad("describe the idea"));
    }
    let title = b.title.map(|t| t.trim().to_string()).filter(|t| !t.is_empty()).unwrap_or_else(|| title_of(&text));
    let (slug, actor) = (access.project.clone(), access.actor.clone());
    let out = app.blocking(move |app| start_idea(app, &slug, actor, title, text)).await?;
    changed(&app);
    Ok((StatusCode::CREATED, Json(out)))
}

fn start_idea(app: &App, slug: &str, actor: Actor, title: String, text: String) -> AppResult<Value> {
    let task = app.with_tracker(slug, |t| {
        let input =
            CreateInput { title, description: Some(text), labels: Some(vec![IDEA_LABEL.into()]), quiet: true, ..Default::default() };
        let task = t.create(&actor, input)?;
        // Refining before the team comes, so assembling it does not read as the
        // owner moving the task (which would wake the orchestrator).
        let note = StatusOptions { note: Some("shaping the idea with a planner".into()), ..Default::default() };
        t.set_status(&Actor::new("genie", Role::Orchestrator), &task.id, Status::Refining, note)
    })?;
    let req = SpawnRequest {
        task: task.id.clone(),
        template: Some(IDEA_TEMPLATE.into()),
        members: Vec::new(),
        models: Default::default(),
        note: None,
        by: actor,
        initiator: None,
    };
    let team = match runtime::spawn_team(app, slug, req) {
        Ok(team) => team,
        Err(e) => {
            // No planner, no idea: a draft nobody talks about would only clutter the tracker.
            let _ = app.with_tracker(slug, |t| t.delete_tasks(&Actor::new("genie", Role::Orchestrator), &task.id, false));
            return Err(e);
        }
    };
    let member = team.members.first().map(|m| m.name.clone()).unwrap_or_default();
    Ok(json!({ "task": task.id, "team": team.id, "member": member }))
}

#[derive(Deserialize)]
struct Plan {
    #[serde(default)]
    epic: Option<PlanEpic>,
    #[serde(default)]
    tasks: Vec<PlanTask>,
}

#[derive(Deserialize)]
struct PlanEpic {
    title: String,
    #[serde(default)]
    goal: String,
    #[serde(default)]
    criteria: Vec<String>,
    #[serde(default)]
    roadmap: Option<String>,
}

#[derive(Deserialize)]
struct PlanTask {
    #[serde(default)]
    key: Option<String>,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    criteria: Vec<String>,
    #[serde(default)]
    deps: Vec<String>,
    #[serde(default, rename = "type")]
    task_type: Option<String>,
    #[serde(default)]
    priority: Option<i64>,
}

#[derive(Deserialize)]
struct ApplyBody {
    plan: Value,
}

async fn apply(State(app): State<Arc<App>>, ctx: Ctx, Path(id): Path<String>, Json(b): Json<ApplyBody>) -> ApiResult<Json<Value>> {
    let access = ctx.access(&app, None).await?;
    people_only(&access)?;
    let plan: Plan = serde_json::from_value(b.plan).map_err(|e| ApiError::bad(format!("plan: {e}")))?;
    let (slug, actor) = (access.project.clone(), access.actor.clone());
    let out = app.blocking(move |app| apply_plan(app, &slug, &actor, &id, plan)).await?;
    changed(&app);
    Ok(Json(out))
}

fn clean(list: &[String]) -> Vec<String> {
    list.iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

/// Checked plan: each task's key, type and the keys it depends on.
fn check(plan: &Plan) -> Result<Vec<(String, TaskType, Vec<String>)>, GenieError> {
    if plan.tasks.is_empty() {
        return Err(GenieError::invalid("the plan has no tasks"));
    }
    if let Some(e) = &plan.epic
        && e.title.trim().is_empty()
    {
        return Err(GenieError::invalid("the epic needs a title"));
    }
    let keys: Vec<String> = plan
        .tasks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            t.key.as_deref().map(str::trim).filter(|k| !k.is_empty()).map(str::to_string).unwrap_or_else(|| format!("t{}", i + 1))
        })
        .collect();
    let mut out = Vec::new();
    for (i, (t, key)) in plan.tasks.iter().zip(&keys).enumerate() {
        if t.title.trim().is_empty() {
            return Err(GenieError::invalid(format!("task {} needs a title", i + 1)));
        }
        if keys[..i].contains(key) {
            return Err(GenieError::invalid(format!("two tasks share the key {key}")));
        }
        let ty = match t.task_type.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            None => TaskType::Task,
            Some(s) => match s.parse::<TaskType>() {
                Ok(TaskType::Epic) | Err(_) => return Err(GenieError::invalid(format!("{key}: type {s} (task, bug or spike)"))),
                Ok(x) => x,
            },
        };
        let deps = clean(&t.deps);
        for d in &deps {
            if d == key {
                return Err(GenieError::invalid(format!("{key} depends on itself")));
            }
            if !keys.contains(d) {
                return Err(GenieError::invalid(format!("{key} depends on {d}, which is not in the plan")));
            }
        }
        out.push((key.clone(), ty, deps));
    }
    Ok(out)
}

fn quote(text: &str) -> String {
    text.trim().lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n")
}

/// Turn the idea into the plan: the epic (or the first task) is the idea itself.
fn apply_plan(app: &App, slug: &str, actor: &Actor, id: &str, plan: Plan) -> AppResult<Value> {
    let checked = check(&plan)?;
    let idea = app.with_tracker(slug, |t| t.get(id))?;
    if !idea.labels.iter().any(|l| l == IDEA_LABEL) || CLOSED.contains(&idea.status) {
        return Err(GenieError::invalid(format!("{} is not an open idea", idea.id)).into());
    }
    if let Some(team) = app.with_tracker(slug, |t| t.bus().active_team_of(&idea.id))? {
        // Not "owner": the orchestrator hears about the plan once, below.
        runtime::stop_team(app, slug, &team, "planned", &actor.name)?;
    }
    let out = app.with_tracker(slug, |t| {
        let idea = t.get(id)?;
        let labels: Vec<String> = idea.labels.iter().filter(|l| *l != IDEA_LABEL).cloned().collect();
        let notes = format!("Разобрано с планировщиком. Исходная идея:\n\n{}", quote(&idea.description));
        let reset_criteria: Vec<i64> = idea.acceptance.iter().map(|a| a.id).collect();
        let mut ids: HashMap<String, String> = HashMap::new();
        let mut created = Vec::new();
        // The idea itself becomes the epic, or the plan's first task.
        let rest: &[PlanTask] = match &plan.epic {
            Some(e) => {
                let input = UpdateInput {
                    title: Some(e.title.trim().to_string()),
                    task_type: Some(TaskType::Epic),
                    description: Some(e.goal.trim().to_string()),
                    plan: e.roadmap.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(str::to_string),
                    labels: Some(labels.clone()),
                    remove_acceptance: reset_criteria,
                    add_acceptance: clean(&e.criteria),
                    append_notes: Some(notes),
                    ..Default::default()
                };
                t.update(actor, &idea.id, input)?;
                &plan.tasks
            }
            None => {
                let first = &plan.tasks[0];
                let input = UpdateInput {
                    title: Some(first.title.trim().to_string()),
                    task_type: Some(checked[0].1),
                    description: Some(first.description.trim().to_string()),
                    labels: Some(labels.clone()),
                    priority: first.priority,
                    remove_acceptance: reset_criteria,
                    add_acceptance: clean(&first.criteria),
                    append_notes: Some(notes),
                    ..Default::default()
                };
                t.update(actor, &idea.id, input)?;
                ids.insert(checked[0].0.clone(), idea.id.clone());
                &plan.tasks[1..]
            }
        };
        let offset = plan.tasks.len() - rest.len();
        for (task, (key, ty, _)) in rest.iter().zip(&checked[offset..]) {
            let input = CreateInput {
                title: task.title.trim().to_string(),
                task_type: Some(*ty),
                description: Some(task.description.trim().to_string()).filter(|d| !d.is_empty()),
                acceptance: clean(&task.criteria),
                priority: task.priority.or(Some(idea.priority)),
                parent: plan.epic.as_ref().map(|_| idea.id.clone()),
                labels: Some(labels.clone()),
                status: Some(Status::Inbox),
                quiet: true,
                ..Default::default()
            };
            let made = t.create(actor, input)?;
            ids.insert(key.clone(), made.id.clone());
            created.push(made.id);
        }
        for (key, _, deps) in &checked {
            if deps.is_empty() {
                continue;
            }
            let input = UpdateInput { add_deps: deps.iter().map(|d| ids[d].clone()).collect(), ..Default::default() };
            t.update(actor, &ids[key], input)?;
        }
        // One word to the orchestrator for the whole batch.
        // Russian: the owner reads it in the activity too.
        let what = match (&plan.epic, created.is_empty()) {
            (Some(_), _) => format!("эпик и задачи {}", created.join(", ")),
            (None, true) => "одна задача".to_string(),
            (None, false) => format!("задачи {}, {}", idea.id, created.join(", ")),
        };
        let note = format!("идея разобрана с планировщиком: {what}; заберите их из входящих");
        t.set_status(actor, &idea.id, Status::Inbox, StatusOptions { note: Some(note), ..Default::default() })?;
        Ok(json!({ "id": idea.id, "epic": plan.epic.is_some(), "created": created }))
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_come_from_the_first_line() {
        assert_eq!(title_of("\n  Остатки с телефона.\nДальше подробности"), "Остатки с телефона");
        let long = "Хочу чтобы кладовщики видели остатки по ячейкам с телефона и могли сразу заказать пополнение без бумажек";
        let t = title_of(long);
        assert!(t.ends_with('…') && t.chars().count() <= 81, "{t}");
    }

    #[test]
    fn plans_are_checked() {
        let plan = |v: Value| serde_json::from_value::<Plan>(v).unwrap();
        assert!(check(&plan(json!({ "tasks": [] }))).is_err());
        assert!(check(&plan(json!({ "tasks": [{ "title": "a", "deps": ["t2"] }] }))).is_err());
        assert!(check(&plan(json!({ "tasks": [{ "title": "a", "type": "epic" }] }))).is_err());
        let ok = check(&plan(json!({ "tasks": [{ "title": "a" }, { "title": "b", "type": "bug", "deps": ["t1"] }] }))).unwrap();
        assert_eq!(ok[1], ("t2".to_string(), TaskType::Bug, vec!["t1".to_string()]));
    }
}
