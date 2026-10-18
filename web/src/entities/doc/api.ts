import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useEffect, useRef, useState } from "react";
import { request } from "@/shared/api";
import type { DocPage, DocReadResult, DocSearchResult } from "./model.ts";

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
}

/** Local keys: `shared/api/client.ts` is owned by another task. */
export const docsKeys = {
  all: ["docs"] as const,
  version: ["docs", "version"] as const,
  tree: ["docs", "tree"] as const,
  search: (q: string) => ["docs", "search", q] as const,
  page: (path: string) => ["docs", "page", path] as const,
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

/** `GET /api/docs/page` with the status preserved (reads cannot use `request()` for that). */
export async function readDocPage(path: string): Promise<DocReadResult> {
  const res = await fetch(`/api/docs/page?path=${encodeURIComponent(path)}`);
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
 * `POST /api/docs/page` with the `X-Genie` guard. Uses `fetch` directly (instead
 * of the shared `request`) because a 422 carries the per-field diagnostics in the
 * body and 404/409 mean a genuine save conflict — the editor must tell them apart.
 */
export async function saveDoc(v: { path: string; content: string; mode: "create" | "update" | "upsert" }): Promise<DocsSaveResponse> {
  const res = await fetch("/api/docs/page", {
    method: "POST",
    headers: { "content-type": "application/json", "x-genie": "1" },
    body: JSON.stringify(v),
  });
  const data = (await res.json().catch(() => ({}))) as Partial<DocsSaveResponse> & { error?: string; diagnostics?: string[] };
  if (res.status === 202) throw new DocSaveError("Изменения отправлены на проверку владельцу раздела. Статья появится после одобрения.", 202);
  if (!res.ok) throw new DocSaveError(data.error ?? `${res.status} ${res.statusText}`, res.status, data.diagnostics ?? []);
  return data as DocsSaveResponse;
}

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

export const useDocPage = (path: string | undefined) =>
  useQuery({
    queryKey: docsKeys.page(path ?? ""),
    queryFn: () => readDocPage(path!),
    enabled: !!path,
  });

export const fetchDocPage = (path: string) => readDocPage(path);

export const useSaveDoc = () => {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: saveDoc,
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
