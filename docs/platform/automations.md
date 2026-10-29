---
title: Automations
summary: Trigger durable workflows from events, schedules, webhooks, or manual runs.
type: reference
status: current
---

# Automations

An automation is a JSON rule with a trigger, limits, and ordered steps. Edit rules and install playbooks in **Settings → Automations**; inspect runs in the working area's Automations screen.

```bash
genie automation create --file rule.json
genie automation install task-done-knowledge
genie automation runs
genie automation run-show <run>
```

## Example

Notify the author when a task closes:

```json
{
  "name": "Task completed",
  "on": { "event": "task.status_changed", "where": { "to": "done" } },
  "limits": { "concurrency": 2, "maxRunsPerHour": 20 },
  "steps": [
    {
      "id": "tell",
      "notify": {
        "to": ["task.author"],
        "title": "{{ event.task.id }} completed",
        "text": "Review the result in Genie."
      }
    }
  ]
}
```

## Triggers

| Trigger | Example | Template context |
| --- | --- | --- |
| Event | `{"event":"task.status_changed","where":{"to":"done"}}` | Event fields, `task`, `actor`, `event` |
| Schedule | `{"schedule":"0 9 * * 1-5","tz":"Europe/London"}` | `schedule.at` |
| Manual | `{"manual":true}` | `inputs`, `task` |
| Webhook | `{"webhook":{"secret":"at-least-16-characters"}}` | `payload` |

Events cover task changes, teams, mail, documents, proposals, releases, repository/CI activity, and [observation](observation.md). `where` supports dotted paths, equality, a list of alternatives, and operators `not`, `exists`, `contains`, `not_contains`, `gt`, `lt`, and `prefix`.

Observation events are `outcome.created`, `outcome.updated`, `observation.started`, `deployment.completed`, `evidence.collected`, `guardrail.violated`, `incident.created`, `incident.resolved`, `incident.dismissed`, `outcome.ready_for_evaluation`, `outcome.evaluated`, `outcome.confirmed`, `outcome.negative`, `outcome.inconclusive`, `outcome.cancelled`, and `followup_task.created`. Their subject is a case (`observation:<id>`), not a task, so templates have no `task` context; use the payload fields instead, and read a payload `task` (such as the one `followup_task.created` carries) under `event.payload`.

Cron accepts five fields, or six with seconds. Run a manual rule with `genie automation run <id> --task G-7 --input key=value`. Webhooks use `POST /api/hooks/<id>?token=<secret>`.

## Steps

Common step fields are `id`, `if`, `retry`, `timeout` (such as `30s` or `45m`), and `onError` (`fail` or `continue`). Templates use `{{ path | filter }}`; filters include `length`, `json`, `lines`, and `upper`. A string consisting only of a substitution retains the substituted value's type. Step results are available as `steps.<id>.output`.

| Step | Purpose |
| --- | --- |
| `task.get`, `task.create`, `task.update` | Read or change a task. Unknown update fields fail. |
| `task.status`, `task.comment` | Change status or add a comment, respecting workflow rules. |
| `task.ready` | Move eligible tasks and newly unblocked dependents to ready (the work loop does this in every non-manual project). |
| `notify` | Deliver web, Telegram, or email notifications. |
| `agent` | Run a one-shot role with a goal, inputs, output contract, workspace, and optional model. |
| `team` | Spawn a team and optionally wait for task status. |
| `ask` | Ask people questions, with reminders, deadline, and timeout behavior. |
| `wait` | Persist a timed delay. |
| `wake_orchestrator` | Send the orchestrator a message. |
| `changelog.add`, `release` | Update project release notes and publish a version. |
| `http` | Make an HTTP request and return its status and body. |

Notification recipients include `event.actor`, `task.author`, `task.assignees`, `project.owners`, `project.admins`, `project.members`, and `@login`.

An `agent` job must submit `genie job output`; missing output fails the attempt. `workspace: worktree` creates a result branch reused by retries. `read-only` disables file-writing tools in the project repository; `scratch` and `none` use an empty job directory. Harness file restrictions have the limits described in [roles and teams](agent-roles-and-teams.md#permissions).

## Durability and limits

Runs are unique per rule and trigger key. Each run stores its rule snapshot and step state, so editing a rule does not rewrite an active run. Jobs, questions, team waits, and delays survive restarts.

A rule does not retrigger itself. Rule cascades stop at depth three. `concurrency`, `maxRunsPerHour`, and `dryRun` bound rule activity. `dryRun` records a run without executing its actions. Cancellation also cancels its jobs:

```bash
genie automation cancel <run>
```

Task status steps use orchestrator permissions, including assisted-mode restrictions and repository delivery gates. External HTTP effects should be idempotent when retries are enabled.

## Playbooks

| Playbook | Behavior |
| --- | --- |
| `task-done-knowledge` | Task completion → documenter → changelog entry → author/admin notification. |
| `needs-owner-escalation` | Remind the author and owners after 24 hours if a task still needs a decision. |

Readiness, the project manager's preparation prompts and dispatch of ready tasks are not playbooks: the server does them itself, in every project except a `manual` one. See [project and task managers](project-and-task-managers.md#work-loop).

The former playbooks `new-task-triage`, `auto-ready`, `ready-start` and `ready-next` were replaced by that work loop. The server switches off installed copies of these playbooks when it starts; later starts change nothing. Rules you wrote keep working, including their `task.ready` and `wake_orchestrator` steps.

Built-in assignment, mention, owner-question, and proposal notifications work without custom rules.
