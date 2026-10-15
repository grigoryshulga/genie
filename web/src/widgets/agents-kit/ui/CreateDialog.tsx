// Creating a role, a team template or a skill: the identifier, the title and
// the description — the file itself appears in the server's data directory.

import { useState } from "react";
import { type Catalogue, CLASS_TITLE, newRoleFile, newTemplate, setFrontmatterKey, useSaveConfig } from "@/entities/agent-config";
import { Icon, Modal } from "@/shared/ui";

export type CreateKind = "roles" | "templates" | "skills";

const NEW_LABEL: Record<CreateKind, string> = { roles: "Новая роль", templates: "Новый шаблон", skills: "Новый навык" };

const ID = /^[a-z][a-z0-9-]*$/;

export function CreateDialog({ kind, cfg, onClose, onCreated }: { kind: CreateKind; cfg: Catalogue; onClose: () => void; onCreated: (id: string) => void }) {
  const [id, setId] = useState("");
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [base, setBase] = useState("class:executor");
  const [body, setBody] = useState("");
  const [error, setError] = useState<string>();
  const save = useSaveConfig();
  const taken = kind === "roles" ? cfg.roles.some((r) => r.id === id) : kind === "templates" ? cfg.teams.some((t) => t.id === id) : cfg.skills.some((s) => s.name === id);
  const submit = async () => {
    setError(undefined);
    try {
      if (kind === "roles") {
        const [how, what] = base.split(":");
        const content = newRoleFile({
          title,
          description,
          base: how === "class" ? what : undefined,
          extends: how === "role" ? what : undefined,
          prompt: body || `You are the **${title}** of a focus team.\n`,
        });
        await save("role", id, { content }, "");
      } else if (kind === "templates") {
        await save("template", id, { template: newTemplate(title, description) }, "");
      } else {
        const text = setFrontmatterKey(`---\n---\n${body || `# ${id}\n`}`, "name", id);
        const content = setFrontmatterKey(text, "description", description.replace(/\s+/g, " ").trim());
        await save("skill", id, { content }, "");
      }
      onCreated(id);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };
  const label = NEW_LABEL[kind];
  return (
    <Modal label={label} onClose={onClose}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
        style={{ display: "flex", flexDirection: "column", minHeight: 0 }}
      >
        <div className="mh">
          {label}
          <span className="grow" />
          <button type="button" className="icon-btn" onClick={onClose} aria-label="Закрыть">
            <Icon.close />
          </button>
        </div>
        <div className="mb">
          <label className="field">
            {kind === "skills" ? "Имя навыка" : "Идентификатор"} — латиницей, строчными, через дефис
            <input value={id} onChange={(e) => setId(e.target.value.trim())} pattern="[a-z][a-z0-9-]*" required autoFocus placeholder={kind === "roles" ? "security-reviewer" : kind === "templates" ? "security-review" : "owasp-review"} />
          </label>
          {id && !ID.test(id) && <span className="ag-warn">Только a–z, цифры и дефис, с буквы.</span>}
          {taken && <span className="ag-warn">Такой уже есть.</span>}
          {kind !== "skills" && (
            <label className="field">
              Название
              <input value={title} onChange={(e) => setTitle(e.target.value)} required placeholder={kind === "roles" ? "Ревьюер безопасности" : "Ревью безопасности"} />
            </label>
          )}
          <label className="field">
            Описание — {kind === "skills" ? "когда агенту им пользоваться" : "по нему оркестратор выбирает"}
            <textarea rows={2} value={description} onChange={(e) => setDescription(e.target.value)} required />
          </label>
          {kind === "roles" && (
            <label className="field">
              Основа
              <select value={base} onChange={(e) => setBase(e.target.value)}>
                <optgroup label="Класс процесса — его разрешения по умолчанию">
                  {(["analyst", "executor", "reviewer", "tester", "documenter"] as const).map((k) => (
                    <option key={k} value={`class:${k}`}>
                      {CLASS_TITLE[k]}
                    </option>
                  ))}
                </optgroup>
                <optgroup label="Роль — промпт и настройки наследуются">
                  {cfg.roles
                    .filter((r) => r.class !== "orchestrator")
                    .map((r) => (
                      <option key={r.id} value={`role:${r.id}`}>
                        {r.title} ({r.id})
                      </option>
                    ))}
                </optgroup>
              </select>
            </label>
          )}
          {kind !== "templates" && (
            <label className="field">
              {kind === "roles" ? "Промпт роли (Markdown)" : "Инструкции навыка (Markdown)"}
              <textarea rows={6} value={body} onChange={(e) => setBody(e.target.value)} placeholder={kind === "roles" && base.startsWith("role:") ? "Пусто — промпт родительской роли" : ""} />
            </label>
          )}
          {kind === "templates" && <p className="muted ag-note">Шаблон начнётся с пары «исполнитель → ревьюер»: состав и связи правятся в редакторе шаблона.</p>}
          {error && (
            <div className="auth-error" role="alert">
              {error}
            </div>
          )}
        </div>
        <div className="mf">
          Файл появится в каталоге данных сервера
          <span className="grow" />
          <button type="button" className="btn ghost" onClick={onClose}>
            Отмена
          </button>
          <button type="submit" className="btn primary" disabled={!ID.test(id) || taken}>
            Создать
          </button>
        </div>
      </form>
    </Modal>
  );
}
