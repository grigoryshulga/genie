---
title: Genie project documentation contract
summary: Frontmatter fields, wiki-links, search, deprecation, and index scope.
type: reference
status: current
tags: [docs, contract, reference]
paths: [crates/genie-core/src/vault.rs, crates/genie/src/knowledge.rs]
verified: 2026-09-29
---

# Genie project documentation contract

Genie indexes the Markdown pages of its knowledge vault — a git repository with a space per project ([knowledge vault](../platform/knowledge-vault.md)). Markdown files remain the source of truth; the SQLite index (`vault-index.db`) is a rebuildable cache, independent of task tracking.

## Frontmatter

A page may begin with a YAML-like block delimited by `---`. Genie supports only the fields and value forms below; this is a deliberately narrow contract, not a general YAML implementation.

```yaml
---
title: Authentication flow
type: guide
status: current
summary: How browser sessions are issued and renewed.
tags: [auth, sessions]
aliases: [login, sign-in, authentication]
paths: [src/auth/**, src/session.ts]
related: [G-42, G-4]
verified: 2026-09-27
---
```

| Field | Type | Meaning |
|---|---|---|
| `title` | non-empty, single-line string | Display title. If omitted or invalid, Genie uses the first H1, then the filename. |
| `type` | `guide`, `reference`, `decision`, `glossary`, `runbook`, or `note` | Page kind. |
| `status` | `draft`, `current`, or `deprecated` | Editorial state. Missing is unknown, not implicitly current. Deprecated pages are excluded from search unless explicitly requested (see Search and reads). |
| `summary` | optional, single-line string | L0/index summary. If omitted or invalid, Genie uses the first body paragraph. |
| `tags` | optional list of strings | Searchable terms. |
| `aliases` | optional list of strings | Alternate searchable names, including translations and synonyms. |
| `paths` | optional list of repo-relative POSIX globs | Code areas described by the page. Supported syntax: `*` within one path segment, `**` across segments, `?` for one non-slash character; other characters are literal. |
| `related` | optional list of task/epic IDs | Explicit Genie task or epic relationships, such as `G-42`. |
| `verified` | optional `YYYY-MM-DD` date | Date a human or agent last reviewed the page against its described code. |

Lists can use inline brackets as above or indented `- item` lines. Values can be plain, single-quoted, or double-quoted strings. Do not use nested objects, multiline scalars, YAML aliases, or implicit timestamps. Invalid fields and malformed frontmatter produce page diagnostics; the Markdown body remains indexed and searchable. Diagnostics are available on page/search results and through the docs service diagnostics list.

`paths` and `verified` are independent. If either is missing, staleness is unknown rather than automatically fresh or stale. A matching code change after `verified` produces only a possible-stale warning; it never hides or invalidates a page. `updated` is derived from the file's Git last-commit date, is cached by the index, and is not a frontmatter field.

## Markdown links

Use `[[architecture/auth]]` for a Genie wiki-link. A target may include a heading (`[[architecture/auth#Session lifecycle]]`) or display text (`[[architecture/auth|authentication]]`). A canonical docs-relative path is preferred. A unique basename or alias may resolve; unresolved and ambiguous targets are reported and are never guessed. Resolved links provide backlinks.

## Search and reads

All `/api/docs` browsing and publication operations use the selected project's configured vault roots. Root ownership comes from `vault.json`, never frontmatter. Tree responses include `roots`, even for empty roots, so clients can create the first article. Search excludes shared and foreign pages before applying its candidate/result limits; the legacy `mine` query parameter cannot widen visibility. Page paths are normalized and checked on the server for both reads and writes. Wiki-link resolution and backlinks consider only pages within these roots. A basename that exists in another project does not make a local link ambiguous. Foreign canonical targets remain unresolved, and projectless roots are not implicitly shared through this API. Operator vault import/sync/reindex remain server-wide maintenance operations.


Search indexes title, summary, headings, body, tags, and aliases using SQLite FTS5. The tokenizer normalizes diacritics and treats the two Russian letter variants for "e" symmetrically. Heading reads return that heading's subtree and mark truncated content explicitly.

Pages with `status: deprecated` are excluded from search results by default. They are returned only when explicitly requested: an explicit `status` filter (`genie docs search --status deprecated`), a page whose `related` list contains a requested task/epic id, or a page chosen for a task's context because it names the task ([L1 agent context](../platform/knowledge-vault.md#agent-context)). The exclusion is applied before the result limit, so a default search never drops a non-deprecated match to make room for a deprecated one. Tree entries and page reads still list deprecated pages with their `deprecated` marker; backlinks are plain source paths with no marker.

The vault lives in `<data>/vault` (or `vault.path`); the `paths` of a page in a project's space are relative to that project's repository. Rebuilding recreates only the index, never the pages or task data.
