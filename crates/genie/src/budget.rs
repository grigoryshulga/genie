//! Budgets for what agents' work costs (`budgets` in the configuration): one task, one
//! epic, one day of a project. Enabled budgets fail closed on unknown prices and
//! pause affected execution without discarding its teams or progress. A person
//! explicitly resumes it after correcting the limit or tariff. Thresholds use
//! reported usage; per-request financial reservations are not implemented here.

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
    pub unpriced_tokens: u64,
}

impl Exceeded {
    pub fn explain(&self) -> String {
        if self.unpriced_tokens > 0 {
            return format!(
                "budget: {} has {} tokens without valid prices; configure complete model/cache tariffs before resuming",
                self.scope.words(),
                self.unpriced_tokens
            );
        }
        format!(
            "budget: the spend of {} is ${:.2}, the limit is ${:.2} (`budgets` in config.json; restart the server after raising it)",
            self.scope.words(),
            self.spent,
            self.limit
        )
    }
}

fn budget_price<'a>(prices: &'a crate::spend::Prices, model: &str) -> Option<&'a crate::config::ModelPrice> {
    // A tariff from another provider with the same model name is not authority
    // for spending. An explicit unqualified operator tariff remains supported.
    prices.get(model).or_else(|| prices.get(model.rsplit_once('/').map_or(model, |(_, tail)| tail)))
}

fn spend(app: &App, rows: &[genie_core::usage::UsageRow]) -> Spend {
    let prices = app.budget_prices().unwrap_or_else(|_| app.cfg.model_prices.clone());
    let mut spent = Spend::default();
    for row in rows {
        match budget_price(&prices, &row.model) {
            Some(price)
                if [Some(price.input), Some(price.output), price.cache_read, price.cache_write]
                    .into_iter()
                    .flatten()
                    .all(|v| v.is_finite() && v >= 0.0) =>
            {
                let (cost, unpriced) = price.split(&row.tokens);
                spent.cost += cost;
                spent.unpriced_tokens = spent.unpriced_tokens.saturating_add(unpriced);
            }
            _ => spent.unpriced_tokens = spent.unpriced_tokens.saturating_add(row.tokens.total().max(1)),
        }
    }
    // Display rounding must never create extra budget headroom.
    spent
}

fn exceeded(scope: Scope, spent: Spend, limit: f64) -> Option<Exceeded> {
    (spent.unpriced_tokens > 0 || spent.cost >= limit).then_some(Exceeded {
        scope,
        spent: spent.cost,
        limit,
        unpriced_tokens: spent.unpriced_tokens,
    })
}

/// Every enabled scope needs usable prices before an agent starts, not only after usage.
pub(crate) fn admit_model(app: &App, slug: &str, task: Option<&str>, model: Option<&str>) -> AppResult<()> {
    let b = app.cfg.budgets;
    if b.per_task <= 0.0 && b.per_epic <= 0.0 && b.per_day <= 0.0 {
        return Ok(());
    }
    let prices = app.budget_prices()?;
    let valid = model.and_then(|m| budget_price(&prices, m)).is_some_and(|p| {
        [Some(p.input), Some(p.output), p.cache_read, p.cache_write].into_iter().all(|v| v.is_some_and(|v| v.is_finite() && v >= 0.0))
    });
    if !valid {
        let note = format!("budget: model {} needs valid input, output, cache-read and cache-write prices", model.unwrap_or("unknown"));
        if let Some(task) = task {
            let _gate = app.execution_gate.lock().map_err(|_| crate::state::AppError::Internal("execution gate poisoned".into()))?;
            app.with_tracker(slug, |t| {
                t.execution().pause(&genie_core::Actor::new("genie", genie_core::Role::Orchestrator), task, "budget", &note)
            })?;
            crate::runtime::halt_scope(app, slug, Some(task))?;
        }
        return Err(crate::state::AppError::Conflict(note));
    }
    Ok(())
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
                let index = TaskIndex::load(t.conn())?;
                let mut cursor = Some(t.get(task)?.id);
                for _ in 0..32 {
                    let Some(id) = cursor else { break };
                    if index.0.get(&id).is_some_and(|row| row.1 != "epic")
                        && let Some(over) = exceeded(Scope::Task(id.clone()), spend(app, &t.usage_of_task(&id)?), b.per_task)
                    {
                        return Ok(Some(over));
                    }
                    cursor = index.0.get(&id).and_then(|row| row.2.clone());
                }
            }
            if b.per_epic > 0.0
                && let Some(epic) = TaskIndex::load(t.conn())?.epic_of(task)
                && let Some(over) = exceeded(Scope::Epic(epic.clone()), spend(app, &t.usage_of_task(&epic)?), b.per_epic)
            {
                return Ok(Some(over));
            }
        }
        if b.per_day > 0.0 {
            let today = genie_core::db::now()[..10].to_string();
            let rows: Vec<_> = usage_since(t.conn(), &today)?.into_iter().filter(|r| r.day == today).collect();
            if let Some(over) = exceeded(Scope::Day, spend(app, &rows), b.per_day) {
                return Ok(Some(over));
            }
        }
        Ok(None)
    })
}

/// After a report of usage on `task`: when a budget is used up, stop the teams it covers and tell people once.
pub fn enforce(app: &App, slug: &str, task: Option<&str>) -> AppResult<()> {
    let Some(over) = check(app, slug, task)? else { return Ok(()) };
    let _gate = app.execution_gate.lock().map_err(|_| crate::state::AppError::Internal("execution gate poisoned".into()))?;
    let root = match &over.scope {
        Scope::Task(id) | Scope::Epic(id) => Some(id.as_str()),
        Scope::Day => None,
    };
    if let Some(root) = root {
        app.with_tracker(slug, |t| {
            t.execution().pause(&genie_core::Actor::new("genie", genie_core::Role::Orchestrator), root, "budget", &over.explain())
        })?;
    }
    if root.is_none() {
        for task in crate::runtime::active_tasks(app, slug)? {
            app.with_tracker(slug, |t| {
                t.execution().pause(&genie_core::Actor::new("genie", genie_core::Role::Orchestrator), &task, "budget", &over.explain())
            })?;
        }
    }
    let covered = crate::runtime::halt_scope(app, slug, root)?;
    let day = genie_core::db::now()[..10].to_string();
    let dedupe = format!("budget:{slug}:{}:{day}", over.scope.key());
    let ctx = serde_json::json!({ "task": { "id": task } });
    let users = notify::resolve(app, slug, &["task.author".into(), "project.owners".into()], &ctx).unwrap_or_default();
    let msg = Message {
        kind: "budget".into(),
        title: format!("Budget used up for {}", over.scope.words()),
        body: format!(
            "{} Execution on it was paused ({}). Raise the limit in `budgets` of config.json and restart the server, or let it be.",
            over.explain(),
            if covered.is_empty() { "none was running".to_string() } else { covered.join(", ") }
        ),
        project: Some(slug.into()),
        task: task.map(str::to_string),
        link: task.map(|t| format!("/active?task={t}")),
        ..Default::default()
    };
    if root.is_none() {
        notify::send(app, &users, &msg, Some(&dedupe))?;
    }
    app.wake_engine.notify_one();
    if !covered.is_empty() {
        let text = format!(
            "{} The agents {} were stopped; do not start them again until the owner raises the limit.",
            over.explain(),
            covered.join(", ")
        );
        crate::runtime::tell_orchestrator(app, slug, task, &text)?;
    }
    Ok(())
}
