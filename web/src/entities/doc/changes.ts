// Pure view logic of documentation changesets: status names, the Changes feed's
// filter state, page markers, commit trailers and unified-diff lines. Nothing here
// touches the DOM, so `node:test` covers it directly.

import type { Changeset, ChangesetStatus, CommitInfo, DocMerge } from "../../shared/api/types.ts";
import { plural } from "../../shared/lib/format.ts";
import type { OpenChangesetRef } from "./model.ts";

/** The Changes feed's four views, in the order the chips read. */
export type ChangesView = "open" | "unreviewed" | "agent" | "all";

export const CHANGES_VIEWS: { id: ChangesView; label: string }[] = [
  { id: "open", label: "Открытые" },
  { id: "unreviewed", label: "Непроверенные" },
  { id: "agent", label: "Сведённые агентом" },
  { id: "all", label: "Все" },
];

export const CHANGESET_STATUS_NAME: Record<ChangesetStatus, string> = {
  open: "Открыт",
  merging: "Сводится",
  published: "Опубликован",
  discarded: "Отменён",
};

export const MERGE_STATUS_NAME: Record<DocMerge["status"], string> = {
  pending: "Ждёт слияния",
  running: "Сводится агентом",
  resolved: "Сведён",
  failed: "Не удалось свести",
};

export function changesetStatusName(status: ChangesetStatus): string {
  return CHANGESET_STATUS_NAME[status];
}

/** `#12` — the number the screens and notifications speak in. */
export function changesetNumber(id: number): string {
  return `#${id}`;
}

/** The author as a person reads it: a name when the server knows one, and the kind. */
export function changesetAuthor(changeset: Pick<Changeset, "author" | "authorName" | "authorKind">): string {
  const name = changeset.authorName && changeset.authorName !== changeset.author ? `${changeset.authorName} (${changeset.author})` : changeset.author;
  return changeset.authorKind === "agent" ? `${name}, агент` : name;
}

/** `3 страницы` — how much work an open changeset carries. */
export function changesetPathsLabel(count: number): string {
  return `${count} ${plural(count, "страница", "страницы", "страниц")}`;
}

// ---------------------------------------------------------------- filters

export interface ChangesFilter {
  view: ChangesView;
  task?: string;
  author?: string;
  path?: string;
}

export function defaultChangesFilter(): ChangesFilter {
  return { view: "open" };
}

/** Reads the feed state from the address, so a link restores the same view. */
export function changesFilterFromParams(params: URLSearchParams): ChangesFilter {
  const raw = params.get("view") ?? "";
  const view = CHANGES_VIEWS.some((entry) => entry.id === raw) ? (raw as ChangesView) : "open";
  return {
    view,
    task: params.get("task") ?? undefined,
    author: params.get("author") ?? undefined,
    path: params.get("path") ?? undefined,
  };
}

export function changesFilterToParams(filter: ChangesFilter): URLSearchParams {
  const next = new URLSearchParams();
  if (filter.view !== "open") next.set("view", filter.view);
  if (filter.task) next.set("task", filter.task);
  if (filter.author) next.set("author", filter.author);
  if (filter.path) next.set("path", filter.path);
  return next;
}

/**
 * The server-side query for a feed view. The agent-resolved view has no server
 * parameter in the HTTP contract, so it is filtered from the published list.
 */
export function changesServerQuery(filter: ChangesFilter): string {
  const params = new URLSearchParams();
  if (filter.view === "open") params.set("status", "open");
  if (filter.view === "unreviewed") params.set("unreviewed", "true");
  if (filter.task) params.set("task", filter.task);
  if (filter.author) params.set("author", filter.author);
  if (filter.path) params.set("path", filter.path);
  return params.toString();
}

/** A published changeset the merge agent resolved; the list carries `origin: "merge"`. */
export function isAgentMergedChangeset(changeset: Pick<Changeset, "origin" | "status">): boolean {
  return changeset.status === "published" && changeset.origin === "merge";
}

/** Published, flagged for review and not decided yet. */
export function isUnreviewedChangeset(changeset: Pick<Changeset, "status" | "reviewNeeded" | "reviewDecision">): boolean {
  return changeset.status === "published" && changeset.reviewNeeded && changeset.reviewDecision === null;
}

export function filterChangesets(changesets: Changeset[], filter: ChangesFilter): Changeset[] {
  return changesets.filter((changeset) => {
    if (filter.view === "open" && changeset.status !== "open") return false;
    if (filter.view === "unreviewed" && !isUnreviewedChangeset(changeset)) return false;
    if (filter.view === "agent" && !isAgentMergedChangeset(changeset)) return false;
    if (filter.task && changeset.task !== filter.task) return false;
    if (filter.author && changeset.author !== filter.author) return false;
    if (filter.path && !changeset.paths.includes(filter.path)) return false;
    return true;
  });
}

/** Open work plus published work waiting for a reviewer: what the navigation counts. */
export function attentionChangesets(changesets: Changeset[]): Changeset[] {
  return changesets.filter((changeset) => changeset.status === "open" || isUnreviewedChangeset(changeset));
}

/** Distinct task ids and authors, for the feed's two selects. */
export function changesFilterOptions(changesets: Changeset[]): { tasks: string[]; authors: string[] } {
  const tasks = new Set<string>();
  const authors = new Set<string>();
  for (const changeset of changesets) {
    if (changeset.task) tasks.add(changeset.task);
    authors.add(changeset.author);
  }
  return { tasks: [...tasks].sort((a, b) => a.localeCompare(b, "ru")), authors: [...authors].sort((a, b) => a.localeCompare(b, "ru")) };
}

// ---------------------------------------------------------------- wire views

/** `GET /changesets/{id}`: the changeset with its commits and full diff. */
export interface ChangesetDetail {
  changeset: Changeset;
  commits: CommitInfo[];
  diff: string;
  /** The pending merge of a `merging` changeset, when the server includes it. */
  merge?: DocMerge;
}

/** Who wrote one side of a pending merge. */
export interface MergeSideInfo {
  author?: string;
  authorName?: string;
  task?: string | null;
  note?: string | null;
}

/** One conflicted page of a pending merge, with the marked text the editor starts from. */
export interface MergePageView {
  path: string;
  base: string;
  ours: string;
  theirs: string;
  marked: string;
}

export interface MergeView {
  merge: DocMerge;
  pages: MergePageView[];
  ours?: MergeSideInfo;
  theirs?: MergeSideInfo;
}

export type PublishResult =
  | { status: "published"; commit: CommitInfo }
  | { status: "merging"; merge: DocMerge };

/**
 * The detail route is specified as "a changeset with its commits and full diff"
 * without naming the envelope. Accept both a nested object and a flat changeset,
 * so the screen keeps working whichever shape the server settles on.
 */
export function normalizeChangesetDetail(
  raw: ChangesetDetail | (Changeset & { commits?: CommitInfo[]; diff?: string; merge?: DocMerge }),
): ChangesetDetail {
  if ("changeset" in raw && raw.changeset) return raw as ChangesetDetail;
  const flat = raw as Changeset & { commits?: CommitInfo[]; diff?: string; merge?: DocMerge };
  return {
    changeset: flat,
    commits: flat.commits ?? [],
    diff: flat.diff ?? "",
    merge: flat.merge,
  };
}

/** The merge detail may arrive as `{ merge, pages }` or as the merge with its pages. */
export function normalizeMergeView(raw: Partial<MergeView> & DocMerge): MergeView {
  if (raw.merge) return raw as MergeView;
  return {
    merge: raw,
    pages: raw.pages ?? (raw.conflicts ?? []).map((path) => ({ path, base: "", ours: "", theirs: "", marked: "" })),
    ours: raw.ours,
    theirs: raw.theirs,
  };
}

// ---------------------------------------------------------------- page markers

export interface PageMark {
  id: string;
  kind: "open" | "unreviewed" | "merging";
  label: string;
  title: string;
  tone: "outline" | "accent" | "neutral";
  changeset?: number;
  merge?: number;
}

/**
 * The compact marks a page's header shows: another author's open draft, a published
 * changeset awaiting review, and a pending merge. The caller's own changeset is the
 * changeset bar above the article, not a mark.
 */
export function pageMarkers(input: {
  openChangesets?: OpenChangesetRef[];
  unreviewed?: Changeset[];
  merges?: Pick<DocMerge, "id" | "status" | "changeset">[];
}): PageMark[] {
  const marks: PageMark[] = [];
  for (const open of input.openChangesets ?? []) {
    marks.push({
      id: `open-${open.id}`,
      kind: "open",
      label: open.task ? `Открыт черновик: ${open.author}, ${open.task}` : `Открыт черновик: ${open.author}`,
      title: "Другой автор правит эту страницу в своём черновике. Откройте изменения, чтобы увидеть разницу.",
      tone: "outline",
      changeset: open.id,
    });
  }
  for (const changeset of input.unreviewed ?? []) {
    marks.push({
      id: `unreviewed-${changeset.id}`,
      kind: "unreviewed",
      label: "Не проверено",
      title: `Опубликовано в ${changesetNumber(changeset.id)} и ждёт проверки владельцев раздела.`,
      tone: "accent",
      changeset: changeset.id,
    });
  }
  for (const merge of input.merges ?? []) {
    if (merge.status !== "pending" && merge.status !== "running") continue;
    marks.push({
      id: `merge-${merge.id}`,
      kind: "merging",
      label: "Сводится",
      title: "Правки пересеклись с более новыми: слиянием занимается агент.",
      tone: "neutral",
      changeset: merge.changeset,
      merge: merge.id,
    });
  }
  return marks;
}

/** May this viewer decide on a published changeset (owners and project admins)? */
export function canReviewChangeset(role: string | undefined): boolean {
  return role === "owner" || role === "admin";
}

/** Who may publish or discard: the author, in an open or merging changeset. */
export function canPublishChangeset(changeset: Pick<Changeset, "author" | "status">, login: string | undefined): boolean {
  return !!login && changeset.author === login && (changeset.status === "open" || changeset.status === "merging");
}

// ---------------------------------------------------------------- commits

export function commitTrailer(commit: Pick<CommitInfo, "trailers">, name: string): string | undefined {
  return commit.trailers.find(([key]) => key.toLowerCase() === name.toLowerCase())?.[1];
}

/** A merge commit the `doc-merger` role produced. */
export function isAgentMergedCommit(commit: Pick<CommitInfo, "trailers">): boolean {
  return commitTrailer(commit, "Genie-Merged-By") !== undefined;
}

export function commitTask(commit: Pick<CommitInfo, "trailers">): string | undefined {
  return commitTrailer(commit, "Genie-Task");
}

export function commitChangeset(commit: Pick<CommitInfo, "trailers">): number | undefined {
  const raw = commitTrailer(commit, "Genie-Changeset");
  const id = raw === undefined ? NaN : Number(raw);
  return Number.isFinite(id) ? id : undefined;
}

/** `30.10.2026` — a version's date without a locale or timezone dependency. */
export function commitDate(iso: string): string {
  const date = /^(\d{4})-(\d{2})-(\d{2})/.exec(iso);
  return date ? `${date[3]}.${date[2]}.${date[1]}` : iso;
}

/** `30.10.2026, 14:05` — a version's date and time from its RFC 3339 stamp. */
export function commitStamp(iso: string): string {
  const date = /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})/.exec(iso);
  return date ? `${date[3]}.${date[2]}.${date[1]}, ${date[4]}:${date[5]}` : iso;
}

// ---------------------------------------------------------------- diffs

export type DiffLineKind = "added" | "removed" | "context" | "hunk" | "meta";

export interface DiffLine {
  kind: DiffLineKind;
  text: string;
}

const META_PREFIXES = [
  "diff ",
  "index ",
  "--- ",
  "+++ ",
  "new file mode",
  "deleted file mode",
  "old mode",
  "new mode",
  "similarity index",
  "dissimilarity index",
  "rename from",
  "rename to",
  "Binary files",
  "\\ No newline",
];

/**
 * Classifies the lines of a unified diff. Everything outside a hunk is metadata
 * (the `diff --git`/`---`/`+++` pair and file-mode notes), so the screen can keep
 * it quiet instead of styling it as a change.
 */
export function parseUnifiedDiff(diff: string): DiffLine[] {
  return diff.replace(/\r\n/g, "\n").split("\n").filter((line, index, all) => !(index === all.length - 1 && line === "")).map((text) => {
    if (META_PREFIXES.some((prefix) => text.startsWith(prefix))) return { kind: "meta" as const, text };
    if (text.startsWith("@@")) return { kind: "hunk" as const, text };
    if (text.startsWith("+")) return { kind: "added" as const, text };
    if (text.startsWith("-")) return { kind: "removed" as const, text };
    return { kind: "context" as const, text };
  });
}

/**
 * A result the server rejects: conflict markers must not survive a merge. The
 * opening and closing markers decide, so a seven-character Markdown underline
 * (`=======`) is not mistaken for the middle of a conflict.
 */
export function hasConflictMarkers(text: string): boolean {
  return /^<{7}(\s|$)/m.test(text) || /^>{7}(\s|$)/m.test(text);
}

/** How many lines a diff adds and removes, for the summary line. */
export function diffStats(lines: DiffLine[]): { added: number; removed: number } {
  return {
    added: lines.filter((line) => line.kind === "added").length,
    removed: lines.filter((line) => line.kind === "removed").length,
  };
}

// ---------------------------------------------------------------- errors

/**
 * The server keeps its error distinctions (403 locked or not an owner, 409 a
 * changeset in the wrong state), and the screens say what the person can do next.
 */
export function docActionError(error: { status?: number; message?: string }): string {
  switch (error.status) {
    case 409:
      return "Состояние изменилось: черновик уже опубликован или отменён. Обновите страницу и повторите.";
    case 403:
      return "Недостаточно прав: страница заблокирована или вы не владелец раздела.";
    case 404:
      return "Страница или изменение не найдены в этом проекте.";
    default:
      return error.message || "Не удалось выполнить действие";
  }
}
