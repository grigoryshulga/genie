// «Агенты» — the server's agents: roles, team templates, skills and the history of changes. Everyone reads
// them; a server administrator edits roles, templates and skills here too (the global settings no longer
// carry them). MCP connections stay in the server's MCP settings, which «Настроить» opens for administrators.

import { useState } from "react";
import { Link, useSearchParams } from "react-router";
import { useAgentConfig } from "@/entities/agent-config";
import { useSession } from "@/entities/session";
import { matchesSearch } from "@/shared/lib";
import { ContextSearch } from "@/widgets/topbar";
import { ErrorState, Icon, PageHeader, Skeleton } from "@/shared/ui";
import { CreateDialog, History, McpTab, RolesTab, SkillsTab, TemplatesTab, type CreateKind } from "@/widgets/agents-kit";
import "./agents-sheets.css";
import "@/shared/ui/agents.css";

type Tab = "roles" | "templates" | "skills" | "mcp" | "history";
const TABS: { id: Tab; title: string; adminOnly?: boolean }[] = [
  { id: "roles", title: "Роли" },
  { id: "templates", title: "Шаблоны команд" },
  { id: "skills", title: "Навыки" },
  { id: "mcp", title: "MCP" },
  { id: "history", title: "История", adminOnly: true },
];
const NEW: Partial<Record<Tab, { kind: CreateKind; label: string }>> = {
  roles: { kind: "roles", label: "Новая роль" },
  templates: { kind: "templates", label: "Новый шаблон" },
  skills: { kind: "skills", label: "Новый навык" },
};

export function AgentsPage() {
  const [sp, setSp] = useSearchParams();
  const cfg = useAgentConfig();
  const isAdmin = !!useSession().data?.user.isAdmin;
  const [creating, setCreating] = useState<CreateKind>();
  const tabs = TABS.filter((t) => !t.adminOnly || isAdmin);
  const tab = (tabs.some((t) => t.id === sp.get("tab")) ? sp.get("tab") : "roles") as Tab;
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
  const create = isAdmin ? NEW[tab] : undefined;
  return (
    <main className="main agents">
      <PageHeader
        title="Агенты"
        toolbar={<>
          <div className="g-tabs" role="tablist" aria-label="Разделы">
            {tabs.map((t) => (
              <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} onClick={() => update(t.id)}>
                {t.title}
              </button>
            ))}
          </div>
          {tab === "mcp" && isAdmin && (
            <Link to="/manage/server/tools?tab=mcp" className="g-btn secondary sm">
              <Icon.gear size={13} />
              <span className="d-only">Настроить</span>
            </Link>
          )}
          {create && c && tab !== "roles" && (
            <button type="button" className="g-btn secondary" onClick={() => setCreating(create.kind)}>
              <Icon.plus size={13} />
              <span className="d-only">{create.label}</span>
            </button>
          )}
        </>}
      />
      <ContextSearch className="mobile-context-search mobile-search-bar" />
      {!c ? (
        cfg.error ? (
          <div className="ag-pane-empty"><ErrorState message={cfg.error.message} onRetry={() => void cfg.refetch()} /></div>
        ) : (
          <div className="ag-pane-empty" aria-busy="true">
            <Skeleton width="60%" height={20} />
            <Skeleton width="40%" />
          </div>
        )
      ) : tab === "roles" ? (
        <RolesTab cfg={c} selected={selected} onSelect={select} readOnly={!isAdmin} onCreate={isAdmin ? () => setCreating("roles") : undefined} />
      ) : tab === "templates" ? (
        <TemplatesTab cfg={c} selected={selected} onSelect={select} readOnly={!isAdmin} />
      ) : tab === "skills" ? (
        <SkillsTab cfg={c} selected={selected} onSelect={select} readOnly={!isAdmin} />
      ) : tab === "mcp" ? (
        <McpTab cfg={c} readOnly />
      ) : (
        <div className="ag-page">
          <History admin />
        </div>
      )}
      {creating && c && (
        <CreateDialog
          kind={creating}
          cfg={c}
          onClose={() => setCreating(undefined)}
          onCreated={(id) => {
            setCreating(undefined);
            update(creating, id);
          }}
        />
      )}
    </main>
  );
}
