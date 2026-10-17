---
title: Roles and teams
summary: Configure agent models, permissions, skills, MCP connections, and team handoffs.
type: reference
status: current
---

# Roles and teams

A **role** defines an agent's prompt, model, skills, permissions, and tool access. A **team template** composes roles and their handoffs. A template cannot expand a role's permissions.

Server administrators edit these files directly or in **Settings → Agents**:

```text
<data>/agents/<id>.md       Role overrides and custom roles
<data>/teams/<id>.json      Team templates
<data>/skills/<id>/SKILL.md Skills
<data>/mcp.json             MCP connections
```

Built-ins ship with the binary. A role file with a built-in ID overrides its fields; a nonempty body replaces its prompt, while `instructions` appends to it. A team file replaces the whole template. Deleting an override restores the built-in.

Valid changes are picked up without restarting the server. Invalid edits leave the last valid configuration active and appear in `genie agents check`. Web edits have history and conflict detection.

## Project and task managers

`project-manager` owns project context, intake, and dispatch. `orchestrator` is the manager of one task, including its separate mandatory retrospective. Both use the existing orchestrator process class and default to reading files. See [management lifecycle](project-and-task-managers.md) for scope, clarification routing, and migration.

Until explicitly configured, `project-manager` inherits the configured `orchestrator` model. Existing orchestrator settings apply to task managers. The legacy `planner` role remains the project manager conversation mode, preserving its model and plan format while using shared project memory.

## Role file

```markdown
---
title: Security reviewer
description: Review authentication and authorization changes.
extends: reviewer
model: provider/model-id
thinking: high
files: read
git: read
skills: [security-review]
mcp: ["github:get_*", "github:list_*"]
projects: [shop]
instructions: Check the security-review skill before issuing a verdict.
---
```

Replace `provider/model-id` with an available pi model. Every runnable role requires a model, either inherited or explicit. A template member or individual launch can override it.

| Field | Meaning |
| --- | --- |
| `title`, `description` | Display name and when the orchestrator should choose the role. |
| `base` | Process class for a new role: `analyst`, `executor`, `reviewer`, `tester`, or `documenter`. Required unless inherited. |
| `extends` | Role to inherit from. |
| `model`, `thinking` | Default model and reasoning level. |
| `names` | Agent name pool. |
| `allow`, `deny` | Add or remove permissions from the class defaults. |
| `files` | `write`, `read`, or `none`. |
| `git` | Maximum Git access: `none`, `read`, or `write`; can only narrow file access. |
| `denyCommands` | Shell command patterns blocked by the harness guard. |
| `skills` | Skill IDs. Explain when to use them in the prompt. |
| `mcp` | Connection IDs, optionally with a tool pattern: `server:get_*`. |
| `stages` | `refinement`, `delivery`, or both. |
| `projects` | Project allowlist; omitted means all projects. |
| `instructions` | Additional prompt text. |

Human project roles (`viewer`, `member`, `admin`, `owner`) are separate from agent roles. The orchestrator is also a separate process class.

## Permissions

| Area | Permissions |
| --- | --- |
| Status | `status.refine`, `status.start`, `status.rework`, `status.submit`, `status.approve`, `status.return` |
| Task | `task.scope`, `task.plan`, `task.check`, `task.create`, `task.block` |
| Knowledge | `docs.read`, `docs.write` |
| Communication | `mail.team`, `mail.orchestrator`, `team.peek` |

All team roles can report through comments, artifacts, notes, labels, and their status line. Only people and the orchestrator control `draft`, `ready`, `needs_owner`, `done`, and `cancelled`; those transitions cannot be granted to a team role. Agents remain scoped to their task and children, with comments and artifacts allowed on their epic. A submitter cannot approve its own work in the same review round.

Server checks on tasks, knowledge, mail, and team operations use the agent's token and current role configuration. Revoking those permissions affects running agents immediately.

`files`, command restrictions, and skill selection rely on the harness. A role with shell access can write files through shell commands even when edit/write tools are disabled. Bubblewrap limits the directories it can reach; it does not turn `files: read` into a read-only worktree.

## Team template

```json
{
  "title": "Pair",
  "description": "Implementation and independent review for a well-defined task.",
  "stage": "delivery",
  "workspace": "worktree",
  "members": [{ "role": "executor" }, { "role": "reviewer" }],
  "relations": [
    { "from": "executor", "to": "reviewer", "type": "handoff", "on": "review" },
    { "from": "reviewer", "to": "executor", "type": "returns", "on": "changes_requested" },
    { "from": "reviewer", "to": "orchestrator", "type": "reports" }
  ]
}
```

`stage` selects refinement or delivery. `workspace` controls the working directory. Relations tell agents who receives work, review findings, and reports; the task state machine still governs transitions.

Built-ins include `standard`, `pair`, `full`, `abap`, `spike`, `research`, and `idea`. Inspect the current template before choosing one. Custom IDs use lowercase letters, digits, and hyphens, starting with a letter. `projects` can restrict a role, template, or connection to selected projects.

## Skills

Skills follow the [Agent Skills format](https://agentskills.io/specification): a directory containing `SKILL.md` and optional supporting files. Genie loads skills from `<data>/skills/` and `skills.paths` in `config.json`.

A role lists its skills by ID. Repository skills in `.agents/skills` and `.pi/skills` are also supplied. Genie passes them explicitly to pi rather than loading every skill installed on the service account.

## MCP connections

`<data>/mcp.json` uses the `mcpServers` format. A role's `mcp` list selects connections and tool patterns. Environment references such as `${env:NAME}` resolve on the server.

The default gateway keeps upstream secrets on the server and rechecks the agent token and role grant on each request. Calls are audited. Connections with `gateway: false` pass configuration to the harness instead; their secrets and access restrictions have weaker isolation.

Guard changes affect the next tool call. Skill changes and the set of MCP connections take effect on the next session start.

## Running teams

Use the team screen to add or remove members, pause, resume, restart, or change an individual's model. Model changes apply after the current step and preserve the conversation. Stopping a team preserves its result workspace; the server can clean up an integrated workspace when it is no longer needed.

```bash
genie agents check
genie team board
genie team peek <member> --deep
```

See [agent runtime](agent-bus.md) for delivery, interruption, and recovery.
