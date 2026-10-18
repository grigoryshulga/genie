---
title: CLI and MCP
summary: Connect a terminal, script, or coding agent using the same operation catalog.
type: reference
status: current
---

# CLI and MCP

The CLI and MCP server expose the same [operation catalog](../platform/backend.md#operation-catalog). The server checks the caller's token and project access for each operation.

## CLI connection

Create a personal token in **Settings → Personal → CLI and MCP tokens**, or use `genie user token <login>` as the local operator. Set it in your shell as `GENIE_TOKEN`; do not commit it.

```bash
export GENIE_URL=http://127.0.0.1:7420
export GENIE_PROJECT=shop
# Set GENIE_TOKEN to your personal token using your secret manager or shell.
genie task list --status review
genie task show G-7
genie docs search "exports"
genie me questions
```

With a token, commands call `GENIE_URL` as its owner. Without a token, the CLI opens the local data directory as the server operator. Use the latter for setup and maintenance on the server machine.

`--project shop` overrides `GENIE_PROJECT`. An inaccessible named project is an error; Genie does not substitute another. Without a selection, it uses the first accessible project. `--json` prints API output for scripts.

## Discover commands

```bash
genie --help
genie task --help
genie task create --help
```

Operation groups include `task`, `team`, `mail`, `docs`, `job`, `automation`, `agents`, `project`, `user`, `me`, and `server`. Task assessment, routing evidence and policy comparisons are under `intelligence`; Jev calls are under `decision`, and project/task management is under `manager`. Repository and delivery commands are under `repos` and `pr`. Operator commands include `serve`, `doctor`, `backup`, `restore`, and `migrate`.

## Task intelligence

```bash
genie --project shop intelligence history --task G-7
genie --project shop intelligence assess --task G-7 --affected-path src/auth/login.rs
genie intelligence reassess --help
genie intelligence resolve --help
genie intelligence shadow --help
```

`history` reads immutable assessments, applied routing plans, execution attempts and
shadow evaluations. `assess` records an assessment; only people can narrow the scope
using confirmed affected paths. `reassess` records a material discovery and applies
mandatory reviewers or pauses the existing team. `resolve` records a human decision
about runtime constraints; it does not resume agents or remove required reviewers.
`shadow` evaluates a candidate policy without applying it and requires a human project
administrator. The latter three commands accept their shared JSON contract through
`--request`.

The MCP tool is `genie_intelligence` with actions `history`, `assess`, `reassess`,
`resolve`, and `shadow`. Caller permissions and assigned task scope are enforced by
the server. See [Task Intelligence](../platform/task-intelligence.md) for request
examples, safety constraints, cost coverage and evaluation limits.

## MCP connection

Connect a client supporting Streamable HTTP to `/mcp`:

```json
{
  "mcpServers": {
    "genie": {
      "url": "http://127.0.0.1:7420/mcp?project=shop",
      "headers": { "Authorization": "Bearer YOUR_PERSONAL_TOKEN" }
    }
  }
}
```

Use your client's secret storage for the token. The exact configuration wrapper depends on the client.

Tools are grouped, such as `genie_task`, `genie_team`, `genie_mail`, and `genie_docs`. An `action` selects an operation, and the tool description lists its arguments. Human tokens expose human operations; agent tokens expose only their permitted operations.

Select a project through the `project` argument, the `X-Genie-Project` header, or the URL query. In local mode with no accounts, a loopback client can omit the token but must send `X-Genie: 1`.

## Become the orchestrator

`genie orchestrate --project shop` launches your pi session with a leased orchestrator identity while the server orchestrator waits. See [interactive orchestrator](../platform/agent-bus.md#interactive-orchestrator) for mail, controls, and lease behavior.
