import { QueryClient, useMutation, useQueryClient } from "@tanstack/react-query";
import { type KeyPrefix, matchesAny } from "./invalidation.ts";

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status = 0,
  ) {
    super(message);
  }
}

export async function request<T>(method: string, url: string, body?: unknown): Promise<T> {
  const res = await fetch(url, {
    method,
    headers: body === undefined ? { "x-genie": "1" } : { "content-type": "application/json", "x-genie": "1" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = (await res.json().catch(() => ({}))) as T & { error?: string };
  if (!res.ok) throw new ApiError(data.error ?? `${res.status} ${res.statusText}`, res.status);
  return data;
}

/** Send a file as the request body (a photo): same headers and errors as `request`. */
export async function upload<T>(method: string, url: string, body: Blob): Promise<T> {
  const res = await fetch(url, { method, headers: { "content-type": body.type || "application/octet-stream", "x-genie": "1" }, body });
  const data = (await res.json().catch(() => ({}))) as T & { error?: string };
  if (!res.ok) throw new ApiError(data.error ?? `${res.status} ${res.statusText}`, res.status);
  return data;
}

export const queryClient = new QueryClient({
  defaultOptions: { queries: { staleTime: 30_000, refetchOnWindowFocus: false, retry: 1 } },
});

export const keys = {
  meta: ["meta"] as const,
  tasks: ["tasks"] as const,
  teams: ["teams"] as const,
  task: (id: string) => ["task", id] as const,
  team: (id: string) => ["team", id] as const,
};

/**
 * Refetch the queries the prefixes match; `null` (an event type the client does
 * not know) has no name for what changed, so everything is refetched.
 */
export function invalidateKeys(qc: QueryClient, prefixes: readonly KeyPrefix[] | null) {
  return prefixes ? qc.invalidateQueries({ predicate: (q) => matchesAny(prefixes, q.queryKey) }) : qc.invalidateQueries();
}

/**
 * A mutation that refetches after it settles. Pass the key prefixes of what it
 * makes stale; without them every query is refetched, as before.
 */
export function useInvalidating<V, R>(fn: (v: V) => Promise<R>, prefixes?: readonly KeyPrefix[] | null) {
  const qc = useQueryClient();
  return useMutation({ mutationFn: fn, onSettled: () => invalidateKeys(qc, prefixes ?? null) });
}
