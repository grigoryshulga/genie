---
title: Task Intelligence and Factory Analytics
summary: Inspect immutable task assessments, enforce routing checks, and evaluate policies using measured execution evidence.
type: guide
status: current
tags: [Jev, routing, analytics, telemetry]
paths: [crates/genie-core/src/intelligence.rs, crates/genie/src/intelligence.rs, crates/genie/src/factory.rs]
---

# Task Intelligence and Factory Analytics

Delivery dispatch records an immutable signal snapshot, routing plan, and execution
attempt in the project's tracker. This instrumentation does not require a provider
call. The existing Jev provider call history remains in `server.db`; routing records
reference those calls rather than copying their requests.

An attempt begins on delivery dispatch or entry into `in_progress`. Review returns,
member changes, restarts and topology upgrades retain it. `done` or `cancelled`
finalizes it; a later execution starts a new attempt. Old tasks have no invented
attempts or risk classifications.

## Project policy

In project Jev settings, enable Task Intelligence and define a version, independent
checks, protected component path prefixes, a fallback template, optional semantic
assessment and a reassessment cooldown. Only a human project administrator can save
policy. A check names an available delivery reviewer role. Protected components must
have checks; fallback templates must satisfy all protected checks. Custom templates
remain supported. `solo` is the built-in single-executor delivery template.

Unknown affected scope includes all protected components. Jev labels cannot remove
these protections. An explicit human team choice is still checked. A person may
record repository-relative affected paths; agents may expand known scope during
reassessment, but cannot turn unknown scope into a claim that protections do not apply.

Semantic assessment asks five independent choice questions about ambiguity, scope,
complexity, verification and novelty. It uses the existing external-context grant,
call/cost allowance, uncertainty thresholds and provider timeout. A bounded read-only
manifest inventory inspects at most 256 entries, two directory levels and 200 ms,
skips symlinks and common generated directories, and caches facts for 60 seconds.
Inventory never establishes security policy or test success. Missing information stays
unknown. Classification confidence is not a probability of successful implementation.

Topology routing and compatible role model/effort routing retain existing Jev modes.
A timeout or abstention in automatic topology selection uses a compatible configured
fallback, or preserves the safe proposal. The applied plan records its fallback
status, required checks, policy fingerprint and responding call IDs. Removing the
last required reviewer is refused.

## Inspect and reassess

```sh
genie intelligence history --task G-123
genie intelligence assess --task G-123 --affected-path src/auth/login.rs
genie intelligence reassess --task G-123 --request '{
  "requestId": "scope-discovery-1",
  "reason": "scope_expansion",
  "evidenceRefs": ["file:src/auth/login.rs"],
  "affectedPaths": ["src/auth/login.rs"]
}'
```

The same operations are available through `genie_intelligence` MCP and
`/api/tasks/{id}/intelligence` project-scoped HTTP routes. Task detail shows the
brief/snapshot/routing chain, models, evidence references, attempts and results.
Agents must stay within their task assignment. Reassessment belongs to the task
manager or a person; shadow evaluation requires a human project administrator.

Material reasons are `sensitive_change`, `scope_expansion`, `irreversible_change`,
`verification_gap`, `review_return`, and `requirement_conflict`. Reuse the request ID
when retrying. Required reviewers join the existing workspace; the task's artifacts,
conversation and attempt survive. Safety upgrades may bypass cooldown; routine
reassessment is debounced. The server does not automatically downgrade a running team.
Unresolved sensitive changes, missing recovery checks, or conflicting requirements
pause members and request an owner decision. Resolve the policy/brief and explicitly
resume members through the existing team controls.

## Statistics

Open **Statistics → Factory**. Each project shows its time window, task cohort,
observed denominators and missingness. Select a topology, model, role, initial signal,
policy/router version, or reason cohort. Comparable routing cohorts include initial
complexity, ambiguity, blast radius and verification. Open supporting tasks to inspect
attempts and the decision timeline. Server-wide statistics keep their existing
server-administrator permission requirement.

The cohort contains the last observed terminal delivery of each task within the
window. First-pass acceptance excludes tasks without a completed first review.
Execution duration starts at recorded execution rather than task creation; the old
creation-to-done metric remains available separately. Active and waiting durations
use known status intervals, not sums of overlapping session durations. Human
acceptance alone is not counted as an intervention. Escalation uses attempts as its
denominator; attempt-telemetry coverage is reported separately.

Accepted-work cost includes attributable unsuccessful attempts, reviews and retries,
not just the final successful turn. Existing daily usage remains authoritative.
New calls add attribution to attempt, stage, role and model transactionally with
accounting; Factory reconciles coverage to daily totals. Old usage cannot be split
losslessly and remains unmeasured. Unpriced tokens prevent a partial amount being
labelled complete. No production regression rate is inferred without monitoring.
Small cohorts are flagged; observational comparisons are not causal team rankings.

## Candidate policies and retrospectives

```sh
genie intelligence shadow --task G-123 --request '{
  "snapshotId": 51,
  "candidate": {
    "templates": ["solo", "pair"],
    "intelligence": {"enabled": true, "version": "candidate-2"}
  }
}'
```

Unspecified candidate fields use the shared policy defaults. Shadow evaluation freezes
the existing snapshot, candidate, eligible templates and baseline references. It uses
the current external-call grant and allowance, saves uncertainty/unavailability, and
starts no teams or sessions. It changes no task status or production policy. It does
not predict unobserved quality or hypothetical cost savings.

Manager source inventories include routing history and evidence-backed proposals.
Published retrospectives retain those proposals and their limitations. A human
administrator must evaluate and explicitly save any production policy change. Policy
fingerprints and immutable records preserve earlier decisions when configuration is
rolled back. Controlled live experiments and the future Observation Plane are outside
this feature.

## Storage and rollback

Tracker schema 7 adds the intelligence/attempt/evaluation tables and new usage
attribution. Migration preserves existing task, event and usage rows. It creates no
historical assessments. Startup rehearses upgrades using the existing migration
mechanism; older binaries refuse schema 7. Back up data before upgrading; binary
rollback requires a compatible data backup. Analytics uses indexed source reads, so
rebuilding means reading the same project records again, without a second event log
or a cross-database transaction.

See the [implementation specification](../development/task-intelligence-spec.md) for
contracts and acceptance tests.
