//! Administrative financial evidence and explicit reconciliation of unknown calls.
use super::{Cx, Entry, Need, Op, Out, enc, register};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

pub fn register(all: &mut Vec<Entry>) {
    register!(all, List, Show, Reconcile);
}

/// List recent model calls, their reserved allowance and actual or uncertain costs.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct List {
    #[arg(long)]
    pub task: Option<String>,
}
impl Op for List {
    const GROUP: &'static str = "budget";
    const NAME: &'static str = "list";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let path = self.task.map_or("/budget/calls".into(), |task| format!("/budget/calls?task={}", enc(&task)));
        let value = cx.call("GET", &path, None).await?;
        Ok(Out::new(serde_json::to_string_pretty(&value).unwrap_or_default(), value))
    }
}

/// Inspect one immutable model-call receipt by its opaque call ID.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Show {
    pub id: String,
}
impl Op for Show {
    const GROUP: &'static str = "budget";
    const NAME: &'static str = "show";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let value = cx.call("GET", &format!("/budget/calls/{}", enc(&self.id)), None).await?;
        Ok(Out::new(serde_json::to_string_pretty(&value).unwrap_or_default(), value))
    }
}

/// Replace a terminal uncertain call's reserved cost with an explicitly verified USD amount.
#[derive(clap::Args, Deserialize, JsonSchema)]
pub struct Reconcile {
    pub id: String,
    /// Verified cost in USD, including zero when evidence confirms no charge.
    #[arg(long)]
    pub cost: f64,
    /// Provider receipt or other evidence explaining the amount.
    #[arg(long)]
    pub note: String,
}
impl Op for Reconcile {
    const GROUP: &'static str = "budget";
    const NAME: &'static str = "reconcile";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let cost = genie_core::model_calls::Money::charge(self.cost).map_err(|e| e.to_string())?;
        let value = cx
            .call("POST", &format!("/budget/calls/{}/reconcile", enc(&self.id)), Some(json!({"costNanos":cost.value(),"note":self.note})))
            .await?;
        Ok(Out::new(serde_json::to_string_pretty(&value).unwrap_or_default(), value))
    }
}
