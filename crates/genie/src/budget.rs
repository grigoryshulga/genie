//! Budgets for what agents' work costs (`budgets` in the configuration): one task, one
//! epic, one day of a project. Tokens are priced as everywhere else (`modelPrices`), so a
//! model without a price is not counted. When a budget is used up the teams it covers
//! stop (`budget`), people are told once, and no new team starts for it until the limit is
//! raised in `config.json` (and the server restarted) or the work moves on.

use genie_core::usage::usage_since;

use crate::notify::{self, Message};
use crate::spend::{Spend, TaskIndex};
use crate::state::{App, AppResult};

/// What a used-up budget covers.
#[derive(Debug, Clone, PartialEq)]
pub enum Scope {
    Task(String),
    Epic(String),
    Day,
}

impl Scope {
    fn key(&self) -> String {
        match self {
            Scope::Task(t) => format!("task:{t}"),
            Scope::Epic(e) => format!("epic:{e}"),
            Scope::Day => "day".into(),
        }
    }
    fn words(&self) -> String {
        match self {
            Scope::Task(t) => format!("the task {t}"),
            Scope::Epic(e) => format!("the epic {e}"),
            Scope::Day => "today".into(),
        }
    }
}

/// A budget that is used up.
#[derive(Debug, Clone, PartialEq)]
pub struct Exceeded {
    pub scope: Scope,
    pub spent: f64,
    pub limit: f64,
}

impl Exceeded {
    pub fn explain(&self) -> String {
        format!(
            "budget: the spend of {} is ${:.2}, the limit is ${:.2} (`budgets` in config.json; restart the server after raising it)",
            self.scope.words(),
            self.spent,
            self.limit
        )
    }
}

fn dollars(app: &App, rows: &[genie_core::usage::UsageRow]) -> f64 {
    Spend::of(&app.cfg.model_prices, rows).cost
}

/// The first used-up budget that covers work on `task` (its task, its epic, the day), if any.
pub fn check(app: &App, slug: &str, task: Option<&str>) -> AppResult<Option<Exceeded>> {
    let b = app.cfg.budgets;
    if b.per_task <= 0.0 && b.per_epic <= 0.0 && b.per_day <= 0.0 {
        return Ok(None);
    }
    app.with_tracker(slug, |t| {
        if let Some(task) = task {
            if b.per_task > 0.0 {
                let spent = dollars(app, &t.usage_of_task(task)?);
                if spent >= b.per_task {
                    return Ok(Some(Exceeded { scope: Scope::Task(task.into()), spent, limit: b.per_task }));
                }
            }
            if b.per_epic > 0.0
                && let Some(epic) = TaskIndex::load(t.conn())?.epic_of(task)
                && epic != task
            {
                let spent = dollars(app, &t.usage_of_task(&epic)?);
                if spent >= b.per_epic {
                    return Ok(Some(Exceeded { scope: Scope::Epic(epic), spent, limit: b.per_epic }));
                }
            }
        }
        if b.per_day > 0.0 {
            let today = genie_core::db::now()[..10].to_string();
            let rows: Vec<_> = usage_since(t.conn(), &today)?.into_iter().filter(|r| r.day == today).collect();
            let spent = dollars(app, &rows);
            if spent >= b.per_day {
                return Ok(Some(Exceeded { scope: Scope::Day, spent, limit: b.per_day }));
            }
        }
        Ok(None)
    })
}

/// After a report of usage on `task`: when a budget is used up, stop the teams it covers and tell people once.
pub fn enforce(app: &App, slug: &str, task: Option<&str>) -> AppResult<()> {
    let Some(over) = check(app, slug, task)? else { return Ok(()) };
    // The teams on the tasks the budget covers (a day: all active teams).
    let covered: Vec<String> = app.with_tracker(slug, |t| {
        let index = TaskIndex::load(t.conn())?;
        Ok(t.bus()
            .list(false)?
            .into_iter()
            .filter(|team| match &over.scope {
                Scope::Task(id) => index.is_under(&team.task, id),
                Scope::Epic(id) => index.is_under(&team.task, id),
                Scope::Day => true,
            })
            .map(|team| team.id)
            .collect())
    })?;
    for team in &covered {
        crate::runtime::stop_team(app, slug, team, "budget", "genie")?;
    }
    let day = genie_core::db::now()[..10].to_string();
    let dedupe = format!("budget:{slug}:{}:{day}", over.scope.key());
    let ctx = serde_json::json!({ "task": { "id": task } });
    let users = notify::resolve(app, slug, &["task.author".into(), "project.owners".into()], &ctx).unwrap_or_default();
    let msg = Message {
        kind: "budget".into(),
        title: format!("Budget used up for {}", over.scope.words()),
        body: format!(
            "{} Teams on it were stopped ({}). Raise the limit in `budgets` of config.json and restart the server, or let it be.",
            over.explain(),
            if covered.is_empty() { "none was running".to_string() } else { covered.join(", ") }
        ),
        project: Some(slug.into()),
        task: task.map(str::to_string),
        link: task.map(|t| format!("/active?task={t}")),
        ..Default::default()
    };
    let _ = notify::send(app, &users, &msg, Some(&dedupe));
    if !covered.is_empty() {
        let text = format!(
            "{} The teams {} were stopped; do not start them again until the owner raises the limit.",
            over.explain(),
            covered.join(", ")
        );
        crate::runtime::tell_orchestrator(app, slug, task, &text)?;
    }
    Ok(())
}
