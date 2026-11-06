// A role has three workspaces: its prompt, settings and a paginated audit trail.
import { plural } from "@/shared/lib";
import { useState } from "react";
import { useSearchParams } from "react-router";
import { exportRole, roleSchema, useDeleteConfig, useRole, type Catalogue, type RoleDef } from "@/entities/agent-config";
import { agentTone } from "@/entities/member";
import { Button, ConfirmDialog, Icon, IconButton, LoadingState, Popover, Select, Tabs, useAct } from "@/shared/ui";
import { FileEditor } from "./FileEditor.tsx";
import { Problems } from "./Problems.tsx";
import { RoleSettings } from "./RoleSettings.tsx";
import { RoleDetails, RolePrompt } from "./RoleDetails.tsx";
import { ConfigHistory } from "./ConfigHistory.tsx";
import { RoleImport } from "./RoleTransfer.tsx";
import { downloadConfigJson } from "./download.ts";
import { ConfigProblemCounts } from "./ConfigProblemCounts.tsx";
import "./config-workspace.css";
import "./role.css";

export function RolesTab({
  cfg,
  selected,
  onSelect,
  onCreate,
  readOnly = false,
  base = "/agents",
}: {
  cfg: Catalogue;
  selected?: string;
  onSelect: (id: string) => void;
  onCreate?: () => void;
  /** The catalogue (/agents) forces read-only even for a server administrator. */
  readOnly?: boolean;
  /** Where this tab lives: cross-links to the other tabs stay inside the same mode. */
  base?: string;
}) {
  const [importing, setImporting] = useState(false);
  const [editingRole, setEditingRole] = useState<string>();
  const [pendingRole, setPendingRole] = useState<string>();
  const team = cfg.roles.filter((r) => r.class !== "orchestrator");
  const orch = cfg.roles.filter((r) => r.class === "orchestrator");
  const current = selected ?? team[0]?.id ?? orch[0]?.id;
  const selectRole = (id: string) => { if (id === current) return; if (editingRole === current) setPendingRole(id); else onSelect(id); };
  const pick = (r: RoleDef) => (
    <button type="button" key={r.id} className={`pick${r.id === current ? " on" : ""}`} onClick={() => selectRole(r.id)} aria-current={r.id === current}>
      <span className="t">
        <span className="ag-class" data-tone={agentTone(r.id, cfg.roles)} aria-hidden="true">
          {r.title.slice(0, 1).toUpperCase()}
        </span>
        <span className="role-name config-name">{r.title}</span>
        <ConfigProblemCounts problems={cfg.problems.filter((p) => p.item === `role:${r.id}`)} />
      </span>
    </button>
  );
  return (
    <div className="split ag-roles-layout config-workspace">
      <section className="split-list ag-list" aria-label="Роли">
        <div className="side-head">
          <span className="side-title">{cfg.roles.length} {plural(cfg.roles.length, "роль", "роли", "ролей")}</span>
          <div className="config-icon-group" role="group" aria-label="Действия со списком ролей">
            {!readOnly && onCreate && <IconButton label="Новая роль" onClick={onCreate}><Icon.plus /></IconButton>}
            {!readOnly && cfg.admin && <><IconButton label="Импорт роли" disabled={editingRole === current} onClick={() => setImporting(true)}><Icon.upload /></IconButton><IconButton label="Скачать схему роли" onClick={() => downloadConfigJson("genie-role.schema.json", roleSchema)}><Icon.file /></IconButton></>}
          </div>
        </div>
        <div className="role-mobile-picker"><Select aria-label="Выбрать роль" value={current} onChange={(e) => selectRole(e.target.value)}>{cfg.roles.map((role) => <option key={role.id} value={role.id}>{role.title}</option>)}</Select></div>
        <div className="role-list-content">
        {team.map(pick)}
        {orch.length > 0 && <span className="label">Оркестратор</span>}
        {orch.map(pick)}
        {!cfg.roles.length && <p className="g-muted ag-list-note">Ничего не найдено.</p>}
        </div>
      </section>
      <section className="split-main" aria-label="Роль">
        {current ? <RolePane key={current} id={current} cfg={cfg} readOnly={readOnly} base={base} onEditingChange={(editing) => setEditingRole(editing ? current : undefined)} /> : <div className="ag-pane-empty">Ролей нет</div>}
      </section>
      {importing && <RoleImport cfg={cfg} onClose={() => setImporting(false)} onCreated={(id) => { setImporting(false); onSelect(id); }} />}
      {pendingRole && <ConfirmDialog title="Отменить изменения роли?" confirmLabel="Отменить и перейти" onClose={() => setPendingRole(undefined)} onConfirm={() => {
        setEditingRole(undefined);
        onSelect(pendingRole);
        setPendingRole(undefined);
      }}>Несохранённые изменения будут потеряны.</ConfirmDialog>}
    </div>
  );
}


type RoleView = "prompt" | "settings" | "history";
function RolePane({ id, cfg, readOnly = false, onEditingChange }: { id: string; cfg: Catalogue; readOnly?: boolean; base?: string; onEditingChange: (editing: boolean) => void }) {
  const detail = useRole(id);
  const [sp, setSp] = useSearchParams();
  const requested = sp.get("view");
  const [editing, setEditing] = useState(false);
  const [dialog, setDialog] = useState<"file" | "delete" | "reset">();
  const remove = useDeleteConfig();
  const act = useAct();
  const d = detail.data;
  if (!d) return <div className="ag-pane-empty">{detail.error?.message ?? <LoadingState label="Загрузка роли" />}</div>;
  const admin = d.admin && !readOnly;
  const view: RoleView = requested === "settings" ? "settings" : requested === "history" && admin ? "history" : "prompt";
  const selectView = (next: RoleView) => setSp((previous) => { const params = new URLSearchParams(previous); params.set("view", next); return params; });
  const edit = (value: boolean) => { setEditing(value); onEditingChange(value); };
  const r = d.role;
  return <>
    <div className="pane-head role-header config-header">
      <div className="ttl"><h2>{r.title}</h2>{r.description && <p className="desc">{r.description}</p>}</div>
      <div className="config-icon-group" role="group" aria-label="Действия роли">
        {!editing && <IconButton label="Экспорт роли" onClick={() => downloadConfigJson(`${r.id}.role.json`, { $schema: "./genie-role.schema.json", ...exportRole(r) })}><Icon.download /></IconButton>}
        {admin && !editing && <IconButton label="Редактировать роль" onClick={() => { if (view === "history") selectView("prompt"); edit(true); }}><Icon.pencil /></IconButton>}
        {admin && !editing && <Popover label="Дополнительные действия роли" icon={<Icon.dots />} iconOnly><div className="role-actions-menu">
          <Button variant="ghost" onClick={() => setDialog("file")}>Открыть файл роли</Button>
          {d.file.content !== null && <Button variant="ghost" onClick={() => setDialog(d.builtin !== null ? "reset" : "delete")}>{d.builtin !== null ? "Вернуть встроенную роль" : "Удалить роль"}</Button>}
        </div></Popover>}
      </div>
    </div>
    <div className="role-tabs config-tabs"><Tabs label="Роль" variant="underline" value={view} onChange={(next) => { if (!editing || next !== "history") selectView(next); }} items={[
      { id: "prompt", label: "Промпт" }, { id: "settings", label: "Настройки" }, ...(admin && !editing ? [{ id: "history" as const, label: "История" }] : []),
    ]} /></div>
    <div className="pane-body role-body config-body">
      <Problems items={d.problems} />
      {editing ? <RoleSettings detail={d} cfg={cfg} view={view === "settings" ? "settings" : "prompt"} onClose={() => edit(false)} /> : view === "history" ? <ConfigHistory key={r.id} kind="role" id={r.id} file={d.file} /> : <>
        <dl className="role-runtime"><div><dt>Модель</dt><dd>{r.model || "По умолчанию"}</dd></div><div><dt>Размышление</dt><dd>{r.thinking || "По умолчанию"}</dd></div></dl>
        {view === "prompt" ? <RolePrompt role={r} /> : <RoleDetails role={r} cfg={cfg} />}
      </>}
    </div>
    {dialog === "file" && <FileEditor kind="role" id={r.id} title={`agents/${r.id}.md`} initial={d.file.content ?? "---\n---\n"} baseHash={d.file.hash} problems={d.problems} onClose={() => setDialog(undefined)} />}
    {(dialog === "delete" || dialog === "reset") && <ConfirmDialog title={dialog === "reset" ? "Вернуть встроенную роль?" : `Удалить роль ${r.title}?`} confirmLabel={dialog === "reset" ? "Вернуть встроенную" : "Удалить"} danger onClose={() => setDialog(undefined)} onConfirm={() => {
      setDialog(undefined); void act(() => remove("role", r.id, d.file.hash), "Изменения сохранены");
    }}>Промпт и настройки роли будут удалены или заменены встроенной версией.</ConfirmDialog>}
  </>;
}
