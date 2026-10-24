# Redesign task breakdown

Generated from [tasks.json](tasks.json) by [build-tasks.mjs](build-tasks.mjs); edit the JSON, then run `node docs/assets/brand/handoff/build-tasks.mjs`. Read the [handoff README](README.md) first. Briefs for each screen are in [screens.md](screens.md).

29 tasks in one epic, about 71 agent hours of work. With independent lanes running in parallel the longest dependency chain is about 28.5 hours (H00 > H01 > H02 > H03 > H05a > H05c > H06 > H07 > H14 > H30 > H33). Estimates are rough (plus or minus 50 percent) and measure agent working time, not elapsed days; review, decisions, and CI waits come on top.

## Creating the tracker tasks

The tracker is the unit of record (see [the CLI guide](../../../development/genie-cli.md)). Preview the commands, then create the epic and tasks against the confirmed project:

```bash
node docs/assets/brand/handoff/build-tasks.mjs --genie     # dry run: prints genie commands
GENIE_PROJECT=genie node docs/assets/brand/handoff/build-tasks.mjs --create
```

Creation uses the caller's rights: people create inbox tasks, agents create drafts. Dependencies are passed with `--dep`, so tasks start only when their predecessors close.

## Plan at a glance

| Key | Phase | Task | Hours | Depends on |
|---|---|---|---|---|
| H00 | Prepare | Build the verification harness: seeded throwaway server and screenshot script | 3 | none |
| H01 | Prepare | Split app/styles.css by area with no visual change | 1.5 | H00 |
| H02 | Foundation | Fonts, tokens, theme switch, and legacy compatibility layer | 2 | H00, H01 |
| H03 | Foundation | Replace hard-coded colours with tokens | 3 | H01, H02 |
| H04 | Foundation | Status, priority, and epic icon set | 1.5 | H02 |
| H05a | Foundation | Core components: actions and navigation controls | 2 | H02, H03 |
| H05b | Foundation | Core components: forms | 1.5 | H05a |
| H05c | Foundation | Core components: overlays and feedback | 1.5 | H05a |
| H05d | Foundation | Core components: data display and reading | 1.5 | H05a |
| H06 | Foundation | Shell: top bar, navigation, status dock, profile menu | 3 | H05a, H05c |
| H07 | Foundation | Character component and state hook | 2.5 | H05c, H06 |
| H10 | Screens (redesign) | Board: lanes, owner lane, state headline | 3 | H04, H05d, H06, H07 |
| H11 | Screens (redesign) | Task list | 1.5 | H04, H05a, H05d, H06 |
| H12 | Screens (redesign) | Task detail: panel and full page | 4.5 | H05b, H05c, H05d, H06 |
| H13 | Screens (redesign) | Epics list and epic page | 2 | H04, H05d, H06 |
| H14 | Screens (redesign) | Team view and agent chat | 5 | H05b, H05c, H05d, H07 |
| H15 | Screens (redesign) | Documentation: tree, reader, editor, search | 3.5 | H05b, H05d, H06 |
| H16 | Screens (redesign) | Agents catalogue | 3 | H05a, H05b, H05d, H06 |
| H17 | Screens (redesign) | Platform pages: answer, notifications, automations, proposals | 1.5 | H05c, H05d, H07 |
| H18 | Screens (redesign) | Authentication, invitation, and first project | 1 | H05b, H07 |
| H19 | Screens (redesign) | Dialogs: palette, new task, owner decision, ideas, teams, artifacts, model menu | 3.5 | H05a, H05b, H05c |
| H20 | Screens (reskin) | Settings: personal (account, sign-in, Telegram, keys, tokens) | 1.5 | H05a, H05b, H05d, H06 |
| H21 | Screens (reskin) | Settings: project (general, repositories, tools, people, channels, knowledge) | 2 | H20 |
| H22 | Screens (reskin) | Settings: server (projects, accounts, providers, tools, health, vault) | 3.5 | H20 |
| H23 | Screens (reskin) | Statistics and costs | 1.5 | H05d |
| H30 | Close-out | Phone and tablet pass | 3 | H10, H11, H12, H13, H14, H15, H16, H17, H18, H19 |
| H31 | Close-out | Accessibility audit | 3 | H10, H11, H12, H13, H14, H15, H16, H17, H18, H19, H20, H21, H22, H23 |
| H32 | Close-out | Documentation site, README hero, and app icon | 3 | H02 |
| H33 | Close-out | Cleanup, standards, and release notes | 2 | H30, H31, H32, H21, H22, H23 |

| Phase | Hours |
|---|---|
| Prepare | 4.5 |
| Foundation | 18.5 |
| Screens (redesign) | 28.5 |
| Screens (reskin) | 8.5 |
| Close-out | 11 |
| **Total** | **71** |

## Parallel lanes

Tasks in the same wave have no dependency on each other. Work in a wave can proceed in separate worktrees; the screen tasks touch disjoint files once `H01` has split the shared stylesheet.

- **Wave 1:** H00 (3h)
- **Wave 2:** H01 (1.5h)
- **Wave 3:** H02 (2h)
- **Wave 4:** H03 (3h), H04 (1.5h), H32 (3h)
- **Wave 5:** H05a (2h)
- **Wave 6:** H05b (1.5h), H05c (1.5h), H05d (1.5h)
- **Wave 7:** H06 (3h), H19 (3.5h), H23 (1.5h)
- **Wave 8:** H07 (2.5h), H11 (1.5h), H12 (4.5h), H13 (2h), H15 (3.5h), H16 (3h), H20 (1.5h)
- **Wave 9:** H10 (3h), H14 (5h), H17 (1.5h), H18 (1h), H21 (2h), H22 (3.5h)
- **Wave 10:** H30 (3h), H31 (3h)
- **Wave 11:** H33 (2h)

## Rules for the whole team

- One task, one worktree, one branch from `origin/main` (see [AGENTS.md](../../../../AGENTS.md)); small PRs, English titles and descriptions.
- Do not edit the same file as another task in flight. Shared files (`g-tokens.css`, `components.css`, `app/styles.css` after the split) change only in the foundation tasks; a screen task that needs a new token or component variant opens a small PR against the foundation first.
- Capture screenshots with the harness (`H00`) before and after every UI change, in both themes and both widths, and attach them to the task.
- The orange rule, the Unbounded size floor, and the no-hard-coded-colours rule are checked in review; see the checklist in [principles.md](../principles.md).
- Behaviour does not change. If a redesign needs a behaviour or contract change, stop, record it on the task, and ask the owner.

## Tasks

### Phase 0: Prepare

#### H00: Build the verification harness: seeded throwaway server and screenshot script

Hours: 3 · Priority: 1 · Depends on: none · Wave 1

Agents cannot judge a redesign without seeing it. Build a repeatable way to run the web UI against realistic data and capture screenshots of every route in both themes. Start a throwaway Genie server (cargo run -p genie -- serve with a temporary data directory) and seed it through the genie CLI or the HTTP API with a project containing tasks in all eleven statuses, epics with children, tasks with acceptance criteria and PR/CI data, owner decisions, notifications, documents, and as many teams and mailboxes as can be created without live agents (record the gap if teams need fixtures). Capture screenshots with Playwright using the preinstalled Chromium (PLAYWRIGHT_BROWSERS_PATH, executablePath /opt/pw-browsers/chromium; do not run playwright install). Check whether npm ci passes on a clean checkout before adding any dependency; the lock file may be out of sync.

**Scope:** web/scripts/ (new); package.json scripts; .gitignore; docs/development.md

**Acceptance criteria**

- One documented command seeds a throwaway server and prints its URL; it never touches the local tracker or real data
- One command captures PNGs for every route listed in handoff/screens.md in ink and paper at 1440x900 and 390x844, with animations disabled and a fixed clock, into an ignored output folder
- The set of baseline screenshots of the current UI is captured and attached to the task before any redesign begins
- The seed covers all eleven statuses, epics, owner decisions, notifications and documents; known gaps (for example teams) are listed in docs/development.md
- docs/development.md explains how to run the harness
- A PR is published and CI passes for its latest head

**Plan:** Investigate server startup and seeding via the ops catalog; write the seed script; write the Playwright capture script; capture baselines; document.

#### H01: Split app/styles.css by area with no visual change

Hours: 1.5 · Priority: 1 · Depends on: H00 · Wave 2

app/styles.css is one 1,144-line file shared by every screen, which would make parallel redesign work conflict. Split it by its own section markers (shell, list, board, detail, team, overlays, mobile, additions, epics, docs, filters, task preview) into per-area stylesheets next to the code that uses them (FSD layers), imported where used. Keep selectors, order-dependent cascade, and values identical.

**Scope:** web/src/app/styles.css; web/src/widgets/*/ui/*.css; web/src/pages/*/ui/*.css; web/src/app/main.tsx

**Acceptance criteria**

- styles.css contains only the root variables, resets and genuinely global rules
- Each area stylesheet is imported by the component that owns it
- Screenshots before and after are pixel-identical for all routes and both viewports
- npm run typecheck, npm run lint:web and npm run build:web pass
- A PR is published and CI passes for its latest head

**Plan:** Map every rule to an owner; move in small commits; diff screenshots after each move; fix cascade-order regressions.

### Phase 1: Foundation

#### H02: Fonts, tokens, theme switch, and legacy compatibility layer

Hours: 2 · Priority: 1 · Depends on: H00, H01 · Wave 3

Make the design tokens live in the app. Self-host Unbounded and Manrope (variable, Latin and Cyrillic subsets, OFL) through the build instead of Google Fonts, so a self-hosted or offline Genie renders correctly. Import g-tokens.css and components.css. Add a theme switch: data-theme on the html element with values system, ink and paper, persisted as a preference, with a matching theme-color meta. Add a compatibility block that maps every legacy variable (--bg, --text, --muted, --accent, --amber, --line, ...) to the new tokens so existing CSS adopts the palette immediately. See design-system.md and g-tokens.css.

**Scope:** web/index.html; web/package.json; web/src/app/main.tsx; web/src/app/styles.css; web/src/shared/ui/g-tokens.css; web/src/shared/lib (theme preference)

**Acceptance criteria**

- No request to fonts.googleapis.com remains; fonts load from the bundle and work offline
- The app renders in the new palette in both themes; no text is unreadable (verified with the harness screenshots)
- A theme preference (system, ink, paper) persists and applies before first paint without a flash
- Every legacy variable is aliased in one clearly marked compatibility block
- test/design-tokens.test.ts passes; the legacy block is listed for removal in H33
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Add font packages; import styles; theme bootstrap script in index.html; write the alias block; verify with screenshots.

#### H03: Replace hard-coded colours with tokens

Hours: 3 · Priority: 1 · Depends on: H01, H02 · Wave 4

About 190 hard-coded hex values in CSS and about 30 in TSX (for example 20 in entities/task StatusIcon, plus agent-config graph, team view, settings) prevent the paper theme and the palette. Replace each with a semantic token; add tokens (with test pairs) where none fits. Colours set through inline style props move to classes.

**Scope:** web/src/**/*.css; web/src/**/*.tsx; web/src/shared/ui/g-tokens.css; test/design-tokens.test.ts

**Acceptance criteria**

- A grep for hex colours outside g-tokens.css and the brand SVG assets returns nothing
- No inline style sets a colour
- New colour tokens add contrast pairs to test/design-tokens.test.ts and the test passes
- Both themes show no regressions in the harness screenshots
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** List all hex occurrences by area; map each to a token; convert area by area; run the screenshot diff.

#### H04: Status, priority, and epic icon set

Hours: 1.5 · Priority: 1 · Depends on: H02 · Wave 4

Replace the eleven status icons (a different hue per status today) with the shape-first set from components.html: dashed rings for pre-work, a ring for ready, a half-filled ring for in progress, a return arrow for changes requested, a ring with a dot for review, filled marks for approved, needs owner and done, a slashed ring for cancelled. Replace priority icons with the ascending bars and the orange urgent square, and the epic icon with the arcs miniature. Roles: neutral, accent (green), signal (orange). Keep aria labels and the StatusIcon, PriorityIcon, EpicIcon exports.

**Scope:** web/src/entities/task/ui/StatusIcon.tsx; web/src/entities/task/ui/Epic.tsx; web/src/entities/task/model.ts; web/src/shared/ui/icons.tsx; test/ (new node:test for the status mapping)

**Acceptance criteria**

- Every status and priority renders the specified shape and colour role in both themes
- Statuses are distinguishable in greyscale
- A node:test asserts the status to (shape, role) mapping so every Status variant is covered
- Aria labels and sizes unchanged for all call sites
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Port the sprite to React; add the mapping and its test; update call sites; check greyscale.

#### H05a: Core components: actions and navigation controls

Hours: 2 · Priority: 1 · Depends on: H02, H03 · Wave 5

Implement React wrappers over components.css for Button (primary, secondary, ghost, danger, sm, busy), IconButton, Tabs, Segmented, Chip, Tag, Menu (checkbox and radio items, keyboard navigation, focus return), Tooltip, Kbd and Breadcrumbs in shared/ui. Replace the legacy .btn, .icon-btn, .seg, .pill, menu and tooltip usages across the app. Follow WAI-ARIA patterns inside the component, not at call sites.

**Scope:** web/src/shared/ui/; legacy class usages across web/src

**Acceptance criteria**

- Each component matches components.html in both themes
- Menus and tabs implement the ARIA pattern with full keyboard support and focus return
- No legacy .btn, .icon-btn, .seg or .pill class remains in TSX
- A node:test covers the pure logic (menu navigation or similar) where it exists
- Screenshots show no layout regressions
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Build components; migrate usages area by area; review screenshots; remove the legacy rules.

#### H05b: Core components: forms

Hours: 1.5 · Priority: 1 · Depends on: H05a · Wave 6

Field with label, help and error; Input, Select, Textarea, Checkbox, Radio, Switch and Search in shared/ui over components.css, using native elements. Replace the ad hoc form markup across dialogs, task detail, and settings with these.

**Scope:** web/src/shared/ui/; form markup across web/src

**Acceptance criteria**

- Every state in components.html (default, hover, focus, invalid with message, disabled) works in both themes
- Labels are associated with controls; errors are announced with aria-describedby
- Form behaviour and validation are unchanged
- No form control outside shared/ui sets its own border, radius or colour
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Build field components; migrate forms; verify keyboard and error announcements.

#### H05c: Core components: overlays and feedback

Hours: 1.5 · Priority: 1 · Depends on: H05a · Wave 6

Modal (with the 20 existing uses), ConfirmDialog, Toast stack, Banner, Skeleton, EmptyState (title, line, optional action, optional character slot) and an ErrorBlock with retry. Modals trap and restore focus and close on Escape. Replace every bare "Loading..." with a skeleton or the working arcs.

**Scope:** web/src/shared/ui/Modal.tsx; web/src/shared/ui/ConfirmDialog.tsx; web/src/shared/ui/toast.tsx; new Banner, Skeleton, EmptyState, ErrorBlock; PageFallback in web/src/app/App.tsx

**Acceptance criteria**

- All 20 modal usages use the new Modal with correct focus trap and restore
- Toasts live in one polite live region
- No bare "Loading..." text remains; every page has a skeleton or arcs loading state
- EmptyState and ErrorBlock are used by at least the board, task list and docs
- Both themes verified
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Build and migrate Modal first; then feedback components; then replace loading and empty states.

#### H05d: Core components: data display and reading

Hours: 1.5 · Priority: 1 · Depends on: H05a · Wave 6

Card and Sheet, Row and GroupHead, Table, Avatar and Avatars (agent square, human circle), Progress and Segments, the working arcs indicator, and Prose (rendered Markdown: restyle shared/ui/Markdown.tsx and markdown.css on g-prose, including code and tables). BarChart takes its colours from the viz tokens and prints values.

**Scope:** web/src/shared/ui/; web/src/entities/member (Avatars); web/src/shared/ui/Markdown.tsx; web/src/shared/ui/markdown.css; web/src/shared/ui/BarChart.tsx

**Acceptance criteria**

- Components match components.html in both themes
- Markdown output uses g-prose with Unbounded h1 and h2, 70ch measure, and scrollable code and tables
- Avatars distinguish agents from people by shape everywhere they appear
- BarChart uses --g-viz-* and prints a value at every bar
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Build components; restyle Markdown and charts; migrate avatar usages.

#### H06: Shell: top bar, navigation, status dock, profile menu

Hours: 3 · Priority: 1 · Depends on: H05a, H05c · Wave 7

Replace the left sidebar with the top bar specified in handoff/screens.md section 0: logo and project switcher, five primary pills, More menu, search pill, New task, bell, theme, profile menu (settings, management, statistics, logout by role), the Working-now status dock with its popover, an offline banner, and a phone bottom-sheet menu. Rebuild the Settings and Management shell with a section rail and a back-to-work action. Keep every route, redirect and shortcut.

**Scope:** web/src/widgets/sidebar/; web/src/widgets/settings-shell/; web/src/app/App.tsx; web/src/app/styles (shell rules)

**Acceptance criteria**

- All navigation destinations from the old sidebar are reachable per the mapping table
- Every shortcut (C, /, Cmd+K, G then B/T/E/S, G then M/I/D/A/P/C, Esc) still works
- Role rules for management items are unchanged
- The waiting-decision counter and unread dot show on the right items
- The Working-now dock lists the same teams as before with their state
- Phone layout uses the bottom sheet; the offline banner appears when the live stream drops
- Both themes verified at 1440 and 390
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Build the bar; port the data hooks from Sidebar; build the dock popover; rebuild the settings shell; remove the grid classes; verify routes and shortcuts.

#### H07: Character component and state hook

Hours: 2.5 · Priority: 2 · Depends on: H05c, H06 · Wave 8

Bring the character into the app. Make the four animated SVGs from docs/assets/brand/character/ available to the bundle (import as assets), add a Genie component (state, size, alt) that respects reduced motion, and a pure function plus hook that derives the state from teams and tasks: thinking when any member is working, sleeping when nothing is active, waking transiently when a team becomes active, surprised when a task returns to changes requested, CI fails, or a decision arrives. Use it in the status dock and the shared EmptyState. If the live event stream lacks a needed signal, degrade to thinking and sleeping and document the gap; do not change server contracts in this task.

**Scope:** web/src/shared/ui/ (Genie); web/src/entities/team or session (state derivation); test/ (node:test for the pure function)

**Acceptance criteria**

- A pure function maps (teams, tasks, recent events) to a state and has node:test coverage for each state and for precedence
- The component shows the static neutral pose under prefers-reduced-motion
- The dock and EmptyState show the character in the right state
- No more than one character per view; never inside lists or toolbars
- Documented list of signals used and any missing
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Asset import; component; pure mapping with tests; hook over query data; integrate dock and empty state.

### Phase 2: Screens (redesign)

#### H10: Board: lanes, owner lane, state headline

Hours: 3 · Priority: 1 · Depends on: H04, H05d, H06, H07 · Wave 9

Build the board from concepts/board.html on the real data: state headline, arcs motif, status strip with the character, filter chips, the orange needs-decision lane, three stage lanes with sheets, and the Done strip. Keep dnd-kit drag and drop and add drop chips for the exact statuses of a lane while dragging (see screens.md section 1). Cards show exact status tags, acceptance segments, agent and human avatars, and working arcs. Phone: one lane per screen with a lane switcher and a Move-to menu.

**Scope:** web/src/pages/board/; web/src/widgets/board/; web/src/entities/task (COLUMNS to lanes mapping, with tests)

**Acceptance criteria**

- The board matches the concept in both themes and shows live data
- Dragging to a lane offers the exact statuses; every move the old board allowed is still possible, with the same server rules and note prompts
- Keyboard dragging works; a Move-to menu exists for phones and keyboards
- URL parameters, mine and show-done preferences, and live invalidation behave as before
- A node:test covers the lane to status mapping
- Loading, empty, error, filtered-empty, and offline states are designed and seen
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Lane model and mapping with tests; layout; cards; drag semantics; states; phone layout.

#### H11: Task list

Hours: 1.5 · Priority: 2 · Depends on: H04, H05a, H05d, H06 · Wave 8

Redesign the task list per screens.md section 2: filter chips opening menus, the result headline, g-row rows with status icon, acceptance segments, assignee, and age, grouped by status. Keep the filter model, URL format and keyboard navigation.

**Scope:** web/src/pages/tasks/; web/src/widgets/task-list/

**Acceptance criteria**

- Existing web-task-filter and view tests pass unchanged
- j, k, arrows, Enter, / and the focus ring work as before
- Filters, order, groups and reset behave as before; state stays in the URL
- Loading, empty, filtered-empty, and error states exist
- Phone filters open as a bottom sheet
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Restyle toolbar and rows; headline; states; phone sheet.

#### H12: Task detail: panel and full page

Hours: 4.5 · Priority: 1 · Depends on: H05b, H05c, H05d, H06 · Wave 8

Redesign the task panel and the task page per screens.md section 3: headline row, property rail, g-prose description, plan and notes, the owner-decision block as the page's orange region, acceptance criteria with a progress header, delivery with PR and CI states, activity timeline, artifacts grid, and the comment composer. Keep both variants, tabs, and the team tab.

**Scope:** web/src/widgets/task-detail/; web/src/pages/task/; web/src/features/owner-decision/; web/src/features/follow-task/

**Acceptance criteria**

- Panel and page work with all permission levels; read-only users see text, not controls
- Owner-decision flows, questionnaires, and clarification tasks behave as before
- CI failure is shown as a banner; waiting on dependencies and cancelled states are shown
- Comment shortcut, follow, delete with confirmation, and artifact viewing work
- No team shows the empty state with the spawn action
- Phone shows a full-screen page with an accordion of properties
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Header and rail; main column; owner decision; delivery; activity; composer; states; phone.

#### H13: Epics list and epic page

Hours: 2 · Priority: 2 · Depends on: H04, H05d, H06 · Wave 8

Redesign per screens.md section 4: epics as cards with the arcs progress motif and an Unbounded open count; the epic page with a headline, goal as the lede, children grouped by status, the aside as a property rail, artifacts, and activity.

**Scope:** web/src/pages/epics/; web/src/pages/epic/

**Acceptance criteria**

- Moving tasks between epics and adding artifacts work as before
- Empty, no-children, and ready-to-close states exist
- The owner-decision box uses the signal treatment
- Both themes and phone verified
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Cards; page layout; states.

#### H14: Team view and agent chat

Hours: 5 · Priority: 2 · Depends on: H05b, H05c, H05d, H07 · Wave 9

Redesign per screens.md section 5: the team mail log and composer, member cards with state and working arcs, the journal aside, the agent conversation feed with collapsible tool steps and the quote-a-step action, the correction composer, pause and idea-plan asides, and the scheme graph in viz colours. The character shows the agent's state in the chat header.

**Scope:** web/src/widgets/team-view/; web/src/pages/agent/; web/src/pages/team/; web/src/features/manage-team/; web/src/features/spawn-team/

**Acceptance criteria**

- Streaming, auto-scroll sticking, mail levels including interrupt, quoting, pause and resume behave as before
- Tool steps are collapsed by default with status and duration and expand to code blocks
- Long outputs collapse behind a Show more control; images use the existing preview
- Aria roles and labels (log, tablist, radiogroup) are preserved
- Phone uses a bottom sheet for the aside and a pinned composer
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Team view; member cards; agent feed and steps; composers; asides; scheme; phone.

#### H15: Documentation: tree, reader, editor, search

Hours: 3.5 · Priority: 2 · Depends on: H05b, H05d, H06 · Wave 8

Reskin and retype per screens.md section 6: the tree as dense rows with type icons, the reader in g-prose, meta as tags, diagnostics as banners, the editor with a preview toggle, the search list, the new-document dialog, the phone tree drawer, and the Proposals segment in the header. Move the docs rules out of app/styles.css (after H01).

**Scope:** web/src/pages/docs/; web/src/entities/doc/; web/src/shared/ui/Markdown.tsx

**Acceptance criteria**

- Wiki-links, backlinks, search modes and the page, q, new, title parameters work as before
- Edit routes and proposals are reachable; the drawer works on phones
- Reading measure is at most 70 characters and tables and code scroll inside their block
- Empty vault, loading, failed, no results, and deprecated-page states exist
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Tree; reader typography; meta and banners; editor; search; drawer; states.

#### H16: Agents catalogue

Hours: 3 · Priority: 3 · Depends on: H05a, H05b, H05d, H06 · Wave 8

Reskin per screens.md section 7: tabs, list and detail panes, forms and dialogs, the template graph in viz colours, read-only presentation. Move agents.css onto the shared components.

**Scope:** web/src/pages/agents/; web/src/widgets/agents-kit/; web/src/entities/agent-config/; web/src/shared/ui/agents.css

**Acceptance criteria**

- Editing rules (administrator only), file editing, history and permission fields behave as before
- The template graph uses viz tokens and agent shapes and is legible in both themes
- Query parameters tab, id and q work
- Read-only mode is visibly read-only
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Shell and tabs; panes; forms and dialogs; graph; read-only mode.

#### H17: Platform pages: answer, notifications, automations, proposals

Hours: 1.5 · Priority: 2 · Depends on: H05c, H05d, H07 · Wave 9

Per screens.md section 8. The Answer page (outside the shell) becomes a brand moment with the surprised character, one sheet per question, and a single orange submit. Notifications get day groups with unread dots and an empty state. Automations and Proposals are split views.

**Scope:** web/src/pages/platform/

**Acceptance criteria**

- The Answer page works from a cold link, handles the closed state, and submits as before
- Notifications mark-read behaviour is unchanged
- Each page has loading, empty and error states
- Both themes and phone verified
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Answer page; notifications; automations; proposals.

#### H18: Authentication, invitation, and first project

Hours: 1 · Priority: 2 · Depends on: H05b, H07 · Wave 9

Per screens.md section 9: a full-bleed brand layout with the display slogan and orange stop, the arcs motif cropped at the corner, the form in a card, and the character. Login, invite, and first-project pages share the layout.

**Scope:** web/src/pages/auth/; web/src/pages/settings/ui (FirstProjectPage)

**Acceptance criteria**

- All existing notes and errors are shown, including server unavailable and no project access
- Both themes and phone verified; the form is fully keyboard operable
- The slogan is a single Unbounded headline of at most six words with the orange stop
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Layout; login; invite; first project.

#### H19: Dialogs: palette, new task, owner decision, ideas, teams, artifacts, model menu

Hours: 3.5 · Priority: 2 · Depends on: H05a, H05b, H05c · Wave 7

Per screens.md section 10, apply the shared overlay components to every dialog and menu and finish their content: command palette with grouped results, new task with presets and idea mode, owner decision with questionnaires, idea shaping, spawn and manage team, add artifact, model menu and picker, artifact modal and confirmations.

**Scope:** web/src/features/*/; web/src/entities/task/ui/ArtifactModal.tsx; web/src/shared/ui/ConfirmDialog.tsx

**Acceptance criteria**

- Each dialog traps and restores focus, closes on Escape, and has a labelled title
- The palette navigates by arrow keys and supports every action it had
- The single orange button in each dialog is the confirming action
- Existing web-idea-plan and web-owner-decision tests pass
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Palette; new task; owner decision; the rest; verify keyboard.

### Phase 3: Screens (reskin)

#### H20: Settings: personal (account, sign-in, Telegram, keys, tokens)

Hours: 1.5 · Priority: 3 · Depends on: H05a, H05b, H05d, H06 · Wave 8

Reskin the Me section per screens.md section 11 on the shared components. Secrets are shown in a monospace block with a copy action. No layout redesign.

**Scope:** web/src/pages/settings/ui (MeSettings, TelegramSettings, TokensTab, ProviderConnectDialog); web/src/shared/ui/settings.css

**Acceptance criteria**

- Every section reachable and functional as before
- Secret masking and token creation flows unchanged
- Both themes and phone verified
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Section rail; forms; token and key lists; dialogs.

#### H21: Settings: project (general, repositories, tools, people, channels, knowledge)

Hours: 2 · Priority: 3 · Depends on: H20 · Wave 9

Reskin the project management sections: general, repositories and environment, tools (skills, MCP, Jev, automations), access and roles, channels, knowledge base. Keep role gating and the legacy redirects.

**Scope:** web/src/pages/settings/ui (ProjectSettings, RepositoriesSection, AutomationsSettings); web/src/shared/ui/project.css

**Acceptance criteria**

- Role gating (project administrators and server administrators) unchanged
- Repository connection flows and Git token management unchanged
- All tabs from SETTINGS_TABS render and work
- Both themes and phone verified
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Rail and tabs; forms; repository section; tables; dialogs.

#### H22: Settings: server (projects, accounts, providers, tools, health, vault)

Hours: 3.5 · Priority: 3 · Depends on: H20 · Wave 9

Reskin the server administration sections, including the large McpAdmin (about 880 lines), ServerSettings (about 700), ProvidersAdmin, and the agent administration routes (roles, templates, skills, history).

**Scope:** web/src/pages/settings/ui (McpAdmin, ServerSettings, ProvidersAdmin, AgentsAdminPage); web/src/widgets/agents-kit/

**Acceptance criteria**

- Server administrator gating unchanged
- MCP editing, provider connection, account creation and role assignment unchanged
- Every section and tab renders and works
- Both themes and phone verified
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Projects and accounts; providers; tools and MCP; health and vault; agent administration.

#### H23: Statistics and costs

Hours: 1.5 · Priority: 3 · Depends on: H05d · Wave 7

Per screens.md section 12: headline numerals, charts on the viz tokens with direct value labels, tables for breakdowns, and a time-range segment. Administrator only, as today.

**Scope:** web/src/pages/settings/ui/ServerCosts.tsx; web/src/pages/settings (ServerStatisticsPage); web/src/entities/usage/; web/src/shared/ui/BarChart.tsx

**Acceptance criteria**

- At most four series per chart, direct-labelled, with 3:1 contrast in both themes
- web-stats test passes unchanged
- Empty and loading states exist
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Numerals; charts; tables; range control; states.

### Phase 4: Close-out

#### H30: Phone and tablet pass

Hours: 3 · Priority: 2 · Depends on: H10, H11, H12, H13, H14, H15, H16, H17, H18, H19 · Wave 10

Audit every redesigned screen at 390 and 768 widths and fix what the per-screen tasks missed: overflow, touch target sizes, bottom sheets, safe-area insets, horizontal scroll of the board, long text, and keyboard overlap. Record before and after screenshots.

**Scope:** web/src (responsive rules)

**Acceptance criteria**

- No horizontal page scroll at 390 and 768 on any route
- All phone controls are at least 40px high and rows at least 44px
- The board, task panel, team view, docs tree, and palette work on a touch device
- Screenshots for every route are attached
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Capture; list defects; fix by area; recapture.

#### H31: Accessibility audit

Hours: 3 · Priority: 2 · Depends on: H10, H11, H12, H13, H14, H15, H16, H17, H18, H19, H20, H21, H22, H23 · Wave 10

Audit all screens for keyboard operation, focus visibility and order, roles and labels, live regions, reduced motion, and colour independence. Add an automated check to the harness (axe-core through Playwright) if it can be added without a heavy dependency. Fix findings. Confirm every colour token has contrast pairs in test/design-tokens.test.ts.

**Scope:** web/src (fixes); web/scripts (a11y check); test/design-tokens.test.ts

**Acceptance criteria**

- A documented keyboard-only walkthrough of the main flows (create task, answer a decision, move a card, read a doc) succeeds
- No critical or serious automated findings remain
- prefers-reduced-motion removes all non-essential motion, verified per screen
- Every colour token is covered by a contrast pair
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Manual walkthroughs; automated scan; fix; document results.

#### H32: Documentation site, README hero, and app icon

Hours: 3 · Priority: 3 · Depends on: H02 · Wave 4

Apply the brand outside the app: the VitePress theme (colours, Unbounded display headings, Manrope body, the new logo lockups, favicon), the README hero SVG, and the application favicon and theme colour. Replace the old periwinkle lettermark usages with the new logo after confirming the draft vector with the owner; update docs/assets/brand/README.md.

**Scope:** docs/.vitepress/theme/; docs/public/; docs/assets/hero.svg; README.md; web/index.html (icon)

**Acceptance criteria**

- The docs site builds (npm run docs:check and docs:build) and renders in the new palette
- The README hero and favicon use the new logo
- The brand README states which logo is current and where the vector source lives
- Docs remain English and links check
- Both themes verified at 1440 and 390 widths, with screenshots attached to the task
- No hard-coded colours, font families, or legacy design variables introduced; the pull request checklist in principles.md is satisfied
- A PR is published and CI passes for its latest head

**Plan:** Theme variables; fonts; logo swap; hero; checks.

#### H33: Cleanup, standards, and release notes

Hours: 2 · Priority: 3 · Depends on: H30, H31, H32, H21, H22, H23 · Wave 11

Remove the legacy compatibility variables and unused classes and CSS, Geist, and the sidebar remnants; update CODING_STANDARDS.md Web UI section with the design-system rules (tokens only, g- components, themes, density); update docs and the changelog; produce the final before and after screenshot set.

**Scope:** web/src (dead code); CODING_STANDARDS.md; CHANGELOG.md; docs/

**Acceptance criteria**

- No legacy design variable, class, or font remains; a grep proves it
- CODING_STANDARDS.md points to the design system and principles
- CHANGELOG.md has an entry for the redesign
- Typecheck, lint, tests, and build pass
- A PR is published and CI passes for its latest head

**Plan:** Grep for leftovers; delete; update documents; final screenshots.
