# Screen briefs

One brief per screen: what exists today (read from the code), the target in the new language, what must not break, and the states to design. Task keys (`H10`, ...) refer to [tasks.md](tasks.md). Read [principles.md](../principles.md) first; component names (`g-btn`, `g-sheet`, ...) are from [components.html](../components.html).

Two kinds of work are mixed in this list on purpose:

- **Redesign** screens are rebuilt in the spirit of [the board concept](../concepts/board.html): new layout, headline-as-state, the character where it fits.
- **Reskin** screens keep their structure and move to the new components, type, and colour. Settings is reskin only.

Anything marked **Decision** has a default the team may proceed with; the owner can overturn it at review.

## 0. Shell and navigation (`H06`)

**Today.** `widgets/sidebar/ui/Sidebar.tsx` (306 lines) renders a left rail: project switcher, context search, "New task" (key `C`), Board / Tasks / Epics with counters (an alert counter for waiting decisions), a Knowledge group (Docs, Proposals), a Team-and-rules group (Agents, Automations, Notifications with an unread dot), a Management group (project settings, global settings, statistics; by role), a "Working now" list of active teams with a status and age, and a footer with the profile, online state, settings, and logout. `App.tsx` turns the rail into a CSS grid (`app`, `with-detail`, `team-view`, `sidebar-compact`). A compact mode is remembered in `localStorage`. `widgets/settings-shell` has its own rail for Settings and Management. Mobile uses `MobileNav`.

**Target.** A top bar replaces the rail ([concept](../concepts/board.html)). Mapping:

| Today | Target |
|---|---|
| Project switcher | Logo area plus project name; click opens a menu of projects (role shown as a caption, viewer marked "read only") |
| Context search | Search pill in the top bar. Its label follows the route (keep `shared/lib/contextSearch.ts`). `/` focuses it; `Cmd/Ctrl+K` opens the command palette |
| New task (`C`) | Secondary button "New task" with a `g-kbd` hint, right side of the bar |
| Board, Tasks, Epics, Docs, Agents | Five primary pills. The selected pill is the only filled one. Tasks carries the waiting-decision counter as a small orange solid tag and the open count as plain text |
| Proposals | Moves into the Docs page header as a segmented control ("Pages / Proposals n"); the route `/docs/proposals` stays |
| Automations | "More" menu in the bar, next to the primary pills |
| Notifications | Bell icon with an unread dot; opens `/notifications` |
| Management (project, server) and Statistics | Profile menu items, shown by the same role rules (`canManageProject`, `isAdmin`) |
| Settings, logout | Profile menu |
| Working now | A status dock in the top bar: the mini character plus "N working"; opens a popover listing active teams with id, short status, age, and an arcs indicator when running or an orange dot when waiting for the owner. Items link to `/team/:id`. The board repeats it as a large strip |
| Online indicator | A dot on the profile avatar; when offline, a `g-banner danger` under the bar ("No connection to the server") |
| Compact sidebar | Removed. Ignore the stored `sidebar-compact` preference |
| Settings and Management rail | Keep a left section rail inside those modes (see Settings) |

**Preserve.** Every route and legacy redirect in `App.tsx`; every keyboard shortcut (`C`, `/`, `Cmd+K`, `G` then `B/T/E/S`, `G` then `M/I/D/A/P/C`, `Esc` closes the task panel); the task detail panel opening beside content (`with-detail`); `rememberWorkRoute`; the lazy-loaded pages; project switching clearing caches as today.

**States.** Offline banner; no projects (first-project page); viewer role (create hidden); long project names (truncate with a title).

**Phone.** The bar becomes logo, search icon, create icon, and a menu icon that opens a bottom sheet with all destinations and the working-now list. The task panel is a full-screen page.

## 1. Board (`H10`, redesign)

**Today.** `pages/board` (106 lines) and `widgets/board` (229 lines). Eight columns from `COLUMNS` in `entities/task/model.ts`: Inbox, Prep (draft and refining), Ready, In progress (in progress and changes requested), Review, Approved, Needs decision, Done (done and cancelled). Cards show id, urgent tag or priority icon, a "rework" mark for changes requested, an epic mark, the title, the open question for needs-owner or the waiting dependencies, epic chip, up to two labels with a "+N", acceptance counter, team avatars, and the assignee. Drag and drop with `@dnd-kit` (pointer, touch with long-press, keyboard), a modal asking for a note on some moves, collapsible columns (Done collapsed unless shown), filters "mine" and epic, preferences in `localStorage`, and a card click opens the task panel (`?task=`).

**Target.** The concept: a headline that states the project (for example "3 decisions are waiting for you." with the orange stop; when nothing waits, a calm variant), the arcs motif, the status strip with the character, filter chips, then lanes:

1. **Needs decision**, orange lane, first. Sheets with the question and answer buttons.
2. **Preparation** (inbox, draft, refining, ready).
3. **Work** (in progress, changes requested).
4. **Review** (review, approved).
5. **Done**, a pill strip with the count.

Each sheet keeps its exact status as a tag; the working arcs show on cards with a running agent; agents are rounded squares and people circles; acceptance criteria are the segmented bar.

**Decision (drag semantics).** Dragging between stages must still set a precise status. While a card is dragged, the target lane expands drop chips for its statuses (Preparation: Inbox, Refining, Ready; Work: In progress; Review: Review, Approved). Dropping on the lane body uses the lane's default (Preparation to Refining, Work to In progress, Review to Review). Keep the note modal for moves that need one. If the owner prefers eight columns, keep this task's visual language and restore the columns.

**Preserve.** URL parameters (`task`, `epic`, `q`), "mine" and show-done preferences, all three drag sensors, the move rules and server errors surfaced as toasts, live invalidation (`web-live-invalidation` behaviour), the context search.

**Data for the headline and strip.** Counts come from the existing task list (needs-owner, running teams, PRs in review). The character state comes from the shared hook in `H07`.

**States.** Loading (lane skeletons), empty project (empty state with the sleeping character and a create action), error with retry, filtered-to-nothing, a very long title (two lines then clamp), many cards (the lane scrolls; the lane header stays), offline.

**Phone.** One lane per screen with snap scrolling and a lane switcher in the chip row; drag is replaced by a "Move to" menu on the card.

## 2. Task list (`H11`, redesign)

**Today.** `pages/tasks` (292 lines) and `widgets/task-list` (119 lines). A toolbar of filter menus (Status with counters, Readiness, Created or Updated with a time range, Priority, Epic, Responsible), Order and Group selects, "Reset", a list grouped by status with 44px rows (id, status icon, title, labels, team and people, age), and a hints footer. `j`/`k`/arrows move a focus, Enter opens, `/` searches, state in the URL through `parseFilter` and `writeFilter` (`entities/task`).

**Target.** The page title states the filtered result, for example an Unbounded count followed by "open tasks" in Manrope; the orange stop is reserved for hero headlines, so it does not appear here. Filters are `g-chip` triggers opening `g-menu` panels (checkbox and radio menu items), with the active value inside the chip. Rows use `g-row` with a status icon, the title in Manrope 600, an acceptance segmented bar, the assignee avatar, and age. Groups use `g-group-head` with the status icon and a count. The key-hints footer becomes a quiet bar with `g-kbd`.

**Preserve.** The filter model and URL format; keyboard navigation and its focus ring; opening a task in the right panel; grouping and ordering; the "nothing found" and loading states.

**States.** Loading rows, error with retry, empty (no tasks), filtered empty (with a reset action), long titles, many assignees.

**Phone.** Filters collapse into one "Filters" chip opening a bottom sheet; rows wrap to two lines.

## 3. Task detail: panel and page (`H12`, redesign)

**Today.** `widgets/task-detail` (about 920 lines across `TaskDetail`, `TaskDelivery`, `TaskManager`) in two variants. The right panel (`?task=`) has a header with crumbs, follow, "Open page", close; a title block with property selects (status, priority, assignee, epic, integration); the epic link; the owner-decision block (`features/owner-decision`); description with an editor; acceptance criteria with checkboxes; delivery (PR, CI); the task manager block; and a footer with the last activity. The full page (`/task/:id/:tab?`) adds tabs ("Description" or "Goal" for epics, "Team"), a main column (docs impact hint, owner decision, description, criteria, delivery, manager, plan or roadmap, notes, artifacts, activity with a comment composer) and a right rail of properties plus the epic box. The Team tab embeds the team view.

**Target.** A reading-first layout: a headline row (id, status as `g-status`, title in Unbounded only on the full page at h2 and in Manrope 700 in the panel), a property rail of `g-field` selects on the right, and a main column in `g-prose` for description, plan, and notes. The owner-decision block is the page's one orange region (a signal sheet with the question and answer controls). Acceptance criteria are checkboxes with a segmented progress header. Delivery shows PR and CI as `g-tag outline` plus `g-status`, with CI failure as a `g-banner danger`. Activity is a timeline: events as quiet rows, comments as sheets, agent versus human by avatar shape. Artifacts are a grid of `g-card flush` thumbnails. The composer is a `g-textarea` with the send button.

**Preserve.** Both variants and their routes; the property edits and their permission rules; the epic box; the owner-decision flows including questionnaires and "Clarification #" tasks; the docs-impact block; delete with confirmation; follow; artifact viewing; comment shortcut `Cmd/Ctrl+Enter`; live updates; the Russian phrasing helpers.

**States.** Loading skeleton, task not found, no permission to edit (controls become read-only text), no team ("Team not assembled" empty state with the sleeping character and the spawn action), CI red, waiting on dependencies, cancelled task.

**Phone.** The panel is a full-screen page; the rail collapses under the title as an accordion of properties.

## 4. Epics and the epic page (`H13`, redesign)

**Today.** `pages/epics` (112) lists open epics with progress; `pages/epic` (404) shows a header with title, status, priority, an owner-decision box, goal and roadmap, a child-task list with a way to move tasks into the epic, an aside with progress, artifacts (add button), and activity.

**Target.** The epics list is a grid of `g-card` with an Unbounded count of open children, the arcs as a progress motif (outer arc proportional to done), and the status. The epic page uses the headline pattern with the goal as the lede, children as a `g-row` list grouped by status, and the aside as a property rail.

**Preserve.** Moving tasks between epics, adding artifacts, the owner-decision box, start and close rules shown by the server.

**States.** No epics (empty state), epic with no children, all children done (a calm "ready to close" banner).

## 5. Team view and agent chat (`H14`, redesign)

**Today.** `widgets/team-view` (about 475 lines) shows the team mail as a log (`role="log"`) with a composer (importance select including "interrupt the step"), an aside with members and the journal, and member cards with actions. `pages/agent` (about 810 lines, with `agent.css`) is the conversation with one agent: tabs for members, breadcrumbs, a feed of messages and tool steps with a "correct from this step" quote action, a correction composer with delivery timing, pause and idea-plan asides. A scheme graph (`TeamScheme`) shows the team.

**Target.** Chat is where the character lives: the agent's state (thinking, sleeping, waking, surprised) in the header. Messages are `g-sheet` for people and a quiet surface for agents; tool steps are collapsed rows with a status icon and duration, expandable to `g-prose` code blocks. The composer is a pill-shaped `g-textarea` with the importance control as a `g-seg`. Members are cards with avatars (square for agents), activity state, and the working arcs. The team scheme uses the viz colours and the arcs motif.

**Preserve.** Live streaming and auto-scroll sticking logic (`stick` near the bottom), mail levels and interrupt semantics, quoting a step, pause and resume, idea-plan flow, member actions, the URLs `/team/:id` and `/team/:id/:member`, all `role="log"` and aria labels.

**States.** Empty mailbox, an agent mid-tool call (working arcs), paused, offline, a stopped team, very long outputs (collapse with "Show more"), images in messages (existing `ImagePreview`).

**Phone.** The aside becomes a bottom sheet; the composer pins above the safe area.

## 6. Documentation (`H15`, reskin plus typography)

**Today.** `pages/docs` (about 1,220 lines) with `entities/doc` (about 780): a tree column with a filter and a search mode, a content column with breadcrumbs, a title block with meta (status, related tasks, tags, diagnostics banners), the rendered Markdown (`shared/ui/Markdown.tsx`, `markdown.css`), an editor (`DocEditor.tsx`, 459 lines) with a preview, a new-document dialog, a search list, and a mobile drawer for the tree. About 390 lines of docs CSS live in `app/styles.css`.

**Target.** Keep the three-pane structure. Typography is the work: `g-prose` for the reading column (Manrope 15-17px, line 1.65, 70ch, Unbounded h1 and h2), the tree as dense `g-row` items with doc-type icons, meta as `g-tag`s, diagnostics as `g-banner`, the editor as a `g-textarea` with a monospace body and a preview toggle (`g-seg`). Page status (draft, current, deprecated) uses the status colours with shapes.

**Preserve.** Wiki-links, backlinks, search modes and query parameters (`page`, `q`, `new`, `title`), edit routes (`/docs/edit`), proposals, the drawer on phones, keyboard and focus behaviour of the tree, document diagnostics.

**States.** Empty vault, loading page, failed page, no search results, deprecated page banner, very long tables and code blocks (horizontal scroll inside the block).

## 7. Agents catalogue (`H16`, reskin)

**Today.** `pages/agents` (77 lines) with tabs Roles, Team templates, Skills, MCP, a search, and a "Configure" link for administrators; `widgets/agents-kit` (about 2,200 lines: roles, templates with a graph, skills, MCP, editors, dialogs, file editor, history) and `entities/agent-config/ui/TemplateGraph.tsx` (SVG), `shared/ui/agents.css` (292 lines).

**Target.** Reskin to `g-tabs`, list and detail panes in `g-card`/`g-row`, forms in `g-field`, the template graph recoloured with the viz tokens (agents as rounded squares) and the arcs motif at node connections. Read-only mode stays visibly read-only.

**Preserve.** Everything about editing rules (admin-only), file editing, history, permission fields, MCP field editors, query parameters (`tab`, `id`, `q`).

## 8. Platform pages (`H17`)

- **Answer** (`/answer`, outside the shell): reached from a notification link, the one screen a person may open cold. Redesign it as a brand moment: headline "Your answers are needed" (or the closed variant), the character in the surprised state, one `g-sheet` per question with the answer control, and a single orange submit. Preserve the token or link handling and the closed state.
- **Notifications** (`/notifications`): grouped by day, `g-row` per item with an unread dot (green) and a type icon; mark-read behaviour unchanged. Empty state with the sleeping character.
- **Automations** (`/automations`): a split view; the rules list on the left, runs on the right with `g-status` per run. Reskin.
- **Proposals** (`/docs/proposals`): a split view of open proposals with the proposal page in `g-prose` and accept or reject buttons. Reskin; reached from the Docs header segment.

## 9. Authentication (`H18`, redesign)

**Today.** `pages/auth/ui/AuthPages.tsx` (about 100 lines): login, invitation, and a note variant; the first-project page lives in `pages/settings` for administrators.

**Target.** Full-bleed brand surfaces. Login: the Unbounded display slogan with the orange stop on the left, the arcs motif cropped at the corner, the form in a `g-card` on the right, and the animated character. Invite and first-project pages share the layout. Both themes work. Preserve all error notes ("Server unavailable", "No access to any project").

## 10. Dialogs and overlays (`H19`)

All move to `g-modal`, `g-scrim`, `g-menu`, `g-toast` through `H05c`; this task does the content of each:

| Overlay | Today | Notes |
|---|---|---|
| Command palette | `features/command-palette` (190 lines) | A `g-modal` with a `g-search` field and `g-menu-item` results grouped (tasks, teams, docs, actions); arrow-key navigation and `Esc` unchanged |
| New task | `features/create-task` (214 lines) | Fields with `g-field`; the idea mode and presets (`NewTaskPreset`) unchanged; the one orange button is "Create" |
| Owner decision | `features/owner-decision` (254 lines) | The signal sheet; questionnaires with radio and checkbox groups; answers unchanged |
| Shape an idea | `features/shape-idea` (315 lines + css) | Conversation with a planner; reuse the chat styles |
| Spawn / manage team | `features/spawn-team`, `features/manage-team` | Forms in `g-modal` |
| Add artifact | `features/add-artifact` | File and text inputs; image preview |
| Model menu and picker | `features/agent-model` (313 lines + css) | `g-menu` with search; keep the grouping |
| Confirm dialogs | `shared/ui/ConfirmDialog.tsx` | The destructive variant uses `g-btn danger` |

## 11. Settings (`H20`, `H21`, `H22`; reskin only)

**Today.** About 4,960 lines in `pages/settings`, plus `widgets/settings-shell`, `shared/ui/settings.css` and `project.css`. Three modes with their own rail:

- **Me** (`/settings/me/:section`): account, sign-in and password, Telegram, providers and keys, CLI and MCP tokens.
- **Project** (`/manage/project/:section`): general (project, agent work), repositories and environment (repositories, Git tokens, environment), tools (skills, MCP servers, Jev, automations), access and roles (members, invitations, activity), channels, knowledge base (pages, backups, remote storage).
- **Server** (`/manage/server/:section`): projects, accounts (list, create, roles in projects), providers, tools (MCP servers, Jev, agents roles/templates/skills/history, health, vault).

The tab structure comes from `SETTINGS_TABS` in `shared/lib/management.ts`.

**Target.** Keep the structure and URLs. The rail becomes a `g-card` list of section links on the left with the active one filled; tabs are `g-tabs`; every form is `g-field` with help and error text; lists and tables are `g-table`; dangerous actions are `g-btn danger` behind `ConfirmDialog`. Destructive and secret-bearing sections (tokens, keys) show values in a `g-mono` block with a copy button. No redesign of layouts; do not invent new sections.

**Preserve.** Role gating (`RequireProjectAdmin`, `RequireServerAdmin`), legacy redirects (`/profile`, `/project`, `/server`, `/settings/project|server`), the "back to work" action and `lastWorkRoute`, secret masking, provider connect flows, repository connect flows, and every server error message.

**Split.** `H20` Me; `H21` Project; `H22` Server (the largest, with `McpAdmin` at about 880 lines, `ServerSettings` at about 700, and `ProvidersAdmin`).

## 12. Statistics and costs (`H23`)

**Today.** `ServerStatisticsPage` and `ServerCosts` in `pages/settings`, `entities/usage`, `shared/ui/BarChart.tsx` (97 lines).

**Target.** A headline numeral per metric (`g-stat`), charts with `--g-viz-*`, direct value labels, `g-table` for the breakdown, and a time-range `g-seg`. Admin-only, as today. The BarChart component takes colours from the viz tokens and prints values at the bars.

## 13. Cross-cutting

- **Loading.** Replace every bare "Loading..." (`PageFallback` and per-page rows) with a `g-skeleton` layout matching the page, or the working arcs for short waits.
- **Empty states.** One shared `EmptyState` (title, one line, optional action, optional character). Use the sleeping character for "nothing here yet", the waking one only for resume moments.
- **Errors.** One shared error block with a retry action and the server message.
- **Offline.** A banner under the top bar; mutation controls disable with a title.
- **Toasts.** A single stack, pill-shaped, bottom-centre; polite live region.
- **Dates.** `timeAgo` for recent, absolute with a tooltip for older; tabular figures.
- **Images and artifacts.** Keep `ImagePreview` behaviour; restyle chrome.
- **Drag and drop.** Only the board and moves between epics use it; keep keyboard alternatives.
