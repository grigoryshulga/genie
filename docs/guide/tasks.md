---
title: Tasks and epics
summary: Move a request through clarification, implementation, review, and acceptance.
type: guide
status: current
---

# Tasks and epics

A task holds the request, acceptance criteria, plan, discussion, artifacts, and history. A team works on one task in its own workspace.

## Task lifecycle

```text
inbox → draft → refining → ready → in_progress → review → approved → done
                                     ↑             │
                                     └─ changes_requested
```

`needs_owner` pauses work for a human decision and remembers the previous status. `cancelled` ends a task without accepting its result.

| Stage | What must happen |
| --- | --- |
| Inbox and refinement | The orchestrator clarifies scope, constraints, dependencies, and acceptance criteria. |
| Ready | The orchestrator checks the Definition of Ready. Epics cannot enter this state. |
| In progress | An executor starts work once dependencies are closed. |
| Review | The executor submits evidence; a reviewer approves it or requests changes. A submitter cannot approve their own submission in the same round. |
| Done | Acceptance criteria are checked, the result is approved, and delivery gates pass. In assisted projects, a person closes the task. |

Server permissions enforce transitions and task scope. People can override task status; agent permissions remain constrained by their role.

## Human decisions

Assign a responsible person in the task. When an agent requests a decision, Genie notifies the assignee and author; without an assignee, it notifies the author and project administrators. Reply in the task or answer the questionnaire. Mention `@login` in a comment to notify a project member.

Project autonomy controls the orchestrator:

| Mode | Behavior |
| --- | --- |
| `assisted` | Agents prepare and deliver work; people close or cancel it. Recommended for a pilot. |
| `autonomous` | The orchestrator can also close and cancel tasks. Repository policies still apply. |
| `manual` | No server orchestrator runs. People direct the work. |

## Assessments and execution history

Open the task's **Task assessment and routing history** section to inspect the
assessment used at dispatch, selected team/models, required checks, evidence,
execution attempts and results. People can record confirmed repository-relative
paths and resolve runtime discoveries with a reason and evidence references.
Resuming the team remains a separate action; current mandatory reviewers are still
required. A safety pause cannot be cleared by changing task status alone.

Review returns and added members keep the same execution attempt and workspace.
Completion or cancellation ends the attempt; restarting execution creates another.
Old tasks retain explicitly missing history rather than reconstructed assessments.
See [Task Intelligence and Factory Analytics](../platform/task-intelligence.md) for
policy setup, reassessment through CLI/API, and measured results.

## Epics

Use an epic for a goal that needs several tasks. Record its success criteria, roadmap, and shared artifacts there. Child tasks receive the epic's goal and artifact list in their context.

Epics cannot contain other epics or receive a team directly. Starting a child task starts the epic. Closing the epic requires all children to be closed and its success criteria checked.

```bash
genie task create "Order exports" --type epic --project shop
genie task create "Export orders to CSV" --epic G-3 --project shop
```

## Delivery and limits

For [hosted repositories](../platform/git-repositories.md), agents push a task branch and open a PR/MR. Failed checks block submission to review; an open, unmerged request blocks completion. Human merge and assisted-mode closure are separate decisions.

`limits.maxActiveTeams` bounds concurrent teams per project; `maxActiveTeamsPerEpic` can bound one epic. Ready tasks wait when capacity is full, and the work loop dispatches them as room frees up, when a task closes or a team stops (see [the work loop](../platform/project-and-task-managers.md#work-loop)).

[Budgets](../platform/settings.md#usage-and-budgets) can stop teams at a task, epic, or daily limit.
