---
title: Knowledge vault
summary: Store, search, review, and sync project knowledge as Markdown in Git.
type: guide
status: current
---

# Knowledge vault

Genie stores knowledge in a separate Git repository at `<data>/vault`, or the configured `vault.path`. Each project has a space; shared spaces can hold team-wide material. Files are the source of truth. `vault-index.db` is a rebuildable search cache.

This project website is built from the repository's `docs/` directory. A running Genie server indexes its **vault**, not this website directory, unless you import those pages.

## Read and write

Use the Docs screen, Obsidian, or the CLI:

```bash
genie docs search "authentication"
genie docs read <path>
genie vault import ./docs --space shop
genie vault reindex
```

Pages support Markdown, frontmatter, wiki-links, heading reads, and backlinks. Search indexes titles, summaries, headings, body text, tags, and aliases. See the [documentation contract](../reference/genie-docs-system.md) for exact field and search behavior.

## Publication policies

`vault.json` assigns owners and separate publication policies for people and agents at space or section level:

| Policy | Result |
| --- | --- |
| `direct` | Commit the change to the vault's main branch. |
| `review` | Create a proposal for a section owner to review in Genie. |
| `locked` | Agents cannot change the section. They can report findings elsewhere. |

Proposals include a diff, preview, and discussion. Review works without a GitHub or GitLab remote. A section's policy still applies when a role has `docs.write`.

## Git and Obsidian

The vault opens directly in Obsidian. Add a remote to sync between the server and people's local clones. Configure `vault.remote` and `vault.syncSecs` in `config.json`; the default sync interval is two minutes. See the current state in **Settings → Management → Knowledge vault**.

The server fetches, merges, pushes, and reindexes changes. Non-overlapping edits merge automatically. For overlapping edits, the server keeps its page and saves the incoming version alongside it as `<page>.conflict-<timestamp>.md`. Owners and administrators are notified. Conflicting deletion does not discard an edited page.

Keep Obsidian's personal workspace and cache files out of Git. The server ignores `.obsidian/` for indexing; the team decides whether to share its other settings.

## Task and code relationships

A page's `related` field names tasks or epics. Its `paths` globs describe code relative to the repository of the space's project. Changes after `verified` produce a possible-stale warning, not a failed page or a hidden result. Projects without code still have knowledge spaces; code-based staleness does not apply.

## Agent context

Agents with `docs.read` receive two context layers:

- **L0:** a compact index of the project's pages in the system prompt, with title, summary, and editorial warnings. The usual budget is about 600 units, capped at 1000. Omitted pages are counted explicitly.
- **L1:** up to three pages at task kickoff, within 4000 units total and 1500 per page. Explicit task/epic relationships rank first, then path matches, then matches in titles, summaries, headings, tags, and aliases. Common words do not drive selection.

Deprecated pages enter L1 only through an explicit task relationship. Long content is cut at section boundaries and marked as truncated. Agents fetch further content with `genie docs search` and `genie docs read`.

## Keep knowledge useful

Update durable pages about the product, architecture, decisions, and procedures. Avoid creating a new page for every completed task. The task-completion playbook can run a documenter, add a changelog entry, and notify the author.

Each project can keep a `changelog.md` in its space. A release moves `Unreleased` entries to a dated version and emits `release.published`; optional code-repository publication copies the notes to its `CHANGELOG.md`.
