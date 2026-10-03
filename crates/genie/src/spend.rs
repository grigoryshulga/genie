//! What the agents' work cost: the tokens of the usage rows priced with
//! `modelPrices` from the configuration, added up per model and grouped by
//! epic, task and chat. A model without a price counts its tokens only.

use std::collections::{BTreeMap, HashMap};

use genie_core::usage::{Tokens, UsageRow};
use serde::{Deserialize, Serialize};

use crate::config::{ModelPrice, price_of};

pub type Prices = BTreeMap<String, ModelPrice>;

/// One model's share.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSpend {
    /// `provider/model`.
    pub model: String,
    pub calls: u64,
    pub tokens: Tokens,
    /// Dollars; `None` when the model has no price.
    pub cost: Option<f64>,
    pub price: Option<ModelPrice>,
}

/// Tokens and money of some work, with its models (most expensive first).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Spend {
    pub calls: u64,
    pub tokens: Tokens,
    /// Dollars, of the models with a price.
    pub cost: f64,
    /// Tokens of models without a price (not in `cost`).
    pub unpriced_tokens: u64,
    pub models: Vec<ModelSpend>,
}

impl Spend {
    pub fn add(&mut self, prices: &Prices, model: &str, calls: u64, tokens: &Tokens) {
        self.calls += calls;
        self.tokens.add(tokens);
        let price = price_of(prices, model).copied();
        match price {
            Some(p) => self.cost += p.cost(tokens),
            None => self.unpriced_tokens += tokens.total(),
        }
        let i = match self.models.iter().position(|m| m.model == model) {
            Some(i) => i,
            None => {
                self.models.push(ModelSpend { model: model.to_string(), price, cost: price.map(|_| 0.0), ..Default::default() });
                self.models.len() - 1
            }
        };
        let m = &mut self.models[i];
        m.calls += calls;
        m.tokens.add(tokens);
        if let (Some(c), Some(p)) = (m.cost.as_mut(), price) {
            *c += p.cost(tokens);
        }
    }

    pub fn add_row(&mut self, prices: &Prices, r: &UsageRow) {
        self.add(prices, &r.model, r.calls, &r.tokens);
    }

    /// Models by cost, then by tokens; costs rounded to cents' hundredths.
    pub fn finish(mut self) -> Spend {
        self.cost = round(self.cost);
        for m in &mut self.models {
            m.cost = m.cost.map(round);
        }
        self.models.sort_by(|a, b| {
            b.cost
                .unwrap_or(-1.0)
                .total_cmp(&a.cost.unwrap_or(-1.0))
                .then(b.tokens.total().cmp(&a.tokens.total()))
                .then(a.model.cmp(&b.model))
        });
        self
    }

    pub fn of(prices: &Prices, rows: &[UsageRow]) -> Spend {
        let mut s = Spend::default();
        for r in rows {
            s.add_row(prices, r);
        }
        s.finish()
    }
}

fn round(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// A group of the spend: an epic, a task or a chat.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    /// The epic or task id, or the chat (`orchestrator`, `<team>/<member>`, `job/<id>`).
    /// Epics also have `""` (tasks outside epics) and `"-"` (work on no task).
    pub id: String,
    pub title: String,
    /// A task's epic; a chat's task.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub spend: Spend,
}

/// What the project's tasks are, for grouping: title, type, parent and status by id.
#[derive(Default)]
pub struct TaskIndex(pub HashMap<String, (String, String, Option<String>, String)>);

impl TaskIndex {
    pub fn load(conn: &rusqlite::Connection) -> rusqlite::Result<TaskIndex> {
        let mut stmt = conn.prepare("SELECT id, title, type, parent, status FROM tasks")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))))?;
        Ok(TaskIndex(rows.collect::<rusqlite::Result<_>>()?))
    }

    pub fn title(&self, id: &str) -> String {
        self.0.get(id).map(|t| t.0.clone()).unwrap_or_default()
    }

    pub fn status(&self, id: &str) -> Option<String> {
        self.0.get(id).map(|t| t.3.clone())
    }

    /// Whether `task` is `ancestor` or lies under it (a subtask, a task of an epic).
    pub fn is_under(&self, task: &str, ancestor: &str) -> bool {
        let mut id = task.to_string();
        for _ in 0..32 {
            if id == ancestor {
                return true;
            }
            match self.0.get(&id).and_then(|t| t.2.clone()) {
                Some(parent) => id = parent,
                None => return false,
            }
        }
        false
    }

    /// The epic of a task (itself for an epic), following parents up.
    pub fn epic_of(&self, id: &str) -> Option<String> {
        let mut id = id.to_string();
        for _ in 0..32 {
            let (_, kind, parent, _) = self.0.get(&id)?;
            if kind == "epic" {
                return Some(id);
            }
            id = parent.clone()?;
        }
        None
    }
}

/// Group the rows by `key` into items, most expensive first, at most `limit`.
pub fn group(prices: &Prices, rows: &[UsageRow], limit: usize, key: impl Fn(&UsageRow) -> Item) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for r in rows {
        let item = key(r);
        let i = *at.entry(item.id.clone()).or_insert_with(|| {
            items.push(item);
            items.len() - 1
        });
        items[i].spend.add_row(prices, r);
    }
    for i in &mut items {
        i.spend = std::mem::take(&mut i.spend).finish();
    }
    items.sort_by(|a, b| b.spend.cost.total_cmp(&a.spend.cost).then(b.spend.tokens.total().cmp(&a.spend.tokens.total())));
    items.truncate(limit);
    items
}

pub fn by_epic(prices: &Prices, rows: &[UsageRow], tasks: &TaskIndex) -> Vec<Item> {
    group(prices, rows, usize::MAX, |r| match &r.task {
        None => Item { id: "-".into(), ..Default::default() },
        Some(t) => match tasks.epic_of(t) {
            Some(e) => Item { title: tasks.title(&e), status: tasks.status(&e), id: e, ..Default::default() },
            None => Item::default(),
        },
    })
}

pub fn by_task(prices: &Prices, rows: &[UsageRow], tasks: &TaskIndex, limit: usize) -> Vec<Item> {
    let tasked: Vec<UsageRow> = rows.iter().filter(|r| r.task.is_some()).cloned().collect();
    group(prices, &tasked, limit, |r| {
        let t = r.task.clone().unwrap_or_default();
        Item {
            title: tasks.title(&t),
            status: tasks.status(&t),
            parent: tasks.epic_of(&t).filter(|e| *e != t),
            id: t,
            spend: Spend::default(),
        }
    })
}

pub fn by_chat(prices: &Prices, rows: &[UsageRow], tasks: &TaskIndex, limit: usize) -> Vec<Item> {
    group(prices, rows, limit, |r| Item {
        id: r.agent.clone(),
        title: r.task.as_deref().map(|t| tasks.title(t)).unwrap_or_default(),
        parent: r.task.clone(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(agent: &str, task: Option<&str>, model: &str, input: u64, output: u64) -> UsageRow {
        UsageRow {
            day: "2026-10-01".into(),
            agent: agent.into(),
            task: task.map(str::to_string),
            model: model.into(),
            calls: 1,
            tokens: Tokens { input, output, ..Default::default() },
        }
    }

    #[test]
    fn models_with_a_price_cost_money_the_rest_only_tokens() {
        let prices: Prices = serde_json::from_value(serde_json::json!({
            "litellm/claude-opus-5-5": { "input": 5, "output": 25, "cacheRead": 0.5 },
            "gpt-6-sol": { "input": 2, "output": 12 }
        }))
        .unwrap();
        let rows = [
            row("t1/executor", Some("A-2"), "litellm/claude-opus-5-5", 1_000_000, 100_000),
            row("t1/executor", Some("A-2"), "openai-codex/gpt-6-sol", 500_000, 0),
            row("orchestrator", None, "ollama/qwen", 70, 30),
            UsageRow {
                tokens: Tokens { cache_read: 2_000_000, ..Default::default() },
                ..row("t1/executor", Some("A-2"), "litellm/claude-opus-5-5", 0, 0)
            },
        ];
        let s = Spend::of(&prices, &rows);
        // 5 + 2.5 for the input and output of opus, 1 for its cache, 1 for sol.
        assert_eq!(s.cost, 9.5);
        assert_eq!(s.unpriced_tokens, 100);
        assert_eq!(s.calls, 4);
        assert_eq!(
            s.models.iter().map(|m| (m.model.as_str(), m.cost)).collect::<Vec<_>>(),
            [("litellm/claude-opus-5-5", Some(8.5)), ("openai-codex/gpt-6-sol", Some(1.0)), ("ollama/qwen", None)]
        );
    }

    #[test]
    fn tasks_group_into_their_epics() {
        let mut idx = TaskIndex::default();
        idx.0.insert("E-1".into(), ("Returns".into(), "epic".into(), None, "open".into()));
        idx.0.insert("T-2".into(), ("Photo".into(), "task".into(), Some("E-1".into()), "done".into()));
        idx.0.insert("T-3".into(), ("Sub".into(), "task".into(), Some("T-2".into()), "open".into()));
        idx.0.insert("T-4".into(), ("Alone".into(), "task".into(), None, "open".into()));
        let prices: Prices = serde_json::from_value(serde_json::json!({ "m": { "input": 1, "output": 1 } })).unwrap();
        let rows = [
            row("a/x", Some("T-2"), "m", 1_000_000, 0),
            row("b/x", Some("T-3"), "m", 2_000_000, 0),
            row("c/x", Some("T-4"), "m", 500_000, 0),
            row("orchestrator", None, "m", 100_000, 0),
        ];
        let epics = by_epic(&prices, &rows, &idx);
        assert_eq!(epics.iter().map(|e| (e.id.as_str(), e.spend.cost)).collect::<Vec<_>>(), [("E-1", 3.0), ("", 0.5), ("-", 0.1)]);
        assert_eq!(epics[0].title, "Returns");
        let tasks = by_task(&prices, &rows, &idx, 2);
        assert_eq!(
            tasks.iter().map(|t| (t.id.as_str(), t.parent.as_deref())).collect::<Vec<_>>(),
            [("T-3", Some("E-1")), ("T-2", Some("E-1"))]
        );
        let chats = by_chat(&prices, &rows, &idx, 10);
        assert_eq!(chats[0].id, "b/x");
        assert_eq!(chats[0].parent.as_deref(), Some("T-3"));
    }
}
