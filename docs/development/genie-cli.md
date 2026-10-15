# Tracking repository work with genie

Use the installed `genie` CLI. Run `genie --help` and `genie task <action> --help` when an option is unclear.

## This repository's tracker

The local tracker is **http://127.0.0.1:7422**, container **`genie-tracker`**, project **`genie`**, task prefix **`GNE`**, autonomy **`manual`**. Manual mode disables the server orchestrator; external agents maintain task progress themselves. This project tracks work performed in host worktrees, so its tracker has no repository mounted inside the container.

Use the container CLI explicitly, independently of the host CLI's default server:

```bash
docker exec -i genie-tracker genie --project genie project list
docker exec -i genie-tracker genie --project genie task list --search '<task keywords>'
```

The container CLI runs as the service user through the image's wrapper. It uses the container's `/data` directly; no token is needed for this local operator route. Use this route only for the task actions authorized by the repository workflow; it does not grant permission to bypass review or completion gates.

### Start and recover

From a checkout of this repository:

```bash
docker compose -f docker-compose.tracker.yml up -d
curl --fail http://127.0.0.1:7422/api/health
```

The compose file uses the locally installed `genie-local:latest` image. On a fresh machine, build it with `docker build -t genie-local:latest .`, or set `GENIE_TRACKER_IMAGE` to an available compatible image. Data is kept in the named Docker volume `genie-tracker_data`; retain this volume when stopping or recreating the container. `manual` is persisted in the project database, not a global container setting.

On a fresh volume only, create the project, then set its mode:

```bash
docker exec -i genie-tracker genie project add genie --name Genie --prefix GNE
docker exec -i genie-tracker genie project update genie --autonomy manual \
  --integration 'External agents publish English PRs and wait for successful CI; the owner merges them.'
```

A local web administrator `gshulga` was created during setup. Its initial password is stored on this machine at `~/.local/share/genie-tracker/admin-password` with owner-only permissions; keep that file outside git. On a fresh installation, create your own administrator with `genie user add <login> --admin --password-stdin` through `docker exec -i`. Passwords and tokens belong in neither task comments nor commits.

### Other environments

For another machine or a remote server, confirm its project first. `GENIE_URL` and `GENIE_TOKEN` select a remote server for the host `genie` CLI. Without a token, that CLI uses a local server data directory (`--data` or `GENIE_DATA`), which may belong to another installation. Always select the confirmed project explicitly. The current directory and a legacy `.genie/genie.db` do not select a server project; `--data` expects server data, not a tracker directory.

## Create or resume a task

If a task is assigned, read it using its ID. Otherwise search first, then create a task:

```bash
docker exec -i genie-tracker genie --project genie task show <task-id>
docker exec -i genie-tracker genie --project genie task create 'Describe the intended result' \
  --description 'Scope, context, and constraints' \
  --ac 'The requested behavior is implemented and verified' \
  --ac 'A PR is published and CI passes for its latest head' \
  --plan 'Implement; validate; publish PR; wait for CI'
```

Use the returned task ID in subsequent commands (`--json` is available for automation). Creation status depends on the caller: people normally create inbox tasks, agents create drafts, and team members may create only subtasks of their assignment (`--parent <task-id>`). `GENIE_TASK` can supply a default task, but explicit IDs make the target clear.

## Record progress

```bash
docker exec -i genie-tracker genie --project genie task status in_progress --task <task-id> \
  --note 'Starting implementation in branch <branch>, worktree <path>, base <sha>'
docker exec -i genie-tracker genie --project genie task comment --task <task-id> --kind progress \
  'Implemented <milestone>; validation: <command and result>; next: <step>'
docker exec -i genie-tracker genie --project genie task check 1 --task <task-id>
docker exec -i genie-tracker genie --project genie task block 'Missing <dependency>; need <action>' --task <task-id>
docker exec -i genie-tracker genie --project genie task artifact --task <task-id> --kind test-report \
  --name validation.md --text - < <host-path-to-report>
```

The status command is an example for a task ready to start, not a shortcut past refinement. Follow the transitions and Definition of Ready reported by the server. Tick a criterion only after verifying it; `task check <N> --undo` clears it. Record blockers using `task block` and its required reason. Attach the command results as a `test-report` artifact when the workflow requires one. The artifact example streams a host file through stdin; `--file` would refer to a path inside the container.

## Review and completion

```bash
docker exec -i genie-tracker genie --project genie task comment --task <task-id> --kind progress \
  'PR: <url>; head: <sha>; local checks: <results>; CI pending'
docker exec -i genie-tracker genie --project genie task status review --task <task-id> \
  --note 'PR published; verification evidence attached; awaiting review and CI'
```

After CI succeeds, add a comment with the PR URL, current head SHA, and passing checks, then check the remaining acceptance criteria. Follow the server's review and approval flow before `task status done`. `task accept` is owner acceptance, not an agent workaround. A permission or workflow rejection is a reason to request the authorized actor's action; retain the current credentials and gates.

The completion gate and CI waiting rules are defined in [AGENTS.md](../../AGENTS.md).
