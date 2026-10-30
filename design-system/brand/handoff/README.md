# Redesign handoff

Everything an agent team needs to redesign the Genie web UI in the new "ink and paper" visual language. Start here; every other document is linked from this page. Status: **ready for the owner to review and start**; nothing in the application has changed yet.

## Mission

Move the whole web UI to the new visual language without changing what it does. The result should be a UI that feels like the concept in [`concepts/board.html`](../concepts/board.html): calm when work goes well, unmistakable when the owner is needed, with a character that reports what the agents are doing.

In scope: the shell and navigation, the board, task list, task detail, epics, team view and agent chat, documentation, the agents catalogue, platform pages (answer, notifications, automations, proposals), authentication, dialogs, Settings and Management, statistics, the documentation site, and the README hero.

Out of scope: server behaviour, API contracts, routes and URL parameters, keyboard shortcuts, permissions, data models, and the Russian product terminology. If a redesign appears to need one of these, stop and ask.

## Reading order

1. This page.
2. [Design principles](../principles.md): the rules, anti-patterns, and the pull request checklist.
3. [Design system](../design-system.md): colour, type, shape, motif, logo, character, board concept.
4. [Components](../components.html): every component and state in both themes (open it from a checkout). Source: [`components.css`](https://github.com/grigoryshulga/genie/blob/dev/web/src/shared/ui/components.css) and [`g-tokens.css`](https://github.com/grigoryshulga/genie/blob/dev/web/src/shared/ui/g-tokens.css).
5. [Board concept](../concepts/board.html), [character animations](../character/animations.html), and [brand specimen](../design-system.html).
6. [Screen briefs](screens.md): what each current screen is, what it becomes, what to preserve.
7. [Task breakdown](tasks.md): 29 tasks with dependencies, acceptance criteria, and parallel lanes.

## Deliverables already in the repository

| What | Where |
|---|---|
| Design tokens (two themes, chart colours, layers, control sizes) | `web/src/shared/ui/g-tokens.css` |
| Component styles, reference implementation | `web/src/shared/ui/components.css` |
| Contrast contract (text, controls, focus, charts) | `test/design-tokens.test.ts` |
| Principles and checklist | `design-system/brand/principles.md` |
| Component and state specimen | `design-system/brand/components.html` |
| Board concept | `design-system/brand/concepts/board.html` |
| Logo lockups (draft vector) | `design-system/brand/logo-*.svg` |
| Arcs motif | `design-system/brand/arcs.svg` |
| Character animations and generator | `design-system/brand/character/` |
| Screen briefs, tasks, task creation script | `design-system/brand/handoff/` |

None of the new files is imported by the application yet. The first foundation tasks wire them in.

## Integration branch and existing foundations (read before H02)

- **Work lands in `dev`.** Branch from `origin/dev`, open PRs against `dev`, and obtain the owner's visual approval of a preview candidate before publishing a UI change (see `https://grigoryshulga.github.io/genie/development/visual-preview`). `main` only receives release PRs. This overrides the `origin/main` wording elsewhere in this handoff.
- **`web/src/shared/ui/tokens.css` was the GNE-25 palette** (legacy `--bg`, `--text`, `--accent` names, accent choices, and a Figma exporter in `scripts/design-system/`). H02 aliased the legacy names to the `--g-` tokens in `compat.css`, and H33 (GNE-69) removed both files together with the accent choices, so `g-tokens.css` is now the only token source and `scripts/design-system/export.mjs` reads it directly.
- **`shared/lib/appearance.ts` and `preferences.ts` already store appearance preferences.** H02's theme switch extends them instead of adding a second store.
- **`test/ui` already holds Playwright browser tests and screenshot baselines** (GNE-31), and `npm run ui:capture` (GNE-41, H00) photographs every route. Baselines change deliberately with each UI task.
- **The logo was replaced on `dev`** (`genie-mark.svg`, line-art genie). H32 starts from it.

## Ground rules

- **Follow the [agent guide](https://grigoryshulga.github.io/genie/agents/guide).** One task, one worktree from `origin/main`, tracked in genie with progress comments, small English PRs, and a task is done only after CI passes on the PR's latest head. Do not merge; the owner merges.
- **Create the tracker tasks first.** `node design-system/brand/handoff/build-tasks.mjs --genie` previews the commands, `--create` runs them against the confirmed project (see [tasks.md](tasks.md)). Confirm the project and credentials before creating anything; the repository tracker is project `genie`, prefix `GNE`.
- **Tokens and `g-` components only.** No hard-coded colours, sizes, or font families outside `g-tokens.css`. If something is missing, open a small PR against the foundation, then use it.
- **Do not edit the same file as another task in flight.** The shared stylesheet is split in `H01` so that screens own their CSS.
- **Behaviour is frozen.** Keep every route, URL parameter, shortcut, permission check, live update, and drag and drop path. Existing tests in `test/` must keep passing unchanged unless the task says why.
- **Evidence over claims.** Attach before and after screenshots (both themes, 1440 and 390) to every UI task. Say "verified" only for what was actually rendered.
- **Language.** User-facing text stays Russian and uses the established terms ([glossary](https://grigoryshulga.github.io/genie/glossary)). Code, comments, docs, PRs: English.

## Decisions

### Made (defaults to proceed with)

| Decision | Default |
|---|---|
| Meaning of colour | Green is system state; orange is "your move" (primary action, signal lane, waiting counters); red is failure |
| Navigation | A top bar with five primary pills (Board, Tasks, Epics, Docs, Agents), a More menu, search, New task, bell, theme, profile; Settings keeps its section rail |
| "Working now" | A status dock with the character and a popover list, replacing the sidebar list |
| Board | Three stage lanes plus an orange needs-decision lane and a Done strip; exact statuses stay as tags; drag shows drop chips for exact statuses |
| Settings | Reskin only; structure, tabs, and URLs unchanged |
| Fonts | Unbounded and Manrope, self-hosted through the build (Latin and Cyrillic subsets) |
| Monospace | Geist Mono stays for IDs and code (self-host it too) |
| Default theme | Ink (dark), with a user preference of system, ink, or paper |
| Character | Four states mapped to real agent state; one per view; static under reduced motion |
| Logo | The draft vectors in `design-system/brand/` until the owner supplies the designer's source |

### Open for the owner (do not block on these)

1. Eight columns or three stages on the board (the work is the same; only the mapping differs).
2. Whether "New task" stays a secondary button (default) or becomes the orange primary.
3. The final logo vector, and replacing the current lettermark in the docs navbar, README, and favicon (`H32`).
4. Whether adding a visual-testing dependency (Playwright or axe-core) to `devDependencies` is acceptable; the default is to keep it out of the product dependency graph and run it from scripts.
5. Which character signals exist in the live event stream; if one is missing, whether the server may add it (separate task, Rust and generated types).
6. Headline copy variants for the board (empty, calm, many urgent).

## Environment and verification

- **Run the UI:** `npm run dev:web` proxies `/api` to `http://127.0.0.1:7420` (override with `GENIE_API_PROXY_TARGET`); the repository tracker is `http://127.0.0.1:7422`. Never develop against production data; the harness in `H00` starts a throwaway server with a seeded project.
- **Capture screenshots:** the preinstalled Chromium (`/opt/pw-browsers/chromium`, `PLAYWRIGHT_BROWSERS_PATH` is set) with Playwright; do not run `playwright install`. Disable animations and fix the clock so captures are comparable.
- **Checks before every push** (see `.github/workflows/ci.yml` and `package.json`): `npm test`, `npm run typecheck`, `npm run lint:web`, `npm run build:web`; plus `npm run docs:check` for documentation changes. Rust checks run only when Rust changes.
- **Known issue to confirm first:** `npm ci` failed on a clean checkout in one environment because the lock file lacked `typescript@5.9.3`. Check whether this reproduces in CI before touching dependencies; do not rewrite the lock file as a side effect.
- **Tracker access:** if the native `genie` CLI or the tracker is not reachable, report the exact blocker and keep the work unfinished, as the agent guide requires.

## Suggested team shape

| Lane | Tasks | Notes |
|---|---|---|
| Foundation (1-2 agents) | H00-H07 | Strictly ordered at first; H05b, H05c, H05d run in parallel after H05a |
| Screens (up to 4 in parallel) | H10-H19 | Disjoint files after H01. Suggested split: board and task list; task detail; team and chat; docs and agents |
| Reskin | H20-H23 | One agent, mostly mechanical |
| Verifier | throughout | Reviews screenshots against the concept and the checklist; does H30-H31 |
| Owner | decisions and review | Answers open decisions, accepts tasks, merges |

Review rhythm: foundation tasks one at a time; screens as they finish, with screenshots in the PR. The owner reviews the first redesigned screen (the board) before the other screen lanes start, so taste corrections happen once.

## Risks and watch-outs

- **Drag and drop on the board.** Three-stage lanes change the drop semantics; keyboard and touch sensors must keep working, and every move the old board allowed must still be possible.
- **Live updates.** Pages rely on query invalidation from the event stream (`shared/api/invalidation.ts`); redesigned components must not break it or cause re-render loops.
- **Unbounded is wide.** Long Russian words overflow on phones above roughly 32px. Use the type tokens (they clamp), `text-wrap: balance`, and check 390px.
- **Two themes double the surface.** Check both on every screen; a token missing from the paper theme fails the contrast test.
- **Role-gated UI.** Viewer, member, admin, and server-admin views differ; screenshot at least viewer and admin for screens with gated controls.
- **Character data.** The state hook needs signals that may not exist; degrade gracefully (thinking and sleeping only) rather than inventing activity.
- **Big files.** `TaskDetail` (770 lines), `AgentChat` (675), `McpAdmin` (883), `DocEditor` (459). Restyle in place; extract only when it removes duplication. Do not rewrite logic.
- **No UI tests exist.** The harness and screenshots are the safety net; add `node:test` coverage for any pure logic you introduce (status mapping, lane mapping, character state).

## Definition of done

For a PR: the checklist in [principles.md](../principles.md#pull-request-checklist) is satisfied, screenshots are attached, the standard checks pass, and CI is green on the latest head.

For a task: all acceptance criteria are checked in genie with evidence, the PR URL and verified head SHA and check results are recorded, and the task has moved through review under the project's workflow.

For the epic: every screen in [screens.md](screens.md) is implemented in both themes at both widths, `H31` and `H33` are done, and the owner has accepted the result.
