---
title: Observation plane implementation plan
summary: Task cards, waves, agent prompts, and the review and fix loop for Phase 1 of the observation plane.
type: guide
status: draft
---

# Observation plane implementation plan

This plan splits [Phase 1 of the observation plane](../platform/observation-plane.md) into small task cards for implementation agents, followed by review and fix rounds. Every card names its files, the specification sections (§N) it implements, the tests it adds, and the commands that must pass. An agent reads only the specification, its own card, and the files the card lists.

## 1. How to run the plan

### 1.1 Branches and worktrees

- Feature work enters `dev` (see [release flow](../development.md#release-flow)). `AGENTS.md` still says "open a PR against `main`"; the release flow added on October 17, 2026 supersedes that for feature work. Create the integration branch once: `git fetch origin dev && git branch feat/observation-plane origin/dev && git push -u origin feat/observation-plane`.
- Each card runs in its own worktree on branch `obs/OBS-NN-<slug>`, created from the **current** `feat/observation-plane` after all of the card's dependencies are merged: `git worktree add -b obs/OBS-04-case-store ../genie-obs-04 origin/feat/observation-plane`.
- Each card ends with one pull request into `feat/observation-plane`. Merge PRs in wave order. After the last wave, one PR `feat/observation-plane` → `dev` carries the feature, with the full CI.
- Commit messages: `feat(observation): <summary> (OBS-NN)`; tests and docs may use `test(observation)` and `docs(observation)`.

### 1.2 Waves

A card starts only when every card in its "Depends on" line is merged. Cards in the same wave touch disjoint files and may run in parallel.

| Wave | Cards | Parallel |
| --- | --- | --- |
| A | OBS-01 | — |
| B | OBS-02, OBS-06, OBS-07 | yes |
| C | OBS-03, OBS-04 | yes |
| D | OBS-05 | — |
| E | OBS-08 | — |
| F | OBS-09, OBS-10 | yes |
| G | OBS-11, OBS-12 | yes |
| H | OBS-13, OBS-14, OBS-15, OBS-16 | yes |
| I | OBS-17 | — |
| Final | Integration review (§4.4) | — |

| Card | Title | Depends on | Unblocks |
| --- | --- | --- | --- |
| OBS-01 | Foundation: types, keys, events, skeleton | — | 02, 06, 07 |
| OBS-02 | Contract validation and change grammar | 01 | 03, 04 |
| OBS-03 | Evaluator and guardrail check | 02 | 05 |
| OBS-04 | Tracker schema 7 and the case lifecycle store | 02 | 05 |
| OBS-05 | Evidence, evaluations, incidents, follow-ups | 03, 04 | 08 |
| OBS-06 | Server schema 7: sources and secrets | 01 | 08 |
| OBS-07 | Adapters: Prometheus- and Sentry-compatible sources | 01 | 08 |
| OBS-08 | Application service | 05, 06, 07 | 09, 10 |
| OBS-09 | Scheduler and wiring | 08 | 16 |
| OBS-10 | HTTP routes | 08 | 11, 12, 16 |
| OBS-11 | CLI and MCP operations | 10 | 16, 17 |
| OBS-12 | Web entity and live updates | 10 | 13, 14, 15 |
| OBS-13 | Web: observation pages | 12 | 17 |
| OBS-14 | Web: task card section | 12 | 17 |
| OBS-15 | Web: observation sources in project settings | 12 | 17 |
| OBS-16 | End-to-end scenarios | 09, 10, 11 | final review |
| OBS-17 | Documentation and agent guidance | 11, 13, 14, 15 | final review |

### 1.3 Tracking

Track the epic and every card in the repository tracker, as `AGENTS.md` requires (project `genie`, prefix `GNE`). Create an epic, then one task per card with the card's title, goal as description, its acceptance criteria as `--ac`, `--parent <epic>`, and `--dep` for each dependency:

```bash
genie --project genie task create 'Observation plane, Phase 1' --type epic \
  --description 'Implement docs/platform/observation-plane.md (Phase 1)'
genie --project genie task create 'OBS-01 Observation foundation: types, keys, events, skeleton' \
  --parent GNE-<epic> --label observation \
  --description '<goal of the card>' --ac '<criterion 1>' --ac '<criterion 2>'
```

Record the branch, worktree, base commit, PR URL, CI result, and review rounds on each task, as the repository workflow describes.

### 1.4 Implementation agent prompt

Use this prompt for every card; replace the placeholders.

```text
You implement one task card of the Genie observation plane.

Repository: <path of your worktree>, branch obs/OBS-<NN>-<slug>, based on origin/feat/observation-plane.
Read first, in this order:
1. AGENTS.md and CODING_STANDARDS.md.
2. docs/platform/observation-plane.md: the sections your card lists (§N), plus §2 and §4.
3. docs/development/observation-plane-plan.md: §2 (rules for all cards) and your card OBS-<NN>.
4. Every existing file your card lists under "Read".

Then:
- Implement exactly the card. Do not change files outside its "Files" list unless compilation requires it; if it does, say so in the PR.
- Follow the signatures and messages in the specification literally. When the specification is silent, choose the simplest behavior consistent with it and list the choice under "Spec questions" in the PR description.
- Add every test the card lists. Tests must assert observable behavior and error messages, not only that code runs.
- Run the card's commands. All must pass. Paste their summary lines into the PR description.
- Commit (message format in plan §1.1), push, open a PR into feat/observation-plane titled "OBS-<NN>: <card title>". The PR description has: Summary, Spec sections, Files, Tests, Commands and results, Spec questions.
- Do not edit generated files by hand; `cargo test` regenerates web/src/shared/api/generated. Commit the regenerated files.
```

### 1.5 Review and fix loop

After each PR, a reviewer (use a stronger model than the implementer) reviews the diff against the card, the specification, and the checklists in §4. Use this prompt:

```text
Review PR <url> (card OBS-<NN>) of the Genie observation plane.
Read docs/platform/observation-plane.md, docs/development/observation-plane-plan.md §2, §4 and card OBS-<NN>, then the diff and the files it touches.
Check: the card's acceptance criteria, the specification sections it cites, the checklists in plan §4.1 and the area checklist in §4.2–§4.3.
Run the card's commands yourself.
Report findings only; do not fix them. One finding per line:
  [blocker|major|minor] <file>:<line> — <problem> — spec §N / card AC n — <expected fix>
blocker: wrong behavior, data loss, security or permission hole, failing command, missing required test.
major: spec deviation without justification, weak test, error kind or message mismatch, missing event or transaction.
minor: naming, comments, duplication, style beyond rustfmt/clippy.
End with "VERDICT: approve" or "VERDICT: changes".
```

The implementer then fixes every blocker and major finding (minor findings at its discretion), replies to each finding with the commit that fixes it or a reason, reruns the commands, and pushes. Repeat review and fix until the verdict is `approve`, at most three rounds; after the third, escalate to a person with the open findings. Merge only with an approving verdict and green CI on the latest head.

## 2. Rules for all cards

1. Module names are `observation` (never `outcome`; `crates/genie/src/outcome.rs` is unrelated). Keys are `O-<id>`/`I-<id>`; journal subjects are `observation:<id>` (§2.2, §4.2).
2. `genie-core` stays synchronous: no Tokio, no reqwest. HTTP, scheduling, and notifications live in `crates/genie`.
3. A state change and its journal event commit in the same transaction (`Tracker::tx`, nested calls join). Never hold `with_tracker`/`with_server` across HTTP.
4. Reuse `GenieError::invalid`/`not_found` and `GenieError::Denied` in the core; in the application layer use `AppError::Bad` (malformed request), `Forbidden` (actor may not), `Conflict` (state in the way), and propagate domain errors. Error messages are English sentences that tell the caller what to do.
5. Timestamps are written with `observation::ts` (RFC 3339, milliseconds, `Z`) so they sort as strings (§4.2). Time-dependent functions take `now` as a parameter; only entrances call `Utc::now()`.
6. Structs crossing the wire derive `ts_rs::TS` with `#[ts(export)]`, `#[serde(rename_all = "camelCase")]`; request bodies also use `deny_unknown_fields`. `serde_json::Value` fields use `#[ts(type = "unknown")]`. Every `Option<T>` field of an exported output struct also carries `#[serde(skip_serializing_if = "Option::is_none")]` (the core convention, as in `AcceptanceCriterion`), so the wire omits it and matches the `field?: T` type that `#[ts(optional_fields)]` generates.
7. SQL is parameterized. No new dependencies in `Cargo.toml` without a reason in the PR (none is expected: `reqwest`, `chrono`, `serde_json`, `axum` are present).
8. UI text is Russian and uses exactly the labels in the "Observation plane" section of `GLOSSARY.md` (repository root); code, comments, and docs are English.
9. Every card runs `cargo fmt --all` and keeps `cargo clippy --all-targets -- -D warnings` clean.
10. Tests: core unit tests in `#[cfg(test)]` modules or `crates/genie-core/tests/`; server tests in `crates/genie/tests/it/observation_*.rs` (registered in OBS-01); web tests in `test/*.test.ts` with `node:test`. Requests through `Harness::router` without a bearer token act as the local operator (a person with admin rights); agent tokens come from `db.create_role_token` (see `crates/genie/tests/it/management.rs`).

## 3. Task cards

### OBS-01 Foundation: types, keys, events, skeleton

- **Goal:** Create every shared type, key helper, event constant, and empty module, so later cards can work in parallel without editing the same registration files.
- **Depends on:** —
- **Spec:** §2.2, §4.1, §4.2, §4.3, §5.1 (types only), §12 (constants)
- **Read:** `crates/genie-core/src/model.rs` (`str_enum!`), `crates/genie-core/src/management.rs` (struct style), `crates/genie-core/src/events.rs`, `crates/genie-core/src/lib.rs`, `crates/genie/src/lib.rs`, `crates/genie/src/http/mod.rs` (`router`), `crates/genie/src/ops/mod.rs` (`catalog`), `crates/genie/tests/it/main.rs`, `crates/genie/tests/it/common/mod.rs`.
- **Files:**
  - `crates/genie-core/src/model.rs`: add `pub(crate) use str_enum;` right after the macro definition.
  - `crates/genie-core/src/observation/mod.rs` (new): module doc comment (what the plane is, link to the spec path); `pub mod contract; pub mod evaluate; pub mod sources; pub mod store;`; all enums of §4.1 via `crate::model::str_enum!`; `impl Default` for `StartOn` (`TasksDone`), `FollowupMode` (`Manual`), and `SourceAuth` (`None`); key helpers `case_key`, `incident_key`, `case_subject`, `parse_case_id`, `parse_incident_id`; `ts`, `parse_ts`; entity structs of §4.3 (`ObservationCase`, `ContractVersion`, `ContractVersionInfo`, `Deployment`, `Evidence`, `Evaluation`, `CriterionResult`, `Incident`, `Followup`, `SourceConfig`, `SecretHint`). Types only, no persistence.
  - `crates/genie-core/src/observation/contract.rs` (new): structs of §5.1 (`Contract`, `Hypothesis`, `Expectation`, `Guardrail`, `ObservationWindows`) with serde `deny_unknown_fields`, camelCase, ts export. No validation yet.
  - `crates/genie-core/src/observation/evaluate.rs`, `sources.rs`, `store.rs` (new): one `//!` doc line each, no code. Later cards fill them, so parallel cards never edit `mod.rs`.
  - `crates/genie-core/src/lib.rs`: `pub mod observation;`.
  - `crates/genie-core/src/events.rs`: every constant of §12 with a one-line doc comment.
  - `crates/genie/src/observation/mod.rs` (new): module doc; `pub mod adapters; pub mod scheduler; pub mod service;`.
  - `crates/genie/src/observation/adapters.rs`, `scheduler.rs`, `service.rs` (new): one `//!` doc line each, no code.
  - `crates/genie/src/lib.rs`: `pub mod observation;`.
  - `crates/genie/src/http/observation.rs` (new): `pub fn routes() -> Router<Arc<App>> { Router::new() }`; `crates/genie/src/http/mod.rs`: `mod observation;` and `.merge(observation::routes())`.
  - `crates/genie/src/ops/observation.rs` (new): `pub fn register(_all: &mut Vec<Entry>) {}`; `crates/genie/src/ops/mod.rs`: `mod observation;` and `observation::register(&mut all);` in `catalog()`. Do **not** add a `GROUPS` entry yet.
  - `crates/genie/tests/it/observation_adapters.rs`, `observation_service.rs`, `observation_scheduler.rs`, `observation_api.rs`, `observation_catalog.rs`, `observation_e2e.rs` (new, one `//!` line each) and their `mod` lines in `tests/it/main.rs`; `crates/genie/tests/it/common/fakemetrics.rs` (new, one `//!` line) and `pub mod fakemetrics;` in `common/mod.rs`.
  - Generated TypeScript files for the new exported types.
- **Tests** (unit, in `observation/mod.rs`): every enum round-trips as its string and rejects an unknown value with `unknown <label> <value>`; `parse_case_id` accepts `O-7`, `o-7`, `7`, `observation:7` and rejects `G-7`, `O-`, `O-x`, `-1`; `parse_incident_id` likewise; `ts(parse_ts(x)) == x` for `2026-10-17T12:00:00.000Z`; a `Contract` with an unknown field fails to deserialize; the §5.1 example deserializes.
- **Commands:** `cargo fmt --all --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test -p genie-core`; `cargo test -p genie --test it catalog` (catalog parity still passes).
- **Acceptance criteria:**
  1. All §4.1 enums, §4.2 helpers, §4.3 structs, §5.1 contract structs, and §12 constants exist with the exact names and wire strings of the specification.
  2. The application, HTTP, ops, and test skeletons compile and change no behavior.
  3. Generated types are committed; all commands pass.

### OBS-02 Contract validation and change grammar

- **Goal:** Make an outcome contract checkable: parse change thresholds and report every contract problem before a contract is stored.
- **Depends on:** OBS-01
- **Spec:** §5.1–§5.3
- **Read:** `crates/genie-core/src/observation/contract.rs`, `crates/genie-core/src/automation.rs` (`parse_duration`).
- **Files:** `crates/genie-core/src/observation/contract.rs`.
- **Implement:** `Change`, `ChangeUnit`, `parse_change`; `Contract::problems(&self, policy) -> Vec<String>`; `Contract::validate(&self, policy) -> Result<()>` (the `contract is invalid: …` error); `Contract::resolved(&self) -> Resolved` (all windows as `chrono::Duration` with defaults; call only on a valid contract, `expect` with a message otherwise); `Contract::needs_baseline(&self, criterion: &str) -> bool`; `Contract::criterion(&self, id) -> Option<CriterionRef<'_>>` (enum `Hypothesis(&Hypothesis)` / `Guardrail(&Guardrail)`); `Contract::sources(&self) -> BTreeSet<String>` (non-manual source names). Problem messages name the field path, such as `hypotheses[0].expect: exactly one of change, max, min`.
- **Tests** (unit): a table for `parse_change` (`-20%`, `+0.1pp`, `+5`, `-0.25%`, ` +3 `, rejects `20%`, `+%`, `+1.2.3`, `+1e3`, `±1`); one test per validation rule of §5.3 asserting the exact message; policy rules both ways; defaults of `resolved()` (`7d`, preferred = minimum, `6h`, `15m`, `1h`); `needs_baseline` for each criterion form; the §5.1 example is valid for `verify` and invalid for `guardrails`.
- **Commands:** `cargo fmt --all --check`; `cargo clippy -p genie-core --all-targets -- -D warnings`; `cargo test -p genie-core observation`.
- **Acceptance criteria:**
  1. Every rule of §5.3 is enforced with a test asserting its message.
  2. `parse_change` follows §5.2 exactly, including the zero rule for hypotheses.
  3. No persistence or HTTP code in this card.

### OBS-03 Evaluator and guardrail check

- **Goal:** Decide, as a pure function, whether a guardrail is violated and what a case's evidence concludes.
- **Depends on:** OBS-02
- **Spec:** §4.3 (usable records), §8, §9
- **Read:** `crates/genie-core/src/observation/mod.rs`, `contract.rs`.
- **Files:** `crates/genie-core/src/observation/evaluate.rs` (stub from OBS-01).
- **Implement:** `is_usable(&Evidence) -> bool`; `deltas(value: f64, baseline: Option<f64>) -> (Option<f64>, Option<f64>)` (absolute, relative percent; relative `None` when baseline is 0); `check_guardrail(&Guardrail, &Evidence) -> Option<String>` (§8 table and number formatting); `EvalInput`, `EvalOutcome`, `evaluate(&EvalInput) -> EvalOutcome` (§9.1–§9.4). Pure functions only.
- **Tests** (unit, build `Evidence` values with a small helper):
  - `deltas`: positive, negative, zero baseline, missing baseline.
  - `check_guardrail`: each limit violated and not violated, unusable record (error, `agent` trust), missing baseline for change limits, message texts of §8.
  - `evaluate`: rule 1 (before minimum window, even with a violation); rule 2 (violation → negative, high); guardrails policy before final, final all ok, final with a never-measured guardrail; verify all met before final → confirmed; verify not met before final → insufficient; verify final not met → negative; final mixed met/insufficient → inconclusive; minimum samples unmet and unknown; relative change with zero baseline → insufficient; `reported` evidence → medium confidence; `agent` evidence ignored; summary text exact for one verify and one guardrails case; details order.
  - Property-style test: for both policies and every combination of per-criterion states over two hypotheses and two guardrails, a `final` evaluation never returns `insufficient`.
- **Commands:** as OBS-02.
- **Acceptance criteria:**
  1. Verdict rules match §9.3 in order, with one test per rule.
  2. A final evaluation never yields `insufficient` (exhaustive test).
  3. Messages and summary strings match §8 and §9.4.

### OBS-04 Tracker schema 7 and the case lifecycle store

- **Goal:** Persist outcome cases and versioned contracts in the project tracker and enforce their lifecycle up to the start of observation.
- **Depends on:** OBS-02
- **Spec:** §6, §12, §13.1
- **Read:** `crates/genie-core/src/db.rs`, `crates/genie-core/src/management.rs` (accessor pattern `Management<'a>(&'a Tracker)`, events, `tx`), `crates/genie-core/src/tracker.rs` (`append_event`, `normalize_id`, `exists`, `get`), `crates/genie-core/tests/tracker.rs` (test helpers), `crates/genie/tests/migrate.rs`.
- **Files:** `crates/genie-core/src/db.rs` (all §13.1 DDL, `SCHEMA_VERSION = 7`), `crates/genie-core/src/observation/store.rs` (stub from OBS-01), `crates/genie-core/tests/observation.rs` (new).
- **Implement** `pub struct Observation<'a>(&'a Tracker)` and `impl Tracker { pub fn observation(&self) -> Observation<'_> }` with:
  - Inputs: `NewCase { title, policy, tasks: Vec<String>, contract: Option<Contract> }`, `CasePatch { title, policy, add_tasks, remove_tasks }`, `CaseFilter { status: Option<CaseStatus>, task: Option<String>, limit: Option<i64> }`, `NewDeployment { reference, environment, at: DateTime<Utc> }`.
  - `create(actor, NewCase) -> Result<ObservationCase>`; `get(id) -> Result<ObservationCase>` (not found: `observation O-<id> not found`); `list(&CaseFilter)` (newest first, limit default 100, max 200); `for_task(task)`; `update(actor, id, CasePatch)`; `set_contract(actor, id, &Contract, reason: &str)`; `contract(id, version: Option<i64>) -> Result<Option<ContractVersion>>`; `contracts(id) -> Result<Vec<ContractVersionInfo>>`; `arm(actor, id, now)`; `start(actor, id, anchor: DateTime<Utc>, how: StartOn)`; `on_task_done(actor, task, now) -> Result<Vec<i64>>`; `record_deployment(actor, id, NewDeployment, now) -> Result<Deployment>`; `deployments(id)`; `request_evaluation(actor, id, final_: bool)`; `cancel(actor, id, reason, now)`.
  - Rules exactly as §6.2–§6.5, including: core-level person checks (`actor.role == Role::Human`) for contract revision after start and `final` requests, returning `GenieError::Denied` with `only a person may …`; task IDs normalized with `Tracker::normalize_id`, unknown task → `task <id> not found`; every mutation writes its §12 event with subject `case_subject(id)` and the payload keys of §12 in the same transaction; `arm` starts at once when `start = tasks_done` and all tasks are done; `start` sets `nextGuardrailAt = anchor`, `nextEvaluationAt = anchor + minimumWindow`; idempotency (`start` on an observing case is a no-op returning the case; `on_task_done` ignores non-waiting cases).
  - Private row mapping `fn case_row(...)` that also loads `tasks` (sorted).
- **Tests** (`crates/genie-core/tests/observation.rs`, tracker in a temp dir): a fresh tracker records schema 7 and has all observation tables; create with and without a contract (version 1 / 0); invalid contract rejected with its message; unknown task rejected; `update` in each status; contract versions increment, an agent revision after start is denied, a person's revision without a reason is rejected, with a reason succeeds and keeps older versions; `arm` without contract / without tasks fails; `arm` with all tasks done starts immediately (anchor = `now`); `on_task_done` starts only when every linked task is done, ignores `cancelled` tasks; `start = deployment` starts on `record_deployment` with anchor = deployment `at`, later deployments do not move the anchor; deployments in the future or older than 30 days rejected; `request_evaluation` keeps `final` over `manual` and denies `final` for an agent; `cancel` from each non-terminal status and rejection from terminal ones; every operation appends the expected event type with subject `observation:<id>` (read with `events_after`); deleting a linked task removes the link and keeps the case.
- **Commands:** `cargo fmt --all --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test -p genie-core`; `cargo test -p genie --test migrate`.
- **Acceptance criteria:**
  1. Schema 7 adds exactly the §13.1 tables and indexes; existing migration tests pass unchanged.
  2. Every transition of §6.2 is implemented with its preconditions, effects, and event, each covered by a test.
  3. State changes and events share a transaction.

### OBS-05 Evidence, evaluations, incidents, follow-ups

- **Goal:** Persist evidence, evaluations, incidents, and follow-ups atomically, and expose the queries the scheduler needs.
- **Depends on:** OBS-03, OBS-04
- **Spec:** §7.2, §7.5, §8, §10, §11.3 (step 5), §11.4 (steps 1, 5), §11.5
- **Read:** `crates/genie-core/src/observation/store.rs`, `evaluate.rs`, `crates/genie-core/tests/observation.rs`.
- **Files:** `crates/genie-core/src/observation/store.rs`, `crates/genie-core/tests/observation.rs`.
- **Implement** on `Observation`:
  - `NewEvidence { criterion, kind, phase, source, query, window_start, window_end, value: Option<f64>, baseline: Option<f64>, samples: Option<i64>, provenance: Value, trust, error: Option<String>, note }` (`contract_version` and `at` are filled by the store; deltas are computed with `evaluate::deltas` when `value` is set).
  - `apply_collection(actor, id, CollectionInput { contract_version: i64, records: Vec<NewEvidence>, now, next_guardrail_at: Option<DateTime<Utc>>, pass: bool, failed: Option<String> }) -> Result<Option<Applied>>` — one transaction: returns `None` without writing when the case is not `observing`/`evaluating` or its contract version differs from `contract_version` (the version the caller snapshotted, D8); `pass` is true for scheduler passes and false for manual reports, which leave `failures` and `lastError` untouched; records with `windowStart > windowEnd` are refused (a zero-length window is valid); incidents opened here are stamped with `now`; validates each `criterion` exists in the current contract and `kind` matches; inserts records; runs `check_guardrail` on every new usable observed guardrail record and calls `open_incident`; sets `nextGuardrailAt` when given; resets `failures` to 0 when `failed` is `None`, else increments it and stores `lastError`; writes one `evidence.collected` event when any record was inserted. `Applied { evidence: Vec<Evidence>, new_incidents: Vec<Incident>, failures: i64 }`.
  - `evidence(id, EvidenceFilter { after, limit, criterion, newest_first })`; `current_evidence(id) -> Vec<Evidence>` (current contract version, all records); `latest_baseline(id, criterion) -> Option<Evidence>` (usable, current version).
  - `begin_evaluation(id) -> Result<bool>` (§11.4 step 1: true from `observing` or `evaluating`, false otherwise); `finish_evaluation(actor, id, &EvalOutcome, trigger, contract_version, now) -> Result<Option<ObservationCase>>` exactly as §11.4 step 5 (insert `Evaluation`, clear request, conclude or reschedule, events, `None` when not `evaluating` or the contract was revised since the snapshot); `evaluations(id, limit)`.
  - `open_incident(actor, id, guardrail, evidence: Option<i64>, summary) -> Result<(Incident, bool)>` (§10.1, events `guardrail.violated` and `incident.created` only when new); `incident(id)`; `incidents(status: Option<IncidentStatus>)` (newest first); `case_incidents(case)`; `resolve_incident(actor, id, note)`; `dismiss_incident(actor, id, note)` (person only, note required); `violated_guardrails(case) -> Vec<String>` (open or resolved, deduplicated).
  - `add_followup(actor, case, incident: Option<i64>, task) -> Result<Followup>` (`followup_task.created`); `followups(case)`; incidents carry their follow-up task IDs.
  - `due(now) -> Vec<ObservationCase>` and `next_due() -> Option<String>` (§11.3, §11.5).
- **Tests** (append to `crates/genie-core/tests/observation.rs`): `apply_collection` on a cancelled case writes nothing; unknown criterion and kind mismatch rejected; deltas stored; a violating guardrail record opens one incident and a second violation does not open another; an `agent` record never opens an incident; failures counter increments and resets; one `evidence.collected` event per call; `finish_evaluation` with `insufficient` reschedules to `min(now + interval, anchor + preferred)` and with `confirmed` concludes (result, confidence, conclusion, cleared schedule, `outcome.confirmed` event); `finish_evaluation` after a cancel returns `None`; `begin_evaluation` is true for `observing` and `evaluating` and false for other statuses; incident resolve/dismiss transitions, dismiss by an agent denied, dismissed incidents excluded from `violated_guardrails`; follow-up recorded and listed on the incident; `due` returns guardrail-due, evaluation-due, and requested cases only; `next_due` is the minimum.
- **Commands:** as OBS-04.
- **Acceptance criteria:**
  1. Collection, incident creation, and scheduling updates are atomic and idempotent as specified.
  2. Conclusion and rescheduling follow §11.4 step 5 exactly.
  3. Person-only incident dismissal is enforced in the core.

### OBS-06 Server schema 7: sources and secrets

- **Goal:** Let project administrators store observation sources and their sealed credentials in the server database.
- **Depends on:** OBS-01
- **Spec:** §4.3 (`SourceConfig`), §7.3 (auth), §13.2
- **Read:** `crates/genie-core/src/server_db.rs` (`open_migrated`, `SERVER_SCHEMA_VERSION`), `crates/genie-core/src/secrets.rs` (repo token functions), `crates/genie-core/src/repos.rs` (project-scoped CRUD style).
- **Files:** `crates/genie-core/src/observation/sources.rs` (stub from OBS-01), `crates/genie-core/src/server_db.rs`, `crates/genie-core/src/secrets.rs`, `crates/genie-core/tests/observation_sources.rs` (new).
- **Implement:** `sources::SCHEMA`; executed in `open_migrated` after `SECRETS_SCHEMA`; `SERVER_SCHEMA_VERSION = 7`; `observation_secrets` in `SECRETS_SCHEMA`. `NewSource` input (all `SourceConfig` fields except timestamps) and `SourceConfig::problems()`; `impl ServerDb { add_observation_source(project, NewSource), update_observation_source(project, name, NewSource), remove_observation_source(project, name), observation_source(project, name) -> Option<SourceConfig>, observation_sources(project) -> Vec<SourceConfig> }`; secrets: `set_observation_secret(project, source, value)` (empty value deletes; the source must exist), `delete_observation_secret`, `observation_secret(project, source) -> Option<String>`, `observation_secret_info(project, source) -> Option<SecretInfo>`, with an AAD distinct from repository tokens. Removing a source deletes its secret in the same transaction. Duplicate name → `source <name> already exists`; unknown → `source <name> not found`.
- **Tests:** schema 7 recorded; validation messages for name, `manual`, URL forms (scheme, user info, query, fragment, trailing slash trimmed), Sentry without organization, timeout bounds; add/update/list/remove; secret round trip, hint, delete, secret for an unknown source rejected, secret removed with the source; a secret of project A is not readable as project B's.
- **Commands:** `cargo fmt --all --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test -p genie-core`; `cargo test -p genie --test migrate`.
- **Acceptance criteria:**
  1. Server schema 7 adds exactly the §13.2 tables; migration tests pass.
  2. Secrets are sealed like repository tokens and never returned by source listings.
  3. Validation follows §13.2 with tested messages.

### OBS-07 Adapters: Prometheus- and Sentry-compatible sources

- **Goal:** Measure a query against a Prometheus-compatible or Sentry-compatible source and return a structured measurement with provenance.
- **Depends on:** OBS-01
- **Spec:** §7.3, §7.4
- **Read:** `crates/genie/src/engine.rs` (`step_http`), `crates/genie/src/git/provider/mod.rs` (error handling of host calls), `crates/genie/tests/it/common/fakehost.rs` (fake HTTP server pattern).
- **Files:** `crates/genie/src/observation/adapters.rs`, `crates/genie/tests/it/common/fakemetrics.rs`, `crates/genie/tests/it/observation_adapters.rs`.
- **Implement:** `Span`, `Measurement`, `SourceError` (+ `Display`), `pub async fn measure(...)` (§7.3), private `prometheus(...)` and `sentry(...)`, `substitute_range(query, span) -> String`, body size limit, auth headers, provenance JSON (§7.4). Fake server: `fakemetrics::spawn(token: Option<&str>) -> FakeMetrics { url, state }` serving `/api/v1/query` and `/api/0/organizations/{org}/events/` with scripted responses (`state.lock().respond(fragment, json)`: the first rule whose `fragment` is contained in the received query wins, so range substitution does not break matching; a later `respond` with the same fragment replaces the response, which lets scheduler tests change a metric between passes), status overrides, a call log of full request URLs and auth headers, and an oversize-body switch.
- **Tests** (`#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`): Prometheus vector with one series, scalar, empty result, two series (message), `status: error`, NaN, non-2xx, 401 → `Unauthorized`, `$__range` replaced by the span seconds, `time` parameter equals the span end, samples query rounded; Sentry count with project and environment parameters, empty data → 0, custom field without samples, missing organization → `Config`; bearer and basic headers sent; a missing secret for bearer → `Config`; oversize body rejected; provenance has no secret and no `Authorization` value.
- **Commands:** `cargo fmt --all --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test -p genie --test it observation_adapters`.
- **Acceptance criteria:**
  1. Both adapters follow §7.3 request and response rules, each rule covered by a test against the fake server.
  2. Secrets never appear in errors or provenance (tested).
  3. No database access in this module.

### OBS-08 Application service

- **Goal:** Give every entrance one set of observation rules: permissions, arming checks, views, manual evidence, follow-up tasks, sources, and notifications.
- **Depends on:** OBS-05, OBS-06, OBS-07
- **Spec:** §6.2 (arm problems), §7.5, §10, §14, §15 (bodies and `CaseView`)
- **Read:** `crates/genie/src/management.rs` (`project_manager`, `authorize_task`, view structs), `crates/genie/src/tasks.rs` (`Caller`, `create`, `CreateBody`, `changed`), `crates/genie/src/notify.rs` (`resolve`, `send`, `Message`), `crates/genie/src/state.rs` (`with_tracker`, `with_server`, `AppError`).
- **Files:** `crates/genie/src/observation/service.rs`, `crates/genie/tests/it/observation_service.rs`.
- **Implement** request bodies of §15 (`CreateCaseBody`, `UpdateCaseBody`, `ContractBody`, `EvaluateBody`, `ReasonBody`, `DeploymentBody`, `EvidenceBody`, `FollowupBody`, `NoteBody`, `SourceBody`, `SecretBody`, `SourceTestBody`), `CaseView`, `SourceView`, and functions taking `(app: &App, project: &str, actor: &Actor, …)` that call the core inside `with_tracker`/`with_server`:
  - `authorize_manage(actor)` — person, or `crate::management::project_manager(actor)` succeeds; else `Forbidden("only people and the project manager manage observation")`. Person-only actions (`final` evaluation, contract revision after start, dismiss) rely on the core checks and map `Denied` to 403 here.
  - `list`, `show` (builds `CaseView`, newest-first limits of §15), `for_task`, `create`, `update`, `set_contract`, `arm` (computes `arm_problems` with sources and secrets from `server.db`; `Conflict` when any), `start`, `evaluate`, `cancel`, `deploy`, `report_evidence` (trust from the actor, §7.5), `incidents`, `resolve_incident`, `dismiss_incident`, `followup` (§10.2 through `crate::tasks::create`, then `add_followup`), and `open_followups_for(app, project, case, incidents)` used by the scheduler for `followup: auto`.
  - Sources (people with admin rights only; the HTTP layer checks `Access::admin`): `sources`, `add_source`, `update_source`, `remove_source`, `set_source_secret`, `clear_source_secret`, `test_source` (calls `adapters::measure` via `state::block_on` outside any lock, window default `1h` ending now).
  - Notification helpers used by the scheduler and here: `notify_incident(app, project, &Incident, &ObservationCase)` and `notify_concluded(app, project, &ObservationCase)` (§18: kinds, links, dedupe keys, project admins and the human author; mail to the project manager through `Management::notify`).
  - After a follow-up task is created, call `crate::tasks::changed(app)`. The scheduler wake-up field does not exist yet (OBS-09 adds it): put a `// OBS-09: wake the observation scheduler` comment after each successful arm, start, evaluate, deploy, and report.
- **Tests:** a team member token and a task-manager token are forbidden for every mutation; a viewer cannot mutate (via `Access` in OBS-10 — here test actors directly); a person and the project manager can create, arm, start; `arm_problems` lists unknown sources and missing secrets and clears when fixed; manual evidence trust is `reported` for a person and `agent` for the project manager; a reported violating guardrail record opens an incident; a follow-up creates a `bug` task with label `observation` linked from the incident; `followup` without an incident creates a `task`; `CaseView` contains contract history, arm problems, incidents with follow-ups; `test_source` returns the fake server's value and maps adapter errors to `GenieError::invalid` (422, §15); §18 notifications (kinds, links, dedupe, recipients, manager mail) are asserted.
- **Commands:** `cargo fmt --all --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test -p genie --test it observation_service`.
- **Acceptance criteria:**
  1. Permissions of §14 hold for every function, each denial tested.
  2. Follow-up tasks are created only through `tasks::create`.
  3. No database lock is held while a source is measured.

### OBS-09 Scheduler and wiring

- **Goal:** Run observation without live agents: start cases after delivery, watch guardrails, evaluate on schedule, and survive restarts.
- **Depends on:** OBS-08
- **Spec:** §6.3, §7.2, §10.2 (auto), §11
- **Read:** `crates/genie/src/engine.rs` (`start`, `pass`, `Plan`, `intake`), `crates/genie/src/state.rs` (`App`, `project_tracker` hook, `SAFETY_NET`, `sleep_until`, `block_on`), `crates/genie/src/cli.rs` (`Command::Serve`).
- **Files:** `crates/genie/src/observation/scheduler.rs`, `crates/genie/src/state.rs` (field `wake_observation: Arc<Notify>`, initialized in `App::open`, notified in the `on_event` hook), `crates/genie/src/cli.rs` (`crate::observation::scheduler::start(&app)` after `spawn_poller`), `crates/genie/src/observation/service.rs` (replace the OBS-09 comments with `app.wake_observation.notify_one()`), `crates/genie/tests/it/observation_scheduler.rs`.
- **Implement:** `start`, `pass_at(app, now) -> AppResult<Plan>`, `Plan { at: Option<DateTime<Utc>>, again: bool }`; intake with cursor `observation` (§11.2); due-case processing (§11.3 steps 1–7); evaluation pass (§11.4) using `evaluate::evaluate`; auto follow-ups and notifications after commit; failure notification at 3 consecutive failures with its dedupe key.
- **Tests** (fake metrics server; call `h.app.blocking(move |app| scheduler::pass_at(app, now)).await` with explicit times):
  - A `waiting` case starts when its last linked task becomes `done` (move the tasks with the tracker); a case with one task still open does not.
  - First pass collects baselines then guardrails; windows in the fake server's call log match §7.2; `nextGuardrailAt` advances by the interval; a pass before it is due makes no calls.
  - A violating guardrail opens one incident across several passes; with `followup: auto` exactly one follow-up task exists.
  - Evaluation at `anchor + minimumWindow` with met hypotheses concludes `confirmed`; with unmet hypotheses stays `observing` and reschedules; at `anchor + preferredWindow` concludes `negative` or `inconclusive` as §9.3 says.
  - A manual `final` request concludes on the next pass; a cancelled case is never written by an in-flight evaluation (cancel between `begin_evaluation` and `finish_evaluation` using the store directly).
  - Source errors are stored as evidence errors, `failures` increments, and the admins are notified once at 3.
  - A case left in `evaluating` (simulated restart) is processed like `observing`.
  - `Plan.at` equals the earliest due time.
- **Commands:** `cargo fmt --all --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test -p genie --test it observation_scheduler`; then the full suite `cargo test --workspace --lib --bins --tests`.
- **Acceptance criteria:**
  1. The scheduler follows §11 with injected time and survives restarts.
  2. The serve command starts it; the journal wakes it.
  3. Every scenario above is a passing test.

### OBS-10 HTTP routes

- **Goal:** Expose the observation service over REST with the documented bodies, permissions, and status codes.
- **Depends on:** OBS-08
- **Spec:** §14, §15
- **Read:** `crates/genie/src/http/management.rs`, `crates/genie/src/http/ctx.rs` (`Access`, `write`, `admin`, `is_human`), `crates/genie/src/http/mod.rs` (`ApiError` mapping), `crates/genie/tests/it/management.rs` (HTTP test style).
- **Files:** `crates/genie/src/http/observation.rs`, `crates/genie/tests/it/observation_api.rs`.
- **Implement:** every route of §15 with `ctx.access(&app, None)`; mutations call `a.write()` then the service; source routes call `a.admin()` and reject agents; numeric path ids parsed with `parse_case_id`/`parse_incident_id` (invalid → 400); work in `app.blocking`.
- **Tests:** create → show → contract → arm → start → evaluate → cancel through HTTP as the local operator; a viewer gets 403 for a mutation and 200 for reads; a team-member token gets 403; unknown case 422 with `observation O-99 not found`; invalid contract 422 with problems; `deny_unknown_fields` → 400 or 422 as the existing extractor returns (assert the current behavior); arm conflict 409 with arm problems; sources CRUD and secret never returned (only `hint`); `test` route with the fake server; `GET /observation/tasks/{task}`.
- **Commands:** `cargo fmt --all --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test -p genie --test it observation_api`; full suite before the PR.
- **Acceptance criteria:**
  1. All §15 routes exist with the documented bodies and status codes.
  2. Secrets never leave the server (tested).
  3. Generated types for the bodies and views are committed.

### OBS-11 CLI and MCP operations

- **Goal:** Make observation available as `genie observe …` on the command line and as MCP tool `genie_observe` from the shared catalog.
- **Depends on:** OBS-10
- **Spec:** §16
- **Read:** `crates/genie/src/ops/management.rs`, `crates/genie/src/ops/mod.rs` (`Op`, `Need`, `Listed`, `GROUPS`, `Cx::text`), `crates/genie/tests/it/catalog.rs`, `crates/genie/tests/it/ops.rs`.
- **Files:** `crates/genie/src/ops/observation.rs`, `crates/genie/src/ops/mod.rs` (`GROUPS` entry after `manager`), `crates/genie/tests/it/observation_catalog.rs`.
- **Implement:** every op of §16 with the listed `Need` and `Listed`, doc comments as summaries (they become CLI help and MCP descriptions), JSON output like `ops/management.rs`.
- **Tests:** CLI lines for `create` with `--contract` JSON and with `--contract-file`, `show`, `arm`, `report`, `incidents`, `source-add`, and `source-secret` with an inline `--value` (stdin is not used in tests); the same ops through `run_json` (MCP path) reject `--contract-file` with the existing "files are read only on the command line" message; a team-member command table does not list `observe` ops; the project manager's table lists the `Orchestrator` ops and not `dismiss` or source ops; existing catalog parity tests pass.
- **Commands:** `cargo fmt --all --check`; `cargo clippy --all-targets -- -D warnings`; `cargo test -p genie --test it observation_catalog catalog ops`; full suite before the PR.
- **Acceptance criteria:**
  1. `genie observe …` and MCP tool `genie_observe` expose exactly the §16 operations.
  2. Agent command tables follow the `Listed` column (tested).

### OBS-12 Web entity and live updates

- **Goal:** Give the web client typed access to observation data and keep it fresh from journal events.
- **Depends on:** OBS-10
- **Spec:** §17 (entity, live updates, labels)
- **Read:** `web/src/entities/task/api.ts`, `web/src/entities/decision/`, `web/src/shared/api/index.ts`, `web/src/shared/api/invalidation.ts`, `test/web-live-invalidation.test.ts`, `test/web-owner-decision.test.ts` (model test style), `steiger.config.ts`.
- **Files:** `web/src/entities/observation/api.ts`, `model.ts`, `index.ts` (new); `web/src/shared/api/invalidation.ts`; `test/web-observation.test.ts` (new); `test/web-live-invalidation.test.ts`.
- **Implement:** hooks `useObservationCases(filter)`, `useObservationCase(id)`, `useTaskObservations(taskId)`, `useObservationIncidents(status)`, `useObservationSources()` and mutations for every §15 route via `useInvalidating`, typed with the generated types; `model.ts` pure helpers: `STATUS_LABEL`, `RESULT_LABEL`, `POLICY_LABEL`, `INCIDENT_LABEL` (§17 table), `caseActions(case, { writable, human })` returning the allowed actions per §14, `formatDelta(evidence)` (`-23.1%`, `+0.05`), `contractTemplate(policy)` (a valid example contract per policy as pretty JSON), `isActive(status)`.
- **Tests:** labels cover every generated enum value; `caseActions` for each status and role; `formatDelta` rounding and null; `contractTemplate` produces JSON that parses and has the right criteria; invalidation maps every §12 event type to `["observations"]` and `followup_task.created` also to task keys.
- **Commands:** `npm test`; `npm run typecheck`; `npm run lint:web`; `npm run build:web`.
- **Acceptance criteria:**
  1. All server routes have typed hooks; no hand-written copies of generated types.
  2. Live events refresh observation queries (tested).

### OBS-13 Web: observation pages

- **Goal:** Let people browse outcome cases and work on one case in the web UI.
- **Depends on:** OBS-12
- **Spec:** §14, §17
- **Read:** `web/src/app/App.tsx`, `web/src/widgets/sidebar/ui/Sidebar.tsx`, `web/src/pages/platform/ui/PlatformPages.tsx` (list page style), `web/src/widgets/task-detail/ui/TaskManager.tsx` (actions, toasts), `web/src/shared/ui`.
- **Files:** `web/src/pages/observations/index.ts`, `web/src/pages/observations/ui/ObservationsPage.tsx`, `ObservationPage.tsx`, `observations.css` (new); `web/src/app/App.tsx` (routes `/observations`, `/observation/:caseId`, both treated as own pages like `/automations`); `web/src/widgets/sidebar/ui/Sidebar.tsx` (item with glossary label `nav`).
- **Implement:** §17 list and case page: filters, table, header, actions per `caseActions`, contract viewer and JSON editor (template button, server error shown inline), arm problems, tasks as links, deployments list and form, evaluations, evidence table, incidents with resolve, dismiss (note required), and follow-up. Semantic controls, labels, keyboard access.
- **Tests:** none beyond `model.ts`; manual check described in the PR (screenshots optional).
- **Commands:** as OBS-12.
- **Acceptance criteria:**
  1. Both pages work against the API with loading, empty, and error states.
  2. Controls follow §14; hidden actions are also refused by the server.

### OBS-14 Web: task card section

- **Goal:** Show a task's outcome cases on its card and let writers start one from there.
- **Depends on:** OBS-12
- **Spec:** §17 (task card)
- **Read:** `web/src/widgets/task-detail/ui/TaskDetail.tsx`, `TaskManager.tsx`.
- **Files:** `web/src/widgets/task-detail/ui/TaskObservation.tsx` (new), `TaskDetail.tsx` (render it after `TaskManager` in both layouts).
- **Implement:** linked cases (key, title, status and result labels, link to `/observation/<id>`); the glossary label `empty` when none; a `create` button for writers with a small form (title prefilled with `newTitle`, policy select), then navigate to the new case.
- **Commands:** as OBS-12.
- **Acceptance criteria:** the section appears for non-epic tasks, creates a proposed case linked to the task, and updates live.

### OBS-15 Web: observation sources in project settings

- **Goal:** Let project administrators configure and test observation sources in project settings.
- **Depends on:** OBS-12
- **Spec:** §13.2, §15 (sources), §17 (settings)
- **Read:** `web/src/widgets/settings-shell/ui/SettingsShell.tsx`, `web/src/pages/settings/ui/ProjectSettings.tsx`, the repositories settings section it renders.
- **Files:** `web/src/pages/settings/ui/ObservationSources.tsx` (new), `ProjectSettings.tsx`, `web/src/widgets/settings-shell/ui/SettingsShell.tsx` (section `observation`, glossary label `section`).
- **Implement:** list, add, edit, remove (confirm), set or clear secret (password input, shows hint only), test query with the result or error; fields shown per kind (Sentry: organization, project, environment, field, dataset). Admins only; others see a read-only notice.
- **Commands:** as OBS-12.
- **Acceptance criteria:** an administrator configures and tests a Prometheus and a Sentry source end to end; the secret value is never displayed after saving.

### OBS-16 End-to-end scenarios

- **Goal:** Prove the closed loop end to end: delivery, observation, incident, follow-up, and conclusion.
- **Depends on:** OBS-09, OBS-10, OBS-11
- **Spec:** §1.1, §6, §9, §10, §11
- **Read:** the observation tests written so far, `crates/genie/tests/it/scenarios.rs`.
- **Files:** `crates/genie/tests/it/observation_e2e.rs`.
- **Implement** three scenarios through HTTP or catalog entrances, with the fake metrics server and `pass_at`:
  1. **Guardrail regression:** task G-1 done → case (`guardrails`, `followup: auto`) observing → error rate crosses `maxChange` → incident, `bug` follow-up task, notification row for the admin, mail to the project manager → final evaluation `negative`; journal contains the §12 events in order.
  2. **Verified improvement:** `verify` case with latency `-20%` target and a minimum sample count → insufficient at the minimum window (samples low) → confirmed later with confidence `high`.
  3. **False positive:** violation → person dismisses the incident → final evaluation `confirmed`; an agent attempting to dismiss is refused.
- **Commands:** `cargo test -p genie --test it observation_e2e`; full suite.
- **Acceptance criteria:** the three scenarios pass and assert states, events, tasks, and notifications.

### OBS-17 Documentation and agent guidance

- **Goal:** Document Phase 1 for users and operators and teach the project manager when and how to use observation.
- **Depends on:** OBS-11, OBS-13, OBS-14, OBS-15
- **Spec:** §18
- **Read:** `docs/platform/automations.md`, `docs/platform/project-and-task-managers.md`, `docs/reference/cli.md`, `docs/platform/decisions.md`, `docs/platform/backend.md`, `GLOSSARY.md`, `CHANGELOG.md`, `docs/.vitepress/config.mts`, `agents/project-manager.md`.
- **Files:** `docs/platform/observation.md` (new user reference: concepts, contract format with the change grammar and `pp` note, sources setup, lifecycle, evaluation rules, incidents and follow-ups, CLI examples, automation events, limits); `docs/platform/automations.md` (event list); `docs/reference/cli.md` (`observe` group); `docs/platform/decisions.md` (PD26); `docs/platform/backend.md` (code map row); `docs/platform/roadmap.md`; `docs/platform/proposals.md` (link); `docs/.vitepress/config.mts` (sidebar under "Work with Genie"); `GLOSSARY.md` (only if labels changed during implementation; the observation section exists already); `CHANGELOG.md` (Unreleased: Added); `agents/project-manager.md` (Observation section, §18); this plan and the specification (`status: current` for the specification).
- **Commands:** `npm ci --prefix docs`; `npm run docs:check`; `npm run docs:build`; `cargo test -p genie --test it` (prompt tests, if any, still pass).
- **Acceptance criteria:** documentation is English, links resolve, and every user-visible behavior of Phase 1 is described once.

## 4. Review checklists

### 4.1 Every card

- [ ] Implements the card's acceptance criteria and nothing outside its files without a stated reason.
- [ ] Names, wire strings, messages, and status codes match the specification.
- [ ] State changes and journal events share a transaction; no lock is held across HTTP.
- [ ] Errors use the right kind (§2 rule 4) and actionable English messages.
- [ ] Tests cover success and error paths, assert messages, and use injected time instead of sleeping.
- [ ] `cargo fmt`, `clippy -D warnings`, and the card's tests pass; generated types are committed and not hand-edited.
- [ ] No secrets in logs, errors, events, provenance, or API responses.

### 4.2 Rust core and server

- [ ] `genie-core` has no async or HTTP code.
- [ ] Schema changes are additive, versioned, and covered by migration tests; older binaries refuse the newer schema through the existing check.
- [ ] Idempotency: repeated intake, start, incident opening, and follow-ups do not duplicate rows.
- [ ] Person-only and manager-only rules of §14 are enforced server-side, independent of the entrance.
- [ ] Journal subjects are `observation:<id>`; payload keys match §12.

### 4.3 Web

- [ ] Uses generated types and `shared/api`; query keys follow §17; mutations invalidate or rely on live events.
- [ ] Russian labels from §17; semantic controls with labels; keyboard reachable.
- [ ] Layer boundaries pass `npm run lint:web` (Steiger).

### 4.4 Final integration review

Before opening `feat/observation-plane` → `dev`:

1. Merge `origin/dev` into the integration branch; resolve conflicts; regenerate types with `cargo test`.
2. Run every check of [Development](../development.md#checks) plus `npm run docs:check`.
3. Reviewer reads the whole diff once against the specification and §4, focusing on cross-card seams: service ↔ scheduler, events ↔ web invalidation, permissions across HTTP, catalog, and web.
4. Smoke test on a local server: configure a fake or real Prometheus source, create and arm a case, mark the task done, run passes (or wait), and see incidents and the conclusion in the UI.
5. Open the PR into `dev` with the Phase 1 summary, the card list with their PRs, validation evidence, and the open questions of specification §21.
