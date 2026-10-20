// «Агенты» — the catalogue of the server's agents: roles, team templates,
// skills and MCP connections (PD13). Everyone reads it (PD14) — even a server
// administrator: the editing lives in the settings mode (PD21, S6), and the
// «Настроить» link leads there.

import { Link, useSearchParams } from "react-router";
import { useAgentConfig } from "@/entities/agent-config";
import { useSession } from "@/entities/session";
import { matchesSearch } from "@/shared/lib";
import { ContextSearch } from "@/widgets/topbar";
import { Icon } from "@/shared/ui";
import { McpTab, RolesTab, SkillsTab, TemplatesTab } from "@/widgets/agents-kit";
import "@/shared/ui/agents.css";

type Tab = "roles" | "templates" | "skills" | "mcp";
const TABS: { id: Tab; title: string }[] = [
  { id: "roles", title: "Роли" },
  { id: "templates", title: "Шаблоны команд" },
  { id: "skills", title: "Навыки" },
  { id: "mcp", title: "MCP" },
];

export function AgentsPage() {
  const [sp, setSp] = useSearchParams();
  const cfg = useAgentConfig();
  const isAdmin = !!useSession().data?.user.isAdmin;
  const tab = (TABS.some((t) => t.id === sp.get("tab")) ? sp.get("tab") : "roles") as Tab;
  const selected = sp.get("id") ?? undefined;
  const update = (nextTab: Tab, id?: string) => setSp((previous) => {
    const next = new URLSearchParams(previous);
    next.set("tab", nextTab);
    if (id) next.set("id", id); else next.delete("id");
    return next;
  });
  const select = (id: string) => update(tab, id);
  const query = sp.get("q") ?? "";
  const c = cfg.data ? { ...cfg.data,
    roles: tab === "roles" ? cfg.data.roles.filter((r) => matchesSearch(query, r.id, r.title, r.description)) : cfg.data.roles,
    teams: tab === "templates" ? cfg.data.teams.filter((t) => matchesSearch(query, t.id, t.title, t.description)) : cfg.data.teams,
    skills: tab === "skills" ? cfg.data.skills.filter((s) => matchesSearch(query, s.name, s.description)) : cfg.data.skills,
    mcp: tab === "mcp" ? cfg.data.mcp.filter((m) => matchesSearch(query, m.id, m.description, ...m.tools?.map((t) => `${t.name} ${t.description ?? ""}`) ?? [])) : cfg.data.mcp,
  } : undefined;
  return (
    <main className="main agents">
      <header className="topbar">
        <h1>Агенты</h1>
        <div className="seg" role="tablist" aria-label="Разделы">
          {TABS.map((t) => (
            <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} className={tab === t.id ? "on" : ""} onClick={() => update(t.id)}>
              {t.title}
            </button>
          ))}
        </div>
        <span className="grow" />
        {isAdmin && (
          <Link to={`/manage/server/${tab}${selected ? `?id=${encodeURIComponent(selected)}` : ""}`} className="btn">
            <Icon.gear size={13} />
            <span className="d-only">Настроить</span>
          </Link>
        )}
      </header>
      <ContextSearch className="mobile-context-search mobile-search-bar" />
      {!c ? (
        <div className="pane-empty">{cfg.error ? cfg.error.message : "Загрузка…"}</div>
      ) : tab === "roles" ? (
        <RolesTab cfg={c} selected={selected} onSelect={select} readOnly />
      ) : tab === "templates" ? (
        <TemplatesTab cfg={c} selected={selected} onSelect={select} readOnly />
      ) : tab === "skills" ? (
        <SkillsTab cfg={c} selected={selected} onSelect={select} readOnly />
      ) : (
        <McpTab cfg={c} readOnly />
      )}
    </main>
  );
}
