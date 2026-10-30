import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { request } from "@/shared/api";
import type { Changeset, CommitInfo, DocMerge, DocOp } from "@/shared/api/types.ts";
import { useToast } from "@/shared/ui";
import {
  changesServerQuery,
  normalizeChangesetDetail,
  normalizeMergeView,
  type ChangesFilter,
  type MergeView,
  type PublishResult,
} from "./changes.ts";
import { fieldsFromPage, serializeDoc, type DocPage, type DocReadResult, type DocSearchResult } from "./model.ts";

export interface DocsTreeResponse {
  roots: string[];
  version: string;
  pages: DocPage[];
  diagnostics: { path: string; diagnostics: string[] }[];
}

export interface DocsSearchResponse {
  query: string;
  results: DocSearchResult[];
}

export interface DocsSaveResponse {
  page: DocReadResult;
  diagnostics: string[];
  /** Present once writes go into a changeset (`POST /edit`, or the compat `POST /page`). */
  changeset?: Changeset;
  commit?: CommitInfo;
}

/** Version selectors of `GET /page`: another branch, or a commit on main. */
export interface DocReadOptions {
  changeset?: number | "main";
  at?: string;
}

/** Local keys: `shared/api/client.ts` is owned by another task. */
export const docsKeys = {
  all: ["docs"] as const,
  version: ["docs", "version"] as const,
  tree: ["docs", "tree"] as const,
  search: (q: string) => ["docs", "search", q] as const,
  page: (path: string, options?: DocReadOptions) => ["docs", "page", path, options?.changeset ?? null, options?.at ?? null] as const,
  changesets: (query = "") => ["docs", "changesets", query] as const,
  changeset: (id: number) => ["docs", "changeset", id] as const,
  history: (path: string) => ["docs", "history", path] as const,
  diff: (path: string, from: string, to: string) => ["docs", "diff", path, from, to] as const,
  merge: (id: number) => ["docs", "merge", id] as const,
};

/** A failed read/write; keeps the HTTP status so 404/409 can be told apart from 422. */
export class DocRequestError extends Error {
  status: number;
  constructor(message: string, status: number) {
    super(message);
    this.name = "DocRequestError";
    this.status = status;
  }
}

function pageQuery(path: string, options?: DocReadOptions): string {
  const params = new URLSearchParams({ path });
  if (options?.changeset !== undefined) params.set("changeset", String(options.changeset));
  if (options?.at) params.set("at", options.at);
  return params.toString();
}

/** `GET /api/docs/page` with the status preserved (reads cannot use `request()` for that). */
export async function readDocPage(path: string, options?: DocReadOptions): Promise<DocReadResult> {
  const res = await fetch(`/api/docs/page?${pageQuery(path, options)}`);
  const data = (await res.json().catch(() => ({}))) as DocReadResult & { error?: string };
  if (!res.ok) throw new DocRequestError(data.error ?? `${res.status} ${res.statusText}`, res.status);
  return data;
}

/** A failed save; keeps the HTTP status and the 422 parser diagnostics. */
export class DocSaveError extends Error {
  status: number;
  diagnostics: string[];
  constructor(message: string, status: number, diagnostics: string[] = []) {
    super(message);
    this.name = "DocSaveError";
    this.status = status;
    this.diagnostics = diagnostics;
  }
}

/**
 * `POST /api/docs/page` with the `X-Genie` guard: the compatibility write that
 * turns a whole page into one `write-page` edit in the caller's changeset. Uses
 * `fetch` directly (instead of the shared `request`) because a 422 carries the
 * per-field diagnostics in the body and 403/409 are states the editor names.
 */
export async function saveDoc(v: { path: string; content: string; mode: "create" | "update" | "upsert" }): Promise<DocsSaveResponse> {
  const res = await fetch("/api/docs/page", {
    method: "POST",
    headers: { "content-type": "application/json", "x-genie": "1" },
    body: JSON.stringify(v),
  });
  const data = (await res.json().catch(() => ({}))) as Partial<DocsSaveResponse> & { error?: string; diagnostics?: string[] };
  if (!res.ok) throw new DocSaveError(data.error ?? `${res.status} ${res.statusText}`, res.status, data.diagnostics ?? []);
  return data as DocsSaveResponse;
}

/** Removes one page under the same publishing policy as an edit. */
export const useRemoveDocPage = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (path: string) => request<{ removed: string }>("DELETE", `/api/docs/page?path=${encodeURIComponent(path)}`),
    onSuccess: () => void qc.invalidateQueries({ queryKey: docsKeys.all }),
  });
};

/** Removes a section folder with its pages and subsections; the server answers with the count. */
export const useRemoveDocSection = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (path: string) => request<{ removed: number; pages: string[] }>("DELETE", `/api/docs/section?path=${encodeURIComponent(path)}`),
    onSuccess: () => void qc.invalidateQueries({ queryKey: docsKeys.all }),
  });
};

export const useDocsTree = () => useQuery({ queryKey: docsKeys.tree, queryFn: () => request<DocsTreeResponse>("GET", "/api/docs/tree") });

/**
 * Docs change signal: poll `/api/docs/version` and invalidate the docs queries
 * when the filesystem signature moves (external edits, another browser tab).
 * SSE already invalidates everything on a server `change` event.
 */
export function useDocsVersion(pollMs = 5000) {
  const qc = useQueryClient();
  const query = useQuery({
    queryKey: docsKeys.version,
    queryFn: () => request<{ version: string }>("GET", "/api/docs/version"),
    refetchInterval: () => (typeof document !== "undefined" && document.visibilityState === "visible" ? pollMs : false),
  });
  const seen = useRef<string | undefined>(undefined);
  const version = query.data?.version;
  useEffect(() => {
    if (version === undefined) return;
    if (seen.current !== undefined && seen.current !== version) void qc.invalidateQueries({ queryKey: docsKeys.all });
    seen.current = version;
  }, [version, qc]);
  return query;
}

export const useDocSearch = (q: string, enabled = true) =>
  useQuery({
    queryKey: docsKeys.search(q),
    queryFn: () => request<DocsSearchResponse>("GET", `/api/docs/search?q=${encodeURIComponent(q)}&limit=30`),
    enabled: enabled && q.trim().length > 0,
  });

export const useDocPage = (path: string | undefined, options?: DocReadOptions) =>
  useQuery({
    queryKey: docsKeys.page(path ?? "", options),
    queryFn: () => readDocPage(path!, options),
    enabled: !!path,
  });

export const fetchDocPage = (path: string, options?: DocReadOptions) => readDocPage(path, options);

export const useSaveDoc = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: saveDoc,
    onSuccess: () => void qc.invalidateQueries({ queryKey: docsKeys.all }),
  });
};

export interface RenameDoc {
  path: string;
  title: string;
  /** A section without an index page: the index is created with the new title. */
  create?: boolean;
}

/**
 * Renames a page or a section. The title lives in the frontmatter, so the path and the body stay;
 * a section's name is its index page's title.
 */
export const useRenameDoc = () => {
  const qc = useQueryClient();
  const toast = useToast();
  return useMutation({
    mutationFn: async ({ path, title, create }: RenameDoc) => {
      if (create) {
        return saveDoc({ path, content: serializeDoc({ title, type: "", status: "", verified: "", tags: [], paths: [], related: [], summary: null, aliases: [] }, ""), mode: "create" });
      }
      const page = await fetchDocPage(path);
      return saveDoc({ path, content: serializeDoc({ ...fieldsFromPage(page), title }, page.content), mode: "update" });
    },
    onSuccess: () => void qc.invalidateQueries({ queryKey: docsKeys.all }),
    onError: (error) => toast(error instanceof Error ? error.message : String(error), "error"),
  });
};

// ---------------------------------------------------------------- changesets

/** The Changes feed (`GET /changesets`); filters are the route's own parameters. */
export const useChangesets = (filter: ChangesFilter = { view: "all" }) =>
  useQuery({
    queryKey: docsKeys.changesets(changesServerQuery(filter)),
    queryFn: () => request<Changeset[]>("GET", `/api/docs/changesets${changesServerQuery(filter) ? `?${changesServerQuery(filter)}` : ""}`),
  });

export const useChangeset = (id: number | undefined) =>
  useQuery({
    queryKey: docsKeys.changeset(id ?? 0),
    queryFn: async () => normalizeChangesetDetail(await request<Changeset & { commits?: CommitInfo[]; diff?: string; merge?: DocMerge }>("GET", `/api/docs/changesets/${id}`)),
    enabled: !!id,
  });

/** Integrate a changeset: `published` with its commit, or `merging` with the pending merge. */
export const usePublishChangeset = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => request<PublishResult>("POST", `/api/docs/changesets/${id}/publish`, {}),
    onSuccess: () => void qc.invalidateQueries({ queryKey: docsKeys.all }),
  });
};

/** Close a changeset without merging; the branch stays as a closed record. */
export const useDiscardChangeset = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => request<{ changeset?: Changeset }>("POST", `/api/docs/changesets/${id}/discard`, {}),
    onSuccess: () => void qc.invalidateQueries({ queryKey: docsKeys.all }),
  });
};

/** Owners mark a published changeset reviewed; creates neither commits nor sync traffic. */
export const useReviewChangeset = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => request<{ changeset?: Changeset }>("POST", `/api/docs/changesets/${id}/review`, { decision: "reviewed" }),
    onSuccess: () => void qc.invalidateQueries({ queryKey: docsKeys.all }),
  });
};

/** Create and publish the inverse changeset. */
export const useRevertChangeset = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (id: number) => request<{ changeset?: Changeset }>("POST", `/api/docs/changesets/${id}/revert`, {}),
    onSuccess: () => void qc.invalidateQueries({ queryKey: docsKeys.all }),
  });
};

/** Moves a page inside the caller's changeset; wiki-links move in the same edit. */
export const useMoveDocPage = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: { from: string; to: string }) => request<{ page: DocReadResult; changeset: Changeset }>("POST", "/api/docs/pages/move", body),
    onSuccess: () => void qc.invalidateQueries({ queryKey: docsKeys.all }),
  });
};

/** Applies `DocOp`s to the caller's changeset version of a page. */
export const useEditDoc = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: { path: string; ops: DocOp[]; note?: string; task?: string; changeset?: number }) =>
      request<{ changeset: Changeset; commit: CommitInfo; page: DocReadResult }>("POST", "/api/docs/edit", body),
    onSuccess: () => void qc.invalidateQueries({ queryKey: docsKeys.all }),
  });
};

// ---------------------------------------------------------------- history and versions

/** `CommitInfo[]` of a page on main, following moves. */
export const useDocHistory = (path: string | undefined) =>
  useQuery({
    queryKey: docsKeys.history(path ?? ""),
    queryFn: () => request<CommitInfo[]>("GET", `/api/docs/history?path=${encodeURIComponent(path!)}&limit=100`),
    enabled: !!path,
  });

/**
 * `GET /diff` answers with a unified diff. The route table does not pin its
 * envelope, so the text itself is accepted as well as `{ diff }`.
 */
export async function fetchDocDiff(path: string, from: string, to: string): Promise<string> {
  const params = new URLSearchParams({ path, from, to });
  const res = await fetch(`/api/docs/diff?${params}`);
  const text = await res.text();
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    try {
      message = (JSON.parse(text) as { error?: string }).error ?? message;
    } catch {
      // Not JSON: keep the status text.
    }
    throw new DocRequestError(message, res.status);
  }
  try {
    const data = JSON.parse(text) as { diff?: string } | string;
    return typeof data === "string" ? data : data.diff ?? "";
  } catch {
    return text;
  }
}

export const useDocDiff = (path: string | undefined, from: string | undefined, to: string | undefined) =>
  useQuery({
    queryKey: docsKeys.diff(path ?? "", from ?? "", to ?? ""),
    queryFn: () => fetchDocDiff(path!, from!, to!),
    enabled: !!path && !!from && !!to && from !== to,
  });

/** Publish a page's content at `at` as a new changeset. */
export const useRestoreDocVersion = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (body: { path: string; at: string }) => request<{ changeset?: Changeset; commit?: CommitInfo }>("POST", "/api/docs/restore", body),
    onSuccess: () => void qc.invalidateQueries({ queryKey: docsKeys.all }),
  });
};

// ---------------------------------------------------------------- pending merges

export const useDocMerge = (id: number | undefined) =>
  useQuery({
    queryKey: docsKeys.merge(id ?? 0),
    queryFn: async () => normalizeMergeView(await request<DocMerge & Partial<MergeView>>("GET", `/api/docs/merges/${id}`)),
    enabled: !!id,
  });

/** Stage resolved pages and publish the merge (`done`); `fail` hands it to people. */
export const useResolveDocMerge = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: { id: number; pages?: { path: string; content: string }[]; done?: boolean; fail?: boolean; contradiction?: boolean; note?: string }) =>
      request<{ merge: DocMerge }>("POST", `/api/docs/merges/${id}/resolve`, body),
    onSuccess: () => void qc.invalidateQueries({ queryKey: docsKeys.all }),
  });
};

/** Debounce for the search input, so typing does not hit the API per keystroke. */
export function useDebounced<T>(value: T, ms = 220): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), ms);
    return () => clearTimeout(timer);
  }, [value, ms]);
  return debounced;
}
