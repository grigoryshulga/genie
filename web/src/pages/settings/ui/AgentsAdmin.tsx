// «Агенты» — the server's agent configuration (PD13, PD14): roles, team
// templates, skills and the history of every change, edited by server
// administrators. The read-only catalogue stays in the work mode (/agents).

import { useEffect, useState } from "react";
import { Link, useNavigate, useParams, useSearchParams } from "react-router";
import { useAgentConfig } from "@/entities/agent-config";
import { Icon } from "@/shared/ui";
import { CreateDialog, RolesTab, SkillsTab, TemplatesTab, type CreateKind } from "@/widgets/agents-kit";
import { History, Problems } from "@/widgets/settings-kit";
import "@/shared/ui/agents.css";

const SECTIONS = ["roles", "templates", "skills", "history"] as const;
type Section = (typeof SECTIONS)[number];
const TITLES: Record<Section, string> = { roles: "Роли", templates: "Шаблоны команд", skills: "Навыки", history: "История" };
const NEW: Partial<Record<Section, string>> = { roles: "Новая роль", templates: "Новый шаблон", skills: "Новый навык" };

export function AgentsAdminPage() {
  const [sp, setSp] = useSearchParams();
  const navigate = useNavigate();
  const cfg = useAgentConfig();
  const [creating, setCreating] = useState<CreateKind>();
  const { section = "roles" } = useParams();
  const linked = sp.get("tab");
  const selected = sp.get("id") ?? undefined;
  // The kit's cross-links switch tabs with ?tab=; here a tab is the path itself.
  useEffect(() => {
    if ((linked === "roles" || linked === "templates" || linked === "skills") && linked !== section)
      navigate(`/settings/server/${linked}${selected ? `?id=${encodeURIComponent(selected)}` : ""}`, { replace: true });
  }, [linked, section, selected, navigate]);
  const select = (id: string) => setSp({ id });
  const c = cfg.data;
  const errors = c?.problems.filter((p) => p.level === "error") ?? [];
  return (
    <main className="main agents">
      <header className="topbar">
        <h1>Агенты</h1>
        <div className="seg" role="tablist" aria-label="Разделы">
          {SECTIONS.map((s) => (
            <button key={s} type="button" role="tab" aria-selected={section === s} className={section === s ? "on" : ""} onClick={() => navigate(`/settings/server/${s}`)}>
              {TITLES[s]}
            </button>
          ))}
        </div>
        <span className="grow" />
        <span className="sub d-only">конфигурация сервера · каталог для всех — в рабочей части</span>
        {NEW[section as Section] && c && (
          <button type="button" className="btn primary" onClick={() => setCreating(section as CreateKind)}>
            <Icon.plus size={13} />
            <span className="d-only">{NEW[section as Section]}</span>
          </button>
        )}
        <Link to={`/agents${selected ? `?tab=${section}&id=${encodeURIComponent(selected)}` : ""}`} className="btn">
          <Icon.userPlus size={13} />
          <span className="d-only">Каталог</span>
        </Link>
      </header>
      {c?.admin && errors.length > 0 && (
        <div className="ag-banner">
          <Problems items={errors} />
          <span className="muted">Сломанные файлы не останавливают работу: действует их последняя рабочая версия.</span>
        </div>
      )}
      {!c ? (
        <div className="pane-empty">{cfg.error ? cfg.error.message : "Загрузка…"}</div>
      ) : section === "roles" ? (
        <RolesTab cfg={c} selected={selected} onSelect={select} base="/settings/server" />
      ) : section === "templates" ? (
        <TemplatesTab cfg={c} selected={selected} onSelect={select} base="/settings/server" />
      ) : section === "skills" ? (
        <SkillsTab cfg={c} selected={selected} onSelect={select} base="/settings/server" />
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
            navigate(`/settings/server/${creating}?id=${encodeURIComponent(id)}`);
          }}
        />
      )}
    </main>
  );
}
