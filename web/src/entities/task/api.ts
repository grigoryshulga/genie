import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useMemo } from "react";
import { invalidateKeys, keys, keysFor, request, useInvalidating } from "@/shared/api";
import type { CommentBody, CreateBody, DocsImpact, StatusBody, UpdateBody } from "@/shared/api";
import type { Task, TaskSummary } from "./model.ts";

/** Every task, closed ones included; views and the board filter on the client. */
export const useTasks = () => useQuery({ queryKey: keys.tasks, queryFn: () => request<TaskSummary[]>("GET", "/api/tasks?closed=1") });

/** Epics by id, from the cached task list. */
export function useEpicMap(): Map<string, TaskSummary> {
  const tasks = useTasks().data;
  return useMemo(() => new Map((tasks ?? []).filter((t) => t.type === "epic").map((t) => [t.id, t])), [tasks]);
}

export const useTask = (id: string | undefined) =>
  useQuery({ queryKey: keys.task(id ?? ""), queryFn: () => request<Task>("GET", `/api/tasks/${encodeURIComponent(id!)}`), enabled: !!id });

/**
 * Non-blocking docs-impact hint for a task; the caller gates `enabled` on the
 * review/done status. Purely additive: no existing URL shape or caching changes.
 */
export const useDocsImpact = (id: string, enabled: boolean) =>
  useQuery({
    queryKey: [...keys.task(id), "docs-impact"] as const,
    queryFn: () => request<DocsImpact>("GET", `/api/tasks/${encodeURIComponent(id)}/docs-impact`),
    enabled: enabled && !!id,
  });

export function useMoveTask() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, ...body }: { id: string } & StatusBody) => request<Task>("POST", `/api/tasks/${encodeURIComponent(id)}/status`, body),
    // Optimistic: the card jumps to its column immediately.
    onMutate: async (v) => {
      await qc.cancelQueries({ queryKey: keys.tasks });
      const prev = qc.getQueryData<TaskSummary[]>(keys.tasks);
      qc.setQueryData<TaskSummary[]>(keys.tasks, (old) => old?.map((t) => (t.id === v.id ? { ...t, status: v.status } : t)));
      return { prev };
    },
    onError: (_e, _v, ctx) => ctx?.prev && qc.setQueryData(keys.tasks, ctx.prev),
    // A move appends `task.status_changed`: the same queries the stream would refetch.
    onSettled: () => invalidateKeys(qc, keysFor({ type: "task.status_changed" })),
  });
}

export const useComment = () =>
  useInvalidating(
    ({ id, ...body }: { id: string } & CommentBody) => request<Task>("POST", `/api/tasks/${encodeURIComponent(id)}/comments`, body),
    keysFor({ type: "task.commented" }),
  );
export const useCheck = () =>
  useInvalidating(
    (v: { id: string; n: number; done: boolean }) => request<Task>("POST", `/api/tasks/${encodeURIComponent(v.id)}/acceptance/${v.n}`, { done: v.done }),
    keysFor({ type: "task.criterion_checked" }),
  );
export const usePatchTask = () =>
  useInvalidating(
    (v: { id: string; patch: UpdateBody }) => request<Task>("PATCH", `/api/tasks/${encodeURIComponent(v.id)}`, v.patch),
    keysFor({ type: "task.updated" }),
  );
export const useCreateTask = () =>
  useInvalidating((v: CreateBody) => request<Task>("POST", "/api/tasks", v), keysFor({ type: "task.created" }));
/** Delete a task for good; `cascade` takes its subtasks along (the server refuses otherwise). */
export function useDeleteTask() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (v: { id: string; cascade?: boolean }) =>
      request<{ ok: boolean; deleted: string[] }>("DELETE", `/api/tasks/${encodeURIComponent(v.id)}${v.cascade ? "?cascade=1" : ""}`),
    onSuccess: (r) => {
      // Drop the deleted tasks first: refetching one of them would only fail.
      for (const id of r.deleted) qc.removeQueries({ queryKey: keys.task(id) });
    },
    // A deletion appends `task.deleted`: the same queries the stream would refetch.
    onSettled: () => invalidateKeys(qc, keysFor({ type: "task.deleted" })),
  });
}
export const useAddArtifact = () =>
  useInvalidating(
    (v: { id: string; name: string; kind: string; text: string; note?: string }) =>
      request<Task>("POST", `/api/tasks/${encodeURIComponent(v.id)}/artifacts`, { name: v.name, kind: v.kind, text: v.text, note: v.note }),
    keysFor({ type: "task.artifact_added" }),
  );

export async function fetchArtifact(task: string, n: number): Promise<{ name: string; kind: string; size: number; text?: string; mime?: string }> {
  return request("GET", `/api/tasks/${encodeURIComponent(task)}/artifacts/${n}`);
}
