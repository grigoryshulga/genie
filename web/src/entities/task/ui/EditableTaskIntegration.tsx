import { useState } from "react";
import { Input, useToast } from "@/shared/ui";
import { usePatchTask } from "../api.ts";
import type { Task, TaskDraft } from "../model.ts";

type IntegrationDraft = TaskDraft & { original: string };

/** Mount with the task ID as its key; server refreshes never replace a draft. */
export function EditableTaskIntegration({ task }: {
  task: Pick<Task, "id" | "mergeStrategy" | "editVersion">;
}) {
  const patch = usePatchTask();
  const toast = useToast();
  const [draft, setDraft] = useState<IntegrationDraft>();
  const begin = () => ({ original: task.mergeStrategy, value: task.mergeStrategy, version: task.editVersion });
  return (
    <Input
      sm
      aria-label="Интеграция"
      placeholder="не задана"
      value={draft?.value ?? task.mergeStrategy}
      readOnly={patch.isPending}
      onFocus={() => { if (!patch.isPending) setDraft((draft) => draft ?? begin()); }}
      onChange={(event) => {
        const value = event.target.value;
        setDraft((draft) => ({ ...(draft ?? begin()), value }));
      }}
      onBlur={() => {
        if (!draft || patch.isPending) return;
        if (draft.value === draft.original || draft.value === task.mergeStrategy) {
          setDraft(undefined);
          return;
        }
        patch.mutate({ id: task.id, patch: { mergeStrategy: draft.value, expectedVersion: draft.version } }, {
          onSuccess: () => setDraft(undefined),
          onError: (error) => toast(`Не удалось: ${error.message}`, "error"),
        });
      }}
      onKeyDown={(event) => {
        if (event.key === "Enter") { event.preventDefault(); event.currentTarget.blur(); }
        if (event.key === "Escape") { event.preventDefault(); setDraft(undefined); }
      }}
    />
  );
}
