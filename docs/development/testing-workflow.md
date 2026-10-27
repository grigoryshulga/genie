---
title: Testing workflow
summary: Check a change in the browser in seconds, keep the local tracker on dev on demand, and keep the disk clean.
type: guide
status: current
---

# Testing workflow

Three loops, from the cheapest to the one that gates publication. Use the cheapest one that answers your question.

| Question | Loop | Cost |
| --- | --- | --- |
| Does this UI change look and behave right? | [Development stand](#development-stand) | seconds, hot reload |
| Does the whole product still work? | `npm run typecheck`, `npm test`, `cargo test`, `npm run test:ui` | minutes; CI runs the same |
| Is this what the owner approves? | [Fixture preview](visual-preview.md#instant-ui-fixtures) (`npm run preview:ui`, no Rust) or, on request, a fixed integration candidate; required for UI changes only | seconds; a candidate is one build |

## Development stand

The stand runs the real server on seeded data, so it checks behaviour as well as looks. `npm run preview:ui` is lighter: it replays recorded API responses without Rust, which suits showing a screen to the owner. Use the stand while building and the fixture preview when presenting.

```bash
npm ci                    # once per worktree
npm run dev:stand
```

The command builds `genie` (incrementally), seeds a disposable data directory and starts the server and Vite's hot-reload server together. Open the printed address and sign in as `owner`. A saved edit in `web/src` appears in the browser without a build, a container or a preview candidate. The Rust code is rebuilt only when you restart the stand after changing it.

The data is written through the CLI by [`test/ui/stand-data.ts`](https://github.com/grigoryshulga/genie/blob/dev/test/ui/stand-data.ts) and covers awkward states on purpose:

- three projects: a Russian one with every status, an English one, and an empty one for empty states;
- four people: an admin owner, two members and a viewer;
- long titles, ten-item acceptance lists, eight labels, nine-message discussions, comments of every kind, artifacts, a blocked task, tasks waiting for the owner, a finished epic and an epic in progress;
- markdown with tables and code, and text with HTML that must be shown as text;
- an integration analyst role with 48 explicit MCP grants, a full connection grant and an unavailable connection, for grouped access and inline editing.
- a team template with a long title, explicit relations, per-member models, reasoning and instructions, plus shared Markdown rules;
- a running team ([`test/ui/stand-team.ts`](https://github.com/grigoryshulga/genie/blob/dev/test/ui/stand-team.ts)) with Markdown mail, unread mail and an executor whose conversation is read from its saved session file, as after an idle session stopped.

Add a case to the data file when a bug needs one; the browser tests do not read it, so their screenshots do not move.

```bash
npm run dev:stand -- --reset      # throw the data away and seed it again
npm run dev:stand -- --no-build   # skip cargo
npm run dev:stand -- --clean      # delete this worktree's stand data
```

Each worktree gets its own data under `~/.cache/genie/stand/<worktree>` and stable ports, so several stands can run side by side. The server listens on loopback only and runs without agents, credentials or integrations. The stand never touches the real tracker's data. To look at real data instead, point Vite at the tracker with `GENIE_API_PROXY_TARGET=http://127.0.0.1:7422 npm run dev:web`; actions you take there change real tasks.

Rust build output is shared by every worktree in `~/.cache/genie/target`. To use it for your own `cargo` commands too, add this to your shell profile:

```bash
export CARGO_TARGET_DIR="$HOME/.cache/genie/target"
```

Parallel builds wait for each other's lock, which is the price of one `target` instead of one per worktree.

## Updating the local tracker

The tracker is updated when you ask for it:

```bash
preview update-tracker
```

It deploys the newest `origin/dev` commit whose CI succeeded, with the safeguards described in [automatic tracker updates](tracker-auto-update.md): the data volumes are kept, a backup is taken before recreation, and a failed start pauses further updates instead of downgrading the data. After a successful update only the newest two tracker images and the images in use are kept. The systemd timer is optional: leave it disabled to update only on command.

## Keeping the disk clean

```bash
preview gc                  # show what would be removed
preview gc --execute        # do it
preview gc --only images,cache --execute
```

The command never deletes branches, rewrites history, forces removal or pushes. By default it handles:

- **Worktrees** whose branch is merged into `origin/dev`, whose every commit has an equivalent there, or whose pull request was squash-merged at exactly that commit. A worktree with uncommitted changes, one without commits of its own, the one you run it from, and every worktree with unmerged commits are kept. For unmerged worktrees it only deletes `target` and `node_modules` that nothing touched for 24 hours, and prints a note when their commits were never pushed.
- **Preview candidates** beyond the newest three whose tasks are merged or whose build failed. Their ports stay reserved.
- **Tracker images** beyond the newest two, except images that a container uses.
- **Volumes** that no container uses and that belong to retired Genie stacks or are anonymous. `genie-tracker_*` and unrelated volumes are never touched.
- **Docker build cache** beyond 8 GB.

Run it after merging a PR and whenever `df` looks tight. Pass `--keep-images`, `--keep-candidates`, `--cache-gb` or `--idle-hours` to change a limit.
