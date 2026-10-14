// The role's skills: all installed ones or a chosen subset, saved as the
// `skills` field of the role file.

import { useState } from "react";
import { type Catalogue, type RoleDetail } from "@/entities/agent-config";
import { useAction } from "./common.tsx";
import { EditDialog, useRoleEdit } from "./EditDialog.tsx";

export function SkillsDialog({ detail, cfg, onClose }: { detail: RoleDetail; cfg: Catalogue; onClose: () => void }) {
  const r = detail.role;
  const [only, setOnly] = useState(r.skills !== undefined);
  const [chosen, setChosen] = useState<string[]>(r.skills ?? []);
  const [busy, setBusy] = useState(false);
  const edit = useRoleEdit(detail);
  const act = useAction();
  const save = async () => {
    setBusy(true);
    if (await act(() => edit([["skills", only ? chosen : undefined]]), "Навыки роли сохранены")) onClose();
    setBusy(false);
  };
  return (
    <EditDialog title={`Навыки роли ${r.id}`} note="Агенты получат навыки со следующего старта сессии" busy={busy} onClose={onClose} onSave={() => void save()}>
      <div className="ag-choice">
        <label>
          <input type="radio" checked={!only} onChange={() => setOnly(false)} />
          Все навыки, установленные для pi, и навыки репозитория
        </label>
        <label>
          <input type="radio" checked={only} onChange={() => setOnly(true)} />
          Только выбранные и навыки репозитория
        </label>
        {only && (
          <div className="ag-checks">
            {cfg.skills.map((s) => (
              <label key={s.name} title={s.description}>
                <input type="checkbox" checked={chosen.includes(s.name)} onChange={(e) => setChosen((c) => (e.target.checked ? [...c, s.name] : c.filter((x) => x !== s.name)))} />
                <span className="mono">{s.name}</span>
                <span className="muted">{s.description}</span>
              </label>
            ))}
            {!cfg.skills.length && <span className="muted">В библиотеке пока нет навыков: добавьте их на вкладке «Навыки».</span>}
          </div>
        )}
      </div>
    </EditDialog>
  );
}
