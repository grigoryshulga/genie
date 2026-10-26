import { useEffect, useRef, useState } from "react";
import { parseTemplateTransfer, templateExample, templateSchema, templateTransferFile, useAgentConfig, useSaveConfig, useTemplate, type Catalogue, type TemplateTransfer } from "@/entities/agent-config";
import { Button, Dialog, Field, Icon, IconButton, Input, useToast } from "@/shared/ui";
import { downloadConfigJson } from "./download.ts";

export function TemplateImport({ cfg, onClose, onCreated }: { cfg: Catalogue; onClose: () => void; onCreated: (id: string) => void }) {
  const [value, setValue] = useState<TemplateTransfer>();
  const [error, setError] = useState<string>();
  const [busy, setBusy] = useState(false);
  const [replace, setReplace] = useState(false);
  const [target, setTarget] = useState<{ id: string; hash: string }>();
  const generation = useRef(0);
  // Search affects the list only: an existing target outside its results still requires consent.
  const catalogue = useAgentConfig().data ?? cfg;
  const existing = catalogue.teams.find((t) => t.id === value?.id);
  const current = useTemplate(existing ? value?.id : undefined);
  const save = useSaveConfig();
  const toast = useToast();
  useEffect(() => {
    const data = current.data;
    if (data && value && data.template.id === value.id && target?.id !== value.id) setTarget({ id: value.id, hash: data.file.path.startsWith("teams/") ? data.file.hash : "" });
  }, [current.data, value, target]);
  const load = async (file?: File) => {
    const sequence = ++generation.current;
    setValue(undefined); setReplace(false); setTarget(undefined); setError(undefined);
    if (!file) return;
    try {
      if (file.size > 1_000_000) throw new Error("Файл шаблона должен быть не больше 1 МБ.");
      const text = await file.text();
      if (sequence !== generation.current) return;
      const parsed = parseTemplateTransfer(text);
      templateTransferFile(parsed, catalogue);
      setValue(parsed);
    } catch (e) { if (sequence === generation.current) setError(e instanceof Error ? e.message : String(e)); }
  };
  const submit = async () => {
    if (!value || busy || (existing && (!replace || target?.id !== value.id))) return;
    setBusy(true); setError(undefined);
    try {
      const parsed = parseTemplateTransfer(JSON.stringify(value));
      await save("template", parsed.id, { template: templateTransferFile(parsed, catalogue) }, existing ? target?.hash : "");
      toast("Шаблон импортирован"); onCreated(parsed.id);
    } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };
  return <Dialog title="Импорт шаблона" wide onClose={() => { if (!busy) onClose(); }} footer={<><Button variant="ghost" disabled={busy} onClick={onClose}>Отмена</Button><Button variant="primary" busy={busy} disabled={!value || (existing && (!replace || target?.id !== value.id))} onClick={() => void submit()}>Импортировать</Button></>}>
    <div className="template-import">
      <div className="config-icon-group" role="group" aria-label="Файлы шаблона">
        <IconButton label="Скачать JSON Schema шаблона" onClick={() => downloadConfigJson("genie-team.schema.json", templateSchema)}><Icon.file /></IconButton>
        <IconButton label="Скачать пример шаблона" onClick={() => downloadConfigJson("team-template.json", { $schema: "./genie-team.schema.json", ...templateExample })}><Icon.download /></IconButton>
      </div>
      <Field label="Файл шаблона (.json)"><Input type="file" accept=".json,application/json" disabled={busy} onChange={(e) => void load(e.target.files?.[0])} /></Field>
      {error && <p role="alert" className="ag-form-error">{error}</p>}
      {value && <>
        <Field label="Идентификатор шаблона"><Input value={value.id} disabled={busy} onChange={(e) => { setValue({ ...value, id: e.target.value }); setReplace(false); setTarget(undefined); }} /></Field>
        <strong>{value.title}</strong>
        <ul className="template-import-members">{value.members.map((member, i) => <li key={i}>{catalogue.roles.find((r) => r.id === member.role)?.title ?? member.role}{member.key && member.key !== member.role ? ` · ${member.key}` : ""}</li>)}</ul>
        {existing && <label className="g-check"><input type="checkbox" checked={replace} disabled={busy} onChange={(e) => setReplace(e.target.checked)} /><span>Заменить существующий шаблон «{existing.title}»</span></label>}
        {current.error && <p role="alert" className="ag-form-error">{current.error.message}</p>}
        <details><summary>JSON</summary><pre className="template-import-json">{JSON.stringify(value, null, 2)}</pre></details>
      </>}
    </div>
  </Dialog>;
}
