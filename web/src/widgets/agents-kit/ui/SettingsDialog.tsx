// The role's settings as a form: every changed field is written back to the
// role's file, the rest keep their inherited or built-in values.

import { useState } from "react";
import { allowDeny, basePermissions, FILES_TITLE, type Catalogue, type RoleDetail, STAGE_TITLE } from "@/entities/agent-config";
import { useMeta, useProjects } from "@/entities/project";
import { ModelPicker } from "@/features/agent-model";
import { ChipInput, Icon } from "@/shared/ui";
import { EditDialog, useRoleSave } from "./EditDialog.tsx";
import { PermissionFields, PermissionReference } from "./PermissionFields.tsx";
import { SkillsFields, useSkillsSettings } from "./SkillsFields.tsx";
import { McpFields, useMcpSettings } from "./McpFields.tsx";

const THINKING = ["", "off", "minimal", "low", "medium", "high", "xhigh", "max"];

/** One choice of a few, as a segmented control. */
function Segments({ label, value, options, onChange }: { label: string; value: string; options: [string, string][]; onChange: (v: string) => void }) {
  return (
    <div role="radiogroup" aria-label={label} className="g-seg ag-seg">
      {options.map(([v, title]) => (
        <button key={v || "default"} type="button" role="radio" aria-checked={value === v} aria-pressed={value === v} onClick={() => onChange(v)}>
          {title}
        </button>
      ))}
    </div>
  );
}

export function SettingsDialog({ detail, cfg, onClose }: { detail: RoleDetail; cfg: Catalogue; onClose: () => void }) {
  const r = detail.role;
  const initial = {
    capabilities: r.capabilities,
    title: r.title,
    description: r.description,
    model: r.model ?? "",
    thinking: r.thinking ?? "",
    files: r.files as string,
    stages: r.stages as string[],
    projects: r.projects ?? [],
    denyCommands: r.denyCommands,
    names: r.names,
    instructions: r.instructions ?? "",
  };
  const skills = useSkillsSettings(detail);
  const mcp = useMcpSettings(detail, cfg);
  const [initialMcp] = useState(mcp.grants);
  const [f, setF] = useState(initial);
  const [busy, saveFields, error] = useRoleSave(detail, onClose);
  const meta = useMeta().data;
  const projects = useProjects().data;
  const set = <K extends keyof typeof f>(k: K, v: (typeof f)[K]) => setF((x) => ({ ...x, [k]: v }));
  const save = async () => {
    const fields: [string, string | string[] | undefined][] = [];
    const text = (k: "title" | "description" | "model" | "thinking" | "instructions") => {
      if (f[k] !== initial[k]) fields.push([k, f[k].trim() ? f[k].trim() : undefined]);
    };
    const items = (k: "projects" | "denyCommands" | "names") => {
      if (f[k].join("\n") !== initial[k].join("\n")) fields.push([k, f[k].length ? f[k] : undefined]);
    };
    text("title");
    text("description");
    text("model");
    text("thinking");
    text("instructions");
    if (f.files !== initial.files) fields.push(["files", f.files]);
    if (f.stages.join() !== initial.stages.join()) fields.push(["stages", f.stages]);
    items("projects");
    items("denyCommands");
    items("names");
    if (JSON.stringify(f.capabilities) !== JSON.stringify(initial.capabilities)) {
      const { allow, deny } = allowDeny(basePermissions(r, cfg.roles, cfg.classes), f.capabilities, cfg.permissions);
      fields.push(["allow", allow], ["deny", deny]);
    }
    if (JSON.stringify(skills.value) !== JSON.stringify(r.skills)) fields.push(["skills", skills.value]);
    // Preserve wildcard grants and unknown entries unless access was actually edited.
    if (JSON.stringify(mcp.grants) !== JSON.stringify(initialMcp)) fields.push(["mcp", mcp.value()]);
    if (!fields.length) return onClose();
    await saveFields(fields, "Настройки роли сохранены");
  };
  const hint = (text: string) => <span className="hint">{text}</span>;
  return (
    <EditDialog title={`Настройки · ${r.title}`} note="Параметры сессии — со следующего запуска" busy={busy} error={error} onClose={onClose} onSave={() => void save()} narrow>
      <div className="ag-settings">
        <section>
          <h4>О роли</h4>
          <label htmlFor="rs-title">Название</label>
          <input id="rs-title" className="g-input" value={f.title} onChange={(e) => set("title", e.target.value)} required />
          <label htmlFor="rs-desc">
            Описание
          </label>
          <textarea id="rs-desc" className="g-textarea" rows={2} value={f.description} onChange={(e) => set("description", e.target.value)} />
        </section>
        <section>
          <h4>Модель</h4>
          <label htmlFor="rs-model">
            Модель
          </label>
          <ModelPicker id="rs-model" value={f.model} onChange={(m) => set("model", m)} fallback={meta?.roleModels?.[r.id]?.model} />
          <span className="lbl">Размышление</span>
          <Segments label="Размышление" value={f.thinking} options={THINKING.map((t) => [t, t || "по умолчанию"])} onChange={(v) => set("thinking", v)} />
        </section>
        <section>
          <h4>Доступ</h4>
          <span className="lbl">Файлы проекта</span>
          <Segments label="Файлы проекта" value={f.files} options={(["write", "read", "none"] as const).map((x) => [x, FILES_TITLE[x]])} onChange={(v) => set("files", v)} />
          <span className="lbl">
            Стадии задачи
          </span>
          <div className="ag-toggles">
            {(["refinement", "delivery"] as const).map((s) => {
              const on = f.stages.includes(s);
              return (
                <button key={s} type="button" aria-pressed={on} className={on ? "on" : ""} onClick={() => set("stages", on ? f.stages.filter((x) => x !== s) : [...f.stages, s])}>
                  <Icon.check />
                  {STAGE_TITLE[s]}
                </button>
              );
            })}
          </div>
          <label htmlFor="rs-deny">
            Запрещённые команды
            {hint("Shell; * — любой текст")}
          </label>
          <ChipInput id="rs-deny" values={f.denyCommands} onChange={(v) => set("denyCommands", v)} placeholder="git push*" mono />
        </section>
        {r.class !== "orchestrator" && <section className="ag-settings-stack">
          <h4>Разрешённые действия</h4>
          <p className="g-muted ag-note">Действуют сразу. Пояснения — в подсказках и технических сведениях.</p>
          <PermissionFields wanted={f.capabilities} onChange={(v) => set("capabilities", v)} />
          <PermissionReference />
        </section>}
        <section className="ag-settings-stack">
          <h4>Навыки</h4>
          <SkillsFields cfg={cfg} state={skills} />
        </section>
        <section className="ag-settings-stack">
          <h4>Инструменты MCP</h4>
          <McpFields cfg={cfg} state={mcp} />
        </section>
        <section>
          <h4>Команда</h4>
          <label htmlFor="rs-projects">
            Проекты
            {hint("Пусто — роль видна во всех")}
          </label>
          <ChipInput id="rs-projects" values={f.projects} onChange={(v) => set("projects", v)} placeholder="Добавить проект" suggestions={projects?.map((p) => p.slug)} />
          <label htmlFor="rs-names">
            Имена участников
            {hint("Пусто — имена по классу")}
          </label>
          <ChipInput id="rs-names" values={f.names} onChange={(v) => set("names", v)} placeholder="Новое имя" />
        </section>
        <section>
          <label htmlFor="rs-extra">
            Особенности роли
            {hint("Дописываются к промпту")}
          </label>
          <textarea id="rs-extra" className="g-textarea" rows={3} value={f.instructions} onChange={(e) => set("instructions", e.target.value)} />
        </section>
      </div>
    </EditDialog>
  );
}
