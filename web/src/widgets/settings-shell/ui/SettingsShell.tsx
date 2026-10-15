// The settings mode (PD21): its own layout — a section menu by the session's
// rights, the same project switcher as the work sidebar, and a way back to the
// last work route.

import { Suspense } from "react";
import { NavLink, Outlet, useLocation, useNavigate } from "react-router";
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
  // The agents editor lives under /settings/server too — the menu keeps the two apart.
  const agentsRoute = /^\/settings\/server\/(roles|templates|skills|history)/.test(pathname);
  const managementRoute = pathname.startsWith("/settings/server") && !agentsRoute;
  const projectRoute = pathname.startsWith("/settings/project") && !pathname.startsWith("/settings/project/automations");
  return (
    <div className="app settings-mode">
      <nav className="sidebar" aria-label="Настройки">
        <div className={`project${many ? " switch" : ""}`} title={project ? (project.hasRepo ? "Проект с репозиторием кода" : "Проект без кода") : undefined}>
          <span className="logo-mark">
            <Icon.mark size={15} />
          </span>
          <span className="txt">
            <span className="name">Настройки</span>
            <span className="sub">{project?.name ?? "genie"}</span>
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
          <span className="nav-label">Настройки</span>
          <NavLink to="/settings/me/account" className={({ isActive }) => `nav-item${isActive || pathname.startsWith("/settings/me") ? " on" : ""}`}>
            <Icon.user size={15} />
            <span className="grow">Личное</span>
          </NavLink>
        </div>

        {(project || session?.user.isAdmin) && (
          <div className="nav-group">
            <span className="nav-label">Администрирование</span>
            {project && (
              <NavLink to="/settings/project/general" className={({ isActive }) => `nav-item${isActive || projectRoute ? " on" : ""}`}>
                <Icon.folder size={15} />
                <span className="grow">Настройки проекта</span>
              </NavLink>
            )}
            {project && (
              <NavLink to="/settings/project/automations" className={({ isActive }) => `nav-item${isActive ? " on" : ""}`}>
                <Icon.bolt size={15} />
                <span className="grow">Автоматизации</span>
              </NavLink>
            )}
            {session?.user.isAdmin && (
              <NavLink to="/settings/server/health" className={({ isActive }) => `nav-item${isActive || managementRoute ? " on" : ""}`}>
                <Icon.server size={15} />
                <span className="grow">Управление</span>
              </NavLink>
            )}
            {session?.user.isAdmin && (
              <NavLink to="/settings/stats" className={({ isActive }) => `nav-item${isActive ? " on" : ""}`}>
                <Icon.chart size={15} />
                <span className="grow">Статистика</span>
              </NavLink>
            )}
            {session?.user.isAdmin && (
              <NavLink to="/settings/server/roles" className={({ isActive }) => `nav-item${isActive || agentsRoute ? " on" : ""}`}>
                <Icon.userPlus size={15} />
                <span className="grow">Агенты</span>
              </NavLink>
            )}
          </div>
        )}

        <span className="grow" />
      </nav>
      <Suspense fallback={<div className="empty">Загрузка…</div>}>
        <Outlet />
      </Suspense>
    </div>
  );
}
