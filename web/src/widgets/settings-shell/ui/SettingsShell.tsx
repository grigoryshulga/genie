// Profile and administration use separate menus and share the project switcher.
//
// The rail is the one place that names the settings area: a card-like column of
// section links on the left, the page itself on the right. Pages no longer repeat
// the title in a header band of their own.

import { Suspense } from "react";
import { Link, Outlet, useLocation, useNavigate } from "react-router";
import { OBSERVATION_TEXT } from "@/entities/observation";
import { PROJECT_ROLE_NAME } from "@/entities/project";
import { useSession, useSwitchProject } from "@/entities/session";
import { lastWorkRoute } from "@/shared/lib";
import { Icon } from "@/shared/ui";
import { SettingsLoading } from "./SettingsLoading.tsx";
import "@/shared/ui/settings.css";

/** Sections that are not in `items` but belong to the "tools" tab of the rail. */
const TOOLS_SECTIONS = /\/(roles|templates|skills|history|automations|decisions|health|vault)$/;

export function SettingsShell() {
  const session = useSession().data;
  const switchProject = useSwitchProject();
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const project = session?.projects.find((p) => p.slug === session.project);
  const many = (session?.projects.length ?? 0) > 1;
  const projectMode = pathname.startsWith("/manage/project");
  const serverMode = pathname.startsWith("/manage/server");
  const title = projectMode ? "Управление проектом" : serverMode ? "Глобальные настройки" : "Настройки";
  const items = projectMode ? [
    ["general", "Общие настройки"], ["repos", "Репозитории и окружение"],
    ["tools", "Инструменты"], ["observation", OBSERVATION_TEXT.section], ["people", "Доступ и роли"],
    ["channels", "Каналы связи"], ["knowledge", "База знаний"],
  ] : serverMode ? [
    ["projects", "Проекты"], ["accounts", "Учётные записи"],
    ["providers", "Провайдеры"], ["tools", "Инструменты"],
  ] : [
    ["account", "Аккаунт"], ["appearance", "Оформление"], ["security", "Вход и пароль"], ["telegram", "Telegram"],
    ["keys", "Провайдеры и ключи"], ["tokens", "Токены CLI и MCP"],
  ];
  const base = projectMode ? "/manage/project" : serverMode ? "/manage/server" : "/settings/me";
  const sub = session?.mode === "local" ? "локальный режим" : project ? `${project.name} · ${PROJECT_ROLE_NAME[project.role]}` : "genie";
  const active = (id: string) => pathname === `${base}/${id}` || (id === "tools" && TOOLS_SECTIONS.test(pathname));
  return (
    <div className="app settings-mode">
      <nav className="st-rail" aria-label={title}>
        <div className="st-rail-head">
          <span className="st-rail-mark" title={project ? (project.hasRepo ? "Проект с репозиторием кода" : "Проект без кода") : undefined}>
            <Icon.mark size={20} />
          </span>
          <span className="st-rail-id">
            <b title={title}>{title}</b>
            <span className="g-muted">{sub}</span>
          </span>
        </div>

        {many && session && (
          <label className="st-rail-switch">
            <span className="sr-only">Сменить проект</span>
            <select className="g-select sm" value={session.project ?? ""} onChange={(e) => void switchProject(e.target.value)}>
              {session.projects.map((p) => (
                <option key={p.slug} value={p.slug}>
                  {p.name}
                  {p.role === "viewer" ? " (чтение)" : ""}
                </option>
              ))}
            </select>
          </label>
        )}

        <div className="g-sheet-nav st-rail-nav">
          <button type="button" className="st-rail-back" aria-label="Вернуться к работе" onClick={() => navigate(lastWorkRoute())}>
            <Icon.back size={15} />
            {/* The icon is the arrow; the label only spells it out on wider screens. */}
            <span className="grow">к работе</span>
          </button>
          {items.map(([id, label]) => (
            <Link key={id} to={`${base}/${id}`} className={active(id) ? "on" : undefined} aria-current={active(id) ? "page" : undefined}>
              {label}
            </Link>
          ))}
        </div>
      </nav>
      <Suspense fallback={<div className="st-rail-fallback"><SettingsLoading rows={5} /></div>}>
        <Outlet />
      </Suspense>
    </div>
  );
}
