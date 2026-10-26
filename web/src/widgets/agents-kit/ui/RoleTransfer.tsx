import { useEffect, useRef, useState } from "react";
import { parseRoleTransfer, roleSchema, roleTemplate, roleTransferFile, useAgentConfig, useRole, useSaveConfig, type Catalogue, type RoleTransfer } from "@/entities/agent-config";
import { Button, Dialog, Field, Input, Markdown, useToast } from "@/shared/ui";

import { downloadConfigJson } from "./download.ts";

export function RoleImport({ cfg, onClose, onCreated }: { cfg: Catalogue; onClose: () => void; onCreated: (id: string) => void }) {
  const [role, setRole] = useState<RoleTransfer>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [replace, setReplace] = useState(false);
  const [target, setTarget] = useState<{ id: string; hash: string }>();
  const generation = useRef(0);
  const catalogue = useAgentConfig().data ?? cfg;
  const existing = catalogue.roles.some((r) => r.id === role?.id);
  const current = useRole(existing ? role?.id : undefined);
  useEffect(() => {
    const data = current.data;
    if (data && data.role.id === role?.id && role && target?.id !== role.id) setTarget({ id: role.id, hash: data.file.hash });
  }, [current.data, role, target]);
  const save = useSaveConfig();
  const toast = useToast();
  const load = async (file: File | undefined) => {
    const loadId = ++generation.current;
    setError(undefined); setRole(undefined); setReplace(false); setTarget(undefined);
    if (!file) return;
    try {
      if (file.size > 1_000_000) throw new Error("Файл роли должен быть не больше 1 МБ.");
      const text = await file.text();
      if (loadId !== generation.current) return;
      const parsed = parseRoleTransfer(text);
      roleTransferFile(parsed, cfg);
      setRole(parsed);
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };
  const submit = async () => {
    if (!role || busy || (existing && (!replace || target?.id !== role.id))) return;
    setBusy(true); setError(undefined);
    try {
      const checked = parseRoleTransfer(JSON.stringify(role));
      await save("role", checked.id, { content: roleTransferFile(checked, cfg) }, existing ? target?.hash : "");
      toast("Роль импортирована");
      onCreated(checked.id);
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };
  return <Dialog title="Импорт роли" wide onClose={() => { if (!busy) onClose(); }} footer={<><Button variant="ghost" disabled={busy} onClick={onClose}>Отмена</Button><Button variant="primary" busy={busy} disabled={!role || (existing && (!replace || target?.id !== role.id))} onClick={() => void submit()}>Импортировать</Button></>}>
    <div className="role-import">
      <div className="ag-actions"><Button sm onClick={() => downloadConfigJson("genie-role.schema.json", roleSchema)}>Скачать JSON Schema</Button><Button sm onClick={() => downloadConfigJson("role-template.json", { $schema: "./genie-role.schema.json", ...roleTemplate })}>Скачать шаблон</Button></div>
      <Field label="Файл роли (.json)"><Input type="file" accept=".json,application/json" disabled={busy} onChange={(e) => void load(e.target.files?.[0])} /></Field>
      {error && <p role="alert" className="ag-warn">{error}</p>}
      {role && <>
        <Field label="Идентификатор роли"><Input value={role.id} disabled={busy} onChange={(e) => { setRole({ ...role, id: e.target.value }); setReplace(false); setTarget(undefined); }} /></Field>
        <div className="role-import-summary"><strong>{role.title}</strong><span>{role.base} · {role.model || "модель не задана"}</span><p>{role.description}</p></div>
        {existing && <label className="g-check"><input type="checkbox" checked={replace} disabled={busy} onChange={(e) => setReplace(e.target.checked)} /><span>Заменить существующую роль «{catalogue.roles.find((r) => r.id === role.id)?.title}»</span></label>}
        {current.error && <p role="alert" className="ag-warn">{current.error.message}</p>}
        <details><summary>JSON</summary><pre className="role-import-json">{JSON.stringify(role, null, 2)}</pre></details>
        <section className="role-import-preview" aria-label="Промпт импортируемой роли"><Markdown text={role.prompt} /></section>
      </>}
    </div>
  </Dialog>;
}
