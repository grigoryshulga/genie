---
title: Quick start
summary: Run Genie, create a project, and configure your first agent team.
type: guide
status: current
---

# Quick start

Genie runs a web UI, API, agent runtime, and knowledge vault on one server. Start with Docker, or build the binary from source.

## Docker

From a checkout of the repository:

```bash
cp docker/.env.example docker/.env
docker compose -f docker/docker-compose.yml up -d --build
printf '%s\n' 'replace-with-a-strong-password' | docker compose -f docker/docker-compose.yml exec -T genie genie user add admin --admin --password-stdin
```

Open [localhost:7420](http://127.0.0.1:7420) and sign in as `admin`. The port is published on loopback by default. See [Docker deployment](docker.md) for volumes, model credentials, network access, and backups.

## From source

Requires stable Rust, Node 22.19+, and Git. Agent sandboxing on Linux also uses bubblewrap.

```bash
npm ci
npm run build:web
GENIE_WEB_DIST=web/dist cargo build --release -p genie
npm install -g @earendil-works/pi-coding-agent
pi  # use /login to connect your model providers, then exit
./target/release/genie serve
```

The web build is embedded in the binary. Open [localhost:7420](http://127.0.0.1:7420). Before the first account exists, access is limited to the local machine. Create an administrator in Settings, or from another terminal:

```bash
printf '%s\n' 'replace-with-a-strong-password' | ./target/release/genie user add admin --admin --password-stdin
```

The remaining examples assume `genie` is on your `PATH`.

## Create a project

Use the first-run screen, or the CLI:

```bash
genie project add shop --name "Shop" --repo ~/code/shop
genie project add product --name "Product"
```

The second project has no code repository. Its agents work in plain directories. For hosted repositories with PR/MR policies, use [repository connections](git-repositories.md).

Set the project to **assisted** in **Settings → Project settings → Agent work**. Agents can work and submit results; a person closes or cancels tasks. Set an integration instruction there as well, such as “Open a pull request; the task owner reviews and merges it.”

## Configure agents

In **Settings → Agents**, assign a model to every role you intend to use, including the orchestrator. There is no default model: a role without `model` cannot run.

The same setting can be saved as `<data>/agents/executor.md`:

```markdown
---
model: provider/model-id
---
```

Replace the model ID with one available to pi. In **Settings → Personal → Providers and keys**, use **+** to connect with OAuth or an API key when the provider supports it. LiteLLM is available in the same list. Administrators can add shared API keys in **Settings → Management → Providers**. A personal credential overrides a shared key. On a server with users, agents use the credentials resolved for the person they work for.

Check the installation as the service user, with its data directory and `PATH`:

```bash
genie agents check
genie doctor
```

Missing models, credentials, sandbox support, or network settings appear in the report. See [roles and teams](agent-roles-and-teams.md) for custom roles, skills, and MCP connections.

## Create the first task

Create a task in the web inbox, or use:

```bash
genie task create "Export orders to CSV" -a "The downloaded file contains the selected orders" --project shop
```

The orchestrator clarifies the request, prepares acceptance criteria, and assembles a team. Answer questions in the task or notification center. Follow the team's work and review its result before closing the task.

Read [the task workflow](../guide/tasks.md) for status gates, epics, and human decisions.

## Next steps

- [CLI and MCP](../reference/cli.md): connect your terminal or coding agent.
- [Operations](operations.md): backups, updates, systemd, and recovery.
- [Knowledge vault](knowledge-vault.md): edit project knowledge in Genie or Obsidian.
- [Pilot guide](pilot.md): introduce Genie to a team.

## Migrate from the pi extension

The original local pi extension was removed on September 29, 2026. Register an existing repository with `genie project add shop --repo ~/code/shop`: Genie reuses its `.genie/` tracker, preserving tasks, history, artifacts, and task numbers.

Remove the old pi package with `pi remove <installation-path>` and replace any old CLI symlink with the server binary. Extension configuration is no longer read. Set models in role files and use `genie orchestrate --project shop` for an interactive orchestrator, or connect your own agent through MCP.
