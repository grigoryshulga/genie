// The shared frame of the dialogs that edit fields of the role file, and the
// hooks that apply those fields to it and save them.

import { type ReactNode, useState } from "react";
import { setFrontmatterKey, type RoleDetail, useSaveConfig } from "@/entities/agent-config";
import { useAction } from "./common.tsx";
import { Icon, Modal } from "@/shared/ui";

/** Changed frontmatter fields of a role file: a key with its new value (`undefined` removes it). */
type RoleFields = [string, string | string[] | undefined][];

/** Save changed fields of the role file. */
export function useRoleEdit(detail: RoleDetail) {
  const save = useSaveConfig();
  return (fields: RoleFields) => {
    let text = detail.file.content ?? "---\n---\n";
    for (const [k, v] of fields) text = setFrontmatterKey(text, k, v);
    return save("role", detail.role.id, { content: text }, detail.file.hash);
  };
}

/** Save changed fields of the role file: `busy` while saving, the dialog closed on success. */
export function useRoleSave(detail: RoleDetail, onClose: () => void) {
  const edit = useRoleEdit(detail);
  const act = useAction();
  const [busy, setBusy] = useState(false);
  const save = async (fields: RoleFields, ok: string) => {
    setBusy(true);
    if (await act(() => edit(fields), ok)) onClose();
    setBusy(false);
  };
  return [busy, save] as const;
}

/** A dialog that edits part of the role file. */
export function EditDialog({ title, note, busy, narrow, onClose, onSave, children }: { title: string; note?: string; busy?: boolean; narrow?: boolean; onClose: () => void; onSave: () => void; children: ReactNode }) {
  return (
    <Modal label={title} onClose={onClose} wide={!narrow}>
      <div className="mh">
        {title}
        <span className="grow" />
        <button type="button" className="icon-btn" onClick={onClose} aria-label="Закрыть">
          <Icon.close />
        </button>
      </div>
      <form
        className="mb ag-form"
        id="ag-edit"
        onSubmit={(e) => {
          e.preventDefault();
          onSave();
        }}
      >
        {children}
      </form>
      <div className="mf">
        {note}
        <span className="grow" />
        <button type="button" className="btn ghost" onClick={onClose}>
          Отмена
        </button>
        <button type="submit" form="ag-edit" className="btn primary" disabled={busy}>
          Сохранить
        </button>
      </div>
    </Modal>
  );
}
