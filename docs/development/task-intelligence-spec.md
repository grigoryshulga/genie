---
title: Task Intelligence and Factory Analytics implementation specification
summary: GNE-16 implementation contracts extracted from the October 18 Task Intelligence RFC.
type: specification
status: implemented
tags: [Jev, routing, analytics, telemetry]
---

# Task Intelligence and Factory Analytics

Source: **RFC: Task Intelligence, Adaptive Routing, and Factory Analytics**, 2026-10-18,
provided with GNE-16. This specification covers all four RFC implementation phases.
Observation adapters, production monitoring, automatic policy updates, and live routing
experiments remain outside this delivery. Publish the task PR against `dev`; release
to `main` is separate, as explicitly directed by the owner.

## Existing implementation and reuse

- `genie-core::Tracker` owns task mutations, versioned briefs, teams, usage, and
  transactional durable events. Add task intelligence here, not to the Jev call log.
- `decisions.rs` and `decisions/team.rs` already implement bounded Jev questions,
  allowances, uncertainty, timeouts, policy-approved template alternatives, separate
  topology/model decisions, cache keys, and explicit joint-mode overrides. Reuse them.
- `server.db.decisions` remains the provider-call/accounting record. References from
  project routing records are evidence pointers; no distributed transaction is needed.
- `runtime::spawn_team` is the common dispatch boundary. Recheck current task, policy,
  and configuration after remote calls and before allocating execution resources.
- `tasks.rs` remains the application service for user/agent/automation task mutations.
- `stats.rs` already supplies completion counts, nearest-rank median/P90, questions,
  review returns, daily throughput, and usage priced with configured model prices.
  Extend `/statistics`; retain its existing server-admin authorization.

## Domain decisions

1. **Attempt:** a durable execution interval for one task, independent of teams and
   individual agent turns. Start on delivery dispatch or entry into `in_progress`,
   whichever comes first. Reuse an open attempt across review returns, member additions,
   model changes, and restarts. Terminal task completion/cancellation finalizes it;
   a later execution creates a new attempt. A stopped/replaced team alone is not a
   failed task. Persist attempt identity separately because neither session nor team
   identity has these semantics. Provider retries are not task attempts.
2. **Review round:** an entry into `review`. A completed review is a subsequent
   `approved` or `changes_requested` verdict. Direct completion without a verdict is
   accepted delivery with unknown first-pass acceptance, not a successful first review.
3. **Delivery:** the existing `done` transition. Execution cycle time starts at the
   attempt boundary; the existing creation-to-done metric keeps its distinct meaning.
   Waiting is measurable only for known status intervals; overlapping sessions are
   never summed into elapsed execution time.
4. **Solo:** an explicit delivery template with one executor. A task manager remains
   responsible for dispatch and acceptance. Solo does not bypass reviewer permissions
   or mandatory checks; eligible templates are selected by declared roles/properties,
   not by hardcoded team names or ranks.
5. **Reassessment:** explicit material discovery (`sensitive_change`, `scope_expansion`,
   `irreversible_change`, `verification_gap`, `review_return`, `requirement_conflict`).
   Append a snapshot and decision. Add a reviewer/specialist to the existing workspace;
   never tear down working teams automatically. Apply a configured cooldown, permit
   immediate safety upgrades, and do not automatically downgrade active teams.
6. **Project profile:** versioned owner/admin policy plus bounded mechanical facts.
   Repository inspection may identify files/test manifests; it cannot define criticality
   or weaken protections. Unknown affected paths preserve uncertainty. Models may
   suggest sensitive scope but cannot clear a policy-derived protection.
7. **Cost:** existing usage is authoritative. Accepted-work cost includes every
   attributable attempt/review/retry of tasks in the delivery cohort, including work
   before the selected reporting window. Period spend remains a separate metric.
   Historical daily usage cannot reliably be split between attempts on the same day:
   expose missing attribution rather than allocating proportional guesses.

## Phase 1: immutable instrumentation

Add a versioned tracker migration with indexed, task-scoped records:

- `execution_attempts`: identity, task, ordinal, start time and start-event reference.
- `task_signal_snapshots`: task, revision, brief version/fingerprint, profile version,
  extractor version, timestamp, optional trigger event, typed signals and evidence.
- `routing_decisions`: task/attempt/snapshot references, decision kind, provider/version,
  policy version, eligible templates, selected template, models, mandatory checks,
  status, override reason and reason codes. Provider decision IDs are references.
- `execution_assessments`: append-only version per attempt, terminal result, delivery
  boundary and references to authoritative task events. Derive recoverable metrics.
- New response attribution records link accounting increments to attempt, stage, role and model in the same transaction; existing daily usage remains authoritative. Missing historical attribution is not backfilled.
- Versioned policy-evaluation records (phase 4) retain candidate policy, frozen input,
  comparison and recommendation evidence without changing active policy.

Typed signal schema version 1 has exactly these names: `ambiguity`, `scope`,
`complexity`, `blast_radius`, `reversibility`, `sensitivity`, `verification`, `novelty`.
Each includes a value, source (`deterministic`, `policy`, `jev`, `runtime`, `human`),
bounded evidence references, assessment timestamp, assessor version, and nullable
confidence. Preserve `unknown` and `not_applicable`. A confidence is classification
certainty, never acceptance probability. Validate values by signal; reject malformed,
duplicate, oversized and non-finite inputs. Store pointers and compact summaries,
not prompts, secrets or repository contents.

Append entity mutations and journal events in the same tracker transaction. Validate
that referenced attempts, snapshots and trigger events belong to the same task. No
endpoint updates/deletes an old snapshot, routing decision or finalized assessment.
Old tasks remain readable with empty telemetry and explicit coverage gaps; do not
invent historical signals. Version checks reject newer schemas. Restart/rebuild reads
the same source records without duplicating attempts or decisions.

## Phase 2: constrained routing

Add project-admin configuration to the existing decision policy, disabled by default
for backward compatibility. It includes a profile version, protected components/path
patterns, mandatory checks and their satisfying roles, safe fallback template, optional
semantic assessment, and reassessment cooldown. Validate referenced templates and roles.
Policy changes are human-admin-only, as existing Jev configuration is.

Resolve compact context from the task brief and bounded known repository facts.
When enabled, ask Jev independent choice questions for ambiguity, scope, complexity,
verification and novelty over the same explicit state. Missing/uncertain answers are
unknown. Policy-derived sensitivity, blast radius, and reversibility are authoritative.
Do not send repository content to a provider without the existing external-context grant.

Before topology/model routing, compute mandatory checks and eligible templates.
Reject any proposed or selected roster that does not satisfy mandatory role separation.
Unresolved owner decisions block dispatch with actionable clarification. High-impact
unknown scope requests bounded read-only preflight; unresolved safety requirements block
dispatch rather than guessing. A configured fallback must satisfy the same policy.
Timeout, abstention, unavailable key, or allowance exhaustion records fallback/abstention;
existing modes (`off`, `observe`, `advise`, `joint`, `select`) retain their authority.
Human explicit choices remain pinned but cannot bypass server safety constraints.

Every successful delivery dispatch appends its applied plan with resolved role model/
reasoning settings, snapshot and attempt. Preview and shadow evaluation must not be
counted as execution or applied routing. Protect stale task/policy/profile input checks.
Runtime reassessment uses the same service and records reasons and evidence before
adding required specialists. Workspace, current sessions, artifacts and prior history
survive. Repeated identical requests are idempotent; cooldown prevents oscillation.

Expose scoped history and reassessment through task application services, project HTTP,
and the shared CLI/MCP operation catalog. Agents can inspect only their permitted tasks.
Web task detail displays brief → snapshots → decisions → execution/review/result evidence.

## Phase 3: Factory Analytics

Extend existing Statistics with a Factory view. One central Rust metric definition feeds
HTTP/CLI exports and UI. Use direct indexed project reads initially; no cross-project
cache or second dashboard/backend. A fresh read is a deterministic rebuild.

Use delivered/cancelled tasks in `[since, until]` as the quality/cost cohort; distinguish
task counts from attempt counts. Expose the exact window and membership. Every metric
reports observed denominator, total eligible population, missing count, numerator/sample
size, unit, and nullable value. Zero observations is unknown, not zero percent.

Metrics: delivered tasks; first-pass acceptance; accepted/rejected/failed/cancelled
attempts where observed; retry rate; completed review rounds; human intervention and
owner clarification; topology escalation and reasons; execution median/P90; active and
waiting duration; all-attempt accepted-work cost; failed-work cost; period total spend.
Do not label a review return as production failure or absent monitoring as no regression.

Break down by initial/final topology (including custom), role/model, initial signals,
router/policy/profile version and override/escalation reason. Apply identical metric
definitions within each cohort. Show a small-sample notice; comparisons are descriptive,
not causal rankings. The UI separates the grouping dimension from the selected group,
uses readable team/signal names, and keeps raw policy/profile identifiers in audit details.
Metrics explain their denominators and missing coverage; technical request keys are not
analytics choices. Drill-down exposes task and attempt IDs, input/decision provenance,
review history, human involvement and known/unpriced usage. Links use project scope.

## Phase 4: evaluation and learning

Provide version/signal cohort comparisons and task-scoped shadow evaluation of a
candidate policy. Freeze decision-time inputs and the policy version; evaluate within
the same eligible choices and provider allowance. Record a shadow decision separately,
including uncertainty/failure. No team, model, session, task status or production policy
may change. Compare selected plans/checks, not invented counterfactual success scores.

Generate a retrospective recommendation with supporting snapshot, routing and cohort
references. Label incomplete evidence and small samples. Recommendations remain proposals;
only an authorized human administrator applies a policy through the existing save path.
Policy rollback creates a new active configuration version and preserves old decisions.
Controlled experiments are a documented future extension, not automatically enabled.

## Acceptance and verification

1. Upgrading a populated old tracker preserves tasks/events/usage and passes integrity;
   newer schema refuses to open. Restart preserves IDs and immutable revisions.
2. New revisions cannot change old snapshots. Cross-task references, invalid values,
   fabricated policy provenance and oversized evidence are rejected transactionally.
3. Independent fake Jev tests cover signal choices, uncertainty, timeout, abstention,
   safe fallback, protected components, owner clarification and stale inputs. Low model
   labels and explicit human roster choices cannot weaken mandatory checks.
4. Solo/custom templates route by capabilities; adding a required reviewer preserves
   workspace and attempt. Duplicate reassessment/cooldown does not spawn extra members.
5. Review return stays in one attempt; terminal rerun creates another. Direct done is
   excluded from first-pass denominator. Cancelled/retry costs enter accepted-work totals.
6. Known event/usage fixtures reconcile API, CLI and UI metrics; missing telemetry and
   unpriced usage stay visible, including old tasks. Period boundaries and nearest-rank
   quantiles have deterministic tests. Drill-down membership matches aggregate denominators.
7. Project membership, read-only rights, task-manager scope and admin-only policy grants
   apply to every new endpoint/operation. No cross-project evidence leakage.
8. Shadow evaluation persists frozen input and recommendations but mutates no production
   execution/policy; failed/uncertain candidates do not fabricate projected outcomes.
9. Rust wire types generate committed TypeScript. Run applicable CI commands from
   `.github/workflows/ci.yml`; attach evidence in GNE-16. Run independent GPT-6.1-Sol
   review after implementation and address material findings before PR handoff.

Implementation checkpoints must record what is implemented and what remains; this
specification is not a claim that the proposed behavior already exists.

## Review-driven clarifications

Runtime safety constraints survive reassessment and require an explicit human
resolution with evidence against the latest snapshot. Member/spec/mail updates,
pause/status changes and applied routing commit atomically; live-session side
effects follow commit. Refinement discoveries do not open delivery attempts.
Historical costs and attempt membership are bounded by the report cutoff. Daily
legacy buckets overlapping an intraday cutoff remain incomplete. Historical policy
replay uses frozen eligible template definitions and the original versioned brief.
