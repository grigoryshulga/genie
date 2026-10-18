//! Outcome cases after delivery (`genie observe`): their contracts, evidence, guardrail incidents
//! and evaluation, and the sources they measure (`docs/platform/observation-plane.md` §16). Each
//! operation calls the route of §15 with the request type the server reads, so the command line,
//! MCP and the web share one contract. The API checks every right (§14).

use std::path::PathBuf;
use std::str::FromStr;

use genie_core::GenieError;
use genie_core::observation::contract::Contract;
use genie_core::observation::{EvidencePhase, ObservationPolicy, SourceAuth, SourceKind, parse_case_id, parse_incident_id};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use super::{Cx, Entry, Listed, Need, Op, Out, enc, opt_json_text, register};
use crate::observation::service::{
    ContractBody, CreateCaseBody, DeploymentBody, EvaluateBody, EvidenceBody, FollowupBody, NoteBody, ReasonBody, SecretBody, SourceBody,
    SourceTestBody, UpdateCaseBody,
};

pub fn register(all: &mut Vec<Entry>) {
    register!(
        all,
        List,
        Show,
        Create,
        Update,
        SetContract,
        Arm,
        Start,
        Evaluate,
        Cancel,
        Deploy,
        Evidence,
        Report,
        Incidents,
        Resolve,
        Dismiss,
        Followup,
        Sources,
        SourceAdd,
        SourceUpdate,
        SourceRemove,
        SourceSecret,
        SourceTest,
    );
}

fn out(value: Value) -> Out {
    Out::new(serde_json::to_string_pretty(&value).unwrap_or_default(), value)
}

/// A case key (`O-1`) or number (`1`) of the command line.
fn case_id(text: &str) -> Result<i64, String> {
    parse_case_id(text).map_err(|e| e.to_string())
}

/// An incident key (`I-1`) or number (`1`) of the command line.
fn incident_id(text: &str) -> Result<i64, String> {
    parse_incident_id(text).map_err(|e| e.to_string())
}

/// A string-backed enum argument, checked by the core's own parser so every entrance says the same.
fn word<T: FromStr<Err = GenieError>>(text: &str) -> Result<T, String> {
    text.parse().map_err(|e: GenieError| e.to_string())
}

/// A key or number as text (`O-1`) or as a JSON number (`1`), so MCP callers may use either.
fn key_of(value: Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn key_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let value = Value::deserialize(d)?;
    key_of(value).ok_or_else(|| serde::de::Error::custom("the id must be a key such as O-1, or a number"))
}

fn opt_key_text<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        value => key_of(value).map(Some).ok_or_else(|| serde::de::Error::custom("the incident must be a key such as I-1, or a number")),
    }
}

/// A secret that must be a JSON string. Any other value is refused with a fixed message that never
/// echoes it: a secret must not reach a log or an error message.
fn opt_secret<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    match Value::deserialize(d)? {
        Value::Null => Ok(None),
        Value::String(text) => Ok(Some(text)),
        _ => Err(serde::de::Error::custom("value must be a string: the bearer token, or user:password")),
    }
}

/// A contract as JSON, given inline or read from a file (the command line only).
fn read_contract(cx: &Cx, inline: Option<String>, file: Option<PathBuf>) -> Result<Option<Contract>, String> {
    cx.text(inline, file)?.map(|text| serde_json::from_str(&text).map_err(|e| format!("the contract must be JSON: {e}"))).transpose()
}

/// Outcome cases of the project, newest first; filter by status or by linked task.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct List {
    /// proposed, waiting, observing, evaluating, concluded or cancelled.
    #[arg(long)]
    #[serde(default)]
    pub status: Option<String>,
    /// Only the cases that observe this task.
    #[arg(long)]
    #[serde(default)]
    pub task: Option<String>,
}
impl Op for List {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "list";
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let path =
            format!("/observation/cases?status={}&task={}", enc(&self.status.unwrap_or_default()), enc(&self.task.unwrap_or_default()));
        Ok(out(cx.call("GET", &path, None).await?))
    }
}

/// An outcome case: its status and contract, why it cannot be armed, its tasks, evaluations, incidents and recent evidence.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Show {
    /// The case: `O-1` or `1`.
    #[arg(value_name = "CASE")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
}
impl Op for Show {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "show";
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = case_id(&self.id)?;
        Ok(out(cx.call("GET", &format!("/observation/cases/{id}"), None).await?))
    }
}

/// Create an outcome case after delivery for its tasks: policy `guardrails` (watch limits) or `verify` (check hypotheses).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Create {
    /// The title (1 to 200 characters).
    #[arg(long)]
    pub title: String,
    /// `guardrails` or `verify`.
    #[arg(long)]
    pub policy: String,
    /// A task the case observes (repeat for several).
    #[arg(long)]
    #[serde(default)]
    pub task: Vec<String>,
    /// The contract as JSON.
    #[arg(long, allow_hyphen_values = true, value_name = "JSON")]
    #[serde(default, deserialize_with = "opt_json_text")]
    #[schemars(with = "Option<serde_json::Map<String, Value>>")]
    pub contract: Option<String>,
    /// Read the contract from a file (command line only).
    #[arg(long)]
    #[serde(default)]
    #[schemars(skip)]
    pub contract_file: Option<PathBuf>,
}
impl Op for Create {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "create";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let body = CreateCaseBody {
            title: self.title,
            policy: word::<ObservationPolicy>(&self.policy)?,
            tasks: self.task,
            contract: read_contract(cx, self.contract, self.contract_file)?,
        };
        Ok(out(cx.send("POST", "/observation/cases", &body).await?))
    }
}

/// Update an outcome case's title, policy or linked tasks; a proposed or waiting case takes any of them, an observing one only its title.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    /// The case: `O-1` or `1`.
    #[arg(value_name = "CASE")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
    /// The new title (1 to 200 characters).
    #[arg(long)]
    #[serde(default)]
    pub title: Option<String>,
    /// `guardrails` or `verify`.
    #[arg(long)]
    #[serde(default)]
    pub policy: Option<String>,
    /// A task to link (repeat for several).
    #[arg(long)]
    #[serde(default)]
    pub add_task: Vec<String>,
    /// A task to unlink (repeat for several).
    #[arg(long)]
    #[serde(default)]
    pub remove_task: Vec<String>,
}
impl Op for Update {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "update";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = case_id(&self.id)?;
        let body = UpdateCaseBody {
            title: self.title,
            policy: self.policy.as_deref().map(word::<ObservationPolicy>).transpose()?,
            add_tasks: (!self.add_task.is_empty()).then_some(self.add_task),
            remove_tasks: (!self.remove_task.is_empty()).then_some(self.remove_task),
        };
        Ok(out(cx.send("PATCH", &format!("/observation/cases/{id}"), &body).await?))
    }
}

/// Set the contract of an outcome case: a new version. After observation started only a person may, with a reason.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetContract {
    /// The case: `O-1` or `1`.
    #[arg(value_name = "CASE")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
    /// The contract as JSON.
    #[arg(long, allow_hyphen_values = true, value_name = "JSON")]
    #[serde(default, deserialize_with = "opt_json_text")]
    #[schemars(with = "Option<serde_json::Map<String, Value>>")]
    pub contract: Option<String>,
    /// Read the contract from a file (command line only).
    #[arg(long)]
    #[serde(default)]
    #[schemars(skip)]
    pub file: Option<PathBuf>,
    /// Why the contract changes; required after observation started.
    #[arg(long)]
    #[serde(default)]
    pub reason: Option<String>,
}
impl Op for SetContract {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "contract";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = case_id(&self.id)?;
        let contract =
            read_contract(cx, self.contract, self.file)?.ok_or("pass the contract as --contract JSON (or --file on the command line)")?;
        let body = ContractBody { contract, reason: self.reason };
        Ok(out(cx.send("PUT", &format!("/observation/cases/{id}/contract"), &body).await?))
    }
}

/// Arm an outcome case so it waits for its tasks; refused, with the problems, while any remains.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Arm {
    /// The case: `O-1` or `1`.
    #[arg(value_name = "CASE")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
}
impl Op for Arm {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "arm";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = case_id(&self.id)?;
        Ok(out(cx.call("POST", &format!("/observation/cases/{id}/arm"), None).await?))
    }
}

/// Start observing an armed outcome case now; its anchor is the current time.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Start {
    /// The case: `O-1` or `1`.
    #[arg(value_name = "CASE")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
}
impl Op for Start {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "start";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = case_id(&self.id)?;
        Ok(out(cx.call("POST", &format!("/observation/cases/{id}/start"), None).await?))
    }
}

/// Request an evaluation of an observing case now; `--final` (people only) concludes it with the evidence it has.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Evaluate {
    /// The case: `O-1` or `1`.
    #[arg(value_name = "CASE")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
    /// A final evaluation: concludes the case with the verdict it gives.
    #[arg(long = "final")]
    #[serde(default, rename = "final")]
    pub final_: bool,
}
impl Op for Evaluate {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "evaluate";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = case_id(&self.id)?;
        let body = EvaluateBody { final_: self.final_.then_some(true) };
        Ok(out(cx.send("POST", &format!("/observation/cases/{id}/evaluate"), &body).await?))
    }
}

/// Cancel an open outcome case with a reason; the reason becomes its conclusion.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Cancel {
    /// The case: `O-1` or `1`.
    #[arg(value_name = "CASE")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
    /// Why the case is cancelled; the reason becomes its conclusion.
    #[arg(long)]
    pub reason: String,
}
impl Op for Cancel {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "cancel";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = case_id(&self.id)?;
        let body = ReasonBody { reason: self.reason };
        Ok(out(cx.send("POST", &format!("/observation/cases/{id}/cancel"), &body).await?))
    }
}

/// Record a deployment of an outcome case; in a case that waits for one, it starts the observation.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Deploy {
    /// The case: `O-1` or `1`.
    #[arg(value_name = "CASE")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
    /// A commit SHA, tag or release name.
    #[arg(id = "ref", long = "ref")]
    #[serde(rename = "ref")]
    pub reference: String,
    /// The environment deployed to, such as `production`.
    #[arg(long)]
    #[serde(default)]
    pub environment: Option<String>,
    /// When it was deployed (RFC 3339; default now).
    #[arg(long)]
    #[serde(default)]
    pub at: Option<String>,
}
impl Op for Deploy {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "deploy";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = case_id(&self.id)?;
        let body = DeploymentBody { reference: self.reference, environment: self.environment, at: self.at };
        Ok(out(cx.send("POST", &format!("/observation/cases/{id}/deployments"), &body).await?))
    }
}

/// Evidence records of an outcome case, oldest first; filter by criterion or continue after a record.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    /// The case: `O-1` or `1`.
    #[arg(value_name = "CASE")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
    /// Only the records of this hypothesis or guardrail.
    #[arg(long)]
    #[serde(default)]
    pub criterion: Option<String>,
    /// Only the records after this record id.
    #[arg(long)]
    #[serde(default)]
    pub after: Option<i64>,
    /// At most this many records (1 to 500).
    #[arg(long)]
    #[serde(default)]
    pub limit: Option<i64>,
}
impl Op for Evidence {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "evidence";
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = case_id(&self.id)?;
        let path = format!(
            "/observation/cases/{id}/evidence?after={}&limit={}&criterion={}",
            self.after.map(|n| n.to_string()).unwrap_or_default(),
            self.limit.map(|n| n.to_string()).unwrap_or_default(),
            enc(&self.criterion.unwrap_or_default())
        );
        Ok(out(cx.call("GET", &path, None).await?))
    }
}

/// Report a manual measurement of a criterion of an observing case; a reported guardrail violation opens its incident at once.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The case: `O-1` or `1`.
    #[arg(value_name = "CASE")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
    /// The hypothesis or guardrail id of the contract.
    #[arg(long)]
    pub criterion: String,
    /// The measured value (a finite number).
    #[arg(long, allow_hyphen_values = true)]
    pub value: f64,
    /// Overrides the baseline of the criterion for the change.
    #[arg(long, allow_hyphen_values = true)]
    #[serde(default)]
    pub baseline: Option<f64>,
    /// How many samples the value covers, when known.
    #[arg(long)]
    #[serde(default)]
    pub samples: Option<i64>,
    /// `observed` (default) or `baseline`.
    #[arg(long)]
    #[serde(default)]
    pub phase: Option<String>,
    /// Start of the measured window (RFC 3339; default: the anchor of observation).
    #[arg(long)]
    #[serde(default)]
    pub from: Option<String>,
    /// End of the measured window (RFC 3339; default: now).
    #[arg(long)]
    #[serde(default)]
    pub to: Option<String>,
    /// Where the value came from, for the people who read the record.
    #[arg(long)]
    #[serde(default)]
    pub note: Option<String>,
}
impl Op for Report {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "report";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = case_id(&self.id)?;
        let body = EvidenceBody {
            criterion: self.criterion,
            phase: self.phase.as_deref().map(word::<EvidencePhase>).transpose()?,
            value: self.value,
            baseline: self.baseline,
            samples: self.samples,
            window_start: self.from,
            window_end: self.to,
            note: self.note,
        };
        Ok(out(cx.send("POST", &format!("/observation/cases/{id}/evidence"), &body).await?))
    }
}

/// Guardrail incidents of the project, newest first; `--status open` lists the ones still open.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Incidents {
    /// open, resolved or dismissed.
    #[arg(long)]
    #[serde(default)]
    pub status: Option<String>,
}
impl Op for Incidents {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "incidents";
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("GET", &format!("/observation/incidents?status={}", enc(&self.status.unwrap_or_default())), None).await?))
    }
}

/// Resolve an incident: the violation still counts in the evaluation.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Resolve {
    /// The incident: `I-1` or `1`.
    #[arg(value_name = "INCIDENT")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
    /// A note on the resolution (optional).
    #[arg(long)]
    #[serde(default)]
    pub note: Option<String>,
}
impl Op for Resolve {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "resolve";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = incident_id(&self.id)?;
        let body = NoteBody { note: self.note };
        Ok(out(cx.send("POST", &format!("/observation/incidents/{id}/resolve"), &body).await?))
    }
}

/// Dismiss an incident as a false positive, with a note: the violation no longer counts. Only a person may.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Dismiss {
    /// The incident: `I-1` or `1`.
    #[arg(value_name = "INCIDENT")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
    /// Why the incident is a false positive (required).
    #[arg(long)]
    pub note: String,
}
impl Op for Dismiss {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "dismiss";
    const NEED: Need = Need::Person;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = incident_id(&self.id)?;
        let body = NoteBody { note: Some(self.note) };
        Ok(out(cx.send("POST", &format!("/observation/incidents/{id}/dismiss"), &body).await?))
    }
}

/// Create a follow-up task for an outcome case: a bug when it answers an incident, otherwise a task.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Followup {
    /// The case: `O-1` or `1`.
    #[arg(value_name = "CASE")]
    #[serde(deserialize_with = "key_text")]
    #[schemars(with = "String")]
    pub id: String,
    /// The incident of this case the task answers: `I-1` or `1`.
    #[arg(long)]
    #[serde(default, deserialize_with = "opt_key_text")]
    #[schemars(with = "Option<String>")]
    pub incident: Option<String>,
    /// The title of the task (default: derived from the incident or the case).
    #[arg(long)]
    #[serde(default)]
    pub title: Option<String>,
    /// The description of the task (default: links to the case, its tasks and the incident evidence).
    #[arg(long)]
    #[serde(default)]
    pub description: Option<String>,
}
impl Op for Followup {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "followup";
    const NEED: Need = Need::Orchestrator;
    const LISTED: Listed = Listed::Orchestrator;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let id = case_id(&self.id)?;
        let body = FollowupBody {
            incident: self.incident.as_deref().map(incident_id).transpose()?,
            title: self.title,
            description: self.description,
        };
        Ok(out(cx.send("POST", &format!("/observation/cases/{id}/followup"), &body).await?))
    }
}

/// The metrics and error sources of the project, with their secrets as hints only (project admins; people only).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Sources {}
impl Op for Sources {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "sources";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("GET", "/observation/sources", None).await?))
    }
}

/// The settings of a source, as `source-add` and `source-update` take them.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceArgs {
    /// The source's name, such as `prom`.
    #[arg(value_name = "NAME")]
    pub name: String,
    /// `prometheus` or `sentry`.
    #[arg(long)]
    pub kind: String,
    /// The base URL of the service, such as `https://prometheus.internal`.
    #[arg(long)]
    pub url: String,
    /// `none` (default), `bearer` or `basic`; the secret is set with `source-secret`.
    #[arg(long)]
    #[serde(default)]
    pub auth: Option<String>,
    /// Sentry organization (required for sentry).
    #[arg(long)]
    #[serde(default)]
    pub organization: Option<String>,
    /// Sentry project to filter by.
    #[arg(long)]
    #[serde(default)]
    pub sentry_project: Option<String>,
    /// Environment to filter by.
    #[arg(long)]
    #[serde(default)]
    pub environment: Option<String>,
    /// Sentry field to read (default `count()`).
    #[arg(long)]
    #[serde(default)]
    pub field: Option<String>,
    /// Sentry dataset (default `errors`).
    #[arg(long)]
    #[serde(default)]
    pub dataset: Option<String>,
    /// Request timeout in seconds (default 30, at most 120).
    #[arg(long)]
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

fn source_body(args: SourceArgs) -> Result<SourceBody, String> {
    Ok(SourceBody {
        name: args.name,
        kind: word::<SourceKind>(&args.kind)?,
        url: args.url,
        auth: args.auth.as_deref().map(word::<SourceAuth>).transpose()?,
        organization: args.organization,
        sentry_project: args.sentry_project,
        environment: args.environment,
        field: args.field,
        dataset: args.dataset,
        timeout_secs: args.timeout_secs,
    })
}

/// Add a metrics or error source to the project (people only); its secret is set with `source-secret`.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceAdd {
    #[command(flatten)]
    #[serde(flatten)]
    pub source: SourceArgs,
}
impl Op for SourceAdd {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "source-add";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let body = source_body(self.source)?;
        Ok(out(cx.send("POST", "/observation/sources", &body).await?))
    }
}

/// Change a source of the project (people only); the name stays the same.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceUpdate {
    #[command(flatten)]
    #[serde(flatten)]
    pub source: SourceArgs,
}
impl Op for SourceUpdate {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "source-update";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let path = format!("/observation/sources/{}", enc(&self.source.name));
        let body = source_body(self.source)?;
        Ok(out(cx.send("PUT", &path, &body).await?))
    }
}

/// Remove a source of the project (people only); its secret goes with it.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceRemove {
    /// The source's name, such as `prom`.
    #[arg(value_name = "NAME")]
    pub name: String,
}
impl Op for SourceRemove {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "source-remove";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        Ok(out(cx.call("DELETE", &format!("/observation/sources/{}", enc(&self.name)), None).await?))
    }
}

/// Set or clear the secret of a source (people only): a bearer token, or `user:password` for basic auth.
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceSecret {
    /// The source's name, such as `prom`.
    #[arg(value_name = "NAME")]
    pub name: String,
    /// The secret: the bearer token, or `user:password`. `-` reads stdin (command line only). The secret is never printed back.
    #[arg(long, allow_hyphen_values = true, conflicts_with = "clear")]
    #[serde(default, deserialize_with = "opt_secret")]
    pub value: Option<String>,
    /// Remove the stored secret instead.
    #[arg(long, conflicts_with = "value")]
    #[serde(default)]
    pub clear: bool,
}
impl Op for SourceSecret {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "source-secret";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let path = format!("/observation/sources/{}/secret", enc(&self.name));
        match (self.clear, self.value) {
            (true, None) => Ok(out(cx.call("DELETE", &path, None).await?)),
            (false, Some(value)) => {
                if value == "-" && !cx.local {
                    return Err("`-` reads stdin on the command line only; pass the secret itself".into());
                }
                // A piped secret ends with a newline; trim it as repository tokens are.
                let value = cx.text(Some(value), None)?.unwrap_or_default().trim().to_string();
                if value.is_empty() {
                    return Err("stdin held no secret; pass --clear to remove the stored one".into());
                }
                Ok(out(cx.send("PUT", &path, &SecretBody { value }).await?))
            }
            _ => Err("pass the secret as --value (`-` reads stdin), or --clear to remove the stored one".into()),
        }
    }
}

/// Test a source with a query over a recent window and show the value it gives (people only).
#[derive(clap::Args, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceTest {
    /// The source's name, such as `prom`.
    #[arg(value_name = "NAME")]
    pub name: String,
    /// The query in the source's language (PromQL for prometheus).
    #[arg(long)]
    pub query: String,
    /// A duration such as `30m`, `1h` or `2d` (default 1h).
    #[arg(long)]
    #[serde(default)]
    pub window: Option<String>,
}
impl Op for SourceTest {
    const GROUP: &'static str = "observe";
    const NAME: &'static str = "source-test";
    const NEED: Need = Need::Admin;
    async fn run(self, cx: &Cx) -> Result<Out, String> {
        let path = format!("/observation/sources/{}/test", enc(&self.name));
        let body = SourceTestBody { query: self.query, window: self.window };
        Ok(out(cx.send("POST", &path, &body).await?))
    }
}
