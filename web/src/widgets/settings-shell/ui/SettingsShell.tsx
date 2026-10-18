// Profile and administration use separate menus and share the project switcher.

import { Suspense } from "react";
import { NavLink, Outlet, useLocation, useNavigate } from "react-router";
import { PROJECT_ROLE_NAME } from "@/entities/project";
import { useSession, useSwitchProject } from "@/entities/session";
import { lastWorkRoute } from "@/shared/lib";
import { Icon } from "@/shared/ui";

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
    ["tools", "Инструменты"], ["people", "Доступ и роли"],
    ["channels", "Каналы связи"], ["knowledge", "База знаний"],
  ] : serverMode ? [
    ["projects", "Проекты"], ["accounts", "Учётные записи"],
    ["providers", "Провайдеры"], ["tools", "Инструменты"],
  ] : [
    ["account", "Аккаунт"], ["appearance", "Оформление"], ["security", "Вход и пароль"], ["telegram", "Telegram"],
    ["keys", "Провайдеры и ключи"], ["tokens", "Токены CLI и MCP"],
  ];
  const base = projectMode ? "/manage/project" : serverMode ? "/manage/server" : "/settings/me";
  return (
    <div className="app settings-mode">
      <nav className="sidebar" aria-label={title}>
        <div className={`project${many ? " switch" : ""}`} title={project ? (project.hasRepo ? "Проект с репозиторием кода" : "Проект без кода") : undefined}>
          <span className="logo-mark">
            <Icon.mark size={22} />
          </span>
          <span className="txt">
            <span className="name">{title}</span>
            {/* One place names the settings area: the sidebar. The pages used to repeat it
                in a header band of their own, which only showed the project and the role. */}
            <span className="sub">
              {session?.mode === "local" ? "локальный режим" : project ? `${project.name} · ${PROJECT_ROLE_NAME[project.role]}` : "genie"}
            </span>
          </span>
          {many && session && (
            <>
              <Icon.updown size={12} className="caret" />
              <select aria-label="Сменить проект" value={session.project ?? ""} onChange={(e) => void switchProject(e.target.value)}>
                {session.projects.map((p) => (
                  <option key={p.slug} value={p.slug}>
                    {p.name}
                    {p.role === "viewer" ? " (чтение)" : ""}
                  </option>
                ))}
              </select>
            </>
          )}
        </div>

        <div className="nav-group">
          <button type="button" className="nav-item" onClick={() => navigate(lastWorkRoute())}>
            <Icon.back size={15} />
            <span className="grow">← к работе</span>
          </button>
        </div>

        <div className="nav-group">
          <span className="nav-label">{title}</span>
          {items.map(([id, label]) => (
            <NavLink key={id} to={`${base}/${id}`} className={({ isActive }) => `nav-item${isActive || (id === "tools" && /\/(roles|templates|skills|history|automations|decisions|health|vault)$/.test(pathname)) ? " on" : ""}`}>
              <span className="grow">{label}</span>
            </NavLink>
          ))}
        </div>

        <span className="grow" />
      </nav>
      <Suspense fallback={<div className="empty">Загрузка…</div>}>
        <Outlet />
      </Suspense>
    </div>
  );
}
