<p align="center">
  <img src="docs/assets/hero.svg" alt="genie: tasks, knowledge and teams of AI agents on one self-hosted server" width="100%">
</p>

**genie** is a self-hosted server for tasks, knowledge and teams of AI agents. People file tasks from the web UI, Telegram or email. An orchestrator clarifies each task and assembles a focused team for it: analyst, executor, reviewer, tester, documenter, each on its own model. The team works in its own git worktree (or a plain directory for projects without code) and asks the responsible person when a decision needs a human.

## Features

- **Web UI and API.** A Linear-style board for tasks, epics, teams, docs and automations, plus a CLI (`genie <object> <action>`) that covers everything the UI does.
- **Agent teams.** Live agent sessions on [pi](https://pi.dev) in a bubblewrap sandbox, configurable roles and team templates, an MCP gateway so other agents (Claude Code, Codex) can use genie's tools.
- **Projects and people.** Multiple projects, per-project roles, task owners, `@login` mentions, invite links. Orchestrator autonomy per project: `autonomous`, `assisted` or `manual`.
- **Epics.** Large work becomes an epic with a goal, success criteria, a roadmap and shared artifacts (requirements, glossary, decisions). Its tasks see all of it, and the epic tracks its own progress.
- **Idea shaping.** Not sure how many tasks an idea will turn into? Describe it as it is and talk it over with a planner agent in chat. It proposes a task, or an epic with tasks and criteria, and you create the plan with one button.
- **Images.** An image artifact (PNG, JPEG, GIF, WebP) gets a thumbnail and a zoomable lightbox. In a team chat, use `!image[artifact:G-7/3]` or `!image[docs/shot.png]` (a file from the team's working copy). External URLs are not supported.
- **Knowledge base.** An Obsidian-compatible vault in git, edited by people and agents alike.
- **Notifications and automations.** Web, Telegram and email delivery, with an automation engine on top.
- **Reliability.** Restarts lose nothing, hot backups, `genie doctor` preflight checks, systemd units.

## Quick start

### Docker

One image with genie, the web UI, pi and git:

```bash
cp .env.example .env
docker compose up -d --build
echo 'your-password' | docker compose exec -T genie genie user add admin --admin --password-stdin
```

Open http://127.0.0.1:7420. Volumes, repositories, model keys, git access, proxies, the prebuilt GHCR image and backups are covered in [docs/platform/docker.md](docs/platform/docker.md).

### From source

Requires Rust (stable), Node 22.19+ and git.

```bash
npm install && npm run build:web && cargo build --release -p genie   # the web UI is embedded into the binary
npm install -g @earendil-works/pi-coding-agent && pi                 # install pi, then /login to your model providers
./target/release/genie serve                                         # http://127.0.0.1:7420
```

On first launch the server offers to create a project, and the "Project & people" page sets up the admin account and invite links. Run `./target/release/genie doctor` to see what is still missing: role models, sandbox, channels, network.

## Documentation

The docs are in Russian.

| Topic | Link |
|---|---|
| Running and operating the server | [getting-started.md](docs/platform/getting-started.md) |
| Docker deployment | [docker.md](docs/platform/docker.md) |
| Running a pilot with a team | [pilot.md](docs/platform/pilot.md) |
| Vision and architecture | [vision.md](docs/platform/vision.md), [backend.md](docs/platform/backend.md) |
| Roles and teams | [agent-roles-and-teams.md](docs/platform/agent-roles-and-teams.md), [agent-bus.md](docs/platform/agent-bus.md) |
| Knowledge vault | [knowledge-vault.md](docs/platform/knowledge-vault.md) |
| Automations | [automations.md](docs/platform/automations.md) |
| Decisions | [decisions.md](docs/platform/decisions.md) |
| Changes | [CHANGELOG.md](CHANGELOG.md) |

## Development

```bash
cargo test                                                   # core, API, agent runtime, sandbox, owner scenarios
cargo clippy --all-targets -- -D warnings && cargo fmt --all --check
npm test && npm run typecheck && npm run build:web
npm run dev:web                                              # Vite dev server, proxies /api to port 7420
GENIE_WEB_SOURCEMAP=1 npm run build:web                      # build with source maps for debugging in the browser
```

Web API types are generated from the Rust structs. After changing them, run `cargo test` and commit `web/src/shared/api/generated`.

## Migrating from the pi extension

The first version of genie, a pi extension with a local tracker, has been removed. Existing repositories move over in place: `genie project add shop --repo ~/projects/my-repo` picks up their `.genie/` with all tasks and numbers. `genie orchestrate` turns your pi session into the project orchestrator. Step by step: [getting-started.md](docs/platform/getting-started.md#переход-с-расширения-pi).
