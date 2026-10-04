//! Repository operations shared by the HTTP API, the runtime and the proxy:
//! what an agent may do where, and the workspaces built from a task's repositories.

use genie_core::repos::{ProjectRepo, TaskRepo};
use genie_core::{GenieError, Role};

use super::policy::{Effective, RoleGit, effective};
use super::store::{self, Placed, Want};
use crate::state::{App, AppError, AppResult};

/// Who an agent is, as far as repositories care.
#[derive(Debug, Clone)]
pub struct AgentId {
    pub role: Role,
    pub role_id: Option<String>,
    pub team: Option<String>,
    pub job: Option<i64>,
}

/// What the agent's role allows in repositories at most.
pub fn role_git(app: &App, a: &AgentId) -> RoleGit {
    use crate::agent_config::FileAccess;
    match a.role_id.as_deref().and_then(|id| app.agents().roles.get(id).cloned()) {
        Some(r) => RoleGit::of(r.git.as_deref(), r.files),
        None => RoleGit::of(None, if matches!(a.role, Role::Analyst | Role::Reviewer) { FileAccess::Read } else { FileAccess::Write }),
    }
}

/// The task an agent works on (`None` for the orchestrator).
pub fn task_of(app: &App, project: &str, a: &AgentId) -> AppResult<Option<String>> {
    Ok(match (&a.team, a.job) {
        (Some(t), _) => Some(app.with_tracker(project, |tr| Ok(tr.bus().get(t)?.task))?),
        (None, Some(j)) => app.with_server(|db| db.job(j))?.task,
        _ => None,
    })
}

/// Attach a repository to a project: the host must be configured and the policy valid; the
/// mirror is fetched at once (a wrong path or token shows now, and the default branch is learned).
/// Returns the repository and a warning when the first fetch failed (the repository is added anyway).
pub fn add_repo(app: &App, project: &str, new: genie_core::repos::NewRepo) -> AppResult<(ProjectRepo, Option<String>)> {
    use genie_core::repos::RepoPatch;
    let hosts = super::hosts::load(&app.data);
    if !hosts.map.contains_key(&new.host) {
        let known: Vec<&str> = hosts.map.keys().map(String::as_str).collect();
        return Err(GenieError::invalid(format!(
            "host {} is not configured in git.json (configured: {})",
            new.host,
            if known.is_empty() { "none".to_string() } else { known.join(", ") }
        ))
        .into());
    }
    if let Some(p) = &new.policy {
        super::policy::Policy::parse(p).map_err(GenieError::invalid)?;
    }
    let repo = app.with_server(|db| db.add_repo(project, new))?;
    match store::sync(app, &repo) {
        Ok(info) => {
            let repo = match (repo.default_branch.is_empty(), info["defaultBranch"].as_str()) {
                (true, Some(d)) => app.with_server(|db| {
                    db.update_repo(project, &repo.name, RepoPatch { default_branch: Some(d.to_string()), ..Default::default() })
                })?,
                _ => repo,
            };
            Ok((repo, info["warning"].as_str().map(str::to_string)))
        }
        Err(e) => Ok((repo, Some(e))),
    }
}

/// The effective policy of one repository for an agent (blocking).
pub fn effective_for(app: &App, project: &str, repo: &ProjectRepo, a: &AgentId) -> AppResult<Effective> {
    let task = task_of(app, project, a)?;
    let access = match &task {
        Some(t) => app.with_server(|db| db.task_repo(project, t, &repo.name))?.map(|r| r.access),
        None => None,
    };
    let repo = store::resolved(app, repo);
    effective(&repo, task.as_deref(), role_git(app, a), access.as_deref()).map_err(|e| GenieError::invalid(e).into())
}

/// The effective policies of every repository of the project for an agent (blocking).
pub fn effective_all(app: &App, project: &str, a: &AgentId) -> AppResult<Vec<Effective>> {
    let repos = app.with_server(|db| db.repos(project))?;
    repos.iter().map(|r| effective_for(app, project, r, a)).collect()
}

/// The repositories of a task, defaulting when it names none: a project with one
/// repository gives it (write, if the project allows); with several, the task must say
/// which it works in.
pub fn ensure_task_repos(app: &App, project: &str, task: &str) -> AppResult<Vec<TaskRepo>> {
    let have = app.with_server(|db| db.task_repos(project, task))?;
    if !have.is_empty() {
        return Ok(have);
    }
    let repos = app.with_server(|db| db.repos(project))?;
    let writable: Vec<&ProjectRepo> = repos.iter().filter(|r| r.access == "write").collect();
    match writable.as_slice() {
        [] => Ok(have),
        [one] => Ok(app.with_server(|db| db.set_task_repos(project, task, &[(one.name.clone(), "write".to_string())]))?),
        many => Err(GenieError::invalid(format!(
            "{task} must name the repositories it works in ({}): `genie repos use <repo>:write … --task {}` or PUT /api/tasks/{task}/repos",
            many.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", "),
            task
        ))
        .into()),
    }
}

/// A workspace for a task: each repository at its mount, the task's branch checked out in those it may write.
/// `name` is the directory under the project's workspaces (a team id, `job-<n>`).
pub fn task_workspace(app: &App, project: &str, name: &str, task: Option<&str>) -> AppResult<(std::path::PathBuf, Vec<Placed>)> {
    let repos = app.with_server(|db| db.repos(project))?;
    let rows = match task {
        Some(t) => ensure_task_repos(app, project, t)?,
        None => Vec::new(),
    };
    let mut wants = Vec::new();
    for repo in repos {
        let row = rows.iter().find(|r| r.repo == repo.name);
        let repo = store::resolved(app, &repo);
        let eff =
            effective(&repo, task, RoleGit::Write, row.map(|r| r.access.as_str())).map_err(|e| AppError::from(GenieError::invalid(e)))?;
        let branch = match (eff.write, eff.task_branch(), task) {
            (true, Some(b), _) => Some(b),
            // A job without a task commits on a local branch that cannot be pushed.
            (false, _, None) if name.starts_with("job-") && repo.access == "write" => Some(format!("genie/{name}")),
            _ => None,
        };
        if let (Some(b), Some(t)) = (&branch, task)
            && row.is_some()
        {
            app.with_server(|db| db.name_branch(project, t, &repo.name, b).map(|_| ()))?;
        }
        wants.push(Want { repo, branch });
    }
    store::assemble(app, project, name, wants, false).map_err(AppError::Internal)
}

/// A read-only view of every repository of the project on its default branch, for agents
/// that do not work on a task (the orchestrator, analysts, read-only jobs). `None` when the project has no repositories.
pub fn view_workspace(app: &App, project: &str) -> Option<std::path::PathBuf> {
    let repos = app.with_server(|db| db.repos(project)).ok().filter(|r| !r.is_empty())?;
    let wants = repos.into_iter().map(|repo| Want { repo, branch: None }).collect();
    match store::assemble(app, project, "main", wants, true) {
        Ok((root, _)) => Some(root),
        Err(e) => {
            eprintln!("genie git: {project}: the repositories' view is unavailable: {e}");
            None
        }
    }
}

/// The repositories part of an agent's prompt: where each is and what the agent may do in it.
pub fn prompt_section(app: &App, project: &str, a: &AgentId) -> String {
    let Ok(all) = effective_all(app, project, a) else { return String::new() };
    if all.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n## Repositories\n\nThe project's code lives in these repositories, each in its own directory of your working directory. `git` works as usual; `origin` is the genie server, which checks every push against the rules below (a refused push says why). You hold no credentials for the git host and need none. Use `genie repos list` to see the rules again.\n\n",
    );
    for e in &all {
        out.push_str(&format!("- {}\n", e.describe()));
    }
    out.push_str("\nWhen your work is committed: `git push` your task's branch, then `genie pr open --repo <name> --title \"…\" --body \"…\"` for each repository you changed.\n");
    out
}
