import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useSession } from "@/entities/session";
import { request } from "@/shared/api";
import type { TaskWatch } from "@/shared/api/generated/TaskWatch.ts";
import { Icon, IconButton, useToast } from "@/shared/ui";
import "./follow-task.css";

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
  const watching = state.data?.watching ?? false;
  return (
    <IconButton
      label="Следить"
      aria-pressed={watching}
      className={watching ? "task-follow on" : "task-follow"}
      disabled={!state.data || state.data.author || mutation.isPending}
      title={state.error?.message ?? (state.data?.author ? "Вы автор: уведомления включены автоматически" : watching ? "Не следить за задачей" : "Следить за задачей")}
      onClick={() => mutation.mutate()}
    >
      {watching ? <Icon.eye /> : <Icon.eyeOff />}
    </IconButton>
  );
}
