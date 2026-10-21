import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useSession } from "@/entities/session";
import { request } from "@/shared/api";
import type { TaskWatch } from "@/shared/api/generated/TaskWatch.ts";
import { Button, useToast } from "@/shared/ui";

export function FollowTask({ task }: { task: string }) {
  const session = useSession().data;
  const project = session?.project ?? "";
  const qc = useQueryClient();
  const toast = useToast();
  const key = ["task-watch", project, task];
  const url = `/api/tasks/${encodeURIComponent(task)}/watch?project=${encodeURIComponent(project)}`;
  const state = useQuery({
    queryKey: key,
    queryFn: () => request<TaskWatch>("GET", url),
    enabled: session?.mode !== "local" && !!project,
  });
  const mutation = useMutation({
    mutationFn: () => request<TaskWatch>(state.data?.watching ? "DELETE" : "PUT", url),
    onSuccess: (data) => qc.setQueryData(key, data),
    onError: (e) => toast(e.message, "error"),
  });
  if (session?.mode === "local") return null;
  if (state.data?.author)
    return (
      <span className="muted" title="Вы автор: уведомления включены автоматически">
        Уведомления включены
      </span>
    );
  return (
    <Button
      sm
      aria-pressed={state.data?.watching ?? false}
      disabled={!state.data || mutation.isPending}
      title={state.error?.message ?? "Новые комментарии и смена статуса"}
      onClick={() => mutation.mutate()}
    >
      {state.data?.watching ? "Слежу за задачей" : "Следить"}
    </Button>
  );
}
