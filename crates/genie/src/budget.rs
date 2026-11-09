//! Budgets for what agents' work costs (`budgets` in the configuration): one task, one
//! epic, one day of a project. Enabled budgets fail closed on unknown prices and
//! pause affected execution without discarding its teams or progress. A person
//! explicitly resumes it after correcting the limit or tariff. Thresholds use
//! reported legacy usage and immutable model-call receipts; in-flight reservations
//! are handled by financial admission rather than killed as exhausted work.

use genie_core::model_calls::Money;
pub use genie_core::model_calls::Scope;
use genie_core::usage::usage_since;

use crate::notify::{self, Message};
use crate::spend::{Prices, Spend};
use crate::state::{App, AppError, AppResult};

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

pub(crate) fn budget_price<'a>(prices: &'a crate::spend::Prices, model: &str) -> Option<&'a crate::config::ModelPrice> {
    // A tariff from another provider with the same model name is not authority
    // for spending. An explicit unqualified operator tariff remains supported.
    prices.get(model).or_else(|| prices.get(model.rsplit_once('/').map_or(model, |(_, tail)| tail)))
}

fn spend(prices: &Prices, rows: &[genie_core::usage::UsageRow]) -> Spend {
    let mut spent = Spend::default();
    for row in rows {
        match budget_price(prices, &row.model) {
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
            _ => {
                spent.unpriced_tokens = spent.unpriced_tokens.saturating_add(
                    [row.tokens.input, row.tokens.output, row.tokens.cache_read, row.tokens.cache_write]
                        .into_iter()
                        .fold(0_u64, u64::saturating_add)
                        .max(1),
                )
            }
        }
    }
    // Display rounding must never create extra budget headroom.
    spent
}

pub(crate) fn legacy_spend(t: &genie_core::Tracker, prices: &Prices, scope: &Scope, day: &str) -> genie_core::Result<Spend> {
    let rows = match scope {
        Scope::Task(task) | Scope::Epic(task) => t.usage_of_task(task)?,
        Scope::Day => usage_since(t.conn(), day)?.into_iter().filter(|r| r.day == day).collect(),
    };
    Ok(spend(prices, &t.model_calls().legacy_rows(rows)?))
}

pub(crate) fn scope_limit(b: &crate::config::Budgets, scope: &Scope) -> f64 {
    match scope {
        Scope::Task(_) => b.per_task,
        Scope::Epic(_) => b.per_epic,
        Scope::Day => b.per_day,
    }
}

pub(crate) fn allowance(
    t: &genie_core::Tracker,
    prices: &Prices,
    b: &crate::config::Budgets,
    scope: &Scope,
    day: &str,
) -> genie_core::Result<(Money, Money)> {
    let limit = Money::limit(scope_limit(b, scope))?;
    if limit.value() == 0 {
        return Ok((limit, Money::default()));
    }
    let spent = legacy_spend(t, prices, scope, day)?;
    if spent.unpriced_tokens > 0 {
        return Err(genie_core::GenieError::Denied(
            Exceeded { scope: scope.clone(), spent: spent.cost, limit: scope_limit(b, scope), unpriced_tokens: spent.unpriced_tokens }
                .explain(),
        ));
    }
    Ok((limit, Money::charge(spent.cost)?))
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
    Ok(check_scopes(app, slug, task, None)?.into_iter().next())
}

fn check_scopes(app: &App, slug: &str, task: Option<&str>, frozen: Option<&[Scope]>) -> AppResult<Vec<Exceeded>> {
    let b = app.cfg.budgets;
    if b.per_task <= 0.0 && b.per_epic <= 0.0 && b.per_day <= 0.0 {
        return Ok(Vec::new());
    }
    let prices = app.budget_prices()?;
    app.with_tracker(slug, |t| {
        let day = genie_core::db::now()[..10].to_string();
        let scopes = match frozen {
            Some(scopes) => scopes.to_vec(),
            None => t.model_calls().scopes(task)?,
        };
        let mut exceeded = Vec::new();
        for scope in scopes {
            let limit = scope_limit(&b, &scope);
            if limit <= 0.0 {
                continue;
            }
            let spent = legacy_spend(t, &prices, &scope, &day)?;
            if spent.unpriced_tokens > 0 {
                exceeded.push(Exceeded { scope, spent: spent.cost, limit, unpriced_tokens: spent.unpriced_tokens });
                continue;
            }
            let used = Money::charge(spent.cost)?.checked_add(t.model_calls().committed(&scope)?)?;
            if used >= Money::limit(limit)? {
                exceeded.push(Exceeded { scope, spent: used.as_dollars(), limit, unpriced_tokens: 0 });
            }
        }
        Ok(exceeded)
    })
}

/// After a report of usage on `task`: when a budget is used up, stop the teams it covers and tell people once.
pub fn enforce(app: &App, slug: &str, task: Option<&str>) -> AppResult<()> {
    apply_limits(app, slug, task, check_scopes(app, slug, task, None)?)
}

/// Settlements and human reconciliations enforce the scopes captured at
/// admission, including an old epic after a task moves while a call runs.
pub(crate) fn enforce_call(app: &App, slug: &str, call: &genie_core::model_calls::Call) -> AppResult<()> {
    let scopes = app.with_tracker(slug, |t| t.model_calls().call_scopes(&call.id))?;
    apply_limits(app, slug, call.task.as_deref(), check_scopes(app, slug, call.task.as_deref(), Some(&scopes))?)
}

fn apply_limits(app: &App, slug: &str, task: Option<&str>, limits: Vec<Exceeded>) -> AppResult<()> {
    let mut failure = None;
    for over in limits {
        if let Err(error) = apply_limit(app, slug, task, &over) {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}

fn apply_limit(app: &App, slug: &str, task: Option<&str>, over: &Exceeded) -> AppResult<()> {
    let _gate = app.execution_gate.lock().map_err(|_| crate::state::AppError::Internal("execution gate poisoned".into()))?;
    let root = match &over.scope {
        Scope::Task(id) | Scope::Epic(id) => Some(id.as_str()),
        Scope::Day => None,
    };
    let mut failure = None;
    let tasks = match root.map_or_else(|| crate::runtime::active_tasks(app, slug), |root| Ok(vec![root.to_string()])) {
        Ok(tasks) => tasks,
        Err(error) => {
            failure.get_or_insert(error);
            Vec::new()
        }
    };
    for task in tasks {
        match app.with_tracker(slug, |t| {
            t.execution().pause(&genie_core::Actor::new("genie", genie_core::Role::Orchestrator), &task, "budget", &over.explain())
        }) {
            Ok(_) | Err(AppError::Genie(genie_core::GenieError::NotFound(_))) => {}
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    // Termination cannot depend on every pause write succeeding. The exhausted
    // budget still fences admission while recoverable persistence errors surface.
    let covered = match crate::runtime::halt_scope(app, slug, root) {
        Ok(covered) => covered,
        Err(error) => {
            failure.get_or_insert(error);
            Vec::new()
        }
    };
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
    if root.is_none()
        && let Err(error) = notify::send(app, &users, &msg, Some(&dedupe))
    {
        failure.get_or_insert(error);
    }
    app.wake_engine.notify_one();
    if !covered.is_empty() {
        let text = format!(
            "{} The agents {} were stopped; do not start them again until the owner raises the limit.",
            over.explain(),
            covered.join(", ")
        );
        if let Err(error) = crate::runtime::tell_orchestrator(app, slug, task, &text) {
            failure.get_or_insert(error);
        }
    }
    failure.map_or(Ok(()), Err)
}
