import { useEffect, useLayoutEffect, useRef } from "react";
import { useLocation, useNavigate } from "react-router";
import { initials, ProjectIcon, useMeta } from "@/entities/project";
import { useLogout, useSession, useSwitchProject } from "@/entities/session";
import { attentionChangesets, useChangesets } from "@/entities/doc";
import { EpicIcon, inTaskViews, OPEN, useTasks } from "@/entities/task";
import { canManageProject, hubHome, saveAppearancePreferences, useTick, visibleHub } from "@/shared/lib";
import { ActionMenu, Banner, Icon, IconButton, type MenuEntry, type NavMenuEntry, Tabs, Tag, useToast } from "@/shared/ui";
import { ContextSearch } from "./ContextSearch.tsx";
import { NotificationsPopover } from "./NotificationsPopover.tsx";
import { Popover } from "./Popover.tsx";
import "./topbar.css";
// The top bar and the shell frame (`.app`) it sits in.
import "./shell.css";

type Props = { onNew: () => void; online: boolean; searchRef: React.RefObject<HTMLInputElement | null>; searchOpen: boolean; onOpenSearch: () => void; onCloseSearch: () => void; onOpenPalette: () => void };

type Primary = "tasks" | "chat" | "knowledge" | "manage";

function activePrimary(pathname: string): Primary | undefined {
  if (["/board", "/tasks", "/task/", "/epic"].some((p) => pathname.startsWith(p))) return "tasks";
  if (pathname.startsWith("/chat")) return "chat";
  if (pathname.startsWith("/docs")) return "knowledge";
  if (pathname.startsWith("/manage")) return "manage";
  return undefined;
}

/** «Задачи» reopens the view the person used last: the list, the board or the epics. */
const TASK_VIEW_KEY = "genie.tasks-view";
const TASK_VIEWS = ["/tasks", "/board", "/epics"];
function rememberedTaskView(): string {
  try {
    const saved = localStorage.getItem(TASK_VIEW_KEY);
    return saved && TASK_VIEWS.includes(saved) ? saved : "/board";
  } catch {
    return "/board";
  }
}

/** Project, navigation, search, create, notifications, and the person's menu. */
export function TopBar({ onNew, online, searchRef, searchOpen, onOpenSearch, onCloseSearch, onOpenPalette }: Props) {
  useTick();
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const meta = useMeta().data;
  const session = useSession().data;
  const switchProject = useSwitchProject();
  const logout = useLogout();
  const attention = attentionChangesets(useChangesets({ view: "all" }).data ?? []).length;
  const tasks = useTasks().data;
  const listed = tasks?.filter(inTaskViews);
  const open = listed ? listed.filter((t) => OPEN.includes(t.status)).length : OPEN.reduce((n, st) => n + (meta?.counts[st] ?? 0), 0);
  const waiting = listed ? listed.filter((t) => t.status === "needs_owner").length : (meta?.counts.needs_owner ?? 0);

  const project = session?.projects.find((p) => p.slug === session.project);
  const canCreate = project?.role !== "viewer";
  const me = session?.mode === "users" ? session.user.name || session.user.login : "Локальный режим";
  const toast = useToast();
  const pill = useRef<HTMLDivElement>(null);
  // Keep the desktop pill stable while replacing its navigation with search.
  useLayoutEffect(() => {
    const node = pill.current;
    if (!node || searchOpen) return;
    const measure = () => {
      const box = node.getBoundingClientRect();
      node.style.setProperty("--g-topbar-rest-width", `${box.width}px`);
      node.style.setProperty("--g-topbar-rest-height", `${box.height}px`);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(node);
    return () => observer.disconnect();
  }, [searchOpen]);
  const searchToggle = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (searchOpen) searchRef.current?.focus();
  }, [searchOpen, searchRef]);
  const closeSearch = () => {
    onCloseSearch();
    requestAnimationFrame(() => searchToggle.current?.focus());
  };
  const manageProject = async (slug: string) => {
    try {
      if (slug !== session?.project) await switchProject(slug);
      navigate("/manage/project/general");
    } catch (error) {
      toast(error instanceof Error ? error.message : "Не удалось открыть проект", "error");
    }
  };

  const isAdmin = !!session?.user.isAdmin;
  const viewer = { serverAdmin: isAdmin, projectAdmin: canManageProject(isAdmin, project?.role) };
  useEffect(() => {
    const view = TASK_VIEWS.find((v) => pathname === v);
    if (view) try { localStorage.setItem(TASK_VIEW_KEY, view); } catch { /* private mode: fall back to the board */ }
  }, [pathname]);

  // Every destination wears an icon: the bar collapses to icons (with the label under them on a phone)
  // when the window is narrow. The label stays in the DOM so the accessible name never changes.
  const label = (text: string) => <span className="g-tab-label">{text}</span>;
  const count = (n: number, what: string) =>
    n > 0 && (
      <>
        <span className="g-caption g-topbar-open" aria-hidden="true">{n}</span>
        <span className="g-sr">{`${what}: ${n}`}</span>
      </>
    );
  const page = (id: string, text: string, to: string, icon: React.ReactNode, extra?: Partial<NavMenuEntry>): NavMenuEntry => ({
    id,
    label: text,
    to,
    icon,
    current: pathname === to || (to !== "/docs" && pathname.startsWith(`${to}/`)),
    ...extra,
  });
  const hub = visibleHub(viewer);
  const primary: { id: Primary; label: React.ReactNode; to: string; title: string; menu?: { label: string; items: NavMenuEntry[] } }[] = [
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
      to: rememberedTaskView(),
      title: "Задачи",
      menu: {
        label: "Виды задач",
        items: [
          page("list", "Список", "/tasks", <Icon.list size={14} />, { current: pathname === "/tasks" || pathname.startsWith("/task/") }),
          page("board", "Доска", "/board", <Icon.board size={14} />),
          page("epics", "Эпики", "/epics", <EpicIcon size={14} />, { current: pathname.startsWith("/epic") }),
        ],
      },
    },
    // A plain link: the project chat has no sibling pages, so its tab carries no menu chevron.
    {
      id: "chat",
      label: (
        <>
          <Icon.chat size={16} />
          {label("Чат")}
        </>
      ),
      to: "/chat",
      title: "Чат",
    },
    {
      id: "knowledge",
      // Open drafts and unreviewed publications belong to the documentation: the count rides on its
      // entry and on the Changes feed.
      label: (
        <>
          <Icon.file size={16} />
          {label("Знания")}
          {count(attention, "изменений к документации")}
        </>
      ),
      to: "/docs",
      title: "Знания",
      menu: {
        label: "Разделы знаний",
        items: [
          page("docs", "Документация", "/docs", <Icon.file size={14} />, { current: pathname.startsWith("/docs") && !pathname.startsWith("/docs/changes") }),
          page("changes", "Изменения", "/docs/changes", <Icon.branch size={14} />, { count: attention > 0 && <Tag tone="solid-signal">{attention}</Tag> }),
        ],
      },
    },
    {
      id: "manage",
      label: (
        <>
          <Icon.gear size={16} />
          {label("Управление")}
        </>
      ),
      to: hubHome(viewer),
      title: "Управление",
      menu: {
        label: "Разделы управления",
        items: hub.map((group) => ({
          id: group.id,
          label: group.label,
          hint: group.hint,
          to: group.items[0].to,
          icon: group.id === "settings" ? <Icon.gear size={14} /> : group.id === "agents" ? <Icon.spark size={14} /> : <Icon.server size={14} />,
          current: group.items.some((item) => pathname === item.to || pathname.startsWith(`${item.to}/`)),
        })),
      },
    },
  ];
  const current = activePrimary(pathname) ?? ("none" as Primary);

  const toggleTheme = () => saveAppearancePreferences({ theme: document.documentElement.dataset.theme === "paper" ? "ink" : "paper" });
  const profile: MenuEntry[] = [
    { id: "palette", label: "Быстрые команды", hint: "⌘K", onSelect: onOpenPalette },
    { id: "settings", label: "Настройки", hint: "G S", onSelect: () => navigate("/settings") },
    ...(session?.mode === "users" ? (["separator", { id: "logout", label: `Выйти (${session.user.login})`, onSelect: () => void logout() }] satisfies MenuEntry[]) : []),
  ];

  return (
    <>
      <header className={`g-topbar${searchOpen ? " searching" : ""}`}>
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
                  label: <><ProjectIcon name={p.name} src={p.icon} /><span className="g-topbar-project-name">{p.name}</span></>,
                  checked: p.slug === session?.project,
                  onSelect: () => void switchProject(p.slug).catch((error: Error) => toast(error.message, "error")),
                  action: {
                    label: `Управление проектом ${p.name}`,
                    icon: <Icon.gear size={16} />,
                    disabled: !canManageProject(isAdmin, p.role),
                    onSelect: () => void manageProject(p.slug),
                  },
                }))}
              />
            )}
          </Popover>
        </div>

        <div className="g-topbar-pill" ref={pill}>
          <nav className="g-topbar-nav" aria-label="Навигация">
            <Tabs animated label="Разделы" items={primary} value={current} />
          </nav>

          {searchOpen && <div className="g-topbar-search-mode">
            <ContextSearch inputRef={searchRef} className="g-topbar-search" onClose={closeSearch} />
            <IconButton label="Закрыть поиск" onClick={closeSearch}><Icon.close size={16} /></IconButton>
          </div>}
          <IconButton label="Открыть поиск" ref={searchToggle} className="g-topbar-search-toggle" onClick={onOpenSearch}>
            <Icon.search size={20} />
          </IconButton>
          {canCreate && (
            <IconButton label="Новая задача" className="g-topbar-create" onClick={onNew}>
              <Icon.plus size={20} />
            </IconButton>
          )}

          <div className="g-topbar-utility">
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
    </>
  );
}
