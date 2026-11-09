import { useRef, useState } from "react";
import { useToast } from "@/shared/ui";
import { usePatchTask } from "../api.ts";
import type { Task, TaskDraft } from "../model.ts";

type TitleDraft = TaskDraft & { original: string };

/** Mount with the task ID as its key so a new task gets a new editor. */
export function EditableTaskTitle({ task, writable, as: Heading = "h1", className, tabIndex }: {
  task: Pick<Task, "id" | "title" | "editVersion">;
  writable: boolean;
  as?: "h1" | "h2";
  className?: string;
  tabIndex?: number;
}) {
  const patch = usePatchTask();
  const toast = useToast();
  const node = useRef<HTMLHeadingElement>(null);
  const [draft, setDraft] = useState<TitleDraft>();
  const cancel = (heading: HTMLHeadingElement) => {
    heading.textContent = task.title;
    setDraft(undefined);
  };
  return (
    <Heading
      ref={node}
      className={className}
      contentEditable={writable && !patch.isPending}
      suppressContentEditableWarning
      spellCheck={false}
      tabIndex={tabIndex}
      onFocus={(e) => {
        if (writable && !patch.isPending && !draft) {
          setDraft({ original: task.title, value: e.currentTarget.textContent ?? task.title, version: task.editVersion });
        }
      }}
      onInput={(e) => {
        const value = e.currentTarget.textContent ?? "";
        setDraft((draft) => ({ original: task.title, version: task.editVersion, ...draft, value }));
      }}
      onBlur={(e) => {
        if (!writable || patch.isPending) return;
        const heading = e.currentTarget;
        const title = (heading.textContent ?? "").trim();
        if (!draft || !title || title === draft.original || title === task.title) {
          cancel(heading);
          return;
        }
        patch.mutate({ id: task.id, patch: { title, expectedVersion: draft.version } }, {
          onSuccess: (saved) => {
            if (node.current === heading) {
              heading.textContent = saved.title;
              setDraft(undefined);
            }
          },
          onError: (error) => toast(`Не удалось: ${error.message}`, "error"),
        });
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") { e.preventDefault(); e.currentTarget.blur(); }
        if (e.key === "Escape") { e.preventDefault(); cancel(e.currentTarget); e.currentTarget.blur(); }
      }}
    >
      {/* Keep React's text child stable while the browser edits the DOM. This
          preserves the caret and draft through both input and live refreshes. */}
      {draft?.original ?? task.title}
    </Heading>
  );
}
