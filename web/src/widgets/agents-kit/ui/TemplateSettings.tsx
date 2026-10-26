import { useState } from "react";
import { memberKey, nextMemberKey, removeTemplateMember, updateTemplateMember, templateDraft, templateRelations, templateTransferFile, TEMPLATE_FORMAT, MAIL_TITLE, ON_STATUSES, ON_TITLE, RELATION_TITLE, STAGE_TITLE, THINKING_LEVELS, TemplateGraph, WORKSPACE_TITLE, useSaveConfig, type Catalogue, type Relation, type RelKind, type TemplateDetail, type TemplateDraft, type TemplateFile, type TemplateMember } from "@/entities/agent-config";
import { useProjects } from "@/entities/project";
import { ModelPicker } from "@/features/agent-model";
import { Button, ChipInput, Field, Icon, IconButton, Input, Select, Textarea, useToast } from "@/shared/ui";
import { MemberAvatar } from "./TemplateViews.tsx";

const KINDS: RelKind[] = ["handoff", "returns", "reports", "consults"];
export function TemplateSettings({ detail: initialDetail, cfg, view, onClose }: { detail: TemplateDetail; cfg: Catalogue; view: "team" | "settings"; onClose: () => void }) {
  const [detail] = useState(initialDetail);
  const [draft, setDraft] = useState(() => templateDraft(detail));
  const [memberIds, setMemberIds] = useState(() => draft.members.map(() => crypto.randomUUID()));
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const save = useSaveConfig();
  const toast = useToast();
  const projects = useProjects().data;
  const roles = cfg.roles.filter((r) => r.class !== "orchestrator");
  const set = <K extends keyof TemplateFile>(key: K, value: TemplateFile[K]) => setDraft((d) => ({ ...d, [key]: value }));
  const members = draft.members;
  const keys = members.map(memberKey);
  const relations = templateRelations(draft, cfg.roles);
  const title = (key: string) => key === "orchestrator" ? "Менеджер задачи" : members.find((m) => memberKey(m) === key)?.name || cfg.roles.find((r) => r.id === members.find((m) => memberKey(m) === key)?.role)?.title || key;
  const setRelation = (index: number, patch: Partial<Relation>) => set("relations", relations.map((r, i) => i === index ? { ...r, ...patch } : r));
  const addMember = () => {
    const role = roles[0]?.id ?? "executor";
    set("members", [...members, { role, key: nextMemberKey(role, members) }]);
    setMemberIds((ids) => [...ids, crypto.randomUUID()]);
  };
  const submit = async () => {
    if (busy) return;
    setError(undefined); setBusy(true);
    try {
      const clean: TemplateDraft = JSON.parse(JSON.stringify(draft));
      const file = templateTransferFile({ ...clean, format: TEMPLATE_FORMAT, id: detail.template.id }, cfg);
      await save("template", detail.template.id, { template: file }, detail.file.path.startsWith("teams/") ? detail.file.hash : "");
      toast("Шаблон сохранён"); onClose();
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };
  return <form noValidate className="template-editor" onSubmit={(e) => { e.preventDefault(); void submit(); }}>
    <div className="config-save-bar"><Button variant="ghost" disabled={busy} onClick={onClose}>Отмена</Button><Button type="submit" variant="primary" busy={busy}>Сохранить</Button></div>
    {error && <p className="ag-form-error" role="alert">{error}</p>}
    <fieldset className="ag-edit-fields" disabled={busy}>
      <div hidden={view !== "team"}>
        <TemplateGraph compact members={members.map((m) => ({ ...m, key: memberKey(m) }))} relations={relations} roles={cfg.roles} onConnect={(from, to) => {
          const type = to === "orchestrator" ? "reports" : "handoff";
          if (!relations.some((r) => r.from === from && r.type === type && r.to.includes(to))) set("relations", [...relations, { from, to: [to], type }]);
        }} />
        <section className="template-section"><Field label="Общие правила"><Textarea rows={6} value={draft.charter ?? ""} onChange={(e) => set("charter", e.target.value || undefined)} /></Field></section>
        <section className="template-section"><div className="template-section-head"><h3>Участники <span className="g-muted">{members.length}</span></h3><IconButton label="Добавить участника" onClick={addMember}><Icon.userPlus /></IconButton></div>
          <div className="template-members">{members.map((member, index) => <MemberEditor key={memberIds[index]} member={member} cfg={cfg} initiallyOpen={index === 0 || index === members.length - 1} onChange={(patch) => setDraft((d) => updateTemplateMember(d, index, patch))} onRemove={members.length > 1 ? () => { setDraft((d) => removeTemplateMember(d, index)); setMemberIds((ids) => ids.filter((_, i) => i !== index)); } : undefined} />)}</div>
        </section>
        <section className="template-section"><div className="template-section-head"><h3>Связи</h3><div className="template-relation-tools"><Select aria-label="Режим связей" value={draft.relations === undefined ? "auto" : "explicit"} onChange={(e) => set("relations", e.target.value === "auto" ? undefined : relations)}><option value="auto">Автоматически</option><option value="explicit">Вручную</option></Select><IconButton label="Добавить связь" onClick={() => set("relations", [...relations, { from: keys[0], to: keys.slice(1, 2), type: "handoff" }])}><Icon.plus /></IconButton></div></div>
          {draft.relations === undefined ? <p className="g-muted">{relations.length} связей по ролям участников</p> : <div className="template-relation-editors">{relations.map((r, index) => <details className="template-relation-editor" key={index} open>
            <summary>{title(r.from)} <span className="template-relation-kind">{RELATION_TITLE[r.type]}</span> {r.to.map(title).join(", ")}</summary>
            <div className="template-member-body"><div className="template-field-grid">
              <Field label="Отправитель"><Select value={r.from} onChange={(e) => setRelation(index, { from: e.target.value, to: r.to.filter((to) => to !== e.target.value) })}>{keys.map((key) => <option key={key} value={key}>{title(key)} · {key}</option>)}{!keys.includes(r.from) && <option value={r.from}>{r.from}</option>}</Select></Field>
              <Field label="Тип связи"><Select value={r.type} onChange={(e) => { const type = e.target.value as RelKind; setRelation(index, { type, on: ["handoff", "returns"].includes(type) ? r.on : undefined, to: type === "reports" ? ["orchestrator"] : r.to.filter((to) => to !== "orchestrator") }); }}>{KINDS.map((kind) => <option key={kind} value={kind}>{RELATION_TITLE[kind]}</option>)}</Select></Field>
              {(r.type === "handoff" || r.type === "returns") && <Field label="При статусе"><Select value={r.on ?? ""} onChange={(e) => setRelation(index, { on: e.target.value || undefined })}><option value="">Без условия</option>{[...new Set([...ON_STATUSES, ...(r.on ? [r.on] : [])])].map((status) => <option key={status} value={status}>{ON_TITLE[status] ?? status}</option>)}</Select></Field>}
              <Field label="Заметка"><Input value={r.note ?? ""} onChange={(e) => setRelation(index, { note: e.target.value || undefined })} /></Field>
            </div>
            {r.type === "reports" ? <p>Получатель: менеджер задачи</p> : <fieldset className="template-recipients"><legend>Получатели</legend>{keys.filter((key) => key !== r.from).map((key) => <label className="g-check" key={key}><input type="checkbox" checked={r.to.includes(key)} onChange={(e) => setRelation(index, { to: e.target.checked ? [...r.to, key] : r.to.filter((to) => to !== key) })} />{title(key)}</label>)}</fieldset>}
            <div className="template-card-actions"><IconButton label="Удалить связь" onClick={() => set("relations", relations.filter((_, i) => i !== index))}><Icon.trash /></IconButton></div></div>
          </details>)}</div>}
        </section>
      </div>
      <div hidden={view !== "settings"}>
        <section className="template-section"><h3>О шаблоне</h3><div className="template-field-grid"><Field label="Название"><Input required value={draft.title} onChange={(e) => set("title", e.target.value)} /></Field><Field label="Описание"><Textarea rows={2} value={draft.description ?? ""} onChange={(e) => set("description", e.target.value || undefined)} /></Field></div></section>
        <section className="template-section"><h3>Работа команды</h3><div className="template-field-grid">
          <Field label="Стадия"><Select value={draft.stage ?? "delivery"} onChange={(e) => set("stage", e.target.value as TemplateFile["stage"])}>{(["refinement", "delivery"] as const).map((stage) => <option key={stage} value={stage}>{STAGE_TITLE[stage]}</option>)}</Select></Field>
          <Field label="Рабочее место"><Select value={draft.workspace ?? "worktree"} onChange={(e) => set("workspace", e.target.value as TemplateFile["workspace"])}>{(["worktree", "repo", "scratch"] as const).map((workspace) => <option key={workspace} value={workspace}>{WORKSPACE_TITLE[workspace]}</option>)}</Select></Field>
          <Field label="Почта"><Select value={draft.mail ?? "open"} onChange={(e) => set("mail", e.target.value as TemplateFile["mail"])}>{(["open", "flow"] as const).map((mail) => <option key={mail} value={mail}>{MAIL_TITLE[mail]}</option>)}</Select></Field>
          <Field label="Проекты"><ChipInput values={draft.projects ?? []} onChange={(values) => set("projects", values.length ? values : undefined)} placeholder="Все проекты" suggestions={projects?.map((project) => project.slug)} /></Field>
        </div></section>
      </div>
    </fieldset>
  </form>;
}

function MemberEditor({ member, cfg, initiallyOpen, onChange, onRemove }: { member: TemplateMember; cfg: Catalogue; initiallyOpen: boolean; onChange: (patch: Partial<TemplateMember>) => void; onRemove?: () => void }) {
  const [open, setOpen] = useState(initiallyOpen);
  const role = cfg.roles.find((r) => r.id === member.role);
  return <details className="template-member" open={open} onToggle={(e) => setOpen(e.currentTarget.open)}>
    <summary><MemberAvatar member={member} cfg={cfg} /><span>{role?.title ?? member.role}</span><code>{memberKey(member)}</code></summary>
    <div className="template-member-body"><div className="template-field-grid">
      <Field label="Роль"><Select value={member.role} onChange={(e) => onChange({ role: e.target.value })}>{cfg.roles.filter((r) => r.class !== "orchestrator").map((r) => <option key={r.id} value={r.id}>{r.title}</option>)}{!role && <option value={member.role}>{member.role}</option>}</Select></Field>
      <Field label="Ключ"><Input value={member.key ?? ""} placeholder={member.role} pattern="[a-z][a-z0-9-]*" onChange={(e) => onChange({ key: e.target.value || undefined })} /></Field>
      <Field label="Имя"><Input value={member.name ?? ""} pattern="[a-z][a-z0-9_-]*" onChange={(e) => onChange({ name: e.target.value || undefined })} /></Field>
      <Field label="Модель"><ModelPicker value={member.model ?? ""} fallback={role?.model} onChange={(model) => onChange({ model: model || undefined })} /></Field>
      <Field label="Размышление"><Select value={member.thinking ?? ""} onChange={(e) => onChange({ thinking: e.target.value || undefined })}><option value="">Как у роли</option>{THINKING_LEVELS.map((level) => <option key={level} value={level}>{level}</option>)}</Select></Field>
    </div><Field label="Инструкции участнику"><Textarea rows={4} value={member.instructions ?? ""} onChange={(e) => onChange({ instructions: e.target.value || undefined })} /></Field>
    <div className="template-card-actions"><IconButton label="Удалить участника" disabled={!onRemove} onClick={onRemove}><Icon.trash /></IconButton></div></div>
  </details>;
}
