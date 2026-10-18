---
title: Decisions with Jev
summary: Configure Jev as an agent tool and a participant in team, model, and reasoning-effort selection.
type: guide
status: current
tags: [Jev, orchestration, models, tools, settings]
paths: [crates/genie/src/decisions/**, crates/genie/src/decisions.rs, crates/genie/src/http/decisions.rs, crates/genie/src/ops/decisions.rs, web/src/pages/settings/ui/JevSettings.tsx]
verified: 2026-10-17
---

# Decisions with Jev

Genie uses one Jev service for explicit agent consultations and selection of new delivery teams. External calls, the agent tool, and all influence modes are disabled by default. Existing teams keep their configured models.

## Enable participation

1. Configure the server connection using the examples below. The default TypeSafe connection uses a personal key at `/settings/me/keys` or a shared provider key. OpenRouter uses `keyProvider: "openrouter"` with its URL and model ID. Personal keys take precedence over shared keys; provider environment variables are used only on local installations without user accounts. Agents do not need the decision service's key.
2. Open the project's Jev decision settings. A project administrator must allow sending context to the configured decision service.
3. Choose independent influence modes for team composition, model, and reasoning effort. Add alternative team templates and verified `(model, thinking)` profiles with descriptions and allowed roles.
4. Optionally enable the agent tool and grant access to specific roles. The tool can operate with automatic selection disabled.
5. Start with observation or advice and evaluate recommendations on representative tasks before granting automatic authority.

A profile permits a specific model/effort pair for specific roles. Verify model access, tool support, and reasoning-effort compatibility for the initiators who will use it. Jev receives the profile description; it does not discover model capabilities from names. Without alternatives, the corresponding part of the proposal stays unchanged.

| Mode | Behavior |
| --- | --- |
| `off` | Skip the assessment. |
| `observe` | Record the assessment without sending advice to the manager or changing the proposal. |
| `advise` | Show the recommendation in a preview and send the spawn-time assessment to the task manager. Keep the proposed team unchanged. |
| `joint` | A confident disagreement blocks creation until the manager reconsiders or records a justified override. |
| `select` | Apply a confident choice within the administrator's approved alternatives. |

Models and reasoning effort are selected as compatible pairs. Each component needs the configured authority to change. If the model is fixed, only profiles for that model are eligible; effort settings are never transplanted from another model's profile. A person's explicit team creation preserves their composition, models, and effort. People can request a preview separately.

Team alternatives must be delivery templates with the same workspace type. They preserve executor and independent reviewer classes present in the proposal. The runtime rechecks role access, stages, member limits, active teams, and budget. Idea conversations and refinement teams do not use automatic selection.

## Agent tool

CLI and MCP share one operation. The task must already exist:

```sh
genie decision evaluate --request '{
  "task": "G-123",
  "purpose": "next-diagnostic-step",
  "state": {
    "goal": "Find the source of export latency",
    "facts": ["Most time is spent reading the database"],
    "constraints": ["Preserve the public API"]
  },
  "questions": {
    "next": {
      "type": "choice",
      "instructions": "Which check should run first for this task?",
      "criteria": {
        "query_plan": "Inspect the query plan and index usage",
        "repeated_reads": "Measure the number of repeated reads",
        "need_evidence": "Obtain a more precise latency profile first"
      }
    }
  }
}'
```

MCP tool `genie_decision` accepts `{"action":"evaluate","request":{...}}`, where `request` is the object above. Agents may omit `task` when their assignment supplies it; they must still provide context and facts explicitly. The HTTP boundary enforces role grants and task scope for both transports. Task managers can assess and preview teams only for their own task, which MCP infers automatically.

Supported questions are `choice` (2–255 named alternatives), `score` (2–10 ordered descriptions), and `bool` (descriptions for `true` and `false`). The public `bool` maps to System One's `noul`. A request accepts up to 32 independent questions evaluated against the same state. Dependent questions require another call.

The result includes a durable record ID, status (`ok`, `uncertain`, or `unavailable`), actual responding model, typed answers, probabilities, confidence, usage, cost, and latency. A choice's `qualified` field reports whether it passed the configured thresholds. Uncertain answers are retained without granting automatic authority. Invalid requests and forbidden access return API errors.

The tool does not execute the selected action or generate a free-text explanation. Agents supply their own explanation and test the hypothesis. Confidence describes certainty in the choice, not the probability of successful implementation.

## Preview a team

```sh
genie team recommend G-123 --template standard
genie team recommend G-123 --member executor --member reviewer
genie team spawn G-123 --member executor:provider/model:high --member reviewer
genie decision list --task G-123
genie decision show 42
genie decision policy
```

`recommend` returns a proposed plan, assessments, and notices. It creates no workspace, team, or agent session, but the assessment is a billable call counted against the task allowance. Observation preserves the proposal; advice lets the task manager compare alternatives before choosing a template or explicit members at spawn.

In `joint` mode, the manager either revises the proposal or justifies keeping it:

```sh
genie team spawn G-123 --template standard \
  --decision-override 'Acceptance requires a separate analyst to verify the source data format.'
```

The justification must contain 12–2000 characters and remains in the audit history. An override restores the entire original plan rather than mixing incompatible profile components. Agents may override only an actual `joint` disagreement; the flag cannot bypass pure `select` authority.

Identical successful or uncertain automatic assessments reuse recorded evidence. Failed assessments can be retried after fixing the connection; the original call remains in history and allowance accounting. An interrupted pending record permits another call after twice the timeout plus one second. Changes to task facts, alternatives, policy, or connection configuration produce a new cache key. Repeated creation of a task's active team does not create another team.

Automations use the same service: initial team creation paths converge on `spawn_team`. A `joint` disagreement can fail an automation step; use `advise` or `select` if reconsideration cannot block it. Jev modes do not change project autonomy, result acceptance, or merge rights.

## Server configuration and storage

Project settings and recent assessments are available in the web UI. The project-scoped API exposes:

- `GET/PUT /api/decisions/policy`: read policy; only human project administrators may save it.
- `POST /api/decisions/evaluate`: requires the enabled tool, an allowed role, task scope, and write access.
- `GET /api/decisions?task=G-123` and `GET /api/decisions/42`: history. Agents must specify their task when listing, and individual reads also enforce scope.
- `POST /api/teams/recommend`: preview using the same input fields as team creation; available to the task manager or a person.

The web UI shows the latest 100 project records. Records survive team shutdown and remain available for result analysis.

The optional `decisions` section of `<data>/config.json` configures the server:

```json
{
  "decisions": {
    "enabled": true,
    "baseUrl": "https://api.typesafe.ai/v1/",
    "keyProvider": "typesafe",
    "model": "jev-1.13.0",
    "timeoutMs": 5000,
    "maxRequestBytes": 32000,
    "inputPrice": 0.042,
    "defaults": {
      "allowExternal": false,
      "toolEnabled": false,
      "team": "off",
      "model": "off",
      "thinking": "off"
    }
  }
}
```

For **OpenRouter**, use:

```json
{
  "decisions": {
    "endpoint": "https://openrouter.ai/api/v1/systemone",
    "keyProvider": "openrouter",
    "model": "typesafe/jev-1.13",
    "inputPrice": 0.042
  }
}
```

Add the key through Genie's personal or shared OpenRouter keys. A userless local installation can use `OPENROUTER_API_KEY`. The endpoint and request format follow [OpenRouter's official documentation](https://openrouter.ai/docs/api/api-reference/systemone/submit-a-system-one-request). Set model IDs and prices according to your provider contract.

For a **custom gateway or another compatible service**, supply its full URL and a server environment variable containing the key:

```json
{
  "decisions": {
    "endpoint": "https://decisions.example.com/custom/systemone",
    "keyProvider": "private-decisions",
    "apiKeyEnv": "GENIE_DECISIONS_API_KEY",
    "model": "jev",
    "inputPrice": 0.042
  }
}
```

Set `GENIE_DECISIONS_API_KEY` in the Genie server process environment. `apiKeyEnv` names the variable rather than containing the key. This explicit service credential works in multi-user mode, takes precedence over personal/shared keys, and does not fall back to another key if missing. Without `apiKeyEnv`, `keyProvider` selects the personal/shared secret namespace; known providers are available in Genie's key settings. It also supplies the model prefix used in usage accounting.

`endpoint` takes precedence over the existing `baseUrl`; with only `baseUrl`, Genie appends `systemone`. Restart the server after changing its configuration. Supported services implement **System One**: Bearer authorization, `{model, state, questions}` requests, and `{model, answers, usage}` responses. An OpenAI Chat Completions endpoint or a different response schema requires an adapter. Existing TypeSafe configuration keeps its behavior. The web UI shows the URL, model, and credential source without exposing the key.

Decision history and project policy tables raise `server.db` to schema version 6. Migration adds tables without deleting existing data. Older binaries refuse to open the new schema; binary rollback requires a compatible backup.

`enabled: false` disables calls for all projects. `defaults` applies until the first saved project policy; saved policies live in `server.db`. Timeout is clamped to 100–30000 ms and request size to at most 128000 bytes. Agents cannot configure the endpoint. HTTPS is required except for loopback HTTP. URL credentials, query parameters, and fragments are rejected; keys travel only in the Authorization header. Redirects are disabled and responses are limited to 256 KiB.

The default allowance is 40 calls and $0.10 per task, with minimum confidence 0.7 and margin 0.2. Before sending, Genie reserves one call and a conservative cost based on request size. A lost response retains the reservation because it may have been billed. Provider requests are not automatically retried. Reported input usage is priced using the configured service rate, including billed malformed responses; tokens also enter task usage and budget accounting. Decision history displays costs with greater precision than summary statistics.

Uncertainty, timeout, provider failure, a missing key, or exhausted allowance preserves the manager's proposal. Changes to facts or configuration during evaluation reject creation so it can be retried against current facts. Network calls hold no SQLite lock. Existing manual operations for modifying active teams and member models do not trigger automatic reassessment.

## Verification

`cargo test -p genie --test it decisions::` exercises the real HTTP boundary against a local fake: wire format, CLI/MCP, grants, task scope, cost, failures, uncertainty, selection modes, previews, overrides, stale facts, and concurrent creation. Tests cover TypeSafe, OpenRouter, a custom endpoint, and task-manager scope without real provider keys or paid calls.

Evaluate decision quality with comparable task runs with and without Jev: acceptance quality, review returns, elapsed time, and cost per accepted task. Integration tests establish mechanism correctness rather than recommendation quality. API references: [TypeSafe models](https://docs.typesafe.ai/models), [choice](https://docs.typesafe.ai/primitives/choice), and [confidence](https://docs.typesafe.ai/confidence).

## Task Intelligence

Optional [Task Intelligence and Factory Analytics](task-intelligence.md) adds versioned signals, mandatory reviewer checks, runtime reassessment, attempt-aware telemetry, and candidate-policy evaluation. Its safety constraints apply independently of Jev selection authority.
