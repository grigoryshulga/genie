// The shared frame of the dialogs that edit fields of the role file, and the
// hooks that apply those fields to it and save them.

import { type ReactNode, useEffect, useRef, useState } from "react";
import { setFrontmatterKey, type RoleDetail, useSaveConfig } from "@/entities/agent-config";
import { Dialog, useToast } from "@/shared/ui";

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
  const toast = useToast();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const save = async (fields: RoleFields, ok: string) => {
    if (busy) return;
    setBusy(true);
    setError(undefined);
    try {
      await edit(fields);
      toast(ok);
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  };
  return [busy, save, error] as const;
}

/** A dialog that edits part of the role file. */
export function EditDialog({ title, note, busy, error, narrow, onClose, onSave, children }: { title: string; note?: string; busy?: boolean; error?: string; narrow?: boolean; onClose: () => void; onSave: () => void; children: ReactNode }) {
  const form = useRef<HTMLFormElement>(null);
  useEffect(() => {
    if (error) form.current?.scrollTo({ top: 0 });
  }, [error]);
  return (
    <Dialog
      title={title}
      onClose={onClose}
      wide={!narrow}
      footer={
        <>
          {note && <span className="ag-save-note">{note}</span>}
          <button type="button" className="g-btn ghost" onClick={onClose}>
            Отмена
          </button>
          <button type="submit" form="ag-edit" className="g-btn primary" disabled={busy}>
            Сохранить
          </button>
        </>
      }
    >
      <form
        ref={form}
        className="ag-form"
        id="ag-edit"
        onSubmit={(e) => {
          e.preventDefault();
          onSave();
        }}
      >
        {error && <div className="ag-warn" role="alert">{error}</div>}
        <fieldset className="ag-edit-fields" disabled={busy}>{children}</fieldset>
      </form>
    </Dialog>
  );
}
