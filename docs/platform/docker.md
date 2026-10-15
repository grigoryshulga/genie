---
title: Docker deployment
summary: Configure persistent storage, credentials, networking, sandboxing, and backups in Docker.
type: runbook
status: current
---

# Docker deployment

The image contains Genie, its web UI, Git, and Pi-Bolt, available as `pi`. The UI assets are built and served separately from the Rust binary, so a web-only image rebuild can reuse the cached server build. Docker Compose publishes port 7420 on loopback and persists server data and workspaces in named volumes.

Node, npm/npx and Yarn remain available for project tools and stdio MCP servers.
Pi-Bolt supplies the Pi SDK import used by Genie's guard extension internally;
the image does not need a second npm Pi installation. Other extension packages
must be resolvable beside the extension; a global npm installation alone does
not make them importable.

The build installs only the `genie-web` npm workspace for the browser bundle.
Rust dependencies are compiled in a separate Docker layer so exported build
caches can reuse them after application or web changes. Node's debug symbols
are removed before it is copied into the final image.

## Start

```bash
cp docker/.env.example docker/.env
docker compose -f docker/docker-compose.yml up -d --build
printf '%s\n' 'replace-with-a-strong-password' | docker compose -f docker/docker-compose.yml exec -T genie genie user add admin --admin --password-stdin
docker compose -f docker/docker-compose.yml exec -u genie genie genie doctor
```

Open [localhost:7420](http://127.0.0.1:7420). Configure role models and provider keys as described in the [quick start](getting-started.md#configure-agents).

To use the published image, set `GENIE_IMAGE=ghcr.io/grigoryshulga/genie:latest` in `docker/.env`, then run:

```bash
docker compose -f docker/docker-compose.yml pull
docker compose -f docker/docker-compose.yml up -d --no-build
```

Authenticate to GHCR first if the package requires it. Pin a release tag or digest for repeatable deployments.

For an existing checkout, move `.env` and `secrets/` into `docker/.env` and `docker/secrets/`.

## Storage and ownership

| Volume | Container path | Contents |
| --- | --- | --- |
| `genie-data` | `/data` | Databases, vault, configuration, sessions, credentials, and caches |
| `genie-workspace` | `/workspace` | Local code repositories and their worktrees |

`GENIE_UID` and `GENIE_GID` default to 1000. Match them to the owner of bind-mounted code. Mount a repository's **parent directory** so Genie can create sibling worktrees:

```yaml
volumes:
  - genie-data:/data
  - genie-workspace:/workspace
  - /srv/code:/workspace/code
```

Then register the repository:

```bash
docker compose -f docker/docker-compose.yml exec -u genie genie genie project add shop --repo /workspace/code/shop
```

Hosted repository connections use the server's [Git proxy and policies](git-repositories.md). Legacy local repositories use their local Git setup.

## Network access

Set these in `docker/.env` for access through a reverse proxy:

```dotenv
GENIE_PUBLIC_URL=https://genie.example.com
GENIE_ALLOW_HOSTS=genie.example.com
```

The public URL supplies notification links and adds its host to the allowlist. Keep the loopback binding for a proxy on the same host. Set `GENIE_HOST_BIND=0.0.0.0` only when the proxy or clients need another interface. Terminate TLS at the proxy and create the admin account before exposing the service.

The host and container port must match because Genie validates the `Host` header against its own port. Inside the container, `localhost` refers to the container; services on the Docker host are reachable through `host.docker.internal`.

## Models and credentials

Use **+** in **Settings → Personal → Providers and keys** to connect with OAuth or an API key when the provider supports it; LiteLLM is in the same list. Administrators can add shared API keys in **Settings → Management → Providers**. Personal credentials override shared keys. Credentials stored in the database are encrypted with `/data/secrets.key`.

Environment keys such as `OPENAI_API_KEY` and `LITELLM_API_KEY` support local mode before accounts exist. On an authenticated server, agents use the keys resolved for their user; the server environment key is not their fallback.

For a custom provider, copy its pi configuration before starting agents:

```bash
docker compose -f docker/docker-compose.yml cp models.json genie:/data/home/.pi/agent/models.json
docker compose -f docker/docker-compose.yml exec -u genie genie chmod 600 /data/home/.pi/agent/models.json
```

Examples for LiteLLM are in `docker/examples/litellm/`. Replace their endpoint and model IDs. Role models belong in `/data/agents/<role>.md`, not in a `roleModels` configuration block.

Genie gives agents a dedicated profile at `/data/pi-agent`. It seeds `models.json` and `auth.json` once from `/data/home/.pi/agent`, then removes package and extension auto-loading from the agent profile. Repository pi resources are not trusted automatically; Genie passes role and repository skills explicitly.

## Secrets from files

The secrets Compose overlay accepts supported `*_FILE` variables. See `docker/docker-compose.secrets.yml` for the mounts and variable names. For example, supply `LITELLM_API_KEY_FILE` rather than putting a key in `docker/.env`.

The LiteLLM tariff reader uses a separate server-only `LITELLM_INFO_TOKEN` or `LITELLM_INFO_TOKEN_FILE` with access to `/model/info`. Configure `litellm.baseUrl` in `/data/config.json`, then run `genie server prices --refresh`. See [usage and budgets](settings.md#usage-and-budgets).

## Sandboxing

The default Compose setup uses `GENIE_SANDBOX=off`: the container is the isolation boundary, and its agents share the service user's access inside it.

To enable bubblewrap inside Docker, use the supplied overlay:

```bash
docker compose -f docker/docker-compose.yml -f docker/docker-compose.sandbox.yml up -d --build
```

The overlay supplies the settings needed for user namespaces and sandbox mounts. Read it before deploying on a host with stricter container policies. Run `genie doctor` in the resulting container to verify support.

## Project tools and resource limits

The server image does not provide every project's toolchain. Add Debian packages at build time with `EXTRA_APT_PACKAGES`. Set `RUST_TOOLCHAIN=stable` to include Rust, clippy, rustfmt, and shared build caches; `GENIE_AGENT_BUILD_JOBS` bounds Cargo jobs per agent.

```dotenv
EXTRA_APT_PACKAGES=python3 python3-venv build-essential
GENIE_MEM_LIMIT=8g
GENIE_PIDS_LIMIT=4096
```

Memory is unlimited by default. Size a limit for your models, concurrent sessions, and project builds. Pi-Bolt uses less memory than npm pi; `runtime.nodeHeapMb` only caps Node's V8 heap when using npm pi. `runtime.maxSessions` and `idleStopSecs` control how many idle processes remain alive.

See [project environments](environment.md) for current setup limits.

## Backups and updates

```bash
docker compose -f docker/docker-compose.yml exec -u genie genie genie backup /data/backups --keep 14
docker compose -f docker/docker-compose.yml cp genie:/data/backups ./backups
```

Back up project source and worktrees separately. Preserve `secrets.key` separately or use `--with-secrets` for trusted backups. See [operations](operations.md#back-up-and-restore) for coverage and restore rules.

Before updating, create and copy out a backup. Pull or rebuild the image, recreate the service, then run `genie doctor`. Rollback requires the old image **and** the pre-update data backup; replacing the image alone does not reverse database migrations.
