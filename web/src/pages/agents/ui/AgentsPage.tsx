// «Агенты» — the catalogue of the server's agents: roles, team templates,
// skills and MCP connections (PD13). Everyone reads it (PD14) — even a server
// administrator: the editing lives in the settings mode (PD21, S6), and the
// «Настроить» link leads there.

import { Link, useSearchParams } from "react-router";
import { useAgentConfig } from "@/entities/agent-config";
import { useSession } from "@/entities/session";
import { Icon } from "@/shared/ui";
import { McpTab, RolesTab, SkillsTab, TemplatesTab } from "@/widgets/agents-kit";
import { Problems } from "@/widgets/settings-kit";
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
  const select = (id: string) => setSp({ tab, id });
  const c = cfg.data;
  const errors = c?.problems.filter((p) => p.level === "error") ?? [];
  return (
    <main className="main agents">
      <header className="topbar">
        <h1>Агенты</h1>
        <div className="seg" role="tablist" aria-label="Разделы">
          {TABS.map((t) => (
            <button key={t.id} type="button" role="tab" aria-selected={tab === t.id} className={tab === t.id ? "on" : ""} onClick={() => setSp({ tab: t.id })}>
              {t.title}
            </button>
          ))}
        </div>
        <span className="grow" />
        <span className="sub d-only">каталог · настраивают администраторы сервера</span>
        {isAdmin && (
          <Link to={`/settings/server/${tab}${selected ? `?id=${encodeURIComponent(selected)}` : ""}`} className="btn">
            <Icon.gear size={13} />
            <span className="d-only">Настроить</span>
          </Link>
        )}
      </header>
      {c?.admin && errors.length > 0 && (
        <div className="ag-banner">
          <Problems items={errors} />
          <span className="muted">Сломанные файлы не останавливают работу: действует их последняя рабочая версия.</span>
        </div>
      )}
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
