---
title: Roadmap
summary: Current capabilities and the next product priorities.
type: note
status: current
---

# Roadmap

The 1.0 goal is a team using Genie every day without routinely repairing it. Reliability and useful results take priority over feature breadth. There is no committed release date.

## Available

- Rust server, web UI, CLI/MCP operation catalog, accounts, project roles, and invitations.
- Tasks, epics, agent teams, configurable roles, live sessions, mail, and human questions.
- GitHub/GitLab repositories, credential proxy, PR/MR policies, CI observation, and merge gates.
- Knowledge vault, search, backlinks, publication review, Git sync, and agent context.
- Durable automations, Telegram/SMTP notifications, usage accounting, budgets, and concurrency limits.
- Docker/systemd deployment, backups and restore, migration checks, health reports, and recovery drills.

See [release notes](../changelog.md) for changes by version.

## Next priorities

| Priority | Work | Evidence needed |
| --- | --- | --- |
| Pilot reliability | Run a short pilot on another real project; resolve runtime and environment failures. | Accepted work, recovery results, cost, and owner effort. |
| Project manager and memory | Persistent project-level planning and a conversation for ongoing work. | Pilot examples where one project orchestrator loses context or planning intent. |
| Epic orchestration | Dedicated epic coordination and explicit release of work into execution. | Task ordering and concurrent-epic needs from the pilot. |
| Routines | Reusable monitoring, cleanup, and digest workflows. | Concrete recurring jobs and their inputs. |
| Richer workspaces and knowledge | Reproducible tools, optional containers, documentation impact, and richer attachments. | Project environment requirements and navigation failures. |

[Product proposals](proposals.md) records the intended sequencing. [Project environments](environment.md) distinguishes current support from proposed provisioning.

## Deferred

SSO, stronger per-role file isolation, additional live harness adapters, Git hosting webhooks/apps, forks/LFS, incoming email, and a visual automation builder remain separate work. Their priority depends on pilot needs.
