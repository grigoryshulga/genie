---
title: Architecture
summary: Rust crates, operation catalog, persistence, runtime, and the web client.
type: reference
status: current
---

# Architecture

`genie serve` is one Rust service for a team. It owns authorization, task transitions, event processing, agent supervision, and knowledge writes. Clients are the React web UI, CLI, MCP consumers, Telegram, and email delivery.

## Code map

| Area | Location | Responsibility |
| --- | --- | --- |
| Domain core | `crates/genie-core/src/` | Task state machine, tracker, teams/mail, vault index, automations, server data. |
| Server and CLI | `crates/genie/src/` | API, authentication, runtime, channels, operator commands. |
| Operation catalog | `crates/genie/src/ops/` | Typed operation arguments, permissions, handlers, CLI and MCP exposure. |
| HTTP | `crates/genie/src/http/` | REST, SSE, MCP server, and web asset serving. |
| Live sessions | `crates/genie/src/sessions.rs` | pi subprocess lifecycle, RPC, supervision, and recovery. |
| Harness extensions | `crates/genie/pi/` | Mail delivery, role guard, and interactive console. |
| Web | `web/src/` | React/TypeScript UI using Feature-Sliced Design. |
| Agent defaults | `agents/`, `config/teams/` | Built-in role prompts and team templates. |
| Deployment | `deploy/`, `docker/` | Service installation, drills, and container setup. |

## Operation catalog

An operation defines its arguments, schema, required permissions, and handler once. The catalog exposes it as both `genie <object> <action>` and an MCP action such as `genie_task` with `action: show`.

The caller's token determines identity, role, project access, and available operations. Clients do not grant themselves permissions. Agents receive a command table filtered by their role. Tests check catalog parity across CLI and MCP.

A CLI invocation with `GENIE_TOKEN` calls the server. Without it, the CLI acts as a local operator against the data directory, which supports initial setup and maintenance.

## Persistence and events

[Task Intelligence and Factory Analytics](task-intelligence.md) extends the same tracker and Statistics page with immutable routing evidence and attempt-aware measurements.

SQLite stores server-wide data in `server.db` and a tracker per project. Task mutations and their event entries share a transaction. Automation cursors, run/step state, mail leases, and notification delivery state persist across restarts.

The knowledge vault is a separate Git repository; its SQLite search index is disposable. See [operations](operations.md#data-directory) for the directory layout.

Agents receive session or turn tokens. Durable mail lives in the core; the harness is a delivery client. Most teams use [live pi sessions](agent-bus.md), while one-shot jobs use turn execution.

## Isolation

Bubblewrap hides server databases, credentials, and unrelated projects while allowing an agent's workspace and selected tool caches. It shares the host network. Docker deployment is a separate layer; the default Compose configuration does not enable nested bubblewrap.

Server permissions and the MCP gateway enforce access by token. Harness rules for file tools and shell patterns are weaker than filesystem isolation. Hosted repository writes pass through the [Git proxy](git-repositories.md).

## Web contract

The web UI consumes REST and SSE. Rust structs generate TypeScript API types through `ts-rs` during `cargo test`; generated files are committed in `web/src/shared/api/generated/`.

Build `web/dist` before the Rust binary to embed it. `GENIE_WEB_DIST=web/dist` makes that build input mandatory. `genie serve --web <directory>` serves an external web build during development.

## Legacy version

The original TypeScript tracker, web server, CLI, and pi extension were removed on September 29, 2026. TypeScript remains for the web client, harness extensions, and their tests. Existing `.genie/` trackers are imported in place; see [migration](getting-started.md#migrate-from-the-pi-extension).
