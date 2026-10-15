// The role's access to MCP connections: none, all tools or named tools per
// server, saved as the `mcp` field of the role file. The named tools are picked
// from the server's last listing (the check cache, closed tools left out);
// without a cache, patterns are typed by hand. Grants for servers the
// catalogue does not know are kept as written.

import { useState } from "react";
import { type Catalogue, globMatch, type RoleDetail } from "@/entities/agent-config";
import { ChipInput } from "@/shared/ui";
import { EditDialog, useRoleSave } from "./EditDialog.tsx";

type Grant = { mode: "none" | "all" | "tools"; tools: string[] };

export function McpDialog({ detail, cfg, onClose }: { detail: RoleDetail; cfg: Catalogue; onClose: () => void }) {
  const r = detail.role;
  const [grants, setGrants] = useState(
    (): Record<string, Grant> =>
      Object.fromEntries(
        cfg.mcp.map((s) => {
          const whole = r.mcp.includes(s.id) || r.mcp.includes("*");
          const tools = r.mcp.filter((g) => g.startsWith(`${s.id}:`)).map((g) => g.slice(s.id.length + 1));
          return [s.id, { mode: whole ? "all" : tools.length ? "tools" : "none", tools } satisfies Grant];
        }),
      ),
  );
  const [busy, saveFields] = useRoleSave(detail, onClose);
  const unknown = r.mcp.filter((g) => g !== "*" && !cfg.mcp.some((s) => s.id === g.split(":")[0]));
  const save = async () => {
    const out: string[] = [...unknown];
    for (const s of cfg.mcp) {
      const g = grants[s.id];
      if (g.mode === "all") out.push(s.id);
      if (g.mode === "tools") out.push(...g.tools.map((t) => `${s.id}:${t}`));
    }
    await saveFields([["mcp", out.length ? out : undefined]], "Доступ к MCP сохранён");
  };
  return (
    <EditDialog title={`MCP роли ${r.id}`} note="Закрытое подключение перестаёт работать сразу; новое агент получит со следующего старта сессии" busy={busy} onClose={onClose} onSave={() => void save()}>
      <div className="ag-grants">
        {cfg.mcp.map((s) => {
          const g = grants[s.id];
          const set = (v: Partial<Grant>) => setGrants((x) => ({ ...x, [s.id]: { ...x[s.id], ...v } }));
          const cached = s.tools ?? [];
          // Grants the cached listing does not know: an exact tool that is gone, or a pattern.
          const extra = g.tools.filter((p) => !cached.some((t) => globMatch(p, t.name)));
          const toggle = (tool: string) => {
            const matched = g.tools.filter((p) => globMatch(p, tool));
            set({ tools: matched.length ? g.tools.filter((p) => !matched.includes(p)) : [...g.tools, tool] });
          };
          return (
            <div key={s.id}>
              <div className="ag-grant">
                <span className="mono">{s.id}</span>
                <span className="muted">{s.description}</span>
                <select value={g.mode} onChange={(e) => set({ mode: e.target.value as Grant["mode"] })} aria-label={`Доступ к ${s.id}`}>
                  <option value="none">нет доступа</option>
                  <option value="all">все инструменты</option>
                  <option value="tools">только выбранные</option>
                </select>
              </div>
              {g.mode === "tools" &&
                (cached.length > 0 ? (
                  <div className="mc-role-tools">
                    {extra.length > 0 && (
                      <div className="chips mono" aria-label={`Выдано сверх списка ${s.id}`}>
                        {extra.map((p) => (
                          <span key={p} className="chip">
                            {p}
                            <button type="button" aria-label={`Убрать ${p}`} onClick={() => set({ tools: g.tools.filter((x) => x !== p) })}>
                              <svg width="10" height="10" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" aria-hidden="true">
                                <path d="M4 4l8 8M12 4l-8 8" />
                              </svg>
                            </button>
                          </span>
                        ))}
                      </div>
                    )}
                    <div className="ag-checks">
                      {cached.map((t) => {
                        const on = g.tools.some((p) => globMatch(p, t.name));
                        return (
                          <label key={t.name} title={t.description}>
                            <input type="checkbox" checked={on} onChange={() => toggle(t.name)} />
                            <span className="mono">{t.name}</span>
                            <span className="muted">{t.description}</span>
                          </label>
                        );
                      })}
                    </div>
                  </div>
                ) : (
                  <div className="mc-role-tools">
                    <ChipInput values={g.tools} onChange={(tools) => set({ tools })} placeholder="шаблоны через запятую, например get_*" mono />
                    <p className="muted ag-note">Проверьте подключение в «Настройки» → «Сервер» → «MCP», чтобы выбирать инструменты списком.</p>
                  </div>
                ))}
            </div>
          );
        })}
      </div>
    </EditDialog>
  );
}
