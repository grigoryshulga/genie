# Tracking repository work with genie

Use the installed `genie` CLI. Run `genie --help` and `genie task <action> --help` when an option is unclear.

## This repository's tracker

The local tracker is **http://127.0.0.1:7422**, container **`genie-tracker`**, project **`genie`**, task prefix **`GNE`**, autonomy **`manual`**. Manual mode disables the server orchestrator; external agents maintain task progress themselves. This project tracks work performed in host worktrees, so its tracker has no repository mounted inside the container.

Use the native host CLI, which connects to this tracker over HTTP:

```bash
genie --project genie project list
genie --project genie task list --search '<task keywords>'
```

The host installation has three parts:

- `~/.local/bin/genie`: launcher that executes the native binary without Docker.
- `~/.local/lib/genie/bin/genie`: native Linux binary, installed from the running image.
- `~/.local/share/genie-tracker/cli.env`: connection profile (`GENIE_URL`, `GENIE_TOKEN`, `GENIE_PROJECT`), readable only by its owner (mode `600`).

With no explicit remote connection in the environment, the launcher loads this profile automatically. Ordinary commands require neither `docker exec` nor shell setup. `--file` reads host paths normally. The old Docker launcher is retained as `genie-docker` for access to the previous installation.

The profile authenticates as the local project owner `gshulga`. Keep credentials outside git, comments, and logs. Follow the repository's task workflow with these credentials; administrative access does not authorize skipping its gates.

### Installing a native CLI on another machine

Install a compatible native `genie` binary into a directory on `PATH`. For a source build, follow [the source build guide](../platform/getting-started.md#from-source). A binary can also be extracted once from this Linux container with `docker cp genie-tracker:/opt/genie/bin/genie <host-destination>`; Docker is not involved when that binary subsequently runs.

Create a personal token through the server's account settings, or during initial setup with `docker exec genie-tracker genie user token <login> --label host-cli`. Save it privately and configure `GENIE_URL=http://127.0.0.1:7422`, `GENIE_TOKEN`, and `GENIE_PROJECT=genie` for the native CLI. On this machine the launcher already loads those values from the private profile above. A compatible Linux host is required for a binary extracted from the image.

### Start and recover the server

Docker is needed for server lifecycle and initial bootstrap only. From a checkout of this repository:

When the [verified dev updater](tracker-auto-update.md) is enabled, it owns tracker deployments. Stop its timer before manual recovery or recreation, and resume it only after checking the deployed revision and retained volumes. Its private Tailscale Service opens the same persistent tracker; the native CLI can continue using loopback.

```bash
docker compose -f docker/docker-compose.tracker.yml up -d
curl --fail http://127.0.0.1:7422/api/health
```

The compose file builds and uses the local `genie-local:latest` image. Rebuild and recreate the tracker from this checkout with `docker compose -p genie-tracker -f docker/docker-compose.tracker.yml up -d --build`; after the first build, web-only changes reuse the cached Rust server layer. To use an available compatible image instead, set `GENIE_TRACKER_IMAGE` and run Compose with `--no-build`. Data is kept in the named Docker volume `genie-tracker_data`; retain this volume when stopping or recreating the container. `manual` is persisted in the project database, not a global container setting.

On a fresh volume only, create the project, then set its mode:

```bash
docker exec -i genie-tracker genie project add genie --name Genie --prefix GNE
docker exec -i genie-tracker genie project update genie --autonomy manual \
  --integration 'External agents publish English PRs to dev and wait for successful CI; for UI changes they first prepare a fixed preview and await owner approval; the owner merges the PRs.'
```

A local web administrator `gshulga` was created during setup. Its initial password is stored on this machine at `~/.local/share/genie-tracker/admin-password` with owner-only permissions; keep that file outside git. On a fresh installation, create your own administrator with `genie user add <login> --admin --password-stdin` through `docker exec -i`. Passwords and tokens belong in neither task comments nor commits.

### Other environments

To select another remote server, supply both `GENIE_URL` and `GENIE_TOKEN` explicitly; the launcher then preserves that connection instead of loading the default profile. Confirm the destination project first. `GENIE_URL` and `GENIE_TOKEN` select a remote server for the host `genie` CLI. The underlying native binary, when run directly without a token, uses a local server data directory (`--data` or `GENIE_DATA`), which may belong to another installation. Always select the confirmed project explicitly. The current directory and a legacy `.genie/genie.db` do not select a server project; `--data` expects server data, not a tracker directory.

## Create or resume a task

If a task is assigned, read it using its ID. Otherwise search first, then create a task:

```bash
genie --project genie task show <task-id>
genie --project genie task create 'Describe the intended result' \
  --description 'Scope, context, and constraints' \
  --ac 'The requested behavior is implemented and verified' \
  --ac 'For a UI change, the owner approves the fixed preview before branch and PR publication' \
  --ac 'A PR is published and CI passes for its latest head' \
  --plan 'Implement; validate; for a UI change prepare preview and await owner approval; publish PR; wait for CI'
```

Use the returned task ID in subsequent commands (`--json` is available for automation). Creation status depends on the caller: people normally create inbox tasks, agents create drafts, and team members may create only subtasks of their assignment (`--parent <task-id>`). `GENIE_TASK` can supply a default task, but explicit IDs make the target clear.

## Record progress

```bash
genie --project genie task status in_progress --task <task-id> \
  --note 'Starting implementation in branch <branch>, worktree <path>, base <sha>'
genie --project genie task comment --task <task-id> --kind progress \
  'Implemented <milestone>; validation: <command and result>; next: <step>'
genie --project genie task check 1 --task <task-id>
genie --project genie task block 'Missing <dependency>; need <action>' --task <task-id>
genie --project genie task artifact --task <task-id> --kind test-report \
  --name validation.md --file <host-path-to-report>
```

The status command is an example for a task ready to start, not a shortcut past refinement. Follow the transitions and Definition of Ready reported by the server. Tick a criterion only after verifying it; `task check <N> --undo` clears it. Record blockers using `task block` and its required reason. Attach the command results as a `test-report` artifact when the workflow requires one. The native CLI reads the artifact directly from the host file.

## Review and completion

Before publishing a UI change (defined in [AGENTS.md](https://github.com/grigoryshulga/genie/blob/main/AGENTS.md)), follow [visual review before publishing](visual-preview.md); any other change is published once its checks pass. For a UI change, record `awaiting_visual_review`, the candidate ID, base and included task SHAs, local/Tailscale links, validation and the review scenario in a progress comment. Keep the task unfinished while awaiting the owner's explicit candidate approval and publication authorization. This milestone is not a new tracker status and does not replace owner acceptance. Record the approval reference before continuing; changes to the candidate composition require renewed approval.

After approval (or, for a change that is not a UI change, after validation), publish the task commit in a PR to `dev` (release PRs go from `dev` to `main`), then:

```bash
genie --project genie task comment --task <task-id> --kind progress \
  'Approved candidate (UI changes): <id>; owner approval: <reference>; PR: <url>; head: <sha>; local checks: <results>; CI pending'
genie --project genie task status review --task <task-id> \
  --note 'PR published; verification evidence attached; awaiting review and CI'
```

After CI succeeds, add a comment with the PR URL, current head SHA, and passing checks, then check the remaining acceptance criteria. Follow the server's review and approval flow before `task status done`. `task accept` is owner acceptance, not an agent workaround. A permission or workflow rejection is a reason to request the authorized actor's action; retain the current credentials and gates.

The completion gate and CI waiting rules are defined in [AGENTS.md](https://github.com/grigoryshulga/genie/blob/main/AGENTS.md).
