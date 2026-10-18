---
title: Roadmap
summary: Current capabilities and the next product priorities.
type: note
status: current
---

# Roadmap

The [project and task manager lifecycle](project-and-task-managers.md) supersedes the earlier proposal for epic orchestrators: execution now belongs to one manager per task, with shared project context and a separate mandatory retrospective.

The 1.0 goal is a team using Genie every day without routinely repairing it. Reliability and useful results take priority over feature breadth. There is no committed release date.

## Available

- Rust server, web UI, CLI/MCP operation catalog, accounts, project roles, and invitations.
- Tasks, epics, agent teams, configurable roles, live sessions, mail, and human questions.
- GitHub/GitLab repositories, credential proxy, PR/MR policies, CI observation, and merge gates.
- Knowledge vault, search, backlinks, publication review, Git sync, and agent context.
- Durable automations, Telegram/SMTP notifications, usage accounting, budgets, and concurrency limits.
- Observation after delivery: outcome cases with guardrails or hypothesis checks, metrics and error sources, guardrail incidents, and follow-up tasks. See [observation](observation.md).
- Docker/systemd deployment, backups and restore, migration checks, health reports, and recovery drills.

See [release notes](../changelog.md) for changes by version.

## Next priorities

| Priority | Work | Evidence needed |
| --- | --- | --- |
| Pilot reliability | Run a short pilot on another real project; resolve runtime and environment failures. | Accepted work, recovery results, cost, and owner effort. |
| Manager pilot | Validate shared memory, task dispatch, clarifications, and separate retrospectives on real work. | Source coverage, recovery results, and owner effort. |
| Project planning | Extend task-manager coordination across epics and queue ordering. | Task ordering and concurrent-epic needs from the pilot. |
| Routines | Reusable monitoring, cleanup, and digest workflows. | Concrete recurring jobs and their inputs. |
| Observation analysis | Deployment provider adapters, an analyst role for incidents, default policies, and evidence retention. | Pilot cases: incident accuracy, false positives, and source load. |
| Richer workspaces and knowledge | Reproducible tools, optional containers, documentation impact, and richer attachments. | Project environment requirements and navigation failures. |

[Product proposals](proposals.md) records the intended sequencing. [Project environments](environment.md) distinguishes current support from proposed provisioning.

## Deferred

SSO, stronger per-role file isolation, additional live harness adapters, Git hosting webhooks/apps, forks/LFS, incoming email, and a visual automation builder remain separate work. Their priority depends on pilot needs.
