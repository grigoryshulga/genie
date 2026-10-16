# Changelog

Notable changes by release. Versions follow [SemVer](https://semver.org/); entries use [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Detailed implementation history remains in Git.

## Unreleased

### Added

- Project-level intake and shared memory, separate task managers, versioned briefs, source-backed clarifications, and mandatory retrospectives after result acceptance. Includes CLI/MCP operations, task UI, durable session evidence, and additive tracker schema 6.

- Shared provider keys managed by server administrators. Personal keys override shared defaults; agents can use a shared key when their user has none.
- Separate Settings mode with personal, project, and server scopes.
- MCP administration for connections, encrypted secrets, hidden tools, and usage statistics, separate from the client tool catalog.
- English documentation and a Genie-themed documentation site with local search and a GitHub Pages workflow.
- Personal keys for all pi providers, LiteLLM tariff imports with separate cache pricing, and failed-check reruns through `genie pr rerun`.
- Orchestrator-reserved turn capacity, runtime queue inspection, and crash/load drills.

### Changed

- Every runnable role must specify a model. Class model defaults and `roleModels` configuration are removed; missing models are reported by health checks and block launch.
- Docker agents use Pi-Bolt as `pi`. Agent profiles disable automatic package and extension loading; role and repository skills are passed explicitly.
- Integrated task workspaces can be cleaned up, and low disk space appears in health checks.

### Removed

- The `pi-mcp-adapter` path; pi's built-in MCP integration is used instead.
- Unused npm/Cargo dependencies and obsolete lock/name-selection helpers.

## [0.4.0] — 2026-10-04

### Added

- Personal LiteLLM credentials, per-agent model changes, token/cost reporting, and task, epic, and daily budgets.
- Per-repository encrypted access tokens, server Git proxy, scoped task repositories, PR/MR delivery policies, and GitHub/GitLab integration.
- CI feedback for task branches and merge commits, review gates for failed checks, stalled-check reports, failed-log excerpts.
- Idea shaping with a planner agent that proposes a task or epic for a person to create.
- Owner-question actions, ready/start/next playbooks, epic concurrency limits, and silent-team supervision.
- Docker deployment, isolated pilot instances, backup restore, migration checks on copies, and rollback drills.
- Configurable roles, team templates, skills, permissions, and live pi sessions.
- A unified operation catalog for CLI and MCP, an interactive orchestrator console, and pilot health checks.

### Changed

- Agents that exhaust retries remain in `error` with a reason.
- Sandbox handling restricts Git hooks/configuration and privileged container sockets.

### Removed

- The TypeScript domain core, local CLI, web server, and original pi extension. Rust owns server behavior; TypeScript remains for the web UI and harness extensions.

### Fixed

- Delivery observation now follows the checked commit, including branches without requests and the target branch after merging.
- Stalled CI and legitimate human waits no longer look like silent teams.
- Backup restoration and schema-version checks prevent unsupported binary-only rollback paths.

## [0.3.0] — 2026-09-28

### Added

- Standalone Rust server with REST/SSE, authentication, users, project roles, invitations, personal tokens, and notifications.
- Agent turn execution, server orchestration, durable automation rules and jobs, Telegram/SMTP delivery, and human questionnaires.
- A separate knowledge vault with Git synchronization, section policies, proposals, review, changelogs, and releases.
- Server administration, statistics, health checks, backups, and end-to-end workflow scenarios.

### Fixed

- Reliability and access checks across the server, agent jobs, channels, and knowledge workflows.

## [0.2.0] — 2026-09-27

### Added

- Markdown documentation indexing with frontmatter, FTS5 search, wiki-links, backlinks, heading reads, and task/code relationships.
- L0/L1 knowledge context and documentation warnings for task review.
- Mail priority and intent, orchestrator digests, and persistent delivery queues.
- Image previews for artifacts and team-chat references; Markdown tables in task content.

### Changed

- Tracker schema version 3 adds mail levels and intents.
- Progress belongs in tasks and agent status rather than routine mail.

### Fixed

- Deprecated pages are excluded from default search.
- Context selection reduces the influence of common terms.
- Documentation and image reads reject unsafe paths and oversized image input.

## [0.1.0] — 2026-09-27

Initial release as a local pi extension.

### Added

- SQLite task tracker with acceptance criteria, dependencies, epics, comments, artifacts, and history.
- Orchestrator workflow and focused agent teams with role-specific models, worktrees, direct mail, and recovery.
- Web board, task/epic detail, team chat, command palette, keyboard navigation, and mobile layouts.
- Local CLI and pi tools for tasks and team control.
- React client organized with Feature-Sliced Design.

### Limitations

- The original web server had no account authentication and was intended for localhost or a trusted tailnet.
- This extension architecture has since been replaced by the standalone server.

[0.4.0]: https://github.com/grigoryshulga/genie/releases/tag/v0.4.0
[0.3.0]: https://github.com/grigoryshulga/genie/releases/tag/v0.3.0
[0.2.0]: https://github.com/grigoryshulga/genie/releases/tag/v0.2.0
[0.1.0]: https://github.com/grigoryshulga/genie/releases/tag/v0.1.0
