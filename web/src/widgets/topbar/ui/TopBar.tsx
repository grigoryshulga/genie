import { useState } from "react";
import { Link, useLocation, useNavigate } from "react-router";
import { useAgentConfig } from "@/entities/agent-config";
import { initials, ProjectIcon, useMeta } from "@/entities/project";
import { useLogout, useSession, useSwitchProject } from "@/entities/session";
import { useNotifications, useProposals } from "@/entities/platform";
import { OBSERVATION_TEXT } from "@/entities/observation";
import { EpicIcon, inTaskViews, OPEN, useTasks } from "@/entities/task";
import { useTeams } from "@/entities/team";
import { canManageProject, saveAppearancePreferences, useTick } from "@/shared/lib";
import { ActionMenu, Banner, Dialog, Icon, IconButton, type MenuEntry, Tabs, Tag } from "@/shared/ui";
import { ContextSearch } from "./ContextSearch.tsx";
import { NotificationsPopover } from "./NotificationsPopover.tsx";
import { Popover } from "./Popover.tsx";
import "./topbar.css";
// The top bar and the shell frame (`.app`) it sits in.
import "./shell.css";

type Props = { onNew: () => void; online: boolean; searchRef: React.RefObject<HTMLInputElement | null>; onOpenSearch: () => void; onOpenPalette: () => void };

type Primary = "board" | "tasks" | "epics" | "docs" | "agents";

function activePrimary(pathname: string): Primary | undefined {
  if (pathname.startsWith("/board")) return "board";
  if (pathname.startsWith("/epic")) return "epics";
  if (pathname.startsWith("/docs")) return "docs";
  if (pathname.startsWith("/agents")) return "agents";
  if (pathname.startsWith("/tasks") || pathname.startsWith("/task/")) return "tasks";
  return undefined;
}

/** Project, navigation, search, create, notifications, and the person's menu. */
export function TopBar({ onNew, online, searchRef, onOpenSearch, onOpenPalette }: Props) {
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
  const unread = useNotifications().data?.unread ?? 0;

  // Every destination wears an icon: the bar collapses to icons (with the label under them on a phone)
  // when the window is narrow. The label stays in the DOM so the accessible name never changes.
  const label = (text: string, short?: string) => (
    <span className="g-tab-label">
      <span className={short ? "g-tab-full" : undefined}>{text}</span>
      {short && (
        <span className="g-tab-short" aria-hidden="true">
          {short}
        </span>
      )}
    </span>
  );
  const primary: { id: Primary; label: React.ReactNode; to: string; title: string }[] = [
    {
      id: "board",
      label: (
        <>
          <Icon.board size={16} />
          {label("Доска")}
        </>
      ),
      to: "/board",
      title: "Доска",
    },
    {
      id: "tasks",
      label: (
        <>
          <Icon.list size={16} />
          {label("Задачи")}
          {waiting > 0 && (
            <Tag tone="solid-signal" title="ждут вашего решения">
              {waiting}
            </Tag>
          )}
          {open > 0 && <span className="g-caption g-topbar-open">{open}</span>}
        </>
      ),
      to: "/tasks",
      title: "Задачи",
    },
    {
      id: "epics",
      label: (
        <>
          <EpicIcon size={16} />
          {label("Эпики")}
        </>
      ),
      to: "/epics",
      title: "Эпики",
    },
    {
      id: "docs",
      // Pending proposals belong to the documentation: the count rides on its entry, and the
      // proposals themselves stay reachable on their own route.
      label: (
        <>
          <Icon.file size={16} />
          {label("Документация", "Доки")}
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
      title: "Документация",
    },
    ...(agents.isSuccess
      ? [
          {
            id: "agents" as const,
            label: (
              <>
                <Icon.spark size={16} />
                {label("Агенты")}
              </>
            ),
            to: "/agents",
            title: "Агенты",
          },
        ]
      : []),
  ];
  const current = activePrimary(pathname) ?? ("none" as Primary);

  const more: MenuEntry[] = [
    // ⌘K is a keyboard's way into the palette; a pointer needs a row of its own, so the overflow carries one.
    { id: "palette", label: "Команды", hint: "⌘K", onSelect: onOpenPalette },
    "separator",
    { id: "automations", label: "Автоматизации", onSelect: () => navigate("/automations") },
    { id: "observations", label: OBSERVATION_TEXT.nav, onSelect: () => navigate("/observations") },
  ];
  const isAdmin = !!session?.user.isAdmin;
  const toggleTheme = () => saveAppearancePreferences({ theme: document.documentElement.dataset.theme === "paper" ? "ink" : "paper" });
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

  return (
    <>
      <header className="g-topbar">
        {/* The project floats apart from the bar as an icon of its own; its name is the button's label. */}
        <div className="g-topbar-project-switcher">
          <Popover
            label={project ? `Проект ${project.name}` : "Проект"}
            className="g-topbar-project"
            trigger={<ProjectIcon name={project?.name ?? meta?.project ?? "genie"} src={project?.icon} />}
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

        <div className="g-topbar-pill">
          <nav className="g-topbar-nav" aria-label="Навигация">
            <Tabs label="Разделы" items={primary} value={current} />
            <Popover label="Ещё" className="g-icon-btn d-only" trigger={<Icon.dots size={16} />}>
              {(close) => <ActionMenu label="Ещё" items={more} onClose={close} />}
            </Popover>
            <button type="button" className="g-bar-more m-only" aria-label="Открыть меню разделов" aria-haspopup="dialog" aria-expanded={sheet} onClick={() => setSheet(true)}>
              <span className="g-bar-more-icon">
                <Icon.dots size={16} />
                {unread > 0 && <i className="g-unread" aria-hidden="true" />}
              </span>
              <span className="g-tab-label" aria-hidden="true">Ещё</span>
            </button>
          </nav>

          <span className="g-topbar-sep d-only" aria-hidden="true" />

          <ContextSearch inputRef={searchRef} className="g-topbar-search d-only" />
          {canCreate && (
            <IconButton label="Новая задача" className="g-topbar-create d-only" onClick={onNew}>
              <Icon.plus size={20} />
            </IconButton>
          )}

          <div className="g-topbar-utility d-only">
            <NotificationsPopover />
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
              {(close) => {
                // Read when the popup opens: appearance can also change in Settings.
                const paper = document.documentElement.dataset.theme === "paper";
                return <ActionMenu label="Профиль" items={[
                  { id: "theme", label: "Светлая тема", hint: paper ? "Вкл" : "Выкл", checked: paper, onSelect: toggleTheme },
                  "separator",
                  ...profile,
                ]} onClose={close} />;
              }}
            </Popover>
          </div>
        </div>
      </header>
      {!online && (
        <Banner tone="danger" title="Нет связи с сервером">
          Данные могут быть устаревшими; изменения недоступны, пока связь не вернётся.
        </Banner>
      )}
      {sheet && (
        <Dialog title="Навигация" sheet onClose={() => setSheet(false)}>
          {/* The destinations live in the bar under the thumb; the sheet carries everything the bar has no room for. */}
          <nav className="g-sheet-nav" aria-label="Разделы" onClick={(e) => (e.target as HTMLElement).closest("a") && setSheet(false)}>
            {canCreate && (
              <button
                type="button"
                onClick={() => {
                  setSheet(false);
                  onNew();
                }}
              >
                <Icon.plus size={16} />
                Новая задача
              </button>
            )}
            <button
              type="button"
              onClick={() => {
                setSheet(false);
                onOpenSearch();
              }}
            >
              <Icon.search size={16} />
              Поиск
            </button>
            {/* A thumb has no ⌘K: the sheet carries the palette as a row like every destination beside it. */}
            <button
              type="button"
              onClick={() => {
                setSheet(false);
                onOpenPalette();
              }}
            >
              <Icon.bolt size={16} />
              Команды
            </button>
            <Link to="/automations">
              <Icon.restart size={16} />
              Автоматизации
            </Link>
            <Link to="/observations">
              <Icon.eye size={16} />
              {OBSERVATION_TEXT.nav}
            </Link>
            <Link to="/notifications">
              <Icon.bell size={16} />
              Уведомления
              {unread > 0 && <span className="g-caption g-sheet-count">{unread}</span>}
            </Link>
            {teams.map((t) => (
              <Link key={t.id} to={`/team/${encodeURIComponent(t.id)}`}>
                <Icon.user size={16} />
                Команда {t.id}
              </Link>
            ))}
            <Link to="/settings">
              <Icon.gear size={16} />
              Настройки
            </Link>
            <button
              type="button"
              onClick={() => {
                setSheet(false);
                toggleTheme();
              }}
            >
              <Icon.theme size={16} />
              Светлая тема
            </button>
            {session?.mode === "users" && (
              <button type="button" onClick={() => void logout()}>
                <Icon.logout size={16} />
                {`Выйти (${session.user.login})`}
              </button>
            )}
          </nav>
        </Dialog>
      )}
    </>
  );
}
