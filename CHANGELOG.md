# Changelog

Notable changes by release. Versions follow [SemVer](https://semver.org/); entries use [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Detailed implementation history remains in Git.

## Unreleased

### Added

- Instant, read-only UI preview using disposable API fixtures, Vite HMR, both themes, responsive frames, and populated/empty/loading/error scenarios. Startup needs no Rust binary or backend server.

### Changed

- Integration previews are requested by the owner, normally after task PRs reach `dev`. Candidate builds no longer belong to thread-settle or worktree-creation actions. Private preview addresses stay out of GitHub PR text.

## [0.6.0] — 2026-10-24

### Added

- Ink-and-paper web interface with dark `ink`, light `paper`, and system appearance preferences; self-hosted Unbounded, Manrope, and Geist Mono fonts; semantic design tokens; and shared accessible controls, menus, dialogs, loading, empty, and error states.
- A top navigation bar, working-team dock, notification popup, command palette touch entry, and configurable project icons with monogram fallbacks. Redesigned task boards, task and epic pages, team chat, settings, statistics, authentication, and documentation use the same design system across desktop and phone layouts.
- Workspace-environment foundations: host and rootless Podman runners, environment recipes and capability checks, snapshot inputs, harness bundles, lifecycle control, and garbage collection. Built-in recipes cover Node/npm, Python/uv, Rust/Cargo, JVM/Gradle, and custom setups. The controller is not yet connected to normal task dispatch; this is infrastructure for subsequent integration.
- Observation plane, Phase 1: outcome cases for delivered tasks, versioned guardrail and hypothesis contracts, Prometheus- and Sentry-compatible sources, incidents, follow-up tasks, and deterministic conclusions. Includes CLI/MCP operations, HTTP APIs, task sections, and project source settings.
- Task Intelligence and Factory Analytics: immutable task assessments, constrained team and model routing, mandatory independent checks, safety pauses, reassessment evidence, and project analytics. Assessment policy is disabled by default and configured by a human project administrator.
- Optional GitHub and Google sign-in using authorization code flow with PKCE. Providers require deployment configuration; existing password sign-in remains available.
- Documentation spaces with tree navigation, a reader and visual editor, search, and fullscreen editing; an updated documentation site, brand assets, and design-system specimen.
- Browser flow, screenshot, keyboard, contrast, focus, and reduced-motion checks, plus a disposable route-capture harness. These checks cover the documented rules, rather than full WCAG conformance.

### Changed

- Task previews open in centered dialogs with compact properties and team details; board cards and lanes use denser layouts with horizontal scrolling where needed.
- Screen styles live with their owning components and reference semantic `--g-` tokens. The exported token library and documentation palette match the two application themes.
- Rust CI uses pinned Pi-Bolt and nextest with controlled test clocks and leaner fixtures to reduce avoidable waits while retaining integration coverage.
- Local fixed-candidate previews support explicit readiness, isolated review data, and named Tailscale service routing.

### Fixed

- Enlarged image artifacts remain interactive inside native dialogs, and the phone participants sheet traps and restores keyboard focus.
- Losing connectivity cancels active and pending pointer, touch, and keyboard drags without closing unrelated dialogs; offline moves and confirmations cannot submit stale changes after reconnection.
- Keyboard navigation, nested Escape handling, focus visibility, mobile navigation, notification menus, decision-card drag previews, and long project names across the redesigned screens.
- Podman readiness and watchdog tests synchronize on observable state instead of timing assumptions.

### Removed

- Legacy accent selections, compatibility token aliases, obsolete UI class names, unused sidebar layouts, the Geist body font, and the unused board headline model. Geist Mono remains for code and numeric metadata.

### Upgrade notes

- Back up the complete server data and project tracker data before upgrading from 0.5.1. This release uses tracker schema 8 and server schema 10; startup applies additive migrations. Older binaries reject newer schemas, so a rollback requires restoring a compatible backup together with the older image.
- Social sign-in stays disabled until provider credentials and an appropriate public HTTPS URL are configured. Rootless Podman environments require a working rootless runtime and compatible host resource controls; unsupported capabilities are rejected rather than silently downgraded.

## [0.5.1] — 2026-10-17

### Changed

- Releases now flow from protected `dev` to protected `main` through pull requests. Main release PRs validate aligned Cargo/npm versions and changelog entries; merges publish a GitHub Release and tag. Docker images publish only from pushes to `main`.
- Feature branches merged into `dev` are automatically deleted after the merge.

## [0.5.0] — 2026-10-17

### Added

- Project-level intake and shared memory, separate task managers, versioned briefs, source-backed clarifications, and mandatory retrospectives after result acceptance. Includes CLI/MCP operations, task UI, durable session evidence, and additive tracker schema 6.

- Shared provider keys managed by server administrators. Personal keys override shared defaults; agents can use a shared key when their user has none.
- Separate Settings mode with personal, project, and server scopes.
- MCP administration for connections, encrypted secrets, hidden tools, and usage statistics, separate from the client tool catalog.
- English documentation and a Genie-themed documentation site with local search and a GitHub Pages workflow.
- Personal keys for all pi providers, LiteLLM tariff imports with separate cache pricing, and failed-check reruns through `genie pr rerun`.
- Orchestrator-reserved turn capacity, runtime queue inspection, and crash/load drills.

### Changed

- The settings gear now opens profile settings only. Project management, global settings, and statistics have separate sidebar destinations with role guards and redirects for old links. Project tools can restrict installed skills and MCP connections; account management adds deletion, project administrator assignment, and copying new credentials.

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

[0.6.0]: https://github.com/grigoryshulga/genie/releases/tag/v0.6.0
[0.5.1]: https://github.com/grigoryshulga/genie/releases/tag/v0.5.1
[0.5.0]: https://github.com/grigoryshulga/genie/releases/tag/v0.5.0
[0.4.0]: https://github.com/grigoryshulga/genie/releases/tag/v0.4.0
[0.3.0]: https://github.com/grigoryshulga/genie/releases/tag/v0.3.0
[0.2.0]: https://github.com/grigoryshulga/genie/releases/tag/v0.2.0
[0.1.0]: https://github.com/grigoryshulga/genie/releases/tag/v0.1.0
