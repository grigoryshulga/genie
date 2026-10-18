//! The two managers' application rules. All entrances share dispatch, task scope,
//! evidence access and retrospective completion; the tracker owns durable state.

use std::collections::HashSet;

use genie_core::management::{TaskManager, managed_task, manager_name};
use genie_core::team::ORCHESTRATOR;
use genie_core::{Actor, Role};
use serde::Serialize;
use serde_json::{Value, json};

use crate::state::{App, AppError, AppResult};

#[derive(Serialize, ts_rs::TS)]
#[ts(export)]
pub struct TaskManagement {
    pub manager: Option<TaskManager>,
    pub clarifications: Vec<genie_core::management::Clarification>,
    pub task: genie_core::Task,
    #[ts(type = "unknown")]
    pub brief: Option<Value>,
}

pub fn authorize_task(app: &App, project: &str, actor: &Actor, task: &str) -> AppResult<()> {
    if let Some(own) = managed_task(&actor.name) {
        let target = app.with_tracker(project, |t| t.normalize_id(task))?;
        if target != own {
            return Err(AppError::Forbidden(format!("task manager {own} cannot manage {target}")));
        }
    }
    Ok(())
}

pub fn project_manager(actor: &Actor) -> AppResult<()> {
    if !matches!(actor.role, Role::Human | Role::Orchestrator) || managed_task(&actor.name).is_some() {
        return Err(AppError::Forbidden("only the project manager or a person makes project decisions".into()));
    }
    Ok(())
}

pub fn dispatch(app: &App, project: &str, actor: &Actor, task: &str) -> AppResult<TaskManager> {
    project_manager(actor)?;
    if actor.role != Role::Human && app.with_server(|db| db.project(project))?.autonomy == "manual" {
        return Err(AppError::Forbidden("manual projects are dispatched by people".into()));
    }
    // Check configuration and budget before committing the release.
    let _ = app.agents().orchestrator().ok_or_else(|| AppError::Bad("task-manager role is missing".into()))?;
    if let Some(exceeded) = crate::budget::check(app, project, Some(task))? {
        return Err(AppError::Conflict(exceeded.explain()));
    }
    let manager = app.with_tracker(project, |t| t.management().dispatch(actor, task))?;
    crate::tasks::changed(app);
    Ok(manager)
}

/// Existing running teams are adopted without replaying their execution. Called
/// by the scheduler, including after a restart. Idea conversations stay in intake.
pub fn reconcile(app: &App, project: &str) -> AppResult<()> {
    let auto_adopt = app.with_server(|db| db.project(project))?.autonomy != "manual";
    app.with_tracker(project, |t| {
        let actor = Actor::new("genie", Role::Orchestrator);
        for team in t.bus().list(false)? {
            if auto_adopt && team.template.as_deref() != Some("idea") && t.management().manager(&team.task)?.is_none() {
                t.management().adopt(&actor, &team.task)?;
            }
        }
        for m in t.management().managers()? {
            let task = t.get(&m.task)?;
            t.management().on_status(&actor, &m.task, task.status)?;
        }
        Ok(())
    })
}

pub fn show(app: &App, project: &str, task: &str) -> AppResult<Value> {
    app.with_tracker(project, |t| {
        let task = t.get(task)?;
        let m = t.management();
        Ok(json!(TaskManagement {
            manager: m.manager(&task.id)?,
            brief: m.brief(&task.id, None).ok(),
            clarifications: m.clarifications(&task.id)?,
            task
        }))
    })
}

/// A stable inventory of all recorded execution turns, including former teams,
/// jobs and the task manager. No UI history limit silently drops older attempts.
pub fn sources(app: &App, project: &str, task: &str) -> AppResult<Value> {
    let (task, teams) = app.with_tracker(project, |t| {
        let task = t.get(task)?;
        let mut q = t.conn().prepare("SELECT team FROM task_team_history WHERE task = ?1 UNION SELECT id FROM teams WHERE task = ?1")?;
        let teams = q.query_map([&task.id], |r| r.get::<_, String>(0))?.collect::<Result<HashSet<_>, _>>()?;
        Ok((task, teams))
    })?;
    let manager = manager_name(&task.id);
    let turns = app.with_server(|db| db.turns(project, None, i64::MAX))?;
    let mut records = Vec::new();
    let closed = task.history.iter().rev().find(|h| matches!(h.to.as_deref(), Some("done" | "cancelled"))).map(|h| h.at.as_str());
    let mut execution_cutoff = closed.map(str::to_string);
    for turn in turns {
        let belongs = turn.agent == manager
            || turn.team.as_ref().is_some_and(|id| teams.contains(id))
            || turn.job.is_some_and(|id| app.with_server(|db| db.job(id)).is_ok_and(|j| j.task.as_deref() == Some(&task.id)));
        // Retrospective turns themselves are not part of the execution being audited.
        if !belongs || turn.status == "skipped" || (turn.agent == manager && closed.is_some_and(|at| turn.started.as_str() > at)) {
            continue;
        }
        if turn.agent == manager
            && let Some(finished) = &turn.finished
            && closed.is_some_and(|at| turn.started.as_str() <= at)
            && execution_cutoff.as_ref().is_none_or(|at| finished > at)
        {
            execution_cutoff = Some(finished.clone());
        }
        records.push(json!({ "id": turn.id.to_string(), "agent": turn.agent, "status": turn.status,
            "started": turn.started, "finished": turn.finished, "available": turn.log.is_some(), "error": turn.error }));
    }
    let agents = app.with_tracker(project, |t| t.management().execution_sources(&task.id, execution_cutoff.as_deref()))?;
    for agent in agents {
        records.push(json!({ "id": format!("session:{agent}"), "agent": agent, "available": true, "kind": "transcript" }));
    }
    let intelligence = app.with_tracker(project, |t| t.intelligence_history(&task.id))?;
    let recommendations = crate::intelligence::recommendations(&intelligence);
    Ok(
        json!({ "task": task.id, "sources": records, "executionCutoff": execution_cutoff, "intelligence": intelligence, "routingRecommendations": recommendations,
        "scope": "All recorded execution turns, including failed/interrupted attempts, former members and the task manager; task history, mail, clarifications and artifacts are available in manager show and team show." }),
    )
}

pub fn source(app: &App, project: &str, task: &str, id: &str, offset: usize) -> AppResult<Value> {
    let inventory = sources(app, project, task)?;
    if !inventory["sources"].as_array().is_some_and(|s| s.iter().any(|s| s["id"] == id)) {
        return Err(AppError::Forbidden("this source does not belong to the task".into()));
    }
    if let Some(agent) = id.strip_prefix("session:") {
        let task = app.with_tracker(project, |t| t.get(task))?;
        let cutoff = inventory["executionCutoff"].as_str();
        let events = app.with_tracker(project, |t| {
            t.management().execution_source(&task.id, agent, cutoff, i64::try_from(offset).unwrap_or(i64::MAX))
        })?;
        let next = events.last().and_then(|e| e["id"].as_i64());
        return Ok(json!({ "id": id, "events": events, "nextOffset": next }));
    }
    let turn_id = id.parse::<i64>().map_err(|_| AppError::Bad("invalid source id".into()))?;
    let turn = app.with_server(|db| db.turn(turn_id))?;
    let text = turn.log.as_deref().unwrap_or_default();
    let chunk: String = text.chars().skip(offset).take(30_000).collect();
    let next = offset.saturating_add(chunk.chars().count());
    Ok(json!({ "id": id, "agent": turn.agent, "available": turn.log.is_some(), "text": chunk,
        "offset": offset, "nextOffset": (next < text.chars().count()).then_some(next), "error": turn.error }))
}

pub enum Coverage<'a> {
    Reviewed(&'a [String]),
    Incomplete(&'a [String]),
    AcceptedMissing,
}

pub fn retrospective(app: &App, project: &str, actor: &Actor, task: &str, report: &str, coverage: Coverage<'_>) -> AppResult<TaskManager> {
    if report.trim().is_empty() {
        return Err(AppError::Bad("a retrospective needs a report".into()));
    }
    let (reviewed, incomplete, accept_missing) = match coverage {
        Coverage::Reviewed(ids) => (ids, false, false),
        Coverage::Incomplete(ids) => (ids, true, false),
        Coverage::AcceptedMissing => (&[] as &[String], false, true),
    };
    authorize_task(app, project, actor, task)?;
    if !matches!(actor.role, Role::Human | Role::Orchestrator) {
        return Err(AppError::Forbidden("only a manager or a person publishes retrospectives".into()));
    }
    if accept_missing && actor.role != Role::Human {
        return Err(AppError::Forbidden("a person must accept missing source coverage".into()));
    }
    let inventory = sources(app, project, task)?;
    let entries = inventory["sources"].as_array().ok_or_else(|| AppError::Internal("missing source inventory".into()))?;
    let gaps: Vec<_> = entries.iter().filter(|s| s["available"] != true || !reviewed.iter().any(|id| s["id"] == *id)).collect();
    if !incomplete && !accept_missing && !gaps.is_empty() {
        return Err(AppError::Conflict(
            "read and name every source in --reviewed, or publish --incomplete; only a person can accept missing coverage".into(),
        ));
    }
    let report = format!(
        "{report}\n\n## Source coverage\n\nReviewed sources: {}.\nCoverage: {}.\n",
        reviewed.join(", "),
        if accept_missing {
            "limitation explicitly accepted by a person"
        } else if incomplete {
            "incomplete; retrospective remains pending"
        } else {
            "all recorded sources reviewed"
        }
    );
    let report = format!(
        "{report}\n## Routing policy proposals\n\n{}\n\nThese proposals require human evaluation and approval; no policy was changed.\n",
        serde_json::to_string_pretty(&inventory["routingRecommendations"]).map_err(genie_core::GenieError::from)?
    );
    app.with_tracker(project, |t| t.management().publish_retrospective(actor, task, &report, !incomplete))
}

pub fn escalate(app: &App, project: &str, actor: &Actor, task: &str, id: i64) -> AppResult<Value> {
    project_manager(actor)?;
    let q = app.with_tracker(project, |t| t.management().escalate(actor, task, id))?;
    crate::tasks::changed(app);
    Ok(json!(q))
}

pub fn project_note(app: &App, project: &str, from: &str, text: &str, task: Option<&str>) -> AppResult<()> {
    app.with_tracker(project, |t| t.bus().notify_manager(ORCHESTRATOR, from, "system", "management", text, task))?;
    crate::tasks::changed(app);
    Ok(())
}
