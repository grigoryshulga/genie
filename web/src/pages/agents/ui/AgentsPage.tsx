// «Агенты»: roles, team templates, skills and MCP connections of the server
// (decision PD13). Everyone sees them; server administrators edit them (PD14).

import { useState } from "react";
import { useSearchParams } from "react-router";
import { type Catalogue, CLASS_TITLE, newRoleFile, newTemplate, setFrontmatterKey, useAgentConfig, useSaveConfig } from "@/entities/agent-config";
import { Icon, Modal } from "@/shared/ui";
import { History, Problems } from "@/widgets/settings-kit";
import { McpTab, SkillsTab } from "./LibraryTabs.tsx";
import { RolesTab } from "./RolesTab.tsx";
import { TemplatesTab } from "./TemplatesTab.tsx";
import "./agents.css";

type Tab = "roles" | "templates" | "skills" | "mcp" | "history";
const TABS: { id: Tab; title: string; admin?: boolean }[] = [
  { id: "roles", title: "Роли" },
  { id: "templates", title: "Шаблоны команд" },
  { id: "skills", title: "Навыки" },
  { id: "mcp", title: "MCP" },
  { id: "history", title: "История", admin: true },
];
const NEW: Partial<Record<Tab, string>> = { roles: "Новая роль", templates: "Новый шаблон", skills: "Новый навык" };

export function AgentsPage() {
  const [sp, setSp] = useSearchParams();
  const cfg = useAgentConfig();
  const [creating, setCreating] = useState<Tab>();
  const tab = (TABS.some((t) => t.id === sp.get("tab")) ? sp.get("tab") : "roles") as Tab;
  const selected = sp.get("id") ?? undefined;
  const select = (id: string) => setSp({ tab, id });
  const c = cfg.data;
  const errors = c?.problems.filter((p) => p.level === "error") ?? [];
  return (
    <main className="main agents">
      <header className="topbar">
        <h1>Агенты</h1>
        <div className="seg" role="tablist" aria-label="Разделы">
          {TABS.filter((t) => !t.admin || c?.admin).map((t) => (
            <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} className={tab === t.id ? "on" : ""} onClick={() => setSp({ tab: t.id })}>
              {t.title}
            </button>
          ))}
        </div>
        <span className="grow" />
        {c && !c.admin && <span className="sub d-only">настраивают администраторы сервера</span>}
        {c?.admin && NEW[tab] && (
          <button type="button" className="btn primary" onClick={() => setCreating(tab)}>
            <Icon.plus size={13} />
            <span className="d-only">{NEW[tab]}</span>
          </button>
        )}
      </header>
      {c?.admin && errors.length > 0 && (
        <div className="ag-banner">
          <Problems items={errors} />
          <span className="muted">Сломанные файлы не останавливают работу: действует их последняя рабочая версия.</span>
        </div>
      )}
      {!c ? (
        <div className="pane-empty">{cfg.error ? cfg.error.message : "Загрузка…"}</div>
      ) : tab === "roles" ? (
        <RolesTab cfg={c} selected={selected} onSelect={select} />
      ) : tab === "templates" ? (
        <TemplatesTab cfg={c} selected={selected} onSelect={select} />
      ) : tab === "skills" ? (
        <SkillsTab cfg={c} selected={selected} onSelect={select} />
      ) : tab === "mcp" ? (
        <McpTab cfg={c} />
      ) : (
        <div className="ag-page">
          <History admin={c.admin} />
        </div>
      )}
      {creating && c && (
        <CreateDialog
          kind={creating}
          cfg={c}
          onClose={() => setCreating(undefined)}
          onCreated={(id) => {
            setCreating(undefined);
            setSp({ tab: creating, id });
          }}
        />
      )}
    </main>
  );
}

const ID = /^[a-z][a-z0-9-]*$/;

function CreateDialog({ kind, cfg, onClose, onCreated }: { kind: Tab; cfg: Catalogue; onClose: () => void; onCreated: (id: string) => void }) {
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
  const label = NEW[kind] ?? "";
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
