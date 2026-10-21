import { useState } from "react";
import { useAgentConfig } from "@/entities/agent-config";
import { ROLE_TITLE_RU } from "@/entities/member";
import { useMeta } from "@/entities/project";
import { useAddMember } from "@/entities/team";
import { Icon, Modal, useToast } from "@/shared/ui";

/** The classic five, for a server without configurable roles. */
const CLASSIC = ["analyst", "executor", "reviewer", "tester", "documenter"].map((id) => ({ id, title: ROLE_TITLE_RU[id] ?? id, description: "" }));

export function AddMemberDialog({ team, onClose }: { team: string; onClose: () => void }) {
  const meta = useMeta().data;
  const cfg = useAgentConfig().data;
  const add = useAddMember();
  const toast = useToast();
  const roles = cfg ? cfg.roles.filter((r) => r.class !== "orchestrator") : CLASSIC;
  const [role, setRole] = useState("tester");
  const described = roles.find((r) => r.id === role)?.description;
  const [name, setName] = useState("");
  const [model, setModel] = useState("");
  const [instructions, setInstructions] = useState("");
  const defaultModel = meta?.roleModels?.[role]?.model;

  const submit = () =>
    add.mutate(
      { team, role, name: name.trim() || undefined, model: model.trim() || undefined, instructions: instructions.trim() || undefined },
      {
        onSuccess: (added) => {
          toast(`${added.map((a) => a.name).join(", ")} присоединился к команде ${team}`);
          onClose();
        },
        onError: (e) => toast(`Не добавлен: ${e.message}`, "error"),
      },
    );

  return (
    <Modal label="Добавить участника" onClose={onClose}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
        className="dlg-form"
      >
        <div className="mh">
          <Icon.userPlus />
          Добавить участника в команду {team}
          <span className="grow" />
          <button type="button" className="g-icon-btn" onClick={onClose} aria-label="Закрыть">
            <Icon.close />
          </button>
        </div>
        <div className="mb">
          <label className="g-field">
            <span className="g-label">Роль</span>
            <select className="g-select" value={role} onChange={(e) => setRole(e.target.value)} autoFocus>
              {roles.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.title}
                  {cfg ? ` (${r.id})` : ""}
                </option>
              ))}
            </select>
          </label>
          {described && <p className="dlg-note">{described}</p>}
          <label className="g-field">
            <span className="g-label">Имя — латиницей, необязательно (иначе подберётся само)</span>
            <input className="g-input" value={name} onChange={(e) => setName(e.target.value)} placeholder="например, murphy" pattern="[a-zA-Z][a-zA-Z0-9_-]*" />
          </label>
          <label className="g-field">
            <span className="g-label">Модель — provider/id, необязательно</span>
            <input className="g-input" value={model} onChange={(e) => setModel(e.target.value)} placeholder={defaultModel ? `по умолчанию: ${defaultModel}` : "модель pi по умолчанию"} />
          </label>
          <label className="g-field">
            <span className="g-label">Что ему сделать</span>
            <textarea rows={3} value={instructions} onChange={(e) => setInstructions(e.target.value)} placeholder="Например: покрыть тестами граничные случаи экспорта" />
          </label>
        </div>
        <div className="mf">
          Участник получит вводное сообщение, команда — уведомление
          <span className="grow" />
          <button type="button" className="g-btn ghost" onClick={onClose}>
            Отмена
          </button>
          <button type="submit" className="g-btn primary" disabled={add.isPending}>
            Добавить
          </button>
        </div>
      </form>
    </Modal>
  );
}
