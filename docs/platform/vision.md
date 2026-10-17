---
title: What is Genie?
summary: A self-hosted workspace where people and agent teams share tasks and knowledge.
type: guide
status: current
---

# What is Genie?

Genie is a self-hosted workspace for tasks, knowledge, and teams of AI agents. One server serves a small team across multiple projects, with or without code repositories.

People submit work in the web UI, Telegram, or through the CLI/API. A project orchestrator clarifies requests, assembles focused agent teams, and asks people for decisions. Each role can use its own model, skills, and MCP tools.

## How the pieces fit

| Piece | Purpose |
| --- | --- |
| Tasks and epics | Scope, acceptance criteria, dependencies, decisions, and delivery evidence. |
| Agent teams | Implementation, analysis, independent review, tests, and documentation in isolated workspaces. |
| Knowledge vault | Markdown in a separate Git repository, editable in Genie and Obsidian. |
| Automations | Durable workflows triggered by events, schedules, webhooks, or people. |
| Repository connections | Task branches, PRs/MRs, CI signals, and merge policies enforced by the server. |
| People and notifications | Project access, responsible people, questions, mentions, and channel delivery. |

## Operating model

Deploy one installation per team. Genie is not a multi-tenant hosting service. The Rust server owns data and permissions; the React UI, CLI, and MCP interface expose the same work.

Start with `assisted` autonomy so people review every completion. Broaden autonomy and merge policies after the team has evidence that its roles, criteria, and tests produce reliable work.

Genie controls the workflow in code: configurable roles do not replace readiness, review, acceptance, or delivery gates. See [tasks and epics](../guide/tasks.md).

## Current boundaries

The main agent harness is pi. Alternative shell-capable harnesses can run turn jobs, but full live-session integration requires an adapter. Project toolchains are provisioned by the operator. Telegram and SMTP are supported; inbound email, SSO, and tenant isolation are not implemented.

See [the roadmap](roadmap.md) for planned work and [architecture](backend.md) for the implementation.
