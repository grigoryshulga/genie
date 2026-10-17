# Genie domain language

Management terms used by the [project and task manager lifecycle](docs/platform/project-and-task-managers.md).

| Term | Meaning |
| --- | --- |
| **Project manager** | Persistent project agent responsible for shared context, human planning, prepared tasks, and the project queue. Russian: менеджер проекта. |
| **Task manager** (orchestrator) | Agent responsible for one task from dispatch and team coordination through result acceptance and retrospective publication. Russian: менеджер задачи, оркестратор. |
| **Task brief** | Recorded goal, requirements, scope, acceptance criteria, constraints, and sources used to dispatch a task. Russian: постановка задачи. |
| **Clarification** | A task-linked question requiring a confirmed agreement or missing decision before some work can proceed. Russian: запрос на уточнение. |
| **Task retrospective** | Evidence-based review of task execution, with findings and proposed improvements to roles, prompts, and coordination. Russian: ретро задачи. |

Use “orchestrator” for the task manager in the new model. The runtime's project-level `Orchestrator` variant and mailbox retain their names for compatibility.
