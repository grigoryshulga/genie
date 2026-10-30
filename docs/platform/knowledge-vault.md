---
title: Knowledge vault
summary: Store, search, review, and sync project knowledge as Markdown in Git.
type: guide
status: current
---

# Knowledge vault

Genie stores knowledge in a separate Git repository at `<data>/vault`, or the configured `vault.path`. Each project has one or more documentation roots bound to it in `vault.json`. Project documentation shows only those roots; unbound and other projects' pages stay outside the project space. Files are the source of truth. `vault-index.db` is a rebuildable search cache.

This project website is built from the repository's `docs/` directory. A running Genie server indexes its **vault**, not this website directory, unless you import those pages.

## Read and write

Use the Docs screen, Obsidian, or the CLI:

```bash
genie docs search "authentication"
genie docs read <path>
genie docs edit <path> --replace-section "Sign-in" --body-file sign-in.md
genie docs publish
genie vault import ./docs --space shop
genie vault reindex
```

Every edit goes into a **changeset**: a git branch `changes/<id>` forked from main, owned by its author and usually their task. The first edit opens it; later edits of the same task join it. The author and the author's team read the changeset's version of a page; everyone else reads main and sees that the page has an open changeset. `genie docs read <path> --changeset <id>` reads another changeset, and `--at <sha>` reads a version on main.

Prefer section operations over whole-page writes: `replace-section`, `insert-section`, `append`, `remove-section` and `set-meta` change one region of a page, so concurrent authors keep working in different regions. `genie docs write` replaces a whole page and stays for people and imports. MCP callers pass the same operations as a JSON `ops` array.

Pages support Markdown, frontmatter, wiki-links, heading reads, and backlinks. Search indexes titles, summaries, headings, body text, tags, and aliases. See the [documentation contract](../reference/genie-docs-system.md) for exact field and search behavior.

## Navigate the project space

The Docs screen opens a space overview with sections and articles. Navigation uses article titles rather than Markdown filenames. Titles come from `title`, then the first H1; legacy filenames receive a readable display name.

Create an article by entering its title and selecting a section. To add a section, choose **Create section**, name it, and choose its parent. Genie creates an overview article at `<section>/index.md`; its title names the section in navigation. Existing `README.md` pages also serve as section overviews, with `index.md` taking priority. Nested directories without an overview still appear as sections with readable names. Editing the overview article's title renames the section in navigation without moving files or breaking links.

Breadcrumbs return to any parent section or the space overview. Type and status badges have their own row above the article title; tags appear in the article footer. Markdown stays the portable storage format and remains editable outside Genie. The article editor opens in visual mode: select text and use the toolbar for bold, italic, strikethrough, headings, lists, quotes, code, links, and dividers. Undo/redo and standard formatting shortcuts work in the visual editor. Switch to **Markdown** for source editing, **Together** for source with live preview. Switching views does not save or discard a draft.

Visual edits serialize back to Markdown and can normalize whitespace and marker style. Tables, task lists, images, code languages, and vault wiki-links retain their meaning. Articles containing raw HTML use source editing, since arbitrary HTML cannot be represented by the visual schema; HTML inside code remains visually editable.

Tree, search, page reads/writes, link resolution, backlinks, changesets, merges, history, and change signatures are scoped to the selected project's configured roots, including for CLI and MCP callers. A project without roots has an empty documentation space; it never falls back to the entire vault. Notes also require a project root. Use `genie --project shop docs space handbook --of shop` to bind an additional unused root. The project API cannot reassign another project's root.

## Publication policies

`vault.json` assigns owners and separate publication policies for people and agents at space or section level:

| Policy | Result |
| --- | --- |
| `direct` | Publishing the changeset lands the change on main. |
| `review` | Publishing still lands the change; it is listed as **unreviewed** for the section owners, who mark it reviewed or revert it. |
| `locked` | Nobody in the role edits the section. They can report findings elsewhere. |

`genie docs publish` merges the changeset into main. Overlapping edits are combined by the merge agent; when two sides conflict, publication answers `merging` and completes when the merge does, so the author never redoes the work. Publication is journaled and appears in the Changes feed for every policy, including `direct`. A task cannot reach `done` while its changeset is still open. A section's policy still applies when a role has `docs.write`.

## Changesets, history and rollback

`genie docs changes` is the project's Changes feed: open changesets, published ones, pending merges, and the review queue. Filters select by status, task, author, page and review state; `--unreviewed` lists published changesets the section owners have not marked reviewed yet.

- `genie docs changeset <id>` shows a changeset with its commits and full diff; the same diff is in the page's open-changeset marker.
- `genie docs history <path>` lists the commits on main that touched a page, following moves, with author, task, changeset and reason.
- `genie docs diff <path> --from <sha> --to <sha>` compares any two versions of a page.
- `genie docs restore <path> --at <sha>` publishes the page's content at that version as a new changeset, itself revertible.
- `genie docs revert <id>` publishes a new changeset with the inverse change, so later overlapping edits reach the merge agent instead of failing.
- `genie docs review <id>` marks a published changeset reviewed; only the touched sections' owners (or the project's admins) may do it.
- `genie docs discard` closes a changeset without publishing it; the branch stays as a record.

A pending merge is read with `genie docs merge-show <id>` and resolved with `genie docs merge-resolve <id> --page <path> --file <merged page> ... --done`, or handed to the owners with `--fail`. The merge agent (`doc-merger`) does this for agents; a person can resolve it the same way. History is read from git, so it is identical in Genie, Obsidian and a plain clone.

## Git and Obsidian

The vault opens directly in Obsidian. Add a remote to sync between the server and people's local clones. Configure `vault.remote` and `vault.syncSecs` in `config.json`; the default sync interval is two minutes. See the current state in **Settings → Management → Knowledge vault**.

The server fetches, imports, publishes, and pushes, then reindexes changes. Commits people push to the main branch become one changeset per sync run, attributed to the newest remote commit's author and published by the server. Non-overlapping edits integrate by themselves. An overlapping edit becomes a pending merge that the merge agent resolves: no `.conflict` copies are written, and a delete against an edit is settled the same way. The main branch is not pushed while a sync changeset waits for its merge, and a later sync reuses that changeset instead of opening a second one.

People may also push branches under `changes/`. Each one appears as an open changeset, attributed to the author of its tip commit, for publication in Genie or by merging it in git. The server pushes the branches of the open changesets it owns and deletes the remote branch of a changeset it published or discarded. Administrators are notified only when the sync itself fails.

Keep Obsidian's personal workspace and cache files out of Git. The server ignores `.obsidian/` for indexing; the team decides whether to share its other settings.

## Task and code relationships

A page's `related` field names tasks or epics. Its `paths` globs describe code relative to the repository of the space's project. Changes after `verified` produce a possible-stale warning, not a failed page or a hidden result. Projects without code still have knowledge spaces; code-based staleness does not apply.

## Agent context

Agents with `docs.read` receive their role's required reading first, then two context layers:

- **L0:** a compact index of the project's pages in the system prompt, with title, summary, and editorial warnings. The usual budget is about 600 units, capped at 1000. Omitted pages are counted explicitly.
- **L1:** up to three pages at task kickoff, within 4000 units total and 1500 per page. Explicit task/epic relationships rank first, then path matches, then matches in titles, summaries, headings, tags, and aliases. Common words do not drive selection.

Deprecated pages enter L1 only through an explicit task relationship. Long content is cut at section boundaries and marked as truncated. Agents fetch further content with `genie docs search` and `genie docs read`.

A space may also declare `required`: pages its agents must follow, as a list of `{ "page": "genie/agents/guide.md", "roles": ["all"] }` where `roles` holds role names (the stems of `agents/*.md`, such as `executor` or `reviewer`) or `all`. Agents with `docs.read` and the orchestrator receive these pages whole, before L0 and L1 and outside their budgets: title, path and a content version, so a change is visible. `genie docs required --role executor` prints the same section. A required page over 12,000 characters is still delivered whole but reported by `genie doctor`, which also lists missing required pages.

## Keep knowledge useful

Update durable pages about the product, architecture, decisions, and procedures. Avoid creating a new page for every completed task. The task-completion playbook can run a documenter, add a changelog entry, and notify the author.

Each project can keep a `changelog.md` in its space. A release moves `Unreleased` entries to a dated version and emits `release.published`; optional code-repository publication copies the notes to its `CHANGELOG.md`.
