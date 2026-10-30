---
title: Documentation vault on changesets
summary: GNE-110 design for branch-based doc edits with explicit publication, section operations, history, rollback, agent conflict resolution and required reading.
type: specification
status: draft
tags: [docs, vault, git, agents, conflicts]
paths: [crates/genie-core/src/vault.rs, crates/genie/src/knowledge.rs, crates/genie/src/vault_sync.rs, crates/genie/src/http/docs.rs, crates/genie/src/ops/docs.rs, crates/genie/src/context.rs]
related: [GNE-110, GNE-111]
---

# Documentation vault on changesets

Status: draft for owner review (GNE-111). Scope: the product's knowledge vault ([knowledge vault](../platform/knowledge-vault.md), [documentation contract](../reference/genie-docs-system.md)), moving this repository's documentation and agent instructions into it, and delivering required reading to agents.

## Goals

1. **Simpler for agents.** An agent edits a section, not a whole page, and never has to redo an edit because someone else wrote first.
2. **Reliable.** Every change is a git commit with a full diff, an author, a task and a reason; any change can be rolled back in one step; nothing is silently lost or silently uncommitted.
3. **A direct process.** Editing opens a branch; publishing merges it into the main version. Open branches are visible on their pages and in one feed, so unpublished work does not drop out of sight.
4. **Conflicts resolve themselves.** A dedicated merge agent combines overlapping edits; people step in through history and rollback, not through `.conflict` copies.
5. **One documentation source.** The repository's `docs/`, `AGENTS.md` and `CODING_STANDARDS.md` become vault pages; agents receive the pages their role must read with their first message.

## Problems this replaces

| Today | Effect |
| --- | --- |
| `Vault::write` replaces the whole page, guarded by `base_hash` | A concurrent edit makes the agent reload and rewrite the page; long pages cost tokens and lose small edits. |
| Proposals store a page snapshot (`proposals.content`) | Approval of a stale proposal fails or `force` overwrites newer text; proposed text is invisible until approved. |
| Remote sync writes `<page>.conflict-<time>.md` | Junk pages in the vault and the index; a person must merge by hand. |
| `git_commit` ignores git failures | The file changes on disk while history does not record it. |
| The documenter commits docs into the code branch | Two documentation locations, no link between them. |
| Agent instructions live in repository files | Each tool finds them differently; there is no per-role required reading. |

## Model

### Vocabulary

- **Main**: the vault's main branch; the published documentation that search, L0/L1 context and the site show by default.
- **Changeset**: a git branch `changes/<id>` forked from main, owned by an author and usually a task. Its commits carry the trailers `Genie-Changeset`, `Genie-Task` and `Genie-Author-Kind`.
- **Edit**: one operation on one page inside a changeset (see [Section operations](#section-operations)).
- **Publish**: merge a changeset into main. **Discard**: close it without merging; the branch is kept as a closed record.
- **Version**: a commit on main. Every page version is addressable by SHA.
- **Unreviewed**: a published changeset that the section's owners have not marked reviewed yet.

The `proposals` table is retired; a changeset replaces it.

### Lifecycle

1. **Open.** The first edit opens a changeset. Agents get one per task automatically; every later edit within the task joins it. A person editing in the web gets one per page and editing session, unless they add the page to a changeset they already have open.
2. **Edit.** Edits commit to the changeset branch. The author (and the author's team) reads the changeset's version of a page by default (read-your-writes); everyone else reads main and sees that the page has an open changeset.
3. **Publish.** The author publishes. Publication merges the branch into main (see [Integration](#integration)). An agent publishes when its documentation is done; a task cannot reach `done` while its changeset is open (the completion gate lists it).
4. **After publication.** In `review` sections the published changeset appears as unreviewed for the owners, who mark it reviewed or revert it.

Visibility of open work:

- A page shows a compact marker for each open changeset touching it, with author, task and a diff link.
- The project's **Changes** feed lists open changesets, published ones (filters: unreviewed, by task, by author, merged by agent) and pending merges, for every policy including `direct`.
- Changesets idle for longer than a configurable time (default three days) notify their author, or the task's owner for an agent.
- Changeset branches are pushed to `vault.remote`, so they are visible in Obsidian and plain git as well.

### Section operations

Agents and the CLI address content by heading path instead of sending a whole page:

| Operation | Effect |
| --- | --- |
| `replace-section <heading>` | Replace a section's body (up to the next heading of the same or higher level). |
| `insert-section <after-heading> <heading>` | Add a new section. |
| `append <heading?>` | Append to a section or the page. |
| `remove-section <heading>` | Remove a section. |
| `set-meta <field> <value>` | Update one frontmatter field. |
| `create-page`, `move-page`, `remove-page` | Whole-page operations; moves update wiki-links in the same changeset. |
| `write-page` | Full replacement; kept for people and imports. |

The agent tool surface becomes `genie_docs` actions `search`, `read` (returns the version read), `edit` (one or more operations plus `note`), `publish`, `discard`, `changes` (own open changesets), `history`, `diff` and `revert`. CLI commands mirror them (`genie docs edit|publish|discard|changes|history|diff|revert`).

Section-sized edits keep concurrent authors in different regions of a page, so most publications merge cleanly.

### Integration

Every change to main (publication, revert, restore, sync, migration) goes through one function under the vault lock:

1. Validate the changeset's result against main: frontmatter contract, wiki-link targets, path scope, `locked` policy.
2. If main has not changed the changeset's pages since the fork, fast-forward or merge.
3. Otherwise run a three-way merge per page (`git merge-file` semantics). A clean merge is committed as a merge commit.
4. A textual conflict becomes a **pending merge** on the changeset and is handed to the [merge agent](#merge-agent). The author gets `merging` instead of an error and does not redo the edit; publication completes when the merge does.
5. Main moves with a compare-and-swap ref update. A failed commit or ref update leaves main and the working tree unchanged and returns an error; the index refreshes only after success.

Writes inside a changeset are commits on its branch and fail the same way: no write without a commit.

### Publication and review

| Policy | Who may publish | After publication |
| --- | --- | --- |
| `direct` | The author | Listed in Changes; no review queue entry. |
| `review` | The author | Listed as **unreviewed** for the section owners until marked reviewed or reverted; L0 marks the page. |
| `locked` | Nobody in the role; edits are refused | — |

Review state lives in the server database (`doc_changesets.review_needed`, `reviewed_by`, `reviewed_at`, `review_decision`), not in git, so marking something reviewed creates neither commits nor sync traffic. Pending merges are kept in `doc_merges`.

### History and rollback

- **Page history**: commits on main touching a page, following moves, each with author, task, changeset and reason.
- **Diff**: any two versions of a page; a changeset's full diff across pages, open or published.
- **View a version**: the page rendered at a commit.
- **Restore a page** to a version: a new changeset published at once, itself revertible.
- **Revert a changeset**: a new changeset with the inverse commits, published through integration, so overlaps with later edits reach the merge agent instead of failing.

History is read from git, so it is the same in Genie, Obsidian and a plain clone.

### Merge agent

A new built-in role `doc-merger` (`agents/doc-merger.md`).

- **Input**: per conflicted region, base/ours/theirs text; both sides' notes, tasks and authors; the page outline.
- **Output**: the merged region and a one-line explanation. The result passes the same validation as any edit; conflict markers are rejected.
- **Authority**: per the owner's decision, its result is published directly. When the two sides contradict each other in meaning, it records `contradiction` in the commit, and the changeset is listed as unreviewed for the owners whatever the policy.
- **Traceability**: the merge commit carries `Genie-Merged-By: doc-merger`; history shows it with a badge, and the pre-merge version of either side can be restored in one action.
- **Failure**: if the agent is unavailable or its output stays invalid after a retry, the pending merge stays on the page and in Changes, owners are notified, and a person resolves it in a three-pane view. No `.conflict` files are created.

### Remote sync

Sync with `vault.remote` keeps its schedule and treats incoming work as changesets:

- Commits pushed to main by people (Obsidian, plain git) are imported as a changeset per person and sync run and integrated like a publication. Overlaps become pending merges for the merge agent; the resolved merge commit has both histories as parents, so the next push reconciles the remote.
- People may also push `changes/*` branches; they appear as open changesets and are published from Genie or by merging them in git.
- A delete on one side and an edit on the other keeps the edited page, as today, and is recorded in Changes.

### Required reading

Agents receive the pages their role must follow with their first message, in full, before L0/L1:

- `vault.json` gains `required`: a list of `{ page, roles }` per space, where `roles` is a list of role names or `all`. Example: the agent guide for `all`, the coding standards for `executor` and `reviewer`.
- Required pages are delivered whole, not truncated, and do not count against the L0/L1 budgets; the kickoff states their versions. A required page above a hard limit (proposed 12,000 characters) is reported by `genie doctor` and the docs check instead of being cut silently.
- When a required page changes on main during a running task, the agent's next turn receives a short notice with the diff.
- Required pages are shown with a badge in the Docs screen; editing one is subject to its section policy like any page (the guide's section is proposed as `review` for agents).

### Documenter

The documenter writes to the vault through `genie_docs edit` in its task's changeset and publishes it when done, instead of committing documentation into the code branch. Its `doc` artifact links the changeset. Files a code repository requires (`README.md`, `CHANGELOG.md`, code comments) stay in the code branch.

## HTTP API

All routes live under `/api/docs`, are scoped to the caller's project roots, and keep the existing capability checks (`docs.read` for reads, `docs.write` for changes). Changeset, merge and commit objects are the Rust wire types (`Changeset`, `DocMerge`, `CommitInfo`) exported to `web/src/shared/api/generated`.

| Route | Purpose |
| --- | --- |
| `GET /page?path=&changeset=&at=` | Read a page. Without `changeset`, an agent or person with an open changeset touching the page reads that version (read-your-writes); `changeset=main` forces main; `at=<sha>` reads a version on main. The response adds `version` (commit read), `changeset` (id or null), `openChangesets` (others' open changesets touching the page: id, author, task) and `unreviewed` (published changesets awaiting review). |
| `POST /page` | Compatibility write: one `write-page` edit into the caller's changeset (see `/edit`). |
| `POST /edit` | Body `{ path, ops: DocOp[], note?, task?, changeset? }`. Applies the operations to the changeset version of the page (opening the caller's changeset when needed) and commits them to its branch. Returns `{ changeset, commit, page }`. |
| `POST /pages/move`, `DELETE /page` | Move or remove a page inside the caller's changeset. |
| `GET /changesets?status=&task=&author=&unreviewed=&path=` | The Changes feed. |
| `GET /changesets/{id}` | A changeset with its commits and full diff against its base or, when published, against main before publication. |
| `POST /changesets/{id}/publish` | Integrate into main. Returns `{ status: "published", commit }` or `{ status: "merging", merge }`. |
| `POST /changesets/{id}/discard` | Close without merging. |
| `POST /changesets/{id}/review` | Body `{ decision: "reviewed" }`; owners of the touched sections only. |
| `POST /changesets/{id}/revert` | Create and publish the inverse changeset. |
| `GET /history?path=&limit=` | `CommitInfo[]` of a page on main, following moves. |
| `GET /diff?path=&from=&to=` | Unified diff of a page between two versions. |
| `POST /restore` | Body `{ path, at }`: publish the page's content at `at` as a new changeset. |
| `GET /merges/{id}` | Pending merge with, per conflicted page, base/ours/theirs, the marked text, and both sides' notes, tasks and authors. |
| `POST /merges/{id}/resolve` | Body `{ pages?: [{ path, content }], done?, fail?, contradiction?, note? }`. Pages are staged on the merge; `done` validates and publishes; `fail` hands it to people. |

Errors keep the existing distinctions: 400/422 invalid input and contract diagnostics, 403 locked or not an owner, 404 outside the project, 409 a changeset in the wrong state.

CLI and MCP (`genie_docs`) mirror them: `search`, `read`, `edit`, `publish`, `discard`, `changes`, `changeset`, `history`, `diff`, `revert`, `restore`, `review`, `merge-show`, `merge-resolve`, `required`.

## Migration

1. **Proposals**: each open proposal becomes an open changeset forked from the newest commit where the page's hash equals `base_hash` (or main at the proposal's creation if none matches), owned by the proposal's author and task. It stays open for its author to publish or discard. Decided proposals are kept as read-only records; the `proposals` table and its routes are removed afterwards.
2. **Conflict copies**: existing `*.conflict-*.md` files become pending merges against their pages and are removed from the tree.
3. **Policies**: `vault.json` keeps its policy names; `review` takes the post-publication meaning. The release notes state the change.

## Moving this repository's documentation into the vault

Target: the repository root keeps only what tools and the release gate require (`README.md`, `CHANGELOG.md`) plus a thin `AGENTS.md`; everything else lives in the `genie` project's vault space.

- `docs/`, `CODING_STANDARDS.md` and `GLOSSARY.md` move to the vault space. The vault's remote is a separate private git repository; the documentation site's CI checks it out and builds it with the VitePress theme, config and checks, which stay in this repository.
- `AGENTS.md` is reworked into the agent guide page in the vault and declared required reading for `all`. The repository keeps a short `AGENTS.md` stub, because host tools (Claude Code, Codex, T3) load that file by name: it tells an agent to read its required pages with `genie docs required --role <role>` and nothing else.
- A running tracker is an accepted requirement for every agent environment (owner decision).
- A code change and its documentation are linked by task ID instead of sharing a commit.

## Phases

| Phase | Result |
| --- | --- |
| 1. Core | Changeset branches, single integration path, commit-or-fail, section operations, `edit`/`publish`/`discard`. |
| 2. Visibility, history and review | Page markers, Changes feed, idle reminders, completion gate, post-publication review, history, diff, restore, revert. |
| 3. Merge agent | Pending merges, `doc-merger` role and dispatch, three-pane fallback. |
| 4. Sync | Remote commits and `changes/*` branches as changesets; `.conflict` files gone. |
| 5. Migration | Proposals and conflict copies migrated; proposal routes removed. |
| 6. Required reading | `required` in `vault.json`, kickoff delivery, change notices, `genie docs required`. |
| 7. Documenter and repository docs | Documenter writes to the vault; `docs/`, `CODING_STANDARDS.md`, `GLOSSARY.md` and the reworked `AGENTS.md` move to the vault; site built from the vault remote. |

## Decisions

Recorded with the owner on 2026-10-30:

- Scope is the product vault; this repository's documentation and agent instructions move into it.
- Editing opens a branch; publishing merges it into main.
- The merge agent publishes its resolutions itself; versions and rollback are the safeguard.
- Open proposals are migrated; there is no dual mode.
- Agents may depend on a running tracker for documentation.
- The Changes feed covers `direct` sections too.
