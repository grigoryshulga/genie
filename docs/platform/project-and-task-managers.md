---
title: Project and task managers
summary: Project intake, task execution, clarification routing, and mandatory retrospectives.
type: reference
status: current
---

# Project and task managers

The **project manager** keeps shared project context, helps people turn goals into requirements, and dispatches prepared tasks within project autonomy. A **task manager** (orchestrator) owns one task from dispatch through execution, acceptance, and its mandatory retrospective. Managers delegate implementation to a team.

Result acceptance and retrospective completion are independent. Accepted results unblock dependent work immediately under the existing workflow; the retrospective must still finish separately.

## Responsibilities

| Project manager | Task manager |
| --- | --- |
| Discuss goals, requirements, constraints, and acceptance criteria with people. | Check the received brief and request missing decisions. |
| Maintain source-backed shared memory and links to relevant documentation. | Choose roles, models, skills, and a suitable team; delegate work. |
| Prepare the queue, check dependencies and blockers, and authorize dispatch. | Monitor progress, review, failures, retries, waiting, and budget. |
| Resolve questions using confirmed decisions or escalate to a person. | Decide within the brief; send scope or authority questions to the project manager. |
| Coordinate priorities and conflicts between tasks. | Verify results against acceptance criteria and review/test evidence. |
| Receive retrospective findings and retain applicable project knowledge. | Publish the result and a separate evidence-based retrospective. |

Shared context lives in persistent project memory rather than one ever-growing model context window. Memory entries require sources, can supersede earlier entries, and remain available across conversations. Personal preferences become project requirements only through an explicit applicable decision.

## Dispatch and versioned briefs

Before dispatch, prepare the task goal, description, scope and constraints, acceptance criteria, dependencies, relevant sources, and responsible person. Record autonomy and integration expectations; tasks without code still need a defined artifact handoff.

Dispatch requires an open, ready, non-epic task without unresolved blockers, owner decisions, or unfinished dependencies. The server checks readiness and creates one durable task manager atomically. Repeating dispatch returns the existing active manager. Execution capacity and budget limits still apply when agents actually run.

The brief snapshots task requirements, acceptance text, dependencies, parent, integration instructions, and assignee. Later requirement changes create a new version, notify the manager, and make pending questions about the previous version obsolete. A conversation provides context but does not replace recorded requirements.

Project autonomy governs dispatch. In `manual`, the project manager does not run or dispatch automatically; a person can explicitly dispatch a prepared task. Previously dispatched tasks and their retrospectives continue. Existing `assisted` acceptance, CI, repository, and merge policies remain in force.

Task managers cannot create unrelated tasks or rewrite requirements, priorities, assignees, dependencies, acceptance criteria, or integration policy. Their team, repository, job, mail, pause, and inspection operations are scoped to their own task. Changes outside that scope go to the project manager.

## Clarifications

The route is **team member → task manager → project manager → person**. A durable question records its task, brief version, blocking flag, recipient state, answer, and source. The project manager answers only with an applicable confirmed decision, current documentation, or delegated authority. Model confidence alone is insufficient. Conflicting sources, missing requirements, and decisions outside authority go to a person.

For example, a confirmed retention policy of 30 days permits a source-backed answer. An unspecified retention period requires a human decision with the available options and consequences.

Waiting for the project manager differs from waiting for a person: `needs_owner` begins only upon escalation. Multiple owner questions retain their own identities; the previous task status resumes after all owner questions are answered. Blocking questions prevent agent acceptance while unrelated work can continue. Repeated questions and answers are idempotent; stale answers to superseded briefs are rejected.

A person answers in the task card, and the server records the owner identity as the source. A project manager answering independently must supply an explicit source. Human comments on a task awaiting clarification reach the project manager so the existing owner-decision workflow remains usable.

## Result acceptance and retrospective

Closing a dispatched task moves its manager to the retrospective state. The team can stop while its manager continues. Cancellation also requires a retrospective of work already performed. Reopening returns the manager to execution and requires a new retrospective after the next close. Historical tasks closed before this feature are not automatically enrolled.

The manager reviews its execution sessions, all current and former teams, replaced members, failed attempts, and task-bound jobs. Linked project-manager and human decisions are available through the clarification history. Unrelated conversations are outside the task scope.

A report includes:

- Execution and waiting chronology, review returns, retries, failures, and stalls.
- Findings linked to specific sessions, messages, or events, with measured time/cost where available.
- A distinction between external waiting and loss of progress inside the team.
- The manager's own delegation, staffing, escalation, blocker detection, and acceptance mistakes.
- Proposed role, prompt, skill, or process changes, their expected effect, and how to validate them.
- Source coverage and explicit limitations for missing logs; hypotheses distinguished from established causes.

`manager sources` inventories all persisted execution turns, including former teams. `manager source` reads turn logs in 30,000-character pages and archived events in pages of 20. New live sessions archive settled messages and tool results independently of the bounded live session view and workspace cleanup. Legacy turn logs may be incomplete; archiving cannot recover already lost data.

A complete report must declare review of every available source. Incomplete reports remain pending until coverage is resolved or a person explicitly accepts missing evidence. Failed reporting can retry without undoing the accepted result. Ending execution without the mandatory report triggers retries with backoff and eventually notifies the project manager after repeated failures.

Reports are task artifacts named `retrospective.md`, with persisted coverage. Publication notifies the project manager. Proposals do not automatically change role configuration or prompts; existing administrator permissions still apply. Project-wide retrospectives may combine repeated findings but do not replace task reports.

## Runtime and persistence

`agents/project-manager.md` defines project management; `agents/orchestrator.md` defines task management. The legacy `planner` ID is the project-manager conversation mode, preserving its existing model and plan format while sharing project memory and management instructions.

The runtime retains `AgentKey::Orchestrator { project }` and the project mailbox name `orchestrator` for compatibility. `AgentKey::TaskManager { project, task }` has its own session and mailbox (`task-manager:G-1`). Team mail to `orchestrator` resolves to that task's manager; project intake and clarification requests reach the project manager. `genie orchestrate` remains the project-manager console. The project manager retains reserved turn capacity; task managers take priority over team members without consuming that reserved slot.

Tracker tables `task_managers`, `task_briefs`, `clarifications`, and `project_memory` persist lifecycle state independently of model sessions. `execution_events` archives live execution evidence; `task_team_history` preserves former team associations even after team deletion. Mail and pending retrospective state survive restarts.

## CLI, MCP, and web

```bash
genie manager memory "export"
genie manager remember --text "Exports use CSV" --source "task:G-1, owner decision"
genie manager dispatch G-1
genie manager show G-1
# A task manager session takes the task ID from its environment:
genie manager clarify --question "Retention is unspecified: 7 or 30 days?"
# The project manager answers with evidence or escalates:
genie manager answer G-1 1 --text "30 days" --source "docs:retention-policy"
genie manager escalate G-1 1
genie manager sources G-1
genie manager source 42 --task G-1
genie manager retrospective --task G-1 --text "Findings, sources, and proposals" --reviewed 42
genie manager retry G-1
```

The same operations are available through MCP group `genie_manager` and `/api/management`. The task card shows manager state, brief version, open questions, dispatch/retry actions, and retrospective status.

## Upgrading an existing installation

1. Tracker schema 6 adds management tables with the normal migration rehearsal on a copy. Existing tasks and their statuses are preserved.
2. `project-manager` inherits the configured `orchestrator` model until given its own. Existing `orchestrator` settings remain task-manager settings. Review full custom prompt overrides against the changed role responsibilities.
3. In automatic projects, the scheduler adopts existing active teams, transfers unread unleased project mail for their tasks, and preserves membership/workspaces. Idea discussions remain conversations. No automatic adoption occurs in `manual`.
4. Result acceptance, reopening, and retrospective states remain separate. Repeated dispatch is idempotent. Closed legacy tasks do not start unsolicited retrospectives.

Behavioral coverage includes independent task managers, dependency readiness, dispatch idempotency, manual autonomy, source-backed and escalated questions, stale answers, brief changes, scope restrictions, former-team evidence, partial reports, reopening, and restart recovery. Scripted agents exercise the complete dispatch–team–acceptance–retrospective cycle; live provider/model behavior requires a separate configured pilot.
