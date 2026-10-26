// One draft and one save for the prompt and configuration, across both role tabs.
import { useState } from "react";
import { allowDeny, basePermissions, FILES_TITLE, THINKING_LEVELS, type Catalogue, type RoleDetail, STAGE_TITLE } from "@/entities/agent-config";
import { useMeta, useProjects } from "@/entities/project";
import { ModelPicker } from "@/features/agent-model";
import { Button, ChipInput, Field, Icon, IconButton, Input, Markdown, Select, Textarea } from "@/shared/ui";
import { useRoleSave } from "./EditDialog.tsx";
import { PermissionFields } from "./PermissionFields.tsx";
import { SkillsFields, useSkillsSettings } from "./SkillsFields.tsx";
import { McpFields, useMcpSettings } from "./McpFields.tsx";

export function RoleSettings({ detail: currentDetail, cfg: currentCfg, view, onClose }: { detail: RoleDetail; cfg: Catalogue; view: "prompt" | "settings"; onClose: () => void }) {
  const [preview, setPreview] = useState(false);
  // Keep the file hash and inherited values from when editing started.
  const [detail] = useState(currentDetail);
  const [cfg] = useState(currentCfg);
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
    prompt: r.prompt ?? "",
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
    const body = f.prompt !== initial.prompt ? f.prompt : undefined;
    if (!fields.length && body === undefined) return onClose();
    await saveFields(fields, "Роль сохранена", body);
  };

  return <form id="role-edit" className="role-editor" onSubmit={(e) => { e.preventDefault(); void save(); }}>
    <div className="role-save-bar"><Button variant="ghost" disabled={busy} onClick={onClose}>Отмена</Button><Button type="submit" variant="primary" busy={busy}>Сохранить</Button></div>
    {error && <p className="ag-warn" role="alert">{error}</p>}
    <fieldset className="ag-edit-fields" disabled={busy}>
      <div className="role-model-fields">
        <Field label="Модель"><ModelPicker value={f.model} onChange={(m) => set("model", m)} fallback={meta?.roleModels?.[r.id]?.model} /></Field>
        <Field label="Размышление"><Select value={f.thinking} onChange={(e) => set("thinking", e.target.value)}><option value="">По умолчанию</option>{THINKING_LEVELS.map((level) => <option key={level} value={level}>{level}</option>)}</Select></Field>
      </div>
      <div hidden={view !== "prompt"}>
        <section className="role-section role-prompt-editor">
          <div className="role-prompt-tools"><IconButton label={preview ? "Редактировать текст промпта" : "Предпросмотр промпта"} aria-pressed={preview} onClick={() => setPreview(!preview)}>{preview ? <Icon.pencil /> : <Icon.eye />}</IconButton></div>
          {preview ? <div className="role-prompt"><Markdown text={f.prompt} /></div> : <Textarea autoFocus={view === "prompt"} className="role-prompt-input" aria-label="Текст промпта" rows={20} spellCheck={false} value={f.prompt} onChange={(e) => set("prompt", e.target.value)} />}
          <Field label="Дополнительные инструкции"><Textarea rows={3} value={f.instructions} onChange={(e) => set("instructions", e.target.value)} /></Field>
        </section>
      </div>
      <div hidden={view !== "settings"} className="role-settings-form">
        <section className="role-section"><h3>О роли</h3><div className="role-field-grid">
          <Field label="Название"><Input autoFocus={view === "settings"} value={f.title} onChange={(e) => set("title", e.target.value)} required /></Field>
          <Field label="Описание"><Textarea rows={2} value={f.description} onChange={(e) => set("description", e.target.value)} /></Field>
        </div></section>
        <section className="role-section"><h3>Доступ к проекту</h3><div className="role-field-grid">
          <Field label="Файлы проекта"><Select value={f.files} onChange={(e) => set("files", e.target.value)}>{(["write", "read", "none"] as const).map((mode) => <option key={mode} value={mode}>{FILES_TITLE[mode]}</option>)}</Select></Field>
          <Field label="Запрещённые команды"><ChipInput values={f.denyCommands} onChange={(v) => set("denyCommands", v)} placeholder="git push*" mono /></Field>
        </div></section>
        <section className="role-section"><h3>Участие в работе</h3>
          <div className="role-stage-options">{(["refinement", "delivery"] as const).map((stage) => <label className="g-check" key={stage}><input type="checkbox" checked={f.stages.includes(stage)} onChange={(e) => set("stages", e.target.checked ? [...f.stages, stage] : f.stages.filter((s) => s !== stage))} />{STAGE_TITLE[stage]}</label>)}</div>
          <div className="role-field-grid"><Field label="Проекты"><ChipInput values={f.projects} onChange={(v) => set("projects", v)} placeholder="Все проекты" suggestions={projects?.map((p) => p.slug)} /></Field><Field label="Имена участников"><ChipInput values={f.names} onChange={(v) => set("names", v)} placeholder="По классу роли" /></Field></div>
        </section>
        {r.class !== "orchestrator" && <section className="role-section"><h3>Разрешённые действия</h3><PermissionFields wanted={f.capabilities} onChange={(v) => set("capabilities", v)} /></section>}
        <section className="role-section"><h3>Навыки</h3><SkillsFields cfg={cfg} state={skills} compact /></section>
        <section className="role-section"><h3>Инструменты MCP</h3><McpFields cfg={cfg} state={mcp} /></section>
      </div>
    </fieldset>
  </form>;
}
