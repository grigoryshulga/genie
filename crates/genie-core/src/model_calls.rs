//! Financial admission for server-owned model calls. An admitted call keeps its
//! maximum exposure until authoritative usage or a human reconciliation closes it.
//! Provider I/O and credentials belong to the application, never to this module.

use crate::usage::{ModelPrice, Tokens, UsageRow};
use crate::{Actor, GenieError, Result, Role, TaskType, Tracker};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(crate) const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS model_workers (
 id TEXT PRIMARY KEY, token_hash TEXT NOT NULL UNIQUE, agent TEXT NOT NULL,
 task TEXT, model TEXT NOT NULL, input_limit INTEGER NOT NULL, output_limit INTEGER NOT NULL,
 profile_fingerprint TEXT NOT NULL, profile TEXT NOT NULL, initiator INTEGER,
 execution_stamp TEXT, closed INTEGER NOT NULL DEFAULT 0, created TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS model_calls (
 id TEXT PRIMARY KEY, worker TEXT NOT NULL REFERENCES model_workers(id),
 agent TEXT NOT NULL, task TEXT, model TEXT NOT NULL, day TEXT NOT NULL,
 input_limit INTEGER NOT NULL, output_limit INTEGER NOT NULL, tariff TEXT NOT NULL,
 reserved INTEGER NOT NULL CHECK(reserved>=0), cost INTEGER CHECK(cost>=0),
 state TEXT NOT NULL CHECK(state IN ('admitted','settled','uncertain','reconciled')),
 tokens TEXT, usage_day TEXT, note TEXT NOT NULL DEFAULT '', created TEXT NOT NULL, finished TEXT,
 CHECK((state IN ('admitted','uncertain') AND cost IS NULL AND tokens IS NULL AND usage_day IS NULL)
   OR (state='reconciled' AND cost IS NOT NULL AND tokens IS NULL AND usage_day IS NULL)
   OR (state='settled' AND cost IS NOT NULL AND tokens IS NOT NULL AND usage_day IS NOT NULL))
);
CREATE TABLE IF NOT EXISTS model_call_scopes (
 call TEXT NOT NULL REFERENCES model_calls(id), scope TEXT NOT NULL, PRIMARY KEY(call,scope)
);
CREATE INDEX IF NOT EXISTS model_calls_task ON model_calls(task,created);
CREATE INDEX IF NOT EXISTS model_calls_day ON model_calls(day);
CREATE INDEX IF NOT EXISTS model_calls_usage ON model_calls(usage_day,agent,task,model) WHERE tokens IS NOT NULL;
CREATE INDEX IF NOT EXISTS model_call_scopes_scope ON model_call_scopes(scope);
CREATE TABLE IF NOT EXISTS model_profile_faults (
 profile TEXT PRIMARY KEY, note TEXT NOT NULL, created TEXT NOT NULL
);
"#;

const NANOS_PER_DOLLAR: f64 = 1_000_000_000.0;

/// Integer nanodollars. Limits round down; exposure and spend round up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct Money(i64);
impl Money {
    pub fn nanos(value: i64) -> Result<Self> {
        if value < 0 {
            return Err(GenieError::invalid("a monetary amount cannot be negative"));
        }
        Ok(Self(value))
    }
    pub fn limit(dollars: f64) -> Result<Self> {
        Self::dollars(dollars, false)
    }
    pub fn charge(dollars: f64) -> Result<Self> {
        Self::dollars(dollars, true)
    }
    fn dollars(value: f64, up: bool) -> Result<Self> {
        let scaled = value * NANOS_PER_DOLLAR;
        if !value.is_finite() || value < 0.0 || !scaled.is_finite() || scaled >= i64::MAX as f64 {
            return Err(GenieError::invalid("monetary amount is outside the supported nanodollar range"));
        }
        let amount = if up { scaled.ceil() } else { scaled.floor() } as i64;
        if value > 0.0 && amount == 0 && !up {
            return Err(GenieError::invalid("a positive budget must be at least one nanodollar; zero means unlimited"));
        }
        Ok(Self(amount))
    }
    pub fn value(self) -> i64 {
        self.0
    }
    pub fn as_dollars(self) -> f64 {
        self.0 as f64 / NANOS_PER_DOLLAR
    }
    pub fn checked_add(self, other: Self) -> Result<Self> {
        self.0.checked_add(other.0).map(Self).ok_or_else(|| GenieError::invalid("monetary amount overflow"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Scope {
    Task(String),
    Epic(String),
    Day,
}
impl Scope {
    pub fn key(&self) -> String {
        match self {
            Self::Task(id) => format!("task:{id}"),
            Self::Epic(id) => format!("epic:{id}"),
            Self::Day => "day".into(),
        }
    }
    pub fn words(&self) -> String {
        match self {
            Self::Task(id) => format!("the task {id}"),
            Self::Epic(id) => format!("the epic {id}"),
            Self::Day => "today".into(),
        }
    }
}

/// Enrollment is made by the runtime, with public model metadata only. No
/// upstream credential is stored in this record or in a call receipt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Worker {
    pub id: String,
    #[serde(skip_serializing)]
    pub token_hash: String,
    #[serde(skip_serializing)]
    pub agent: String,
    pub task: Option<String>,
    pub model: String,
    pub input_limit: u64,
    pub output_limit: u64,
    pub profile_fingerprint: String,
    #[serde(skip_serializing)]
    pub profile: Value,
    #[serde(skip_serializing)]
    pub initiator: Option<i64>,
    #[serde(skip_serializing)]
    pub execution_stamp: Option<String>,
    pub closed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallState {
    Admitted,
    Settled,
    Uncertain,
    Reconciled,
}
impl CallState {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "admitted" => Ok(Self::Admitted),
            "settled" => Ok(Self::Settled),
            "uncertain" => Ok(Self::Uncertain),
            "reconciled" => Ok(Self::Reconciled),
            _ => Err(GenieError::invalid("invalid model call state")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Call {
    pub id: String,
    #[serde(skip_serializing)]
    pub worker: String,
    #[serde(skip_serializing)]
    pub agent: String,
    pub task: Option<String>,
    pub model: String,
    pub day: String,
    pub input_limit: u64,
    pub output_limit: u64,
    pub tariff: ModelPrice,
    pub reserved_nanos: Money,
    pub cost_nanos: Option<Money>,
    pub state: CallState,
    pub tokens: Option<Tokens>,
    pub usage_day: Option<String>,
    pub note: String,
    pub created: String,
    pub finished: Option<String>,
}

pub struct NewCall {
    pub id: String,
    pub worker: String,
    pub task: Option<String>,
    pub output_limit: u64,
    pub tariff: ModelPrice,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub scope: Scope,
    pub used: Money,
    pub pending: Money,
    pub limit: Money,
    pub needed: Money,
}
impl Refusal {
    /// Waiting can help only when active calls, rather than committed or
    /// uncertain charges, currently occupy the required allowance.
    pub fn temporarily_reserved(&self) -> bool {
        self.pending.value() > 0
            && self.used >= self.pending
            && i128::from(self.used.value() - self.pending.value()) + i128::from(self.needed.value()) <= i128::from(self.limit.value())
    }
}
#[derive(Debug, Clone, PartialEq)]
pub enum Admission {
    Granted(Call),
    /// The ID was already admitted. Its provider request must not be sent again.
    Existing(Call),
    Refused(Refusal),
}

pub struct Calls<'a>(&'a Tracker);
impl Tracker {
    pub fn model_calls(&self) -> Calls<'_> {
        Calls(self)
    }
}

fn json_column<T: serde::de::DeserializeOwned>(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<T> {
    let text: String = row.get(column)?;
    serde_json::from_str(&text).map_err(|e| rusqlite::Error::FromSqlConversionFailure(column, rusqlite::types::Type::Text, Box::new(e)))
}

impl Calls<'_> {
    pub fn enroll(&self, worker: &Worker) -> Result<()> {
        if worker.id.is_empty()
            || worker.token_hash.is_empty()
            || worker.agent.is_empty()
            || worker.model.is_empty()
            || worker.input_limit == 0
            || worker.output_limit == 0
            || worker.input_limit > i64::MAX as u64
            || worker.output_limit > i64::MAX as u64
            || worker.closed
        {
            return Err(GenieError::invalid("invalid model worker enrollment"));
        }
        if let Some(task) = &worker.task {
            self.0.execution().ensure_running(task)?;
        }
        self.0.conn().execute("INSERT INTO model_workers(id,token_hash,agent,task,model,input_limit,output_limit,profile_fingerprint,profile,initiator,execution_stamp,created) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
            params![worker.id,worker.token_hash,worker.agent,worker.task,worker.model,worker.input_limit as i64,worker.output_limit as i64,worker.profile_fingerprint,serde_json::to_string(&worker.profile)?,worker.initiator,worker.execution_stamp,crate::db::now()])?;
        Ok(())
    }
    pub fn worker(&self, id: &str) -> Result<Worker> {
        self.0.conn().query_row("SELECT id,token_hash,agent,task,model,input_limit,output_limit,profile_fingerprint,profile,initiator,execution_stamp,closed FROM model_workers WHERE id=?1", [id], |r| Ok(Worker {
            id:r.get(0)?,token_hash:r.get(1)?,agent:r.get(2)?,task:r.get(3)?,model:r.get(4)?,input_limit:r.get::<_,i64>(5)? as u64,output_limit:r.get::<_,i64>(6)? as u64,profile_fingerprint:r.get(7)?,profile:json_column(r,8)?,initiator:r.get(9)?,execution_stamp:r.get(10)?,closed:r.get(11)?,
        })).optional()?.ok_or_else(||GenieError::not_found("model worker not found"))
    }
    pub fn enrolled_token(&self, hash: &str) -> Result<bool> {
        Ok(self.0.conn().query_row("SELECT EXISTS(SELECT 1 FROM model_workers WHERE token_hash=?1)", [hash], |r| r.get(0))?)
    }
    pub fn profile_fault(&self, fingerprint: &str) -> Result<Option<String>> {
        Ok(self.0.conn().query_row("SELECT note FROM model_profile_faults WHERE profile=?1", [fingerprint], |r| r.get(0)).optional()?)
    }
    pub fn call(&self, id: &str) -> Result<Call> {
        let raw = self.0.conn().query_row("SELECT id,worker,agent,task,model,day,input_limit,output_limit,tariff,reserved,cost,state,tokens,usage_day,note,created,finished FROM model_calls WHERE id=?1",[id], |r| Ok((
            Call { id:r.get(0)?,worker:r.get(1)?,agent:r.get(2)?,task:r.get(3)?,model:r.get(4)?,day:r.get(5)?,input_limit:r.get::<_,i64>(6)? as u64,output_limit:r.get::<_,i64>(7)? as u64,tariff:json_column(r,8)?,reserved_nanos:Money(r.get(9)?),cost_nanos:r.get::<_,Option<i64>>(10)?.map(Money),state:CallState::Admitted,tokens:r.get::<_,Option<String>>(12)?.map(|s|serde_json::from_str(&s)).transpose().map_err(|e|rusqlite::Error::FromSqlConversionFailure(12,rusqlite::types::Type::Text,Box::new(e)))?,usage_day:r.get(13)?,note:r.get(14)?,created:r.get(15)?,finished:r.get(16)? },r.get::<_,String>(11)?)))
            .optional()?.ok_or_else(||GenieError::not_found("model call not found"))?;
        Ok(Call { state: CallState::parse(&raw.1)?, ..raw.0 })
    }
    pub fn scopes(&self, task: Option<&str>) -> Result<Vec<Scope>> {
        let mut next = task.map(|id| self.0.get(id)).transpose()?;
        let mut scopes = Vec::new();
        while let Some(task) = next {
            if scopes.len() >= 32 {
                return Err(GenieError::invalid("invalid task ancestry"));
            }
            let scope = if task.task_type == TaskType::Epic { Scope::Epic(task.id) } else { Scope::Task(task.id) };
            if scopes.contains(&scope) {
                return Err(GenieError::invalid("invalid task ancestry"));
            }
            scopes.push(scope);
            next = task.parent.map(|id| self.0.get(&id)).transpose()?;
        }
        scopes.push(Scope::Day);
        Ok(scopes)
    }
    pub fn call_scopes(&self, id: &str) -> Result<Vec<Scope>> {
        let mut stmt = self.0.conn().prepare("SELECT scope FROM model_call_scopes WHERE call=?1 ORDER BY scope")?;
        let keys = stmt.query_map([id], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        keys.into_iter()
            .map(|key| {
                if key == "day" {
                    Ok(Scope::Day)
                } else if let Some(id) = key.strip_prefix("task:") {
                    Ok(Scope::Task(id.into()))
                } else if let Some(id) = key.strip_prefix("epic:") {
                    Ok(Scope::Epic(id.into()))
                } else {
                    Err(GenieError::invalid("invalid frozen model-call scope"))
                }
            })
            .collect()
    }
    pub fn exposure(&self, scope: &Scope) -> Result<Money> {
        self.exposure_on(scope, &crate::db::now()[..10])
    }
    pub fn pending(&self, scope: &Scope) -> Result<Money> {
        self.balance_on(scope, &crate::db::now()[..10], 1)
    }
    /// An in-flight reservation is not an instruction to kill its admitted
    /// worker. Runtime exhaustion checks use committed/uncertain exposure.
    pub fn committed(&self, scope: &Scope) -> Result<Money> {
        self.balance_on(scope, &crate::db::now()[..10], 2)
    }
    fn exposure_on(&self, scope: &Scope, day: &str) -> Result<Money> {
        self.balance_on(scope, day, 0)
    }
    fn balance_on(&self, scope: &Scope, day: &str, part: i64) -> Result<Money> {
        let value=self.0.conn().query_row("SELECT COALESCE(SUM(CASE WHEN c.state IN ('admitted','uncertain') THEN c.reserved ELSE c.cost END),0) FROM model_calls c JOIN model_call_scopes s ON s.call=c.id WHERE s.scope=?1 AND (?1<>'day' OR c.day=?2) AND (?3=0 OR (?3=1 AND c.state='admitted') OR (?3=2 AND c.state<>'admitted'))",params![scope.key(),day,part],|r|r.get(0))?;
        Money::nanos(value)
    }
    /// The closure supplies the configured limit (0: unlimited) and legacy spend.
    /// It runs under the same BEGIN IMMEDIATE as admission and scope discovery.
    pub fn admit(&self, mut input: NewCall, allowance: impl Fn(&Scope, &str, &Tracker) -> Result<(Money, Money)>) -> Result<Admission> {
        self.0.tx(|| {
            let at=crate::db::now();
            let day=&at[..10];
            input.task=input.task.as_deref().map(|id|self.0.get(id).map(|t|t.id)).transpose()?;
            match self.call(&input.id) {
                Ok(call)=> {
                    if call.worker!=input.worker || call.task!=input.task || call.output_limit!=input.output_limit {
                        return Err(GenieError::Denied("a model call ID cannot be rebound".into()));
                    }
                    return Ok(Admission::Existing(call));
                }
                Err(GenieError::NotFound(_))=>{},
                Err(error)=>return Err(error),
            }
            let worker=self.worker(&input.worker)?;
            if worker.closed {return Err(GenieError::Denied("model worker is closed".into()));}
            if let Some(note)=self.profile_fault(&worker.profile_fingerprint)? {return Err(GenieError::Denied(format!("model profile needs correction: {note}")));}
            if worker.task.is_some() && worker.task!=input.task {return Err(GenieError::Denied("model worker task cannot change".into()));}
            if let Some(task)=&input.task {self.0.execution().ensure_running(task)?;}
            if let Some(task)=&worker.task
                && worker.execution_stamp.as_ref()!=Some(&self.0.execution().stamp(task)?) {
                return Err(GenieError::Denied("execution changed since model worker enrollment".into()));
            }
            if input.id.is_empty() || input.output_limit==0 || input.output_limit>worker.output_limit {return Err(GenieError::invalid("invalid model output bound"));}
            let reserved=reserve(&input.tariff,worker.input_limit,input.output_limit)?;
            let scopes=self.scopes(input.task.as_deref())?;
            for scope in &scopes {
                let (limit,legacy)=allowance(scope,day,self.0)?;
                let used=legacy.checked_add(self.exposure_on(scope,day)?)?;
                if limit.value()>0 && (used>=limit || used.checked_add(reserved)?>limit) {
                    return Ok(Admission::Refused(Refusal {scope:scope.clone(),used,pending:self.balance_on(scope,day,1)?,limit,needed:reserved}));
                }
            }
            self.0.conn().execute("INSERT INTO model_calls(id,worker,agent,task,model,day,input_limit,output_limit,tariff,reserved,state,created) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'admitted',?11)",params![input.id,input.worker,worker.agent,input.task,worker.model,day,worker.input_limit as i64,input.output_limit as i64,serde_json::to_string(&input.tariff)?,reserved.value(),at])?;
            for scope in scopes {self.0.conn().execute("INSERT INTO model_call_scopes(call,scope) VALUES (?1,?2)",params![input.id,scope.key()])?;}
            self.call(&input.id).map(Admission::Granted)
        })
    }

    pub fn settle(&self, id: &str, tokens: Tokens) -> Result<Call> {
        self.0.tx(|| {
            let call = self.call(id)?;
            if let Some(previous) = &call.tokens {
                if *previous == tokens && call.state == CallState::Settled {
                    return Ok(call);
                }
                return Err(GenieError::Denied("a settled model call cannot be charged again with different usage".into()));
            }
            let input = tokens.input.checked_add(tokens.cache_read).and_then(|n| n.checked_add(tokens.cache_write));
            if input.and_then(|n| n.checked_add(tokens.output)).is_none_or(|n| n > i64::MAX as u64)
                || [tokens.input, tokens.output, tokens.cache_read, tokens.cache_write].iter().any(|n| *n > i64::MAX as u64)
            {
                return Err(GenieError::invalid("model usage overflows the supported token range"));
            }
            complete_price(&call.tariff)?;
            let cost = charge_tokens(&call.tariff, &tokens)?;
            let at = crate::db::now();
            let usage_day = &at[..10];
            self.0.record_usage_on(&call.agent, call.task.as_deref(), &call.model, &tokens, usage_day)?;
            self.0.conn().execute(
                "UPDATE model_calls SET state='settled',tokens=?2,cost=?3,usage_day=?4,finished=?5 WHERE id=?1",
                params![id, serde_json::to_string(&tokens)?, cost.value(), usage_day, at],
            )?;
            if input.is_some_and(|n| n > call.input_limit) || tokens.output > call.output_limit || cost > call.reserved_nanos {
                let worker = self.worker(&call.worker)?;
                self.0.conn().execute(
                    "INSERT OR IGNORE INTO model_profile_faults(profile,note,created) VALUES (?1,?2,?3)",
                    params![
                        worker.profile_fingerprint,
                        "Provider usage exceeded the admitted model bounds; correct the profile before sending more calls",
                        at
                    ],
                )?;
            }
            self.event("model_call.settled", &call, "genie", "orchestrator", json!({"costNanos":cost,"tokens":tokens}))?;
            self.call(id)
        })
    }

    pub fn uncertain(&self, id: &str, note: &str) -> Result<Call> {
        self.0.tx(|| {
            let call = self.call(id)?;
            if call.state != CallState::Admitted {
                return Ok(call);
            }
            self.0
                .conn()
                .execute("UPDATE model_calls SET state='uncertain',note=?2,finished=?3 WHERE id=?1", params![id, note, crate::db::now()])?;
            self.event("model_call.uncertain", &call, "genie", "orchestrator", json!({"reservedNanos":call.reserved_nanos,"note":note}))?;
            self.call(id)
        })
    }

    pub fn reconcile(&self, actor: &Actor, id: &str, cost: Money, note: &str) -> Result<Call> {
        if actor.role != Role::Human {
            return Err(GenieError::Denied("only a person reconciles uncertain model spend".into()));
        }
        if note.trim().is_empty() {
            return Err(GenieError::invalid("a reconciliation needs evidence or a reason"));
        }
        self.0.tx(|| {
            let call = self.call(id)?;
            if call.state == CallState::Reconciled && call.cost_nanos == Some(cost) && call.note == note {
                return Ok(call);
            }
            if call.state != CallState::Uncertain {
                return Err(GenieError::Denied("only an uncertain terminal call can be reconciled".into()));
            }
            self.0.conn().execute(
                "UPDATE model_calls SET state='reconciled',cost=?2,note=?3,finished=?4 WHERE id=?1",
                params![id, cost.value(), note, crate::db::now()],
            )?;
            self.event("model_call.reconciled", &call, &actor.name, "human", json!({"costNanos":cost,"note":note}))?;
            self.call(id)
        })
    }

    /// A lost server cannot prove a provider did not accept an admitted call.
    pub fn recover(&self) -> Result<Vec<Call>> {
        self.0.tx(|| {
            self.0.conn().execute("UPDATE model_workers SET closed=1 WHERE closed=0", [])?;
            let mut stmt = self.0.conn().prepare("SELECT id FROM model_calls WHERE state='admitted' ORDER BY created,id")?;
            let ids = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            ids.into_iter()
                .map(|id| self.uncertain(&id, "Server restarted before authoritative model usage was settled; maximum exposure retained"))
                .collect()
        })
    }

    pub fn list(&self, task: Option<&str>, limit: usize) -> Result<Vec<Call>> {
        let mut stmt =
            self.0.conn().prepare("SELECT id FROM model_calls WHERE (?1 IS NULL OR task=?1) ORDER BY created DESC,id DESC LIMIT ?2")?;
        let ids = stmt.query_map(params![task, limit.min(500) as i64], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
        ids.into_iter().map(|id| self.call(&id)).collect()
    }

    /// Usage aggregates include settled gateway calls for Statistics. Remove
    /// their exact tokens before pricing legacy usage, then add the immutable
    /// ledger exposure. This prevents both double charging and tariff rewrites.
    pub fn legacy_rows(&self, mut rows: Vec<UsageRow>) -> Result<Vec<UsageRow>> {
        for row in &mut rows {
            let mut stmt = self.0.conn().prepare(
                "SELECT tokens FROM model_calls WHERE usage_day=?1 AND agent=?2 AND task IS ?3 AND model=?4 AND tokens IS NOT NULL",
            )?;
            let tokens = stmt
                .query_map(params![row.day, row.agent, row.task.as_deref(), row.model], |r| json_column::<Tokens>(r, 0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            for accounted in tokens {
                let subtract = |value: u64, part: u64| {
                    value.checked_sub(part).ok_or_else(|| GenieError::invalid("model ledger and usage aggregates disagree"))
                };
                row.tokens = Tokens {
                    input: subtract(row.tokens.input, accounted.input)?,
                    output: subtract(row.tokens.output, accounted.output)?,
                    cache_read: subtract(row.tokens.cache_read, accounted.cache_read)?,
                    cache_write: subtract(row.tokens.cache_write, accounted.cache_write)?,
                };
                row.calls = row.calls.checked_sub(1).ok_or_else(|| GenieError::invalid("model ledger and usage call counts disagree"))?;
            }
        }
        rows.retain(|row| {
            row.calls > 0 || [row.tokens.input, row.tokens.output, row.tokens.cache_read, row.tokens.cache_write].iter().any(|n| *n > 0)
        });
        Ok(rows)
    }

    fn event(&self, kind: &str, call: &Call, actor: &str, role: &str, data: Value) -> Result<()> {
        // Unassigned intake calls have no public event carrying a private
        // response identity; their receipt remains available to administrators.
        crate::events::append(
            self.0.conn(),
            kind,
            call.task.as_deref(),
            actor,
            role,
            json!({"call":call.id,"model":call.model,"data":data}),
        )?;
        Ok(())
    }
}

pub fn complete_price(price: &ModelPrice) -> Result<()> {
    if ![Some(price.input), Some(price.output), price.cache_read, price.cache_write]
        .into_iter()
        .all(|v| v.is_some_and(|v| v.is_finite() && v >= 0.0))
    {
        return Err(GenieError::invalid("complete finite nonnegative model tariffs are required"));
    }
    Ok(())
}
pub fn reserve(price: &ModelPrice, input: u64, output: u64) -> Result<Money> {
    complete_price(price)?;
    let input_price = token_rate(price.input)?
        .max(token_rate(price.cache_read.unwrap_or_default())?)
        .max(token_rate(price.cache_write.unwrap_or_default())?);
    token_cost(&[(input, input_price), (output, token_rate(price.output)?)])
}

/// Round each tariff up to nanodollars/token once. Reservation and settlement
/// then use exact integer arithmetic instead of rounded floating-point totals.
fn token_rate(dollars_per_million: f64) -> Result<Money> {
    Money::charge(dollars_per_million / 1_000_000.0)
}
fn token_cost(parts: &[(u64, Money)]) -> Result<Money> {
    let mut cost = Money::default();
    for (count, rate) in parts {
        let part = i64::try_from(i128::from(*count) * i128::from(rate.value()))
            .map_err(|_| GenieError::invalid("model charge exceeds the supported range"))?;
        cost = cost.checked_add(Money::nanos(part)?)?;
    }
    Ok(cost)
}
pub fn charge_tokens(price: &ModelPrice, tokens: &Tokens) -> Result<Money> {
    complete_price(price)?;
    token_cost(&[
        (tokens.input, token_rate(price.input)?),
        (tokens.output, token_rate(price.output)?),
        (tokens.cache_read, token_rate(price.cache_read.unwrap_or_default())?),
        (tokens.cache_write, token_rate(price.cache_write.unwrap_or_default())?),
    ])
}
