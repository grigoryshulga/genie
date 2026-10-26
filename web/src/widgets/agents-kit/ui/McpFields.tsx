// The role's access to MCP connections: none, all tools or named tools per
// server, saved as the `mcp` field of the role file. The named tools are picked
// from the server's last listing (the check cache, closed tools left out);
// without a cache, patterns are typed by hand. Grants for servers the
// catalogue does not know are kept as written.

import { useState } from "react";
import { type Catalogue, globMatch, type McpGrant, mcpGrants, roleMcpAccess, type RoleDetail } from "@/entities/agent-config";
import { ChipInput } from "@/shared/ui";


export function useMcpSettings(detail: RoleDetail, cfg: Catalogue) {
  const access = detail.role.mcp;
  const [grants, setGrants] = useState(() => mcpGrants(access, cfg.mcp));
  const unknown = access.filter((g) => g !== "*" && !cfg.mcp.some((s) => s.id === g.split(":")[0]));
  return { grants, setGrants, value: () => roleMcpAccess(access, cfg.mcp, grants), unknown };
}

export function McpFields({ cfg, state }: { cfg: Catalogue; state: ReturnType<typeof useMcpSettings> }) {
  const { grants, setGrants, unknown } = state;
  const [search, setSearch] = useState<Record<string, string>>({});
  return (
      <div className="ag-grants">
        {!cfg.mcp.length && <p className="g-muted ag-note">Подключений пока нет. Добавьте их в настройках MCP.</p>}
        {unknown.length > 0 && <details className="ag-advanced">
          <summary>Подключения вне библиотеки · {new Set(unknown.map((g) => g.split(":")[0])).size}</summary>
          <ul className="ag-mcp-tools">{unknown.map((g) => <li key={g}><code>{g}</code></li>)}</ul>
        </details>}
        {cfg.mcp.map((s) => {
          const g = grants[s.id];
          const set = (v: Partial<McpGrant>) => setGrants((x) => ({ ...x, [s.id]: { ...x[s.id], ...v } }));
          const cached = s.tools ?? [];
          const query = (search[s.id] ?? "").toLocaleLowerCase();
          const visible = cached.filter((t) => `${t.name} ${t.description ?? ""}`.toLocaleLowerCase().includes(query));
          // Grants the cached listing does not know: an exact tool that is gone, or a pattern.
          const extra = g.tools.filter((p) => !cached.some((t) => globMatch(p, t.name)));
          const toggle = (tool: string) => {
            const matched = g.tools.filter((p) => globMatch(p, tool));
            set({ tools: matched.length ? g.tools.filter((p) => !matched.includes(p)) : [...g.tools, tool] });
          };
          return (
            <div key={s.id}>
              <div className="ag-grant">
                <span><strong title={s.description}>{s.id}</strong></span>
                <select value={g.mode} onChange={(e) => set({ mode: e.target.value as McpGrant["mode"] })} aria-label={`Доступ к ${s.id}`}>
                  <option value="none">нет доступа</option>
                  <option value="all">все инструменты</option>
                  <option value="tools">только выбранные</option>
                </select>
              </div>
              {g.mode === "tools" &&
                (cached.length > 0 ? (
                  <details className="mc-role-tools ag-mcp-tool-picker">
                    <summary>Инструменты · {g.tools.length}</summary>
                    <input type="search" className="g-input" aria-label={`Поиск инструментов ${s.id}`} placeholder="Найти инструмент…" value={search[s.id] ?? ""} onChange={(e) => setSearch((x) => ({ ...x, [s.id]: e.target.value }))} />
                    {extra.length > 0 && (
                      <div className="chips g-mono" aria-label={`Выдано сверх списка ${s.id}`}>
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
                    {!visible.length && <p className="g-muted ag-note">Ничего не найдено.</p>}
                    <div className="ag-checks">
                      {visible.map((t) => {
                        const on = g.tools.some((p) => globMatch(p, t.name));
                        return (
                          <label key={t.name} title={t.description}>
                            <input type="checkbox" checked={on} onChange={() => toggle(t.name)} />
                            <span><strong>{t.name}</strong>{t.description && <span className="g-muted ag-option-description">{t.description}</span>}</span>
                          </label>
                        );
                      })}
                    </div>
                  </details>
                ) : (
                  <details className="mc-role-tools ag-mcp-pattern-picker">
                    <summary>Инструменты · {g.tools.length}</summary>
                    <ChipInput values={g.tools} onChange={(tools) => set({ tools })} placeholder="шаблоны через запятую, например get_*" mono />
                  </details>
                ))}
            </div>
          );
        })}
      </div>

  );
}
