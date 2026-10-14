// The role's access to MCP connections: none, all tools or named tools per
// server, saved as the `mcp` field of the role file. Grants for servers the
// catalogue does not know are kept as written.

import { useState } from "react";
import { type Catalogue, type RoleDetail } from "@/entities/agent-config";
import { useAction } from "./common.tsx";
import { EditDialog, useRoleEdit } from "./EditDialog.tsx";

type Grant = { mode: "none" | "all" | "tools"; tools: string };

export function McpDialog({ detail, cfg, onClose }: { detail: RoleDetail; cfg: Catalogue; onClose: () => void }) {
  const r = detail.role;
  const [grants, setGrants] = useState(
    (): Record<string, Grant> =>
      Object.fromEntries(
        cfg.mcp.map((s) => {
          const whole = r.mcp.includes(s.id) || r.mcp.includes("*");
          const tools = r.mcp.filter((g) => g.startsWith(`${s.id}:`)).map((g) => g.slice(s.id.length + 1));
          return [s.id, { mode: whole ? "all" : tools.length ? "tools" : "none", tools: tools.join(", ") } satisfies Grant];
        }),
      ),
  );
  const [busy, setBusy] = useState(false);
  const edit = useRoleEdit(detail);
  const act = useAction();
  const unknown = r.mcp.filter((g) => g !== "*" && !cfg.mcp.some((s) => s.id === g.split(":")[0]));
  const save = async () => {
    const out: string[] = [...unknown];
    for (const s of cfg.mcp) {
      const g = grants[s.id];
      if (g.mode === "all") out.push(s.id);
      if (g.mode === "tools") out.push(...g.tools.split(/[,\n]/).map((x) => x.trim()).filter(Boolean).map((t) => `${s.id}:${t}`));
    }
    setBusy(true);
    if (await act(() => edit([["mcp", out.length ? out : undefined]]), "Доступ к MCP сохранён")) onClose();
    setBusy(false);
  };
  return (
    <EditDialog title={`MCP роли ${r.id}`} note="Закрытое подключение перестаёт работать сразу; новое агент получит со следующего старта сессии" busy={busy} onClose={onClose} onSave={() => void save()}>
      <div className="ag-grants">
        {cfg.mcp.map((s) => {
          const g = grants[s.id];
          const set = (v: Partial<Grant>) => setGrants((x) => ({ ...x, [s.id]: { ...x[s.id], ...v } }));
          return (
            <div key={s.id} className="ag-grant">
              <span className="mono">{s.id}</span>
              <span className="muted">{s.description}</span>
              <select value={g.mode} onChange={(e) => set({ mode: e.target.value as Grant["mode"] })} aria-label={`Доступ к ${s.id}`}>
                <option value="none">нет доступа</option>
                <option value="all">все инструменты</option>
                <option value="tools">только инструменты…</option>
              </select>
              {g.mode === "tools" && <input value={g.tools} onChange={(e) => set({ tools: e.target.value })} placeholder="get_*, list_commits" aria-label={`Инструменты ${s.id}`} />}
            </div>
          );
        })}
      </div>
    </EditDialog>
  );
}
