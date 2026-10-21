// Changes of one configuration item (or all of them) with a way back to any
// earlier version. Server administrators only.

import { useState } from "react";
import { type ConfigChange, type ConfigKind, useConfigHistory, useDeleteConfig, useSaveConfig } from "@/entities/agent-config";
import { timeAgo } from "@/shared/lib";
import { useAct } from "@/shared/ui";

/** Which kind and id a history item (`role:x`, `team:x`, `skill:x`, `mcp`) is about. */
export function itemTarget(item: string): { kind: ConfigKind; id: string; label: string } {
  const [prefix, id = ""] = item.split(":");
  if (prefix === "role") return { kind: "role", id, label: `роль ${id}` };
  if (prefix === "team") return { kind: "template", id, label: `шаблон ${id}` };
  if (prefix === "skill") return { kind: "skill", id, label: `навык ${id}` };
  return { kind: "mcp", id: "", label: "MCP-подключения" };
}

/** Whether a change can be undone from the history: not the first mcp.json, nor a skill's supporting file (upload it again). */
function restorable(c: ConfigChange): boolean {
  const t = itemTarget(c.item);
  if (t.kind === "skill" && !c.path.endsWith("/SKILL.md")) return false;
  return !(c.before === undefined && t.kind === "mcp");
}

export function History({ item, admin }: { item?: string; admin: boolean }) {
  const history = useConfigHistory(item, admin);
  const [open, setOpen] = useState<number>();
  const save = useSaveConfig();
  const remove = useDeleteConfig();
  const act = useAct();
  if (!admin) return null;
  const restore = (c: ConfigChange) => {
    const t = itemTarget(c.item);
    if (c.before === undefined) {
      if (t.kind === "mcp") return;
      void act(() => remove(t.kind as Exclude<ConfigKind, "mcp">, t.id), "Файл удалён, как было до этой правки");
    } else {
      void act(() => save(t.kind, t.id, { content: c.before }, undefined), "Вернули версию до этой правки");
    }
  };
  if (!history.data?.length) return <p className="g-muted ag-empty">{history.isPending ? "Загрузка…" : "Правок через веб пока не было."}</p>;
  return (
    <ol className="ag-history">
      {history.data.map((c) => (
        <li key={c.id}>
          <button type="button" className="ag-history-row" aria-expanded={open === c.id} onClick={() => setOpen(open === c.id ? undefined : c.id)}>
            <span className="who">{c.user}</span>
            <span className="what">
              {c.before === undefined ? "создал" : c.after === undefined ? "удалил" : "изменил"} <span className="g-mono">{c.path}</span>
              {!item && <span className="g-muted"> · {itemTarget(c.item).label}</span>}
            </span>
            <span className="when">{timeAgo(c.at)}</span>
          </button>
          {open === c.id && (
            <div className="ag-history-body">
              <div className="compare">
                <div>
                  <div className="hd">До</div>
                  <pre className="bd view">{c.before ?? "— файла не было —"}</pre>
                </div>
                <div>
                  <div className="hd">После</div>
                  <pre className="bd view">{c.after ?? "— файл удалён —"}</pre>
                </div>
              </div>
              {restorable(c) && (
                <button type="button" className="g-btn secondary" onClick={() => restore(c)}>
                  Вернуть версию до этой правки
                </button>
              )}
            </div>
          )}
        </li>
      ))}
    </ol>
  );
}
