---
title: Documentation vault on changesets
summary: GNE-110 design for versioned, immediately visible doc edits with section operations, history, rollback and agent conflict resolution.
type: specification
status: draft
tags: [docs, vault, git, agents, conflicts]
paths: [crates/genie-core/src/vault.rs, crates/genie/src/knowledge.rs, crates/genie/src/vault_sync.rs, crates/genie/src/http/docs.rs, crates/genie/src/ops/docs.rs]
related: [GNE-110, GNE-111]
---

# Documentation vault on changesets

Status: draft for owner review (GNE-111). Scope: the product's knowledge vault ([knowledge vault](../platform/knowledge-vault.md), [documentation contract](../reference/genie-docs-system.md)) and moving this repository's `docs/` into it.

## Goals

1. **Simpler for agents.** An agent edits a section, not a whole page, and never has to redo an edit because someone else wrote first.
2. **Reliable.** Every change is a git commit with a full diff, an author, a task and a reason; any change can be rolled back in one step; nothing is silently lost or silently uncommitted.
3. **Immediately visible.** A submitted change is the current version of the page for everyone (people, agents, L0/L1 context) at once. Drafts that disappear from view are not part of the model.
4. **Conflicts resolve themselves.** A dedicated merge agent combines overlapping edits; people step in through history and rollback, not through `.conflict` copies.
5. **One documentation source.** The repository's `docs/` becomes a vault space; the site is built from the vault.

## Problems this replaces

| Today | Effect |
| --- | --- |
| `Vault::write` replaces the whole page, guarded by `base_hash` | A concurrent edit makes the agent reload and rewrite the page; long pages cost tokens and lose small edits. |
| Proposals store a page snapshot (`proposals.content`) | Approval of a stale proposal fails or `force` overwrites newer text; proposed text is invisible until approved. |
| Remote sync writes `<page>.conflict-<time>.md` | Junk pages in the vault and the index; a person must merge by hand. |
| `git_commit` ignores git failures | The file changes on disk while history does not record it. |
| The documenter commits docs into the code branch | Two documentation locations, no link between them. |

## Model

### Vocabulary

- **Edit**: one operation on one page (see [Section operations](#section-operations)).
- **Changeset**: the edits an author makes for one purpose, usually one task. It is a group of commits on the vault's main branch sharing a `Genie-Changeset: <id>` trailer, plus the trailers `Genie-Task`, `Genie-Author-Kind` and `Genie-Base` (the commit the author read). Commits stay small and individually revertible; the changeset is the unit for review, history and rollback.
- **Version**: a commit. Every page version is addressable by commit SHA.
- **Unreviewed**: a published changeset that the section's owners have not marked reviewed yet.

The main branch is always the current documentation. There is no separate draft store; the `proposals` table is retired.

### Write path

All writes (web, CLI, MCP, the documenter, sync, the merge agent) go through one integration function under the vault lock:

1. Parse and validate the edit against the page at `HEAD`: frontmatter contract, wiki-link targets, path scope, `locked` policy.
2. If the edit's `base` commit equals the page's last-changing commit, apply it directly.
3. Otherwise run a three-way merge of the affected region (`base` → `HEAD` versus `base` → edit), using `git merge-file` semantics. A clean merge is applied.
4. A textual conflict is parked as a **pending merge** (`refs/genie/pending/<id>`, holding base/ours/theirs) and handed to the merge agent. The author gets `merging` instead of an error and does not retry.
5. The result is written and committed in one step. A failed commit restores the file and returns an error; the index refreshes only after a successful commit.

The response to an author is one of `saved {commit}`, `merging {pending id}` or a validation error. Agents never receive a "reload and apply again" conflict.

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

Heading paths are resolved at `HEAD`; a heading renamed since `base` is matched through the merge in step 3. Section-sized edits keep concurrent agents in different regions of a page, so most concurrent edits merge cleanly.

The agent tool surface becomes `genie_docs` actions `search`, `read` (now returns `commit`), `edit` (one or more operations plus `note` and `task`), `history`, `diff` and `revert`. Every edit within a task joins that task's changeset automatically.

### Publication and review

The `review` policy changes meaning from *gate before publication* to *review after publication*:

| Policy | Result |
| --- | --- |
| `direct` | Published; no review queue entry. |
| `review` | Published at once and listed as **unreviewed** for the section owners until they mark it reviewed or revert it. |
| `locked` | Refused for the role, unchanged. |

Visibility of unreviewed content:

- Pages show a compact "unreviewed changes" marker linking to the diff.
- L0 marks pages with unreviewed changes; L1 includes the page as current, since immediate visibility to other agents is the goal.
- The project's **Changes** feed lists changesets with filters (unreviewed, by task, by author, merged by agent).

Review state lives in the server database (`doc_reviews`: changeset, reviewer, decision, time), not in git, so marking something reviewed does not create commits or sync traffic.

### History and rollback

- **Page history**: commits touching a page, following moves, each with author, task, changeset and reason.
- **Diff**: any two versions of a page; a changeset's full diff across pages.
- **View a version**: the page rendered at a commit.
- **Restore a page** to a version: a new commit, itself revertible.
- **Revert a changeset**: reverts its commits as a new changeset. Later edits to the same lines go through the normal write path, so overlaps reach the merge agent instead of failing.

History is read from git, so it is the same in Genie, Obsidian and a plain clone.

### Merge agent

A new built-in role `doc-merger` (`agents/doc-merger.md`).

- **Input**: per conflicted region, base/ours/theirs text; both sides' notes, tasks and authors; the page outline.
- **Output**: the merged page region and a one-line explanation. The result passes the same validation as any edit; conflict markers are rejected.
- **Authority**: per the owner's decision, its result is published directly in every section the original edit could publish to. It does not escalate by default; when the two sides contradict each other in meaning, it records `contradiction` in the commit and the changeset appears as unreviewed for the owners.
- **Traceability**: the commit carries `Genie-Merged-By: doc-merger` and `Genie-Merge-Of: <ours>,<theirs>`; history shows it with a badge, and either side can be restored in one action.
- **Failure**: if the agent is unavailable or produces invalid output after a retry, the pending merge stays visible on the page and in the Changes feed, owners are notified, and a person resolves it in a three-pane view. No `.conflict` files are created.

### Remote sync

Sync with `vault.remote` keeps its schedule but uses the same pipeline:

- Fetch and attempt the merge as today.
- On overlapping edits, the server's main branch stays as is; each conflicted page becomes a pending merge whose "theirs" is the remote version, handled by the merge agent. The resolved merge commit has both histories as parents, so the next push reconciles the remote.
- A delete on one side and an edit on the other keeps the edited page, as today, and is recorded in the Changes feed.
- Commits from the remote receive changeset IDs on import (by person and sync run), so they are reviewable and revertible like any other changeset.

### Documenter

The documenter writes to the vault through `genie_docs edit` under its task's changeset instead of committing documentation into the code branch. Its `doc` artifact links the changeset. Code-adjacent files a repository requires (`README.md`, `CHANGELOG.md`, `AGENTS.md`, code comments) stay in the code branch.

## Migration

1. **Proposals**: each open proposal becomes a changeset. Its base is the newest commit where the page's hash equals `base_hash` (or the proposal's creation time if none matches). It goes through the write path, so it is published as unreviewed and conflicts reach the merge agent. Decided proposals are kept as read-only records; the `proposals` table and its routes are removed after migration.
2. **Conflict copies**: existing `*.conflict-*.md` files become pending merges against their pages and are removed from the tree.
3. **Policies**: `vault.json` keeps its policy names; `review` takes the new meaning. The release notes state the change.

## Moving the repository docs into the vault

Target: this repository keeps only the files tools need at its root (`README.md`, `CHANGELOG.md`, `AGENTS.md`, `CODING_STANDARDS.md`, `GLOSSARY.md`); `docs/` content lives in the `genie` project's vault space.

- The vault's remote is a separate private git repository. The documentation site's CI checks it out and builds it with the existing VitePress theme and checks; the theme and its config stay in this repository as a UI concern.
- Agents read process documentation through `genie docs read/search` (MCP or CLI). `AGENTS.md` links are replaced with vault paths.
- A code change and its documentation are linked by task ID instead of sharing a commit.

This step depends on the tracker being reachable from every agent environment, so it is the last phase and has its own decision point (see [Open questions](#open-questions)).

## Phases

| Phase | Result |
| --- | --- |
| 1. Core | Single write path, commit-or-fail, changeset trailers, section operations, `read` returns commit. |
| 2. Publication and history | `review` as post-publication review, unreviewed markers in UI and L0, Changes feed, history, diff, restore, revert. |
| 3. Merge agent | Pending merges, `doc-merger` role and dispatch, three-pane fallback. |
| 4. Sync | Remote conflicts through pending merges; changeset IDs for imported commits; `.conflict` files gone. |
| 5. Migration | Proposals and conflict copies migrated; proposal routes removed. |
| 6. Documenter and repository docs | Documenter writes to the vault; this repository's `docs/` moves to the vault; site built from the vault remote. |

## Open questions

1. **Agents outside Genie.** Host agents (for example, in T3 worktrees) would read process docs only through the tracker. Is a running tracker an acceptable requirement, or should a read-only export of the vault be checked out next to the code?
2. **Granularity of a changeset without a task.** Edits by people in the web UI without a task: one changeset per editing session (proposed: per page, closed after 30 minutes of inactivity) or one per save?
3. **Review queue scope.** Should `direct` sections also show agent changesets in the Changes feed (unflagged), so owners can audit everything in one place? Proposed: yes.
