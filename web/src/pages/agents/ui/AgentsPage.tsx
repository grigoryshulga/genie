// The server's agent catalog: roles, team templates, skills, MCP connections and the history of changes, a
// management hub page each. Everyone reads them; a server administrator edits roles, templates and skills
// here. MCP connections stay in the server's tool settings, which «Настроить» opens for administrators.

import { useState } from "react";
import { Link, useSearchParams } from "react-router";
import { useAgentConfig } from "@/entities/agent-config";
import { useSession } from "@/entities/session";
import { matchesSearch } from "@/shared/lib";
import { ErrorState, Icon, Skeleton } from "@/shared/ui";
import { HubActions } from "@/widgets/settings-shell";
import { CreateDialog, History, McpTab, RolesTab, SkillsTab, TemplatesTab, type CreateKind } from "@/widgets/agents-kit";
import "./agents-sheets.css";
import "@/shared/ui/agents.css";

export type AgentsTab = "roles" | "templates" | "skills" | "mcp" | "history";
const NEW: Partial<Record<AgentsTab, { kind: CreateKind; label: string }>> = {
  roles: { kind: "roles", label: "Новая роль" },
  templates: { kind: "templates", label: "Новый шаблон" },
  skills: { kind: "skills", label: "Новый навык" },
};

export function AgentsPage({ tab }: { tab: AgentsTab }) {
  const [sp, setSp] = useSearchParams();
  const cfg = useAgentConfig();
  const isAdmin = !!useSession().data?.user.isAdmin;
  const [creating, setCreating] = useState<CreateKind>();
  const selected = sp.get("id") ?? undefined;
  const select = (id: string) => setSp((previous) => {
    const next = new URLSearchParams(previous);
    next.set("id", id);
    return next;
  });
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
      <HubActions>
          {tab === "mcp" && isAdmin && (
            <Link to="/manage/server/tools?tab=mcp" className="g-btn secondary sm">
              <Icon.gear size={13} />
              <span className="d-only">Настроить</span>
            </Link>
          )}
          {create && c && tab !== "roles" && tab !== "templates" && (
            <button type="button" className="g-btn secondary" onClick={() => setCreating(create.kind)}>
              <Icon.plus size={13} />
              <span className="d-only">{create.label}</span>
            </button>
          )}
      </HubActions>

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
        <TemplatesTab cfg={c} selected={selected} onSelect={select} readOnly={!isAdmin} onCreate={isAdmin ? () => setCreating("templates") : undefined} />
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
          cfg={cfg.data ?? c}
          onClose={() => setCreating(undefined)}
          onCreated={(id) => {
            setCreating(undefined);
            select(id);
          }}
        />
      )}
    </main>
  );
}
