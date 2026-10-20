---
title: Observation plane proposal
summary: Original proposal for an independent post-delivery outcome and runtime feedback loop.
type: note
status: draft
---

> The implementation design derived from this proposal is in [Observation plane: Phase 1 specification](observation-plane.md).

# Genie Observation Plane
## Proposal for an Independent Outcome and Runtime Feedback Loop

## 1. Problem

Genie currently focuses on producing and delivering software changes:

`intent -> task -> implementation -> verification -> delivery`

A task can be completed successfully while the delivered change fails in production, produces no measurable benefit, or causes a regression that only appears after deployment.

Extending the existing task lifecycle with long-running observation states would make task management unnecessarily complex. Not every task requires post-delivery observation, and different outcomes require different timing and data sources.

The proposed solution is a separate **Observation Plane** that consumes delivery events and evaluates what happened after a change reached the real environment.

---

## 2. Core Principle

Separate two responsibilities:

### Delivery Plane

Responsible for producing an accepted change.

`intent -> task -> implementation -> tests -> review -> merge/deploy -> delivered`

### Observation Plane

Responsible for evaluating the real-world consequences of delivered changes.

`delivered change -> observation -> evidence -> evaluation -> conclusion -> learning / incident / new task`

The two planes are independent.

A delivery task may reach `Done` while its related observation remains active for hours or days.

Observation is optional and policy-driven.

---

## 3. New First-Class Entity: Outcome Case

Introduce an `OutcomeCase` entity.

An Outcome Case represents post-delivery observation of one or more delivered changes.

Example:

```yaml
id: O-37
title: Checkout latency rollout

source_tasks:
  - G-140
  - G-141
  - G-142

status: observing
policy: outcome

created_at: 2026-10-18T12:00:00Z
next_evaluation_at: 2026-10-21T12:00:00Z
```

Relationships should support:

- one Task -> zero or many Outcome Cases;
- one Outcome Case -> one or many Tasks;
- one deployment/release -> one or many Outcome Cases.

This allows observation to follow the real deployment unit rather than the implementation task structure.

---

## 4. Outcome Contract

When observation is needed, the Delivery Plane should produce an immutable or versioned `OutcomeContract`.

The contract defines expectations before results are known.

Example:

```yaml
outcome:
  hypothesis:
    checkout_latency_p95:
      expected_change: "-20%"

  guardrails:
    error_rate:
      max_change: "+0.1pp"

    conversion:
      min_change: "-0.2%"

  observation:
    minimum_window: 24h
    preferred_window: 72h
    minimum_samples: 10000

  sources:
    - prometheus.checkout_latency
    - sentry.checkout_errors
    - posthog.checkout_conversion
```

The contract should be created before post-delivery evidence is evaluated.

This prevents retrospective rewriting of expectations.

---

## 5. Observation Policies

Observation should not be mandatory for every task.

Introduce a policy field:

```text
none
guardrails
verify
outcome
experiment
```

### `none`

No post-delivery observation.

Typical use:

- documentation changes;
- formatting;
- internal cleanup without observable runtime impact.

### `guardrails`

Watch only for regressions or incidents.

Typical use:

- dependency updates;
- migrations;
- infrastructure changes;
- risky refactors.

### `verify`

Verify a predefined technical result after deployment.

Typical use:

- latency reduction;
- memory reduction;
- queue processing throughput;
- database load reduction.

### `outcome`

Evaluate a product or business outcome.

Typical use:

- onboarding changes;
- checkout redesign;
- search improvements.

### `experiment`

Evaluate a controlled experiment.

Typical use:

- A/B tests;
- recommendation changes;
- ranking changes;
- pricing experiments.

The Project Manager may suggest a policy during planning, but the policy must remain explicitly visible and overridable.

---

## 6. Two Observation Mechanisms

The Observation Plane needs two independent execution modes.

## 6.1 Guardrail Watch

Event-driven and near-real-time.

Purpose: react immediately to abnormal signals.

Examples:

- crash spike;
- error-rate increase;
- security alert;
- severe latency regression;
- failed health checks.

Flow:

```text
deployment
    |
    +-- normal --------------------------> continue observation
    |
    +-- guardrail violation
              |
              v
           incident
              |
              v
       investigation team
              |
              v
          new task
```

A guardrail violation should be able to create an Incident immediately without waiting for the normal evaluation window.

---

## 6.2 Outcome Evaluation

Time- or sample-driven.

Purpose: determine whether the intended result was achieved.

Example state flow:

```text
proposed
   |
waiting
   |
observing
   |
   +-- insufficient evidence --> observing
   |
evaluating
   |
   +-- confirmed
   +-- negative
   +-- inconclusive
```

`inconclusive` must be a first-class result.

The system must not force a success/failure judgment when evidence is insufficient or attribution is unclear.

Evaluation triggers may include:

```text
time >= observation_window
OR samples >= minimum_samples
OR explicit external event
OR manual evaluation request
```

No agent process should remain alive while waiting.

The scheduler should persist the Outcome Case and wake evaluation when a trigger becomes true.

---

## 7. Evidence Sources

Agents should not manually inspect dashboards whenever structured data is available.

Introduce a standard adapter layer:

```text
Observation Plane
      |
      +-- Metrics Adapter
      +-- Logs Adapter
      +-- Errors Adapter
      +-- Analytics Adapter
      +-- Tracing Adapter
      +-- Deployment Adapter
```

Potential integrations:

- Prometheus
- Grafana
- Datadog
- Sentry
- Loki
- OpenTelemetry
- PostHog
- Amplitude
- custom HTTP/SQL sources

Suggested adapter interface:

```text
query()
snapshot()
compare()
subscribe()
```

Adapters should return structured evidence.

Example:

```yaml
metric: checkout_error_rate

baseline:
  value: 0.14
  window: 7d

observed:
  value: 0.19
  window: 72h

delta:
  absolute_pp: 0.05

samples: 187421
```

LLMs should interpret evidence, not extract numbers from screenshots.

---

## 8. Observation Team

The Observation Plane should use a different role model from the Delivery Plane.

Suggested roles:

### Outcome Manager

Owns the Outcome Case.

Responsibilities:

- determine required evidence;
- decide whether evidence is sufficient;
- coordinate investigations;
- evaluate outcome contracts;
- decide whether to continue observing;
- escalate uncertainty;
- create incidents or follow-up tasks;
- publish final conclusions.

### Evidence Collector

Preferably implemented mostly as deterministic infrastructure rather than an LLM.

Responsibilities:

- execute source queries;
- collect snapshots;
- normalize evidence;
- persist provenance.

### Analyst

Responsibilities:

- compare expected and observed results;
- identify trends;
- assess confidence;
- detect confounding factors;
- produce structured conclusions.

### Incident Investigator

Spawned when a guardrail is violated.

Responsibilities may include:

- log analysis;
- trace analysis;
- deployment correlation;
- Git history inspection;
- rollback verification.

Temporary specialist agents may be created as needed.

---

## 9. Incident Feedback Loop

Observation must be able to create new work in the Delivery Plane.

Example:

```text
G-142 delivered
      |
      v
O-37 observes checkout
      |
      v
error spike detected
      |
      v
I-17 incident
      |
      v
investigation
      |
      v
G-149 "Fix checkout regression"
      |
      v
Delivery Plane
```

This creates a real closed engineering loop rather than a one-way task pipeline.

---

## 10. Attribution

Outcome evaluation must distinguish correlation from causation.

An Outcome Case should track potentially related changes and confidence.

Example:

```yaml
suspected_changes:
  - task: G-142
    confidence: 0.87

  - task: G-138
    confidence: 0.14
```

Evidence may include:

- deployment timing;
- changed components;
- affected traces;
- rollback results;
- experiment assignment;
- dependency graph;
- release composition.

The system must support the conclusion:

`Attribution cannot be determined reliably.`

---

## 11. Learning and Project Memory

Completed Outcome Cases should produce structured learning.

Possible outputs:

- confirmed hypothesis;
- rejected hypothesis;
- unexpected regression;
- operational constraint;
- newly discovered invariant;
- new monitoring requirement;
- changed observation policy.

Example:

```yaml
learning:
  type: operational_constraint

  statement:
    "Checkout migrations require a 24h error-rate guardrail."

  evidence:
    - O-37
    - I-17

  confidence: high
```

This may be proposed for Project Memory using the existing provenance and review model.

---

## 12. Factory Self-Observation

The Observation Plane should also evaluate Genie itself.

Suggested system-level metrics:

```text
first-pass acceptance rate
escaped defect rate
rollback rate
production regression rate
human interventions per task
median cost per accepted task
median review rounds
incident rate by task category
outcome success rate
```

This enables retrospectives to optimize not only prompts and roles, but the structure of the Delivery Plane itself.

Example:

```text
Database migrations
current strategy:
  executor + reviewer

observed:
  14% incident rate

candidate strategy:
  planner + executor + adversarial reviewer + guardrail watch
```

Organizational structure becomes an experimentally optimizable variable.

---

## 13. Event Model

The planes should communicate through durable events rather than shared runtime state.

Example events:

```text
task.delivered
deployment.started
deployment.completed
outcome.created
observation.started
guardrail.violated
evidence.collected
outcome.ready_for_evaluation
outcome.confirmed
outcome.negative
outcome.inconclusive
incident.created
followup_task.created
```

This keeps both systems loosely coupled and independently recoverable.

---

## 14. Minimal Data Model

### OutcomeCase

```yaml
id:
title:
status:
policy:

source_tasks: []
deployments: []

contract_id:

created_at:
next_evaluation_at:

observation_started_at:
observation_completed_at:

result:
confidence:

evidence_refs: []
incident_refs: []
followup_task_refs: []
```

### OutcomeContract

```yaml
id:
version:

hypotheses: []
guardrails: []

minimum_window:
preferred_window:
minimum_samples:

sources: []

evaluation_rules: []
```

### EvidenceRecord

```yaml
id:
source:
query:
timestamp:

value:
baseline:
delta:

sample_size:

provenance:
trust_level:
```

---

## 15. MVP

The first version should avoid product analytics, experiments, and complex causal inference.

### Phase 1

Implement:

- `OutcomeCase`;
- `OutcomeContract`;
- `none / guardrails / verify` policies;
- scheduled observation;
- Prometheus-compatible metrics source;
- Sentry-compatible error source;
- structured Evidence Records;
- guardrail violations;
- manual or automatic follow-up task creation;
- `confirmed / negative / inconclusive` results.

This is enough to validate the architecture.

### Phase 2

Add:

- deployment provider integrations;
- tracing/log adapters;
- automatic incident investigation;
- project memory updates;
- configurable observation policies by task class.

### Phase 3

Add:

- product analytics;
- experiments;
- attribution analysis;
- organizational optimization;
- factory self-observation.

---

## 16. Non-Goals

The first implementation should not attempt to:

- keep agents alive while waiting;
- force every task into post-delivery observation;
- replace existing observability systems;
- infer causality from weak evidence;
- automatically mutate project policy without review;
- merge the Delivery and Observation lifecycles into one state machine.

---

## 17. Design Principle

The Delivery Plane changes the system.

The Observation Plane determines what the system did in response.

Neither plane should be responsible for validating itself.

The architectural target is:

```text
             +---------------------------+
             |                           |
             v                           |
        Delivery Plane                   |
             |                           |
             v                           |
          Release                        |
             |                           |
             v                           |
       Observation Plane                 |
          |          |                   |
          |          +--> Incident ------+
          |
          +--> Outcome Learning
          |
          +--> Factory Learning
                    |
                    v
              Process Change
                    |
                    +--------------------> Delivery Plane
```

This turns Genie from a task-to-change system into a closed-loop software engineering system.
