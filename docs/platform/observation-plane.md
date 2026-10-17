---
title: "Observation plane: Phase 1 specification"
summary: Implementation design for outcome cases, contracts, evidence, guardrail incidents, and evaluation after delivery.
type: note
status: draft
---

# Observation plane: Phase 1 specification

This page turns the [observation plane proposal](observation-plane-proposal.md) into an implementable design for Phase 1 (MVP). The task breakdown for implementation agents is in the [implementation plan](../development/observation-plane-plan.md). Section numbers (§N) are stable references for task cards and reviews.

The design is grounded in the code as of `main` at `562ae02` (release 0.5.1). Where this page and the proposal differ, this page wins for Phase 1; every deviation is listed in §2.3.

## 1. Scope

### 1.1 Goals (Phase 1)

- First-class **outcome cases** (`O-<n>`) linked to one or many tasks, with policy `guardrails` or `verify`.
- Versioned **outcome contracts** written before any post-delivery evidence is evaluated.
- Scheduled, persistent observation: no agent process stays alive while waiting.
- **Evidence records** from a Prometheus-compatible metrics source, a Sentry-compatible error source, and manual reports, with provenance and trust level.
- **Guardrail watch**: a violated guardrail opens an **incident** (`I-<n>`) immediately.
- Manual or automatic **follow-up tasks** created through the normal task services.
- Deterministic evaluation with results `confirmed`, `negative`, `inconclusive`.
- Access through the shared operation catalog (CLI and MCP), REST, journal events, and the web UI.

### 1.2 Non-goals (Phase 1)

Taken from the proposal (§16) plus implementation limits:

- No product analytics, experiments, attribution analysis, or causal inference. The evaluator records `attribution: not assessed`.
- No LLM in the evaluation path. The evaluator is a pure, deterministic function (§9). LLM analysts and incident investigators are Phase 2.
- No change to the task state machine, task table, or the delivery gates. The planes stay separate.
- No deployment provider integration. Deployments are recorded explicitly (§6.5).
- No push subscriptions from sources; the scheduler polls.
- No automatic project policy or memory mutation.

## 2. Fit with the current architecture

### 2.1 What the code already provides

| Need | Existing mechanism | Location |
| --- | --- | --- |
| Durable event bus between planes | Per-project journal `events` written in the same transaction as the change; subscribers keep cursors in `event_cursors` and acknowledge | `crates/genie-core/src/events.rs`, `Tracker::append_event`, `Tracker::ack_events` |
| Background loop that sleeps until a deadline or a wake-up | Automation engine `start`/`pass`/`Plan`, `SAFETY_NET`, `sleep_until`, `App::wake_*` notifies | `crates/genie/src/engine.rs`, `crates/genie/src/state.rs` |
| Wake on every journal event | `Tracker::on_event` hook installed in `App::project_tracker` | `crates/genie/src/state.rs` |
| Delivery facts | `task.status_changed` (to `done`), `cr.merged`, `ci.*` events; `task_repos` rows | `crates/genie-core/src/tracker.rs`, `crates/genie/src/git/delivery.rs` |
| Durable lifecycle entity next to tasks, with versioned snapshots | Task managers, briefs, clarifications (`task_managers`, `task_briefs`) | `crates/genie-core/src/management.rs` |
| Application rules shared by all entrances | `crates/genie/src/management.rs`, `crates/genie/src/tasks.rs` (`Caller`, `tasks::create`) | |
| Project-manager authority check | `crate::management::project_manager(&actor)` | `crates/genie/src/management.rs` |
| Sealed credentials | `secrets.rs` (`repo_secrets` pattern) | `crates/genie-core/src/secrets.rs` |
| CLI and MCP from one definition | Operation catalog (`Op`, `register!`, `GROUPS`) | `crates/genie/src/ops/` |
| Versioned schema migrations with rehearsal | `SCHEMA_VERSION`, `SERVER_SCHEMA_VERSION`, `migrate::preflight` | `crates/genie-core/src/db.rs`, `server_db.rs`, `migrate.rs` |
| Notifications | `notify::resolve`, `notify::send`; project-manager mail via `Management::notify` | `crates/genie/src/notify.rs` |
| Outbound HTTP from synchronous code | `state::block_on` + `reqwest` (see `engine::step_http`) | `crates/genie/src/engine.rs` |
| Generated web types | `ts-rs` with `#[ts(export)]`, written by `cargo test` | `.cargo/config.toml` |
| Live web refresh | `keysFor` maps journal event types to query-key prefixes | `web/src/shared/api/invalidation.ts` |

### 2.2 Constraints found in the code

- `crates/genie/src/outcome.rs` already exists and means "how an agent run ended". **Do not reuse the name.** All new code uses the module name `observation`.
- Task IDs are `<PREFIX>-<n>` with a project-chosen prefix, and the automation engine resolves every event `subject` as a task (`engine::event_context`). Case and incident keys could collide with a prefix such as `O`. Therefore journal events use the subject `observation:<id>`, never `O-<id>` (§12).
- `genie-core` must stay synchronous (no Tokio, no HTTP). Adapters, the scheduler, and notifications live in `crates/genie`.
- Database locks must be released before outbound HTTP. The scheduler reads, releases, queries sources, then writes in a new transaction (§11).
- Persistent state and its journal event share one transaction (coding standards).
- The UI language is Russian; documentation, identifiers, and code comments are English.

### 2.3 Decisions and deviations from the proposal

| ID | Decision | Reason |
| --- | --- | --- |
| D1 | Module name `observation` in both crates; tables prefixed `observation_`. | `outcome` is taken (§2.2). |
| D2 | Cases, contracts, evidence, evaluations, incidents, deployments, and follow-ups live in the **project tracker** (`genie.db`, schema 6 → 7). Sources and their secrets live in **`server.db`** (schema 6 → 7). | Case state and journal events must commit together; secrets must stay out of tracker backups, like `repo_secrets`. |
| D3 | Policy is a property of the case. `none` is represented by the absence of a case; no task column is added. | Keeps the delivery lifecycle untouched (proposal §16). Per-task-class defaults are Phase 2. |
| D4 | Phase 1 policies: `guardrails`, `verify`. `outcome` and `experiment` are not accepted yet. | Proposal §15. The enum is extended in Phase 2/3. |
| D5 | `status` and `result` are separate fields: status `concluded` carries result `confirmed`, `negative`, or `inconclusive`. | Smaller state machine; result stays first-class. |
| D6 | Evaluation is deterministic (§9). "Insufficient evidence" keeps the case observing; the final evaluation never returns insufficient. | Proposal §6.2 without an LLM in the critical path. |
| D7 | A guardrail violation opens an incident at once, but the case concludes `negative` only at an evaluation at or after the minimum window, giving people time to dismiss a false positive. | Proposal §6.1 and §10 (weak evidence). |
| D8 | The contract may change freely in `proposed`/`waiting`. After observation starts, only a person may revise it, with a mandatory reason; evidence keeps the contract version it was collected under, and evaluation uses only the current version's evidence. | Proposal §4: no silent retrospective rewriting. |
| D9 | Evidence trust levels: `measured` (adapter), `reported` (a person), `agent` (an agent report). The evaluator ignores `agent` evidence. | Proposal §17: neither plane validates itself. |
| D10 | Case keys `O-<id>`, incident keys `I-<id>` (database autoincrement IDs, per project). Journal subject `observation:<id>`. | §2.2 collision. |
| D11 | Deployments are explicit markers recorded through the API; Phase 2 adds provider adapters. | No deployment source exists today. |
| D12 | Adapters are an enum dispatch (`match kind`) with one async `measure` function, not a trait object. The proposal's `query/snapshot/compare/subscribe` map to `measure` (query, snapshot), the evaluator (compare), and polling (subscribe, deferred). | Simpler code; no `async_trait`. |

## 3. Glossary

Russian names and every UI label are in [the glossary](https://github.com/grigoryshulga/genie/blob/main/GLOSSARY.md#observation-plane), outside this site because the documentation is English-only.

| Term | Meaning |
| --- | --- |
| Outcome case | Post-delivery observation of one or more delivered tasks. Key `O-<n>`. |
| Outcome contract | Versioned expectations of a case: hypotheses, guardrails, windows, sources. |
| Hypothesis | An expected result that `verify` checks, such as "p95 latency −20%". |
| Guardrail | A limit that must not be crossed, such as "error rate +0.1pp at most". |
| Evidence record | One structured measurement with provenance. |
| Incident | An open guardrail violation of a case. Key `I-<n>`. |
| Anchor | The moment observation started; windows are measured from it. |

## 4. Domain model

### 4.1 Enumerations

All are string-backed, stored as TEXT, serialized as the same string, and exported to TypeScript. Define them with the existing `str_enum!` macro from `crates/genie-core/src/model.rs` (export it with `pub(crate) use str_enum;` right after the macro definition).

| Rust type | Values | Notes |
| --- | --- | --- |
| `ObservationPolicy` | `guardrails`, `verify` | D3, D4 |
| `CaseStatus` | `proposed`, `waiting`, `observing`, `evaluating`, `concluded`, `cancelled` | §6 |
| `CaseResult` | `confirmed`, `negative`, `inconclusive` | Set only with `concluded` |
| `Verdict` | `confirmed`, `negative`, `inconclusive`, `insufficient` | Output of one evaluation |
| `Confidence` | `low`, `medium`, `high` | §9.4 |
| `CriterionKind` | `hypothesis`, `guardrail` | |
| `CriterionState` | `met`, `not_met`, `ok`, `violated`, `insufficient` | Per-criterion evaluation detail |
| `EvidencePhase` | `baseline`, `observed` | §7.2 |
| `TrustLevel` | `measured`, `reported`, `agent` | D9 |
| `EvalTrigger` | `window`, `manual`, `final` | §11.4 |
| `StartOn` | `tasks_done`, `deployment`, `manual` | §5.1 |
| `FollowupMode` | `manual`, `auto` | §10.2 |
| `IncidentStatus` | `open`, `resolved`, `dismissed` | §10.1 |
| `SourceKind` | `prometheus`, `sentry` | §7.3 |
| `SourceAuth` | `none`, `bearer`, `basic` | §7.3 |

### 4.2 Keys and IDs

- A case has an integer `id` (autoincrement) and a display key `O-<id>`; an incident has `id` and key `I-<id>`.
- `parse_case_id(s)` accepts `O-7`, `o-7`, `7`, and `observation:7` and returns `7`; anything else is `GenieError::Invalid("unknown observation id <s>")`. `parse_incident_id(s)` accepts `I-3`, `i-3`, `3`.
- `case_key(id) -> "O-<id>"`, `incident_key(id) -> "I-<id>"`, `case_subject(id) -> "observation:<id>"`.
- Timestamps are RFC 3339 UTC strings with milliseconds, exactly as `genie_core::db::now()` produces (`2026-10-17T12:00:00.000Z`), so string comparison orders them. Use one helper `observation::ts(DateTime<Utc>) -> String` (`to_rfc3339_opts(SecondsFormat::Millis, true)`) and `observation::parse_ts(&str) -> Result<DateTime<Utc>>` everywhere.

### 4.3 Entities (wire shapes)

All structs: `#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]`, `#[serde(rename_all = "camelCase")]`, `#[ts(export)]`. Optional fields use `Option<T>` and `#[ts(optional_fields)]` like the existing core types.

`ObservationCase`

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | `i64` | |
| `key` | `String` | `O-<id>` |
| `title` | `String` | Non-empty, at most 200 characters |
| `policy` | `ObservationPolicy` | |
| `status` | `CaseStatus` | |
| `result` | `Option<CaseResult>` | Only when `concluded` |
| `confidence` | `Option<Confidence>` | Only when `concluded` |
| `conclusion` | `String` | Summary of the concluding evaluation, or the cancellation reason |
| `contractVersion` | `i64` | `0` while no contract exists |
| `tasks` | `Vec<String>` | Linked task IDs, sorted |
| `author` | `String` | Actor name that created it |
| `created`, `updated` | `String` | |
| `armedAt` | `Option<String>` | Entered `waiting` |
| `observationStartedAt` | `Option<String>` | The anchor |
| `observationCompletedAt` | `Option<String>` | Concluded or cancelled |
| `nextEvaluationAt` | `Option<String>` | |
| `nextGuardrailAt` | `Option<String>` | |
| `evaluationRequested` | `Option<EvalTrigger>` | `manual` or `final` when requested (§6.4) |
| `failures` | `i64` | Consecutive scheduler passes with a collection error |
| `lastError` | `Option<String>` | Last collection error, for the UI |

`ContractVersion`: `caseId`, `version` (1-based), `contract: Contract`, `author`, `reason` (empty unless revised after start), `created`.

`Deployment`: `id`, `caseId`, `ref` (Rust field `reference` with `#[serde(rename = "ref")]`; a commit SHA, tag, or release name; 1–200 chars), `environment` (may be empty), `at`, `recordedBy`, `created`.

`Evidence`

| Field | Type | Meaning |
| --- | --- | --- |
| `id`, `caseId`, `contractVersion` | `i64` | |
| `criterion` | `String` | Hypothesis or guardrail id |
| `kind` | `CriterionKind` | |
| `phase` | `EvidencePhase` | |
| `source` | `String` | Source name or `manual` |
| `query` | `String` | The query sent (after placeholder substitution); empty for manual |
| `at` | `String` | When it was recorded |
| `windowStart`, `windowEnd` | `String` | Measured window |
| `value` | `Option<f64>` | `None` when `error` is set |
| `baseline` | `Option<f64>` | Copied from the baseline record for change criteria |
| `deltaAbs` | `Option<f64>` | `value − baseline` |
| `deltaRel` | `Option<f64>` | `(value − baseline) / abs(baseline) × 100`, in percent; `None` when baseline is 0 |
| `samples` | `Option<i64>` | Sample count when known |
| `provenance` | `serde_json::Value` (`#[ts(type = "unknown")]`) | §7.4 |
| `trust` | `TrustLevel` | |
| `error` | `Option<String>` | Collection failure |
| `note` | `String` | Free text for manual reports |

A record is **usable** when `error` is `None`, `value` is `Some`, and `trust` is `measured` or `reported`.

`Evaluation`: `id`, `caseId`, `contractVersion`, `at`, `trigger: EvalTrigger`, `verdict: Verdict`, `confidence: Option<Confidence>`, `summary: String`, `details: Vec<CriterionResult>`, `actor`.

`CriterionResult`: `criterion`, `kind: CriterionKind`, `state: CriterionState`, `evidence: Option<i64>` (record used), `reason: String` (human-readable English, such as `"delta -23.1% meets target -20%"`).

`Incident`: `id`, `key` (`I-<id>`), `caseId`, `caseKey`, `guardrail`, `status: IncidentStatus`, `summary`, `evidence: Option<i64>`, `created`, `closed: Option<String>`, `closedBy: Option<String>`, `note`, `followups: Vec<String>` (task IDs).

`Followup`: `id`, `caseId`, `incident: Option<i64>`, `task`, `actor`, `created`.

`SourceConfig` (server-wide per project): `name`, `kind: SourceKind`, `url`, `auth: SourceAuth`, `organization: Option<String>`, `sentryProject: Option<String>`, `environment: Option<String>`, `field: Option<String>`, `dataset: Option<String>`, `timeoutSecs: Option<u64>`, `created`, `updated`. The API adds `secret: Option<SecretHint>` (`hint`, `updated`) and never returns the secret.

## 5. Outcome contract

### 5.1 JSON shape

Stored as canonical JSON (`serde_json::to_string` of the struct). Field names are camelCase; unknown fields are rejected (`#[serde(deny_unknown_fields)]`).

```json
{
  "hypotheses": [
    {
      "id": "checkout_latency_p95",
      "source": "prom",
      "query": "histogram_quantile(0.95, sum(rate(http_request_duration_seconds_bucket{route=\"/checkout\"}[$__range])) by (le))",
      "samplesQuery": "sum(increase(http_requests_total{route=\"/checkout\"}[$__range]))",
      "expect": { "change": "-20%" }
    }
  ],
  "guardrails": [
    { "id": "error_rate", "source": "prom", "query": "100 * sum(rate(http_requests_total{code=~\"5..\"}[$__range])) / sum(rate(http_requests_total[$__range]))", "maxChange": "+0.1pp" },
    { "id": "checkout_errors", "source": "sentry", "query": "transaction:/checkout level:error", "max": 50 }
  ],
  "observation": {
    "start": "tasks_done",
    "baselineWindow": "7d",
    "minimumWindow": "24h",
    "preferredWindow": "72h",
    "minimumSamples": 10000,
    "evaluationInterval": "6h",
    "guardrailInterval": "15m",
    "guardrailWindow": "1h"
  },
  "followup": "manual"
}
```

Rust types (in `genie_core::observation::contract`):

```rust
pub struct Contract {
    #[serde(default)] pub hypotheses: Vec<Hypothesis>,
    #[serde(default)] pub guardrails: Vec<Guardrail>,
    pub observation: ObservationWindows,
    #[serde(default)] pub followup: FollowupMode,   // default manual
}
pub struct Hypothesis { pub id: String, pub source: String, #[serde(default)] pub query: String,
                        pub samples_query: Option<String>, pub expect: Expectation }
pub struct Expectation { pub change: Option<String>, pub max: Option<f64>, pub min: Option<f64> }
pub struct Guardrail { pub id: String, pub source: String, #[serde(default)] pub query: String,
                       pub max_change: Option<String>, pub min_change: Option<String>,
                       pub max: Option<f64>, pub min: Option<f64> }
pub struct ObservationWindows { #[serde(default)] pub start: StartOn,            // default tasks_done
    pub baseline_window: Option<String>,   // default "7d"
    pub minimum_window: String,            // required
    pub preferred_window: Option<String>,  // default = minimum_window
    pub minimum_samples: Option<u64>,
    pub evaluation_interval: Option<String>, // default "6h"
    pub guardrail_interval: Option<String>,  // default "15m"
    pub guardrail_window: Option<String> }   // default "1h"
```

`Contract::resolved(&self) -> Resolved` returns the windows as `chrono::Duration` with defaults applied. Durations use `genie_core::automation::parse_duration` (`30s`, `15m`, `24h`, `2d`).

### 5.2 Change grammar

`parse_change(s) -> Result<Change, String>` where `Change { value: f64, unit: ChangeUnit }` and `ChangeUnit` is `Relative` or `Absolute`.

- Pattern: optional surrounding whitespace, a **required** sign `+` or `-`, a decimal number (`\d+(\.\d+)?`), then an optional unit: `%` (relative) or `pp` (absolute). No unit means absolute.
- `value` keeps the sign: `"-20%"` → `{ value: -20.0, unit: Relative }`; `"+0.1pp"` → `{ value: 0.1, unit: Absolute }`; `"+5"` → `{ value: 5.0, unit: Absolute }`.
- Zero (`"+0%"`) is rejected for hypotheses (`expect.change must not be zero`) but allowed for guardrails (`maxChange: "+0"` means "must not grow").
- `pp` treats the metric as already expressed in percent: the delta is `value − baseline` in the metric's own units. Document this in the user docs.
- Delta of a record against a change: relative uses `deltaRel` (percent), absolute uses `deltaAbs`.

### 5.3 Validation

`Contract::problems(&self, policy: ObservationPolicy) -> Vec<String>` returns every problem (empty means valid). The core rejects an invalid contract with `GenieError::Invalid(format!("contract is invalid: {}", problems.join("; ")))`. Rules:

1. Policy `verify`: at least one hypothesis. Policy `guardrails`: no hypotheses and at least one guardrail.
2. Criterion ids match `^[a-z0-9_]{1,64}$` and are unique across hypotheses and guardrails.
3. `source` matches `^[a-z0-9][a-z0-9_-]{0,31}$` or equals `manual`. Whether a named source exists is checked when the case is armed (§6.2), not here.
4. `query` is non-empty unless `source` is `manual`; at most 4000 characters. `samplesQuery`, when present, is non-empty and at most 4000 characters.
5. `expect` has exactly one of `change`, `max`, `min`; `change` parses (§5.2) and is not zero.
6. A guardrail has at least one of `maxChange`, `minChange`, `max`, `min`; each present change parses.
7. Every duration parses and is positive. `guardrailInterval` ≥ 1 minute. `minimumWindow` ≤ `preferredWindow` ≤ 90 days. `baselineWindow` ≤ 90 days. `guardrailWindow`, `guardrailInterval`, and `evaluationInterval` ≤ the effective `preferredWindow` (`minimumWindow` when absent); the default `guardrailWindow` is `min(1h, preferredWindow)`.
8. `minimumSamples`, when present, is ≥ 1.
9. At most 20 hypotheses and 20 guardrails.

`Contract::needs_baseline(criterion)` is true for a hypothesis whose `expect.change` is set and for a guardrail with `maxChange` or `minChange`.

## 6. Lifecycle

### 6.1 States

```text
proposed ──arm──▶ waiting ──start / all tasks done / deployment──▶ observing ⇄ evaluating ──▶ concluded(result)
   │                 │                                               │
   └──────cancel─────┴──────────────────cancel──────────────────────┘──▶ cancelled
```

`evaluating` is held only while the scheduler evaluates a case; after a restart a case found in `evaluating` is handled exactly like `observing`.

### 6.2 Transitions

"Manager" means a person with project write access or the project manager agent (`crate::management::project_manager` succeeds). Task managers, team members, and jobs may not change cases (§14).

| Transition | Who | Preconditions | Effects | Event |
| --- | --- | --- | --- | --- |
| create → `proposed` | Manager | Title non-empty; policy valid; every task exists; contract (optional) valid for the policy | Insert case; link tasks; contract version 1 if given | `outcome.created` |
| update (title, policy, tasks) | Manager | Status `proposed` or `waiting` (title also in `observing`); a policy change re-validates the current contract | Update; `updated` | `outcome.updated` |
| set contract | Manager in `proposed`/`waiting`; **person only** with non-empty `reason` in `observing`/`evaluating` | Valid for the policy; rejected in `concluded`/`cancelled` | New version `n+1`; `contractVersion = n+1` | `outcome.updated` (`contractVersion`, `reason`) |
| arm: `proposed` → `waiting` | Manager | Contract exists; ≥ 1 linked task; no arm problems (below) | `armedAt`; if `start = tasks_done` and every linked task is `done`, start at once in the same transaction | `outcome.updated` (`status`) and maybe `observation.started` |
| start: `waiting` → `observing` | Manager (any `start` mode); scheduler (`tasks_done`); deployment record (`deployment`) | Status `waiting` | Anchor = now (manual, tasks done) or the deployment's `at`; `nextGuardrailAt = anchor`; `nextEvaluationAt = anchor + minimumWindow` | `observation.started` |
| request evaluation | Manager; `final` only by a person | Status `observing` | `evaluationRequested = manual` or `final` | `outcome.ready_for_evaluation` |
| evaluation (scheduler) | System | `observing`/`evaluating` | §11.4 | `outcome.evaluated` or `outcome.<result>` |
| cancel | Manager | Not `concluded`/`cancelled`; reason non-empty | `conclusion = reason`; `observationCompletedAt` when it had started | `outcome.cancelled` |

**Arm problems** (application layer, `observation::arm_problems`), each an English sentence; arming fails with `AppError::Conflict(problems.join("; "))` when any exists:

- `no contract yet`; `contract is invalid: …` (re-validated).
- `link at least one task`.
- `unknown source <name>` for each non-`manual` source missing from the project's sources.
- `source <name> needs a secret` when its `auth` is `bearer` or `basic` and no secret is set.

The case view returns the same list as `armProblems` so the UI can explain a disabled button.

### 6.3 Automatic start

The scheduler's intake (§11.2) reads `task.status_changed` events with `to = "done"`. For each case in `waiting` with `start = tasks_done` linked to that task, it starts the case when **all** linked tasks are `done`. `cancelled` tasks do not count as done; a person can start the case manually.

### 6.4 Evaluation requests

`evaluationRequested` holds the strongest pending request: `final` beats `manual`. The scheduler clears it after the evaluation is recorded.

### 6.5 Deployments

`POST …/deployments` records `{ ref, environment?, at? }` (`at` defaults to now, may not be in the future, at most 30 days old). Allowed in `waiting`, `observing`, `evaluating`. In `waiting` with `start = deployment`, the case starts with anchor = `at`. Later deployments are recorded for the record only; they do not move the anchor in Phase 1. Event `deployment.completed`.

## 7. Evidence

### 7.1 Sources

A criterion names a source configured for the project (§7.3) or `manual`. The scheduler collects only non-manual criteria. Manual evidence arrives through `POST …/evidence` (§15).

### 7.2 Windows

Let `A` be the anchor and `now` the pass time (injected for tests).

| Record | Window |
| --- | --- |
| Baseline (criteria that need one, §5.3) | `[A − baselineWindow, A]` |
| Hypothesis, observed | `[A, now]` |
| Guardrail, observed | `[max(A, now − guardrailWindow), now]` |

A baseline is collected once per criterion and contract version: the scheduler collects it when no usable baseline record exists for that criterion and version, at most once per guardrail interval. An observed record copies the baseline value of the latest usable baseline record and computes `deltaAbs` and `deltaRel` (§4.3). Without a baseline both deltas are `None`.

### 7.3 Adapters

Code: `crates/genie/src/observation/adapters.rs`.

```rust
pub struct Span { pub start: DateTime<Utc>, pub end: DateTime<Utc> }
pub struct Measurement { pub value: f64, pub samples: Option<u64>, pub query: String, pub provenance: serde_json::Value }
pub enum SourceError { Config(String), Unauthorized(String), Http { status: u16, body: String }, Parse(String), Network(String) }
pub async fn measure(src: &SourceConfig, secret: Option<&str>, query: &str, samples_query: Option<&str>, span: Span)
    -> Result<Measurement, SourceError>;
```

Common rules:

- One shared `reqwest::Client` per call with `timeout = timeoutSecs` (default 30, max 120). Response bodies over 1 MiB are an error (`Parse("response larger than 1 MiB")`).
- Auth: `none`; `bearer` sends `Authorization: Bearer <secret>`; `basic` expects the secret as `user:password` and sends HTTP basic auth. A missing secret for `bearer`/`basic` is `Config("source <name> needs a secret")`.
- 401/403 → `Unauthorized`; other non-2xx → `Http` with the first 500 characters of the body.
- Secrets never appear in errors, provenance, logs, or events.
- `SourceError` implements `Display`; the scheduler stores `to_string()` in `Evidence.error`.

**Prometheus-compatible** (`kind = prometheus`, also Thanos, VictoriaMetrics, Mimir):

- Placeholder: every `$__range` in the query is replaced by the span length in whole seconds with suffix `s` (minimum `1s`), for example `86400s`.
- Request: `GET {url}/api/v1/query?query=<urlencoded>&time=<span.end as unix seconds with 3 decimals>`.
- Response `{"status":"success","data":{"resultType":…,"result":…}}`:
  - `vector` with exactly one element → `result[0].value[1]` parsed as `f64`; zero elements → `Parse("query returned no series")`; more → `Parse("query returned N series; aggregate them into one, for example with sum()")`.
  - `scalar` → `result[1]` parsed as `f64`.
  - Any other type → `Parse("unsupported result type <t>")`. `NaN` or infinite values → `Parse("query returned a non-finite value")`.
  - `status: "error"` → `Parse(<error field>)`.
- `samples_query`, when present, is measured the same way and rounded to the nearest non-negative integer.

**Sentry-compatible** (`kind = sentry`):

- Requires `organization`. Request: `GET {url}/api/0/organizations/{organization}/events/?field=<field>&query=<urlencoded query>&start=<RFC 3339>&end=<RFC 3339>&dataset=<dataset>` plus `&project=<sentryProject>` and `&environment=<environment>` when set. `field` defaults to `count()`; `dataset` defaults to `errors`.
- Response `{"data":[{"<field>": <number>}]}` → value from `data[0][field]`; an empty `data` array means `0`.
- `samples` = the value rounded when `field` starts with `count`, else `None`.

### 7.4 Provenance

`provenance` JSON written by the scheduler: `{ "adapter": "prometheus"|"sentry", "url": "<request URL without credentials>", "status": <HTTP status>, "durationMs": <n>, "excerpt": "<first 300 chars of the body>" }`. Manual records: `{ "adapter": "manual", "reportedBy": "<actor>" }`.

### 7.5 Manual evidence

Body: `criterion` (must exist in the current contract), `phase` (default `observed`), `value` (finite), `baseline` (optional, finite), `samples` (optional, ≥ 0), `windowStart`/`windowEnd` (default anchor and now; `windowStart < windowEnd`), `note`. Allowed in `observing`/`evaluating`. Trust is `reported` for a person and `agent` for the project manager. Deltas are computed server-side. A `reported` guardrail record runs the guardrail check (§8) at once.

## 8. Guardrail watch

`check_guardrail(g: &Guardrail, record: &Evidence) -> Option<String>` (pure, in `genie_core::observation::evaluate`) returns a violation message, or `None` when the record is not usable or no limit is crossed. Checks in order; the first crossed limit wins:

| Limit | Violated when | Message |
| --- | --- | --- |
| `max` | `value > max` | `value 61 exceeds max 50` |
| `min` | `value < min` | `value 3 is below min 5` |
| `maxChange` | the matching delta is `Some` and `delta > change.value` | `change +0.18pp exceeds +0.1pp` |
| `minChange` | the matching delta is `Some` and `delta < change.value` | `change -0.4% is below -0.2%` |

Numbers in messages are formatted with at most 4 decimal places and trailing zeros removed (`61`, `0.18`, `-0.4`). Changes always carry a sign; relative changes end with `%`, absolute ones with `pp` when the threshold used `pp`, otherwise no unit.

On a violation the scheduler opens an incident (§10.1) in the same transaction that stores the record.

## 9. Evaluation

Code: `genie_core::observation::evaluate::evaluate(input: &EvalInput) -> EvalOutcome`, pure and deterministic.

```rust
pub struct EvalInput<'a> {
    pub policy: ObservationPolicy,
    pub contract: &'a Contract,
    pub anchor: DateTime<Utc>,
    pub now: DateTime<Utc>,
    pub trigger: EvalTrigger,
    pub evidence: &'a [Evidence],   // the case's records for the current contract version
    pub violated: &'a [String],     // guardrail ids with an open or resolved (not dismissed) incident
}
pub struct EvalOutcome { pub verdict: Verdict, pub confidence: Option<Confidence>, pub summary: String, pub details: Vec<CriterionResult> }
```

### 9.1 Per-criterion state

For each hypothesis, take the **latest usable observed** record of that criterion (highest `id`):

1. None → `insufficient` ("no usable evidence").
2. `minimumSamples = n` and the record's `samples` is `None` or `< n` → `insufficient` ("samples X below minimum n" / "sample count unknown").
3. `expect.change`: the matching delta is `None` → `insufficient` ("no baseline" or "baseline is zero"). Otherwise `met` when (`target < 0` and `delta ≤ target`) or (`target > 0` and `delta ≥ target`), else `not_met`.
4. `expect.max`: `met` when `value ≤ max`; `expect.min`: `met` when `value ≥ min`; else `not_met`.

For each guardrail: `violated` when its id is in `violated`; otherwise `ok` when at least one usable observed record exists; otherwise `insufficient` ("never measured").

### 9.2 Final evaluation

`final = trigger == Final || now ≥ anchor + preferredWindow`.

### 9.3 Verdict (first matching rule)

1. Not final and `now < anchor + minimumWindow` → `insufficient` ("minimum window not reached").
2. Any guardrail `violated` → `negative`.
3. Policy `guardrails`:
   - not final → `insufficient`;
   - final and every guardrail `ok` → `confirmed`;
   - final otherwise → `inconclusive`.
4. Policy `verify`:
   - every hypothesis `met` and every guardrail `ok` → `confirmed`;
   - not final → `insufficient`;
   - final and any hypothesis `not_met` → `negative`;
   - final otherwise → `inconclusive`.

A final evaluation never returns `insufficient`. Tests must assert this for every combination of states.

### 9.4 Confidence and summary

- `confirmed`/`negative`: `high` when every record behind a `met`, `not_met`, or `ok` state is `measured`; `medium` when any is `reported`. A `negative` caused only by a violated guardrail is `high`.
- `inconclusive`: `low`. `insufficient`: `None`.
- `summary` is deterministic English: `"<verdict>: <m>/<n> hypotheses met; guardrails: <ok>/<g> ok, <v> violated; attribution: not assessed"`. For policy `guardrails` omit the hypotheses part. `details` lists hypotheses first, then guardrails, in contract order.

## 10. Incidents and follow-ups

### 10.1 Incidents

- `open_incident(case, guardrail, evidence, summary)` is idempotent: at most one `open` incident per case and guardrail (partial unique index). A repeated violation returns the existing incident and appends nothing; the new evidence record still exists.
- Creating an incident writes `guardrail.violated` and `incident.created`, notifies project admins and the case author (when the author is a person), and mails the project manager (`Management::notify(ORCHESTRATOR, "observation", text, first linked task)`).
- `resolve` (`open` → `resolved`, manager, optional note) — the violation still counts for evaluation.
- `dismiss` (`open`/`resolved` → `dismissed`, **person only**, note required) — the violation no longer counts (false positive).
- An incident blocks nothing in the delivery plane.

### 10.2 Follow-up tasks

- `POST …/followup { incident?, title?, description? }` (manager) creates a task through `crate::tasks::create` with the caller's identity, type `bug` when an incident is given, else `task`, label `observation`, and a description that links the case (`/observation/<id>`), its tasks, and the incident evidence. Default titles: `Regression: <guardrail> (<case key>)` or `Outcome not achieved: <case title>` (truncate to 120 characters). Records a `Followup` and writes `followup_task.created`.
- With `followup: "auto"` in the contract, the scheduler performs the same call for each newly created incident as caller `Caller::automation(project, Actor::new("observation", Role::Orchestrator))`. Repeated violations of an already open incident do not create more tasks.
- A `negative` or `inconclusive` conclusion never creates tasks automatically in Phase 1; it notifies and mails the project manager, who decides.

## 11. Scheduler

Code: `crates/genie/src/observation/scheduler.rs`.

### 11.1 Loop

- `pub fn start(app: &Arc<App>)` spawns a loop like `engine::start`: `app.blocking(|app| pass_at(app, Utc::now()))`, then sleep until `plan.at` (capped by `SAFETY_NET`) or `app.wake_observation.notified()`. Errors are logged with prefix `genie observation:` and retried after 10 seconds.
- `pub fn pass_at(app: &App, now: DateTime<Utc>) -> AppResult<Plan>` runs one pass for every project; tests call it directly with a chosen `now`.
- Started from `cli.rs` next to `git::delivery::spawn_poller` (both run regardless of `runtime.enabled`).
- `App` gains `pub wake_observation: Arc<Notify>`; the tracker `on_event` hook notifies it for every event.

### 11.2 Intake

Per project, cursor name `observation`: read events after the cursor (batches of 200); for `task.status_changed` with `payload.to == "done"`, call the core `on_task_done(&Actor::new("genie", Role::Orchestrator), task, now)`; then acknowledge the last id. Intake must be idempotent: starting an already started case is a no-op.

### 11.3 Due cases

`due(now)` returns cases in `observing`/`evaluating` where `nextGuardrailAt ≤ now`, `nextEvaluationAt ≤ now`, or `evaluationRequested` is set. For each due case, in this order:

1. **Snapshot** (tracker lock): case, current contract, latest usable baseline per criterion.
2. **Sources** (server lock): the project's source configs and secrets for the criteria to collect.
3. **Baselines**: for each criterion that needs a baseline and has none, measure the baseline window (§7.2).
4. **Guardrails** when `nextGuardrailAt ≤ now`: measure every non-manual guardrail over its window.
5. **Write** (one tracker transaction, conditional on the case still being `observing`/`evaluating`): insert the records; run `check_guardrail` on new observed guardrail records and open incidents; set `nextGuardrailAt = now + guardrailInterval`; set `failures` to 0 when every measurement succeeded, else increment it and store `lastError`; write one `evidence.collected` event (`{ case, records, errors }`) when any record was inserted.
6. **Side effects after commit**: notifications for new incidents; automatic follow-ups (§10.2); when `failures` reaches 3, notify project admins once per streak (dedupe key `observation-failures:<project>:<case>:<armedAt>`).
7. **Evaluation** when `nextEvaluationAt ≤ now` or requested (§11.4).

Steps 2–4 run without any database lock; adapters run with `state::block_on`.

### 11.4 Evaluation pass

1. `begin_evaluation`: `observing` or `evaluating` → `evaluating` (a case left in `evaluating` by a crash is taken again); returns false for any other status, and the pass skips the case.
2. Trigger: requested `final` → `final`; requested `manual` → `manual`; `now ≥ anchor + preferredWindow` → `final`; else `window`.
3. Measure every non-manual hypothesis over `[A, now]` (and its samples query); store the records as in §11.3 step 5.
4. Load the current version's records and the violated guardrails; call `evaluate`.
5. `finish_evaluation` in one transaction: insert the `Evaluation`; clear `evaluationRequested`; then
   - `insufficient` → status `observing`, `nextEvaluationAt = min(now + evaluationInterval, anchor + preferredWindow)`, event `outcome.evaluated`;
   - otherwise → status `concluded`, `result`, `confidence`, `conclusion = summary`, `observationCompletedAt = now`, `nextEvaluationAt`/`nextGuardrailAt` cleared, event `outcome.confirmed`, `outcome.negative`, or `outcome.inconclusive`.
   If the case is no longer `evaluating` (cancelled meanwhile), write nothing and return.
6. After a conclusion: notify project admins and the case author; mail the project manager with the summary and the follow-up command.

A pass that fails between steps 1 and 5 leaves the case in `evaluating`; the next pass handles it as `observing` (§6.1).

### 11.5 Plan

`Plan.at` is the earliest `nextGuardrailAt`/`nextEvaluationAt` among `observing`/`evaluating` cases of all projects; `Plan.again` is true when a case with a pending request remains.

## 12. Journal events

Constants in `crates/genie-core/src/events.rs`. Subject is always `observation:<case id>`. Every payload includes `case` (id) and `key` (`O-<id>`); listed fields are additional.

| Constant | Type | Payload |
| --- | --- | --- |
| `OUTCOME_CREATED` | `outcome.created` | `title`, `policy`, `tasks` |
| `OUTCOME_UPDATED` | `outcome.updated` | changed fields: `title`, `policy`, `tasks`, `status`, `contractVersion`, `reason` |
| `OBSERVATION_STARTED` | `observation.started` | `anchor`, `start` (`tasks_done`/`deployment`/`manual`), `tasks` |
| `DEPLOYMENT_COMPLETED` | `deployment.completed` | `deployment` (id), `ref`, `environment`, `at` |
| `EVIDENCE_COLLECTED` | `evidence.collected` | `records` (count), `errors` (count) |
| `GUARDRAIL_VIOLATED` | `guardrail.violated` | `guardrail`, `evidence`, `message` |
| `INCIDENT_CREATED` | `incident.created` | `incident` (id), `incidentKey`, `guardrail` |
| `INCIDENT_RESOLVED` | `incident.resolved` | `incident`, `note` |
| `INCIDENT_DISMISSED` | `incident.dismissed` | `incident`, `note` |
| `OUTCOME_READY` | `outcome.ready_for_evaluation` | `trigger` |
| `OUTCOME_EVALUATED` | `outcome.evaluated` | `evaluation`, `verdict` (`insufficient`), `nextEvaluationAt` |
| `OUTCOME_CONFIRMED` | `outcome.confirmed` | `evaluation`, `confidence`, `summary`, `tasks` |
| `OUTCOME_NEGATIVE` | `outcome.negative` | same |
| `OUTCOME_INCONCLUSIVE` | `outcome.inconclusive` | same |
| `OUTCOME_CANCELLED` | `outcome.cancelled` | `reason` |
| `FOLLOWUP_TASK_CREATED` | `followup_task.created` | `task`, `incident` (or null) |

The proposal's `task.delivered` is the existing `task.status_changed` to `done`; `deployment.started` is not emitted in Phase 1. Automations can trigger on all of these event types with no engine change (`docs/platform/automations.md` lists them after OBS-17).

## 13. Persistence

### 13.1 Tracker schema 7

Append to `SCHEMA` in `crates/genie-core/src/db.rs` and set `SCHEMA_VERSION = 7`. Only new tables: no column migrations and no row changes, so the existing migration rehearsal applies unchanged.

```sql
CREATE TABLE IF NOT EXISTS observation_cases (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  title TEXT NOT NULL,
  policy TEXT NOT NULL CHECK(policy IN ('guardrails', 'verify')),
  status TEXT NOT NULL DEFAULT 'proposed'
    CHECK(status IN ('proposed', 'waiting', 'observing', 'evaluating', 'concluded', 'cancelled')),
  result TEXT CHECK(result IN ('confirmed', 'negative', 'inconclusive')),
  confidence TEXT CHECK(confidence IN ('low', 'medium', 'high')),
  conclusion TEXT NOT NULL DEFAULT '',
  contract_version INTEGER NOT NULL DEFAULT 0,
  author TEXT NOT NULL,
  created TEXT NOT NULL,
  updated TEXT NOT NULL,
  armed_at TEXT,
  observation_started_at TEXT,
  observation_completed_at TEXT,
  next_evaluation_at TEXT,
  next_guardrail_at TEXT,
  evaluation_requested TEXT CHECK(evaluation_requested IN ('manual', 'final')),
  failures INTEGER NOT NULL DEFAULT 0,
  last_error TEXT
);
CREATE INDEX IF NOT EXISTS observation_cases_status ON observation_cases(status);
CREATE TABLE IF NOT EXISTS observation_case_tasks (
  case_id INTEGER NOT NULL REFERENCES observation_cases(id) ON DELETE CASCADE,
  task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  PRIMARY KEY (case_id, task)
);
CREATE INDEX IF NOT EXISTS observation_case_tasks_task ON observation_case_tasks(task);
CREATE TABLE IF NOT EXISTS observation_contracts (
  case_id INTEGER NOT NULL REFERENCES observation_cases(id) ON DELETE CASCADE,
  version INTEGER NOT NULL,
  content TEXT NOT NULL,
  author TEXT NOT NULL,
  reason TEXT NOT NULL DEFAULT '',
  created TEXT NOT NULL,
  PRIMARY KEY (case_id, version)
);
CREATE TABLE IF NOT EXISTS observation_deployments (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  case_id INTEGER NOT NULL REFERENCES observation_cases(id) ON DELETE CASCADE,
  ref TEXT NOT NULL,
  environment TEXT NOT NULL DEFAULT '',
  at TEXT NOT NULL,
  recorded_by TEXT NOT NULL,
  created TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS observation_evidence (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  case_id INTEGER NOT NULL REFERENCES observation_cases(id) ON DELETE CASCADE,
  contract_version INTEGER NOT NULL,
  criterion TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('hypothesis', 'guardrail')),
  phase TEXT NOT NULL CHECK(phase IN ('baseline', 'observed')),
  source TEXT NOT NULL,
  query TEXT NOT NULL DEFAULT '',
  at TEXT NOT NULL,
  window_start TEXT NOT NULL,
  window_end TEXT NOT NULL,
  value REAL,
  baseline REAL,
  delta_abs REAL,
  delta_rel REAL,
  samples INTEGER,
  provenance TEXT NOT NULL DEFAULT '{}',
  trust TEXT NOT NULL CHECK(trust IN ('measured', 'reported', 'agent')),
  error TEXT,
  note TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS observation_evidence_case ON observation_evidence(case_id, contract_version, criterion, id);
CREATE TABLE IF NOT EXISTS observation_evaluations (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  case_id INTEGER NOT NULL REFERENCES observation_cases(id) ON DELETE CASCADE,
  contract_version INTEGER NOT NULL,
  at TEXT NOT NULL,
  trigger TEXT NOT NULL CHECK(trigger IN ('window', 'manual', 'final')),
  verdict TEXT NOT NULL CHECK(verdict IN ('confirmed', 'negative', 'inconclusive', 'insufficient')),
  confidence TEXT CHECK(confidence IN ('low', 'medium', 'high')),
  summary TEXT NOT NULL,
  details TEXT NOT NULL DEFAULT '[]',
  actor TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS observation_evaluations_case ON observation_evaluations(case_id, id);
CREATE TABLE IF NOT EXISTS observation_incidents (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  case_id INTEGER NOT NULL REFERENCES observation_cases(id) ON DELETE CASCADE,
  guardrail TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'open' CHECK(status IN ('open', 'resolved', 'dismissed')),
  summary TEXT NOT NULL,
  evidence INTEGER REFERENCES observation_evidence(id) ON DELETE SET NULL,
  created TEXT NOT NULL,
  closed TEXT,
  closed_by TEXT,
  note TEXT NOT NULL DEFAULT ''
);
CREATE UNIQUE INDEX IF NOT EXISTS observation_incidents_open ON observation_incidents(case_id, guardrail) WHERE status = 'open';
CREATE TABLE IF NOT EXISTS observation_followups (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  case_id INTEGER NOT NULL REFERENCES observation_cases(id) ON DELETE CASCADE,
  incident INTEGER REFERENCES observation_incidents(id) ON DELETE SET NULL,
  task TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
  actor TEXT NOT NULL,
  created TEXT NOT NULL,
  UNIQUE (case_id, task)
);
```

Deleting a task removes its links and follow-up rows by cascade; the case stays (it may then have zero tasks, which only blocks arming).

### 13.2 Server schema 7

Set `SERVER_SCHEMA_VERSION = 7`. Add `observation::sources::SCHEMA` (executed in `server_db::open_migrated` after `SECRETS_SCHEMA`) and a secrets table in `SECRETS_SCHEMA`:

```sql
-- genie_core::observation::sources::SCHEMA
CREATE TABLE IF NOT EXISTS observation_sources (
  project TEXT NOT NULL REFERENCES projects(slug) ON DELETE CASCADE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL CHECK(kind IN ('prometheus', 'sentry')),
  url TEXT NOT NULL,
  config TEXT NOT NULL DEFAULT '{}',
  created TEXT NOT NULL,
  updated TEXT NOT NULL,
  PRIMARY KEY (project, name)
);
-- appended to secrets::SECRETS_SCHEMA
CREATE TABLE IF NOT EXISTS observation_secrets (
  project TEXT NOT NULL REFERENCES projects(slug) ON DELETE CASCADE,
  source TEXT NOT NULL,
  sealed TEXT NOT NULL,
  hint TEXT NOT NULL DEFAULT '',
  updated TEXT NOT NULL,
  PRIMARY KEY (project, source)
);
```

`config` holds `auth`, `organization`, `sentryProject`, `environment`, `field`, `dataset`, `timeoutSecs`. Source validation: `name` matches `^[a-z0-9][a-z0-9_-]{0,31}$` and is not `manual`; `url` starts with `http://` or `https://`, has no user info, query, or fragment, and is stored without a trailing slash; Sentry requires `organization`; `timeoutSecs` 1–120. Removing a source removes its secret in the same transaction. A source in use by a waiting or observing case can be removed; collection then records `Config("unknown source <name>")` errors.

## 14. Permissions

| Action | Person, write | Person, viewer | Project manager agent | Task manager, member, job | Project admin |
| --- | --- | --- | --- | --- | --- |
| Read cases, evidence, incidents | yes | yes | yes | yes | yes |
| Create, update, arm, start, cancel, set contract before start | yes | no | yes | no | yes |
| Revise contract after start (with reason) | yes | no | no | no | yes |
| Record deployment, request evaluation | yes | no | yes | no | yes |
| Request final evaluation | yes | no | no | no | yes |
| Report manual evidence | yes (`reported`) | no | yes (`agent`) | no | yes |
| Resolve incident, create follow-up | yes | no | yes | no | yes |
| Dismiss incident | yes | no | no | no | yes |
| Manage sources and secrets, test a source | no | no | no | no | yes (people only) |

The application layer enforces this; the web only hides controls. Denials are `AppError::Forbidden` (HTTP 403) with an actionable English message, such as `only people and the project manager manage observation` or `only a person may dismiss an incident`.

## 15. HTTP API

Routes in `crates/genie/src/http/observation.rs`, merged in `http::router`, under `/api`. Project selection follows `ctx.access(&app, None)` like the management routes. Request bodies: `#[serde(rename_all = "camelCase", deny_unknown_fields)]`, `#[derive(ts_rs::TS)]`, `#[ts(export)]`.

| Method and path | Body / query | Response |
| --- | --- | --- |
| `GET /observation/cases` | `?status=<s>&task=<id>&limit=<n≤200>` | `ObservationCase[]`, newest first |
| `POST /observation/cases` | `CreateCaseBody { title, policy, tasks: string[], contract?: Contract }` | `CaseView` |
| `GET /observation/cases/{id}` | | `CaseView` |
| `PATCH /observation/cases/{id}` | `UpdateCaseBody { title?, policy?, addTasks?: string[], removeTasks?: string[] }` | `CaseView` |
| `PUT /observation/cases/{id}/contract` | `ContractBody { contract, reason? }` | `CaseView` |
| `POST /observation/cases/{id}/arm` | `{}` | `CaseView` |
| `POST /observation/cases/{id}/start` | `{}` | `CaseView` |
| `POST /observation/cases/{id}/evaluate` | `EvaluateBody { final?: bool }` | `CaseView` |
| `POST /observation/cases/{id}/cancel` | `ReasonBody { reason }` | `CaseView` |
| `POST /observation/cases/{id}/deployments` | `DeploymentBody { ref, environment?, at? }` | `CaseView` |
| `GET /observation/cases/{id}/evidence` | `?after=<id>&limit=<n≤500>&criterion=<id>` | `Evidence[]`, oldest first |
| `POST /observation/cases/{id}/evidence` | `EvidenceBody` (§7.5) | `CaseView` |
| `POST /observation/cases/{id}/followup` | `FollowupBody { incident?: number, title?, description? }` | `CaseView` |
| `GET /observation/incidents` | `?status=<s>` | `Incident[]` |
| `POST /observation/incidents/{id}/resolve` | `NoteBody { note? }` | `Incident` |
| `POST /observation/incidents/{id}/dismiss` | `NoteBody { note }` | `Incident` |
| `GET /observation/tasks/{task}` | | `ObservationCase[]` linked to the task |
| `GET /observation/sources` | | `SourceView[]` (admin) |
| `POST /observation/sources` | `SourceBody` | `SourceView` (admin) |
| `PUT /observation/sources/{name}` | `SourceBody` (name must match) | `SourceView` (admin) |
| `DELETE /observation/sources/{name}` | | `{ "ok": true }` (admin) |
| `PUT /observation/sources/{name}/secret` | `SecretBody { value }` | `SourceView` (admin) |
| `DELETE /observation/sources/{name}/secret` | | `SourceView` (admin) |
| `POST /observation/sources/{name}/test` | `SourceTestBody { query, window?: string }` (default `1h`) | `{ value, samples, query, provenance }` or 422 with the adapter error (admin) |

`CaseView` (application layer, exported): `case: ObservationCase`, `contract: Option<ContractVersion>` (current), `contracts: Vec<ContractVersionInfo>` (`version`, `author`, `reason`, `created`), `armProblems: Vec<String>`, `deployments`, `evaluations` (newest 50, newest first), `incidents`, `followups`, `evidence` (newest 100, newest first).

Status codes follow the existing mapping: domain `Invalid`/`NotFound`/`Denied` → 422, `AppError::Bad` → 400, `Forbidden` → 403, `Conflict` → 409. Unknown case → 422 `observation O-9 not found` (domain not-found, same as tasks).

## 16. CLI and MCP

New catalog group `observe` in `crates/genie/src/ops/observation.rs`, registered in `catalog()`, with `GROUPS` entry `("observe", "Outcome cases after delivery: contracts, evidence, guardrail incidents and evaluation")` placed after `manager`. MCP tool `genie_observe`.

| Op `NAME` | Command line | Need | Listed |
| --- | --- | --- | --- |
| `list` | `genie observe list [--status S] [--task G-1]` | Read | Orchestrator |
| `show` | `genie observe show O-1` | Read | Orchestrator |
| `create` | `genie observe create --title T --policy verify --task G-1 [--task G-2] [--contract JSON \| --contract-file F]` | Orchestrator | Orchestrator |
| `update` | `genie observe update O-1 [--title T] [--policy P] [--add-task G-3] [--remove-task G-2]` | Orchestrator | Orchestrator |
| `contract` | `genie observe contract O-1 (--contract JSON \| --file F) [--reason R]` | Orchestrator | Orchestrator |
| `arm` | `genie observe arm O-1` | Orchestrator | Orchestrator |
| `start` | `genie observe start O-1` | Orchestrator | Orchestrator |
| `evaluate` | `genie observe evaluate O-1 [--final]` | Orchestrator | Orchestrator |
| `cancel` | `genie observe cancel O-1 --reason R` | Orchestrator | Orchestrator |
| `deploy` | `genie observe deploy O-1 --ref SHA [--environment E] [--at TS]` | Orchestrator | Orchestrator |
| `evidence` | `genie observe evidence O-1 [--criterion C] [--after N] [--limit N]` | Read | Orchestrator |
| `report` | `genie observe report O-1 --criterion C --value V [--baseline B] [--samples N] [--phase P] [--from TS --to TS] [--note T]` | Orchestrator | Orchestrator |
| `incidents` | `genie observe incidents [--status open]` | Read | Orchestrator |
| `resolve` | `genie observe resolve I-1 [--note T]` | Orchestrator | Orchestrator |
| `dismiss` | `genie observe dismiss I-1 --note T` | Person | Nobody |
| `followup` | `genie observe followup O-1 [--incident I-1] [--title T] [--description D]` | Orchestrator | Orchestrator |
| `sources` | `genie observe sources` | Admin | Nobody |
| `source-add` | `genie observe source-add NAME --kind prometheus --url U [--auth bearer] [--organization O] [--sentry-project P] [--environment E] [--field F] [--dataset D] [--timeout-secs N]` | Admin | Nobody |
| `source-update` | same flags as `source-add` | Admin | Nobody |
| `source-remove` | `genie observe source-remove NAME` | Admin | Nobody |
| `source-secret` | `genie observe source-secret NAME --value -` (`-` reads stdin; `--clear` removes) | Admin | Nobody |
| `source-test` | `genie observe source-test NAME --query Q [--window 1h]` | Admin | Nobody |

Output: pretty JSON of the API response, as `ops/management.rs` does. Ids accept `O-1`/`1` and `I-1`/`1`; the op passes the numeric id to the API. `--contract-file`/`--file` are read only on the command line (`Cx::text`); MCP callers pass `contract` as an object.

## 17. Web UI

Russian user-facing text, existing layers (`steiger.config.ts`), TanStack Query through `shared/api`, generated types from `web/src/shared/api/generated`.

- **Entity** `web/src/entities/observation`: `api.ts` (queries and mutations; query keys all start with `"observations"` except sources, which use `"observation-sources"`), `model.ts` (pure helpers, tested by `test/web-observation.test.ts`), `index.ts`.
- **Live updates**: `keysFor` maps `outcome.*`, `observation.started`, `deployment.completed`, `evidence.collected`, `guardrail.violated`, `incident.*` to `[["observations"]]`, and `followup_task.created` to `[["observations"], ["tasks"], ["task"]]`.
- **Pages** `web/src/pages/observations`: list `/observations` (filter: active / concluded / all; columns: key, title, policy, status and result, tasks, next evaluation, open incidents) and case page `/observation/:caseId` (header with status, result, confidence; actions according to §14; contract viewer and JSON editor with policy templates; arm problems; tasks; deployments with a form; evaluations; evidence table; incidents with resolve, dismiss, follow-up). Sidebar item with the glossary label `nav`, next to Automations.
- **Task card**: `TaskObservation` section in `widgets/task-detail` listing linked cases with status and result, and a create button (glossary labels `section`, `empty`, `create`, `newTitle`; policy select) for writers; it opens the new case page.
- **Settings**: project management section (glossary label `section`) with sources (add, edit, remove, set or clear secret, test query). Admins only, like repositories.

All Russian labels (statuses, results, policies, incident states, navigation, buttons) are defined in [the glossary](https://github.com/grigoryshulga/genie/blob/main/GLOSSARY.md#observation-plane); `model.ts` uses exactly those strings.

## 18. Agents, notifications, and documentation

- `agents/project-manager.md` gains an "Observation" section: propose a case during planning for risky or measurable changes, write the contract before delivery, never conclude or dismiss, react to incident and conclusion mail with follow-ups or escalation. English, like the existing prompt.
- Notification kinds: `observation.incident` and `observation.concluded`, link `/observation/<id>`, dedupe keys `observation-incident:<project>:<incident id>` and `observation-concluded:<project>:<case id>`.
- Documentation (OBS-17): user reference `docs/platform/observation.md`, a decisions row (PD25: "Observation is a separate plane with its own lifecycle; it consumes delivery events and creates work only through task services"), the architecture code map, the CLI reference, the automations event list, `CHANGELOG.md` (Unreleased), the VitePress sidebar, and the roadmap. This specification's status becomes `current` once Phase 1 ships.

## 19. Risks and limits

- Evidence grows with time: about `24h / guardrailInterval` records per guardrail per day (96 at 15 minutes). Phase 2 adds retention or downsampling; Phase 1 caps the guardrail interval at ≥ 1 minute and the windows at 90 days.
- Polling a slow source holds the scheduler (one loop, sequential cases). Timeouts bound it; Phase 2 may parallelize per project.
- Source URLs are set by project administrators; the server makes requests from its own network. Document that this reaches whatever the server can reach.
- `pp` semantics depend on the metric already being a percentage (§5.2).
- Correlation is not causation: Phase 1 conclusions always state `attribution: not assessed`.

## 20. Later phases (not in this plan)

- **Phase 2**: deployment provider adapters (GitHub deployments, Argo CD) that emit `deployment.completed`; Loki and OpenTelemetry trace adapters; an observation analyst role invoked through an automation `agent` step on `guardrail.violated` and `outcome.inconclusive` for incident investigation; project memory proposals from concluded cases via the existing provenance model; default policies per task type or label; evidence retention.
- **Phase 3**: policies `outcome` and `experiment`, product analytics adapters (PostHog, Amplitude), experiment assignment, attribution with `suspected_changes` and confidence, and factory self-observation metrics (first-pass acceptance, escaped defects, rollback rate, cost per accepted task, incident rate by task category).

## 21. Open questions

1. Should `none` be recorded explicitly per task (D3)? Phase 1 shows "no observation" in the task card; an explicit field would touch the task model.
2. Should a person be able to conclude a case manually with a chosen result? Phase 1 offers only `final` evaluation and cancellation.
3. Should auto follow-ups also be created for `negative` conclusions (§10.2)?
4. Default `guardrailInterval` 15 minutes and `evaluationInterval` 6 hours: acceptable for pilot sources and their rate limits?
