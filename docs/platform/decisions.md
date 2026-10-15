---
title: Design decisions
summary: Durable product and architecture decisions, with links to current behavior.
type: decision
status: current
---

# Design decisions

Keep decisions here; keep instructions in their topic pages. Older implementation plans and superseded defaults live in Git history.

| ID | Decision | Current reference |
| --- | --- | --- |
| PD1 | One installation per team; no multi-tenancy. | [Overview](vision.md) |
| PD2 | Start locally; support managed deployment separately. Docker and systemd are now available. | [Deployment](docker.md) |
| PD3 | Knowledge lives in a separate Obsidian-compatible Git vault. | [Vault](knowledge-vault.md) |
| PD4 | Publish knowledge through per-section policies for people and agents. | [Publication](knowledge-vault.md#publication-policies) |
| PD5 | Telegram and email are the initial channels; projects may manage dedicated bots with personal, bot-scoped connections. | [Notifications](settings.md#notifications) |
| PD6 | Support projects without code repositories. | [Quick start](getting-started.md#create-a-project) |
| PD7 | Track model usage and enforce task, epic, and daily budgets. Revised October 3, 2026. | [Budgets](settings.md#usage-and-budgets) |
| PD8 | Rust backend and CLI. | [Architecture](backend.md) |
| PD9 | Expose an agent contract independent of a single client harness. | [CLI and MCP](../reference/cli.md) |
| PD10 | Move the core to Rust before adding platform features. Migration completed September 29, 2026. | [Legacy version](backend.md#legacy-version) |
| PD11 | Keep the server, UI, harness extensions, and prompts in one repository and release. | [Development](../development.md) |
| PD12 | One pi process per live agent; only people and the orchestrator can interrupt. Idle time is configurable, currently 300 seconds by default. | [Runtime](agent-bus.md) |
| PD13 | Roles, templates, skills, and connections are server configuration; the web edits the same files. | [Roles and teams](agent-roles-and-teams.md) |
| PD14 | Only server administrators edit agent configuration. | [Roles and teams](agent-roles-and-teams.md) |
| PD15 | Configurable permissions refine a fixed process class; workflow gates stay in code. | [Permissions](agent-roles-and-teams.md#permissions) |
| PD16 | Route MCP through a server gateway by default; retain explicit direct connections. | [MCP connections](agent-roles-and-teams.md#mcp-connections) |
| PD17 | Preserve an interactive orchestrator as a server client with a leased console token. | [Interactive orchestrator](agent-bus.md#interactive-orchestrator) |
| PD18 | Generate CLI and MCP actions from the same operation catalog. | [Catalog](backend.md#operation-catalog) |
| PD19 | Supply L0 page metadata and L1 task knowledge in agent context. | [Agent context](knowledge-vault.md#agent-context) |
| PD20 | Separate MCP administration, secrets, and usage from the client tool catalog. | [MCP connections](agent-roles-and-teams.md#mcp-connections) |
| PD21–23 | Separate work and Settings in the same SPA. Separate admin hosting and API namespaces remain deferred. | [Settings](settings.md) |
| PD24 | Every runnable role names a model. Remove class defaults and `roleModels`. | [Role file](agent-roles-and-teams.md#role-file) |

## Repository decisions

| ID | Decision |
| --- | --- |
| GR1 | Store repository-scoped tokens encrypted on the server. |
| GR2 | Agents use the server Git proxy without hosting credentials. |
| GR3 | People merge by default; broader merge policies are explicit. |
| GR4 | Read access follows role/policy; writes also require task repository scope. |
| GR5 | A remote can serve several projects with separate policy and credentials. |
| GR6 | Role Git access can narrow file access, never expand it. |
| GR7 | Support GitHub, GitLab, and plain Git through host configuration. |

See [repositories and delivery](git-repositories.md).

## Pilot defaults and open questions

Use assisted autonomy, responsible people on tasks, bubblewrap where supported, daily backups, and measured pilot outcomes. Harness file restrictions inside a writable worktree remain soft. People currently participate as task owners/assignees rather than template members. Access to server-wide statistics remains administrative.
