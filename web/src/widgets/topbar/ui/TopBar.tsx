import { useState } from "react";
import { Link, useLocation, useNavigate } from "react-router";
import { useAgentConfig } from "@/entities/agent-config";
import { initials, ProjectIcon, useMeta } from "@/entities/project";
import { useLogout, useSession, useSwitchProject } from "@/entities/session";
import { useProposals } from "@/entities/platform";
import { OBSERVATION_TEXT } from "@/entities/observation";
import { inTaskViews, OPEN, useTasks } from "@/entities/task";
import { useTeams } from "@/entities/team";
import { canManageProject, saveAppearancePreferences, timeAgo, useTick } from "@/shared/lib";
import { ActionMenu, Banner, Button, Character, characterState, Dialog, Icon, Kbd, type MenuEntry, Tabs, Tag, WorkingArcs } from "@/shared/ui";
import { ContextSearch } from "./ContextSearch.tsx";
import { NotificationsPopover } from "./NotificationsPopover.tsx";
import { Popover } from "./Popover.tsx";
import "./topbar.css";

type Props = { onNew: () => void; online: boolean; searchRef: React.RefObject<HTMLInputElement | null>; onOpenSearch: () => void };

type Primary = "board" | "tasks" | "epics" | "docs" | "agents";

function activePrimary(pathname: string): Primary | undefined {
  if (pathname.startsWith("/board")) return "board";
  if (pathname.startsWith("/epic")) return "epics";
  if (pathname.startsWith("/docs")) return "docs";
  if (pathname.startsWith("/agents")) return "agents";
  if (pathname.startsWith("/tasks") || pathname.startsWith("/task/")) return "tasks";
  return undefined;
}

/** The shell's top bar: project, the five primary destinations, search, create, the working-now dock, and the person's menu. */
export function TopBar({ onNew, online, searchRef, onOpenSearch }: Props) {
  useTick();
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const meta = useMeta().data;
  const session = useSession().data;
  const switchProject = useSwitchProject();
  const logout = useLogout();
  const proposals = useProposals().data?.length ?? 0;
  const teams = useTeams().data?.filter((t) => t.state === "active") ?? [];
  const tasks = useTasks().data;
  const agents = useAgentConfig();
  const listed = tasks?.filter(inTaskViews);
  const open = listed ? listed.filter((t) => OPEN.includes(t.status)).length : OPEN.reduce((n, st) => n + (meta?.counts[st] ?? 0), 0);
  const waiting = listed ? listed.filter((t) => t.status === "needs_owner").length : (meta?.counts.needs_owner ?? 0);

  const project = session?.projects.find((p) => p.slug === session.project);
  const canCreate = project?.role !== "viewer";
  const me = session?.mode === "users" ? session.user.name || session.user.login : "Локальный режим";
  const [sheet, setSheet] = useState(false);

  const primary: { id: Primary; label: React.ReactNode; to: string }[] = [
    { id: "board", label: "Доска", to: "/board" },
    {
      id: "tasks",
      label: (
        <>
          Задачи
          {waiting > 0 && (
            <Tag tone="solid-signal" title="ждут вашего решения">
              {waiting}
            </Tag>
          )}
          {open > 0 && <span className="g-caption g-topbar-open">{open}</span>}
        </>
      ),
      to: "/tasks",
    },
    { id: "epics", label: "Эпики", to: "/epics" },
    {
      id: "docs",
      // Pending proposals belong to the documentation: the count rides on its entry, and the
      // proposals themselves stay reachable on their own route.
      label: (
        <>
          Документация
          {proposals > 0 && (
            <>
              <span className="g-caption g-topbar-open" aria-hidden="true">
                {proposals}
              </span>
              <span className="g-sr">{`предложений к документации: ${proposals}`}</span>
            </>
          )}
        </>
      ),
      to: "/docs",
    },
    ...(agents.isSuccess ? [{ id: "agents" as const, label: "Агенты", to: "/agents" }] : []),
  ];
  const current = activePrimary(pathname) ?? ("none" as Primary);

  const more: MenuEntry[] = [
    { id: "automations", label: "Автоматизации", onSelect: () => navigate("/automations") },
    { id: "observations", label: OBSERVATION_TEXT.nav, onSelect: () => navigate("/observations") },
  ];
  const isAdmin = !!session?.user.isAdmin;
  const profile: MenuEntry[] = [
    { id: "settings", label: "Настройки", hint: "G S", onSelect: () => navigate("/settings") },
    ...(canManageProject(isAdmin, project?.role) ? [{ id: "manage", label: "Управление проектом", onSelect: () => navigate("/manage/project/general") } satisfies MenuEntry] : []),
    ...(isAdmin
      ? ([
          { id: "server", label: "Глобальные настройки", onSelect: () => navigate("/manage/server/projects") },
          { id: "stats", label: "Статистика", onSelect: () => navigate("/statistics") },
        ] satisfies MenuEntry[])
      : []),
    ...(session?.mode === "users" ? (["separator", { id: "logout", label: `Выйти (${session.user.login})`, onSelect: () => void logout() }] satisfies MenuEntry[]) : []),
  ];

  const working = teams.filter((t) => t.members.some((m) => m.activity === "working")).length;
  const needOwner = teams.filter((t) => t.taskInfo?.status === "needs_owner").length;
  const state = characterState({ working, waiting: needOwner });
  const toggleTheme = () => saveAppearancePreferences({ theme: document.documentElement.dataset.theme === "paper" ? "ink" : "paper" });

  return (
    <>
      <header className="g-topbar">
        <div className="g-topbar-project-switcher">
          <Popover
            label={project ? `Проект ${project.name}` : "Проект"}
            className="g-topbar-project"
            trigger={
              <>
                <ProjectIcon name={project?.name ?? meta?.project ?? "genie"} src={project?.icon} />
                <span className="name">{project?.name ?? meta?.project ?? "genie"}</span>
                {(session?.projects.length ?? 0) > 1 && <Icon.updown size={12} />}
              </>
            }
          >
            {(close) => (
              <ActionMenu
                label="Проекты"
                onClose={close}
                items={(session?.projects ?? []).map((p) => ({
                  id: p.slug,
                  // A role next to every name crowded the list: the project's own settings say what
                  // the person may do there, so the switcher lists names only, each on one line.
                  label: <><ProjectIcon name={p.name} src={p.icon} /><span className="g-topbar-project-name">{p.name}</span></>,
                  checked: p.slug === session?.project,
                  onSelect: () => void switchProject(p.slug),
                }))}
              />
            )}
          </Popover>
        </div>

        <nav className="g-topbar-nav d-only" aria-label="Навигация">
          <Tabs label="Разделы" items={primary} value={current} />
          <Popover label="Ещё" className="g-icon-btn" trigger={<Icon.dots size={16} />}>
            {(close) => <ActionMenu label="Ещё" items={more} onClose={close} />}
          </Popover>
        </nav>

        <span className="g-topbar-grow" />

        <ContextSearch inputRef={searchRef} className="g-topbar-search d-only" />
        {canCreate && (
          <Button sm className="d-only" onClick={onNew} title="Новая задача">
            <Icon.plus size={14} />
            Новая задача
            <Kbd>C</Kbd>
          </Button>
        )}

        <Popover
          label={working || needOwner ? `Работают сейчас: ${teams.length}` : "Команды"}
          className="g-dock"
          align="right"
          popup="dialog"
          trigger={
            <>
              <Character state={state} size={26} />
              <span className="d-only">{working ? `${working} в работе` : needOwner ? "ждут вас" : "тихо"}</span>
            </>
          }
        >
          {() => (
            <div className="g-menu g-dock-list" role="group" aria-label="Работают сейчас">
              <div className="g-menu-head">Работают сейчас</div>
              {teams.length === 0 && <div className="g-menu-head">Нет активных команд</div>}
              {teams.map((t) => {
                const status = t.taskInfo?.status;
                const needs = status === "needs_owner";
                const isWorking = t.members.some((m) => m.activity === "working");
                return (
                  <Link key={t.id} to={`/team/${encodeURIComponent(t.id)}`} className="g-menu-item">
                    {needs ? <span className="g-dot-signal" aria-hidden="true" /> : isWorking ? <WorkingArcs size={14} /> : <span className="g-dot-idle" aria-hidden="true" />}
                    <span className="g-mono">{t.id}</span>
                    <span className="hint">{needs ? "ждёт вас" : timeAgo(t.created)}</span>
                  </Link>
                );
              })}
            </div>
          )}
        </Popover>

        <NotificationsPopover />
        <button type="button" className="g-icon-btn d-only" aria-label="Сменить тему" title="Сменить тему: чернила или бумага" onClick={toggleTheme}>
          <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" aria-hidden="true">
            <circle cx="8" cy="8" r="6" />
            <path d="M8 2a6 6 0 0 1 0 12z" fill="currentColor" stroke="none" />
          </svg>
        </button>
        <Popover
          label={`Профиль: ${me}`}
          className="g-avatar-btn"
          align="right"
          trigger={
            <>
              {session?.mode === "users" && session.user.avatar ? <img src={session.user.avatar} alt="" /> : <span className="init">{initials(me)}</span>}
              <i className={`g-online${online ? "" : " off"}`} aria-hidden="true" />
            </>
          }
        >
          {(close) => <ActionMenu label="Профиль" items={profile} onClose={close} />}
        </Popover>

        <button type="button" className="g-icon-btn m-only" aria-label="Поиск" onClick={onOpenSearch}>
          <Icon.search size={17} />
        </button>
        {canCreate && (
          <button type="button" className="g-icon-btn m-only" aria-label="Новая задача" onClick={onNew}>
            <Icon.plus size={17} />
          </button>
        )}
        <button type="button" className="g-icon-btn m-only" aria-label="Открыть меню разделов" aria-haspopup="dialog" aria-expanded={sheet} onClick={() => setSheet(true)}>
          <Icon.menu size={18} />
        </button>
      </header>
      {!online && (
        <Banner tone="danger" title="Нет связи с сервером">
          Данные могут быть устаревшими; изменения недоступны, пока связь не вернётся.
        </Banner>
      )}
      {sheet && (
        <Dialog title="Навигация" sheet onClose={() => setSheet(false)}>
          <nav className="g-sheet-nav" aria-label="Разделы" onClick={(e) => (e.target as HTMLElement).closest("a") && setSheet(false)}>
            {primary.map((p) => (
              <Link key={p.id} to={p.to} aria-current={p.id === current ? "page" : undefined}>
                {p.label}
              </Link>
            ))}
            <Link to="/automations">Автоматизации</Link>
            <Link to="/observations">{OBSERVATION_TEXT.nav}</Link>
            <Link to="/notifications">Уведомления</Link>
            <Link to="/settings">Настройки</Link>
            {teams.map((t) => (
              <Link key={t.id} to={`/team/${encodeURIComponent(t.id)}`}>
                Команда {t.id}
              </Link>
            ))}
          </nav>
        </Dialog>
      )}
    </>
  );
}
