// Team composition is the primary workspace; settings and generated prompts stay out of its way.
import { useState } from "react";
import { useSearchParams } from "react-router";
import { exportTemplate, templateFile, templateSchema, STAGE_TITLE, useDeleteConfig, useTemplate, type Catalogue, type Problem, type TeamDef } from "@/entities/agent-config";
import { Button, ConfirmDialog, Icon, IconButton, LoadingState, Popover, Select, Tabs, useAct } from "@/shared/ui";
import { ConfigHistory } from "./ConfigHistory.tsx";
import { ConfigProblemCounts } from "./ConfigProblemCounts.tsx";
import { FileEditor } from "./FileEditor.tsx";
import { Problems } from "./Problems.tsx";
import { TemplateSettings } from "./TemplateSettings.tsx";
import { TemplateImport } from "./TemplateTransfer.tsx";
import { TemplatePrompts, TemplateSettingsView, TemplateTeam } from "./TemplateViews.tsx";
import { downloadConfigJson } from "./download.ts";
import "./config-workspace.css";
import "./template.css";

function diagnostics(t: TeamDef, problems: Problem[]): Problem[] {
  const items = problems.filter((p) => p.item === `team:${t.id}`);
  return [...items, ...t.warnings.filter((message) => !items.some((p) => p.message === message)).map((message): Problem => ({ item: `team:${t.id}`, level: "warning", message }))];
}
export function TemplatesTab({ cfg, selected, onSelect, onCreate, readOnly = false, base = "/agents" }: { cfg: Catalogue; selected?: string; onSelect: (id: string) => void; onCreate?: () => void; readOnly?: boolean; base?: string }) {
  const [importing, setImporting] = useState(false);
  const [editingId, setEditingId] = useState<string>();
  const [pendingId, setPendingId] = useState<string>();
  const current = selected ?? cfg.teams[0]?.id;
  const select = (id: string) => { if (id === current) return; if (editingId === current) setPendingId(id); else onSelect(id); };
  return <div className="split ag-templates-layout config-workspace">
    <section className="split-list ag-list" aria-label="Шаблоны команд">
      <div className="side-head"><span className="side-title">Шаблоны</span><div className="config-icon-group" role="group" aria-label="Действия со списком шаблонов">
        {!readOnly && onCreate && <IconButton label="Новый шаблон" onClick={onCreate}><Icon.plus /></IconButton>}
        {!readOnly && cfg.admin && <><IconButton label="Импорт шаблона" disabled={editingId === current} onClick={() => setImporting(true)}><Icon.upload /></IconButton><IconButton label="Скачать схему шаблона" onClick={() => downloadConfigJson("genie-team.schema.json", templateSchema)}><Icon.file /></IconButton></>}
      </div></div>
      <div className="template-mobile-picker"><Select aria-label="Выбрать шаблон" value={current} onChange={(e) => select(e.target.value)}>{cfg.teams.map((t) => <option key={t.id} value={t.id}>{t.title}</option>)}</Select></div>
      <div className="template-list-content">{(["delivery", "refinement"] as const).map((stage) => {
        const teams = cfg.teams.filter((t) => t.stage === stage);
        return teams.length > 0 && <div key={stage} className="template-list-group"><span className="label">{STAGE_TITLE[stage]}</span>{teams.map((t) => <button type="button" key={t.id} className={`pick${t.id === current ? " on" : ""}`} aria-current={t.id === current} onClick={() => select(t.id)}>
          <span className="t"><span className="config-name">{t.title}</span><span className="template-member-count" role="img" aria-label={`Участников: ${t.members.length}`} title={`Участников: ${t.members.length}`}><Icon.user />{t.members.length}</span><ConfigProblemCounts problems={diagnostics(t, cfg.problems)} /></span>
        </button>)}</div>;
      })}{!cfg.teams.length && <p className="g-muted ag-list-note">Ничего не найдено.</p>}</div>
    </section>
    <section className="split-main" aria-label="Шаблон">{current ? <TemplatePane key={current} id={current} cfg={cfg} readOnly={readOnly} base={base} onEditingChange={(editing) => setEditingId(editing ? current : undefined)} /> : <div className="ag-pane-empty">Шаблонов нет</div>}</section>
    {importing && <TemplateImport cfg={cfg} onClose={() => setImporting(false)} onCreated={(id) => { setImporting(false); onSelect(id); }} />}
    {pendingId && <ConfirmDialog title="Отменить изменения шаблона?" confirmLabel="Отменить и перейти" onClose={() => setPendingId(undefined)} onConfirm={() => { setEditingId(undefined); onSelect(pendingId); setPendingId(undefined); }}>Несохранённые изменения будут потеряны.</ConfirmDialog>}
  </div>;
}

type TemplateView = "team" | "settings" | "prompts" | "history";
function TemplatePane({ id, cfg, readOnly, base, onEditingChange }: { id: string; cfg: Catalogue; readOnly: boolean; base: string; onEditingChange: (editing: boolean) => void }) {
  const detail = useTemplate(id);
  const [sp, setSp] = useSearchParams();
  const [editing, setEditing] = useState(false);
  const [dialog, setDialog] = useState<"file" | "remove">();
  const remove = useDeleteConfig();
  const act = useAct();
  const d = detail.data;
  if (!d) return <div className="ag-pane-empty">{detail.error?.message ?? <LoadingState label="Загрузка шаблона" />}</div>;
  const t = d.template, admin = d.admin && !readOnly;
  const hasFile = d.file.content !== null && d.file.path.startsWith("teams/");
  const requested = sp.get("view");
  const view: TemplateView = requested === "settings" ? "settings" : !editing && requested === "prompts" ? "prompts" : !editing && requested === "history" && admin ? "history" : "team";
  const setView = (next: TemplateView) => setSp((previous) => { const params = new URLSearchParams(previous); params.set("view", next); return params; });
  const edit = (value: boolean) => { setEditing(value); onEditingChange(value); };
  return <>
    <div className="pane-head config-header"><div className="ttl"><h2>{t.title}</h2>{t.description && <p className="desc">{t.description}</p>}</div><div className="config-icon-group" role="group" aria-label="Действия шаблона">
      {!editing && <IconButton label="Экспорт шаблона" onClick={() => downloadConfigJson(`${t.id}.team.json`, { $schema: "./genie-team.schema.json", ...exportTemplate(t) })}><Icon.download /></IconButton>}
      {admin && !editing && <><IconButton label="Редактировать шаблон" onClick={() => { if (view !== "settings") setView("team"); edit(true); }}><Icon.pencil /></IconButton><Popover label="Дополнительные действия шаблона" icon={<Icon.dots />} iconOnly><div className="template-actions-menu"><Button variant="ghost" onClick={() => setDialog("file")}>Открыть файл шаблона</Button>{hasFile && <Button variant="ghost" onClick={() => setDialog("remove")}>{d.builtin !== null ? "Вернуть встроенный шаблон" : "Удалить шаблон"}</Button>}</div></Popover></>}
    </div></div>
    <div className="config-tabs"><Tabs label="Шаблон" variant="underline" value={view} onChange={setView} items={[{ id: "team", label: "Команда" }, { id: "settings", label: "Настройки" }, ...(!editing ? [{ id: "prompts" as const, label: "Промпты" }, ...(admin ? [{ id: "history" as const, label: "История" }] : [])] : [])]} /></div>
    <div className="pane-body config-body template-body"><Problems items={diagnostics(t, d.problems)} />
      {editing ? <TemplateSettings detail={d} cfg={cfg} view={view === "settings" ? "settings" : "team"} onClose={() => edit(false)} /> : view === "history" ? <ConfigHistory kind="template" id={t.id} file={hasFile ? d.file : { ...d.file, hash: "" }} /> : view === "prompts" ? <TemplatePrompts id={t.id} /> : view === "settings" ? <TemplateSettingsView template={t} /> : <TemplateTeam template={t} cfg={cfg} base={base} />}
    </div>
    {dialog === "file" && <FileEditor kind="template" id={t.id} title={`teams/${t.id}.json`} initial={hasFile ? d.file.content ?? "" : d.builtin ?? JSON.stringify(templateFile(t), null, 2)} baseHash={hasFile ? d.file.hash : ""} problems={d.problems} onClose={() => setDialog(undefined)} />}
    {dialog === "remove" && <ConfirmDialog title={d.builtin !== null ? "Вернуть встроенный шаблон?" : `Удалить шаблон ${t.title}?`} confirmLabel={d.builtin !== null ? "Вернуть" : "Удалить"} danger={d.builtin === null} onClose={() => setDialog(undefined)} onConfirm={() => { setDialog(undefined); void act(() => remove("template", t.id, d.file.hash), "Изменения сохранены"); }}>Идущие команды сохранят свой снимок шаблона.</ConfirmDialog>}
  </>;
}
