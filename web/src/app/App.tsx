import { Suspense, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createBrowserRouter, Navigate, Outlet, useLocation, useNavigate, useOutletContext, useParams, useSearchParams } from "react-router";
import { LEGACY_VIEWS, type PresetId, PRESETS, useTasks } from "@/entities/task";
import { useTeamMap } from "@/entities/team";
import type { PaletteActions } from "@/features/command-palette";
import type { NewTaskPreset } from "@/features/create-task";
import { InvitePage, LoginPage } from "@/pages/auth";
import { BoardPage } from "@/pages/board";
import { useSession } from "@/entities/session";
import { OnlineProvider, useLiveUpdates, useOnline } from "@/shared/api";
import { canManageProject, hubHome, managementDestination, isTyping, rememberWorkRoute } from "@/shared/lib";
import { Dialog, LoadingState, Skeleton } from "@/shared/ui";
import { TopBar } from "@/widgets/topbar";
import { lazyPage } from "./lazyPage.ts";
import { useTaskModalMotion } from "./useTaskModalMotion.ts";
import "./task-modal.css";
// Legacy cross-layer compatibility layer (`#root` height plus old class names other sheets still
// target). It stays with the app shell until the H33 cleanup retires those rules.
import "./styles/additions.css";
// The markdown surface's prose: the shell loads it here, after the avatar and shell primitives it has
// to override (`.md` also matches the medium avatar size), and before the phone layer, which is the
// order the central sheet used to give it.
import "@/shared/ui/markdown.css";
// The app-wide responsive layer and the desktop centring column: the shell, the eager board screen
// and every page the shell renders itself. Lazy screens re-inline them in their own chunk (see
// `<screen>-sheets.css`) so that they still load after that screen's area sheets.
import "@/shared/ui/mobile.css";
// Shared document layout rules load after the responsive layer.
import "@/shared/ui/search-and-docs.css";
import "@/shared/ui/content.css";

// Only the entry screen is eager: the gate (its login page and the shell) and the board,
// which is what `/` redirects to. Every other page is a chunk fetched when its route is
// first shown; the three dialogs/panels the shell renders on demand are chunks too.
const TasksPage = lazyPage(() => import("@/pages/tasks").then((m) => m.TasksPage));
const TaskPage = lazyPage(() => import("@/pages/task").then((m) => m.TaskPage));
const TeamView = lazyPage(() => import("@/pages/team").then((m) => m.TeamView));
const AgentChat = lazyPage(() => import("@/pages/agent").then((m) => m.AgentChat));
const ProjectChat = lazyPage(() => import("@/pages/chat").then((m) => m.ProjectChat));
const EpicsPage = lazyPage(() => import("@/pages/epics").then((m) => m.EpicsPage));
const EpicPage = lazyPage(() => import("@/pages/epic").then((m) => m.EpicPage));
const DocsPage = lazyPage(() => import("@/pages/docs").then((m) => m.DocsPage));
const AgentsPage = lazyPage(() => import("@/pages/agents").then((m) => m.AgentsPage));
const ManageShell = lazyPage(() => import("@/widgets/settings-shell").then((m) => m.ManageShell));
const HubTabsSection = lazyPage(() => import("@/widgets/settings-shell").then((m) => m.HubTabsSection));
const MeSettingsPage = lazyPage(() => import("@/pages/settings").then((m) => m.MeSettingsPage));
const AutomationsSettingsPage = lazyPage(() => import("@/pages/settings").then((m) => m.AutomationsSettingsPage));
const ProjectSettingsPage = lazyPage(() => import("@/pages/settings").then((m) => m.ProjectSettingsPage));
const ServerSettingsPage = lazyPage(() => import("@/pages/settings").then((m) => m.ServerSettingsPage));
const ServerStatisticsPage = lazyPage(() => import("@/pages/settings").then((m) => m.ServerStatisticsPage));
const SettingsShell = lazyPage(() => import("@/widgets/settings-shell").then((m) => m.SettingsShell));
const FirstProjectPage = lazyPage(() => import("@/pages/settings").then((m) => m.FirstProjectPage));
const AnswerPage = lazyPage(() => import("@/pages/platform").then((m) => m.AnswerPage));
const AutomationsPage = lazyPage(() => import("@/pages/platform").then((m) => m.AutomationsPage));
const NotificationsPage = lazyPage(() => import("@/pages/platform").then((m) => m.NotificationsPage));
const ProposalsPage = lazyPage(() => import("@/pages/platform").then((m) => m.ProposalsPage));
const ObservationsPage = lazyPage(() => import("@/pages/observations").then((m) => m.ObservationsPage));
const ObservationPage = lazyPage(() => import("@/pages/observations").then((m) => m.ObservationPage));
const TaskDetail = lazyPage(() => import("@/widgets/task-detail").then((m) => m.TaskDetail));
const CommandPalette = lazyPage(() => import("@/features/command-palette").then((m) => m.CommandPalette));
const NewTaskDialog = lazyPage(() => import("@/features/create-task").then((m) => m.NewTaskDialog));

/** What a page shows while its chunk loads: the shape of a page header and its first lines. */
function PageFallback() {
  return (
    <div className="empty">
      <LoadingState label="Загрузка страницы" width={420}>
        <Skeleton width={200} height={22} />
        <Skeleton height={14} />
        <Skeleton height={14} width="72%" />
      </LoadingState>
    </div>
  );
}

function Shell() {
  const online = useOnline();
  const navigate = useNavigate();
  const location = useLocation();
  const [sp, setSp] = useSearchParams();
  const [dialog, setDialog] = useState<"new" | "palette" | undefined>();
  const [preset, setPreset] = useState<NewTaskPreset | undefined>();
  const newTask = useCallback((p?: NewTaskPreset) => {
    setPreset(p);
    setDialog("new");
  }, []);
  const searchRef = useRef<HTMLInputElement>(null);
  const [searchOpen, setSearchOpen] = useState(false);
  const focusSearch = useCallback(() => { setSearchOpen(true); searchRef.current?.focus(); }, []);
  const teams = useTeamMap();
  const tasks = useTasks().data;
  const teamRoute = location.pathname.startsWith("/team/");
  const taskRoute = location.pathname.startsWith("/task/");
  const docsRoute = location.pathname.startsWith("/docs");
  const platformRoute = ["/manage", "/notifications"].some((p) => location.pathname.startsWith(p));
  const chatRoute = location.pathname.startsWith("/chat");
  // Pages without the task list or board: opening a task from the palette goes to the list.
  const ownPage = teamRoute || taskRoute || location.pathname.startsWith("/epic") || docsRoute || platformRoute || chatRoute;
  const openTaskId = teamRoute || taskRoute ? undefined : (sp.get("task") ?? undefined);
  const openTask = tasks?.find((t) => t.id === openTaskId);
  const taskModalOpen = !!openTaskId;

  // Capture the opener before Dialog.showModal() moves focus. Restore after React removes the
  // dialog, unless navigation removed the opener or another modal has taken over.
  useLayoutEffect(() => {
    if (!taskModalOpen) return;
    const opener = document.activeElement;
    return () => {
      requestAnimationFrame(() => {
        if (opener instanceof HTMLElement && opener.isConnected && !document.querySelector("dialog:modal")) opener.focus();
      });
    };
  }, [taskModalOpen]);

  // The settings mode's «← к работе» returns here (PD21).
  useEffect(() => {
    if (!location.pathname.startsWith("/settings")) rememberWorkRoute(location.pathname, location.search);
  }, [location]);


  const closeTask = useCallback(() => {
    const next = new URLSearchParams(sp);
    next.delete("task");
    setSp(next);
  }, [sp, setSp]);
  const requestCloseTask = useTaskModalMotion(openTaskId, location.pathname === "/board", closeTask);

  const actions: PaletteActions = useMemo(
    () => ({
      newTask: () => newTask(),
      newIdea: () => newTask({ idea: true }),
      go: (where: "board" | "tasks" | "epics" | "settings") => navigate(`/${where}`),
      preset: (id: PresetId, mine?: boolean) => navigate(presetPath(id, mine)),
      openTask: (id) => {
        const next = new URLSearchParams(ownPage ? undefined : sp);
        next.set("task", id);
        navigate({ pathname: ownPage ? "/tasks" : location.pathname, search: `?${next}` });
      },
      openTeam: (id) => navigate(`/team/${encodeURIComponent(id)}`),
      openDoc: (path) => navigate(`/docs?page=${encodeURIComponent(path)}`),
      openDocs: () => navigate("/docs"),
      newDoc: (kind, title) => navigate(`/docs?new=${kind}${title ? `&title=${encodeURIComponent(title)}` : ""}`),
      searchDocs: (q) => navigate(`/docs?q=${encodeURIComponent(q)}`),
    }),
    [navigate, sp, location.pathname, ownPage, newTask],
  );

  // Global shortcuts (Linear-style): C, /, ⌘K, Esc, G then B/T/E (sections) or M/I/D/A/P/C (task list presets)
  const gPending = useRef(0);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setDialog("palette");
        return;
      }
      // Native dialogs own Escape and focus; background shortcuts must not act on the covered page.
      if (dialog || openTaskId) return;
      if (isTyping(e)) return;
      if (Date.now() - gPending.current < 1200) {
        const sections: Record<string, "board" | "tasks" | "epics" | "settings"> = { b: "board", t: "tasks", e: "epics", s: "settings" };
        const presets: Record<string, PresetId> = { m: "open", i: "inbox", d: "decisions", a: "working", p: "prep", c: "done" };
        gPending.current = 0;
        if (sections[e.key]) {
          actions.go(sections[e.key]);
          e.preventDefault();
          return;
        }
        if (presets[e.key]) {
          actions.preset(presets[e.key], e.key === "m");
          e.preventDefault();
          return;
        }
      }
      if (e.key === "g") gPending.current = Date.now();
      else if (e.key === "c") {
        e.preventDefault();
        newTask();
      } else if (e.key === "/") {
        e.preventDefault();
        focusSearch();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dialog, openTaskId, closeTask, actions, navigate, newTask, focusSearch]);

  const agentRoute = /^\/team\/[^/]+\/[^/]+/.test(location.pathname);
  const cls = agentRoute ? "app team-view agent-view" : teamRoute ? "app team-view" : "app";
  return (
    <div className={cls}>
      <TopBar onNew={() => newTask()} online={online} searchRef={searchRef} searchOpen={searchOpen} onOpenSearch={focusSearch} onCloseSearch={() => setSearchOpen(false)} onOpenPalette={() => setDialog("palette")} />
      <Suspense fallback={<PageFallback />}>
        <Outlet context={{ onNew: newTask }} />
      </Suspense>
      {openTaskId && (
        <Dialog bare className="task-modal" title={`Задача ${openTaskId}`} onClose={requestCloseTask}>
          <Suspense fallback={<PageFallback />}>
            <TaskDetail key={openTaskId} id={openTaskId} team={openTask?.team ? teams.get(openTask.team) : undefined} onClose={requestCloseTask} />
          </Suspense>
        </Dialog>
      )}
      {dialog === "new" && (
        <Suspense fallback={null}>
          <NewTaskDialog
            preset={preset}
            onClose={() => setDialog(undefined)}
            onIdea={(team, member) => {
              setDialog(undefined);
              navigate(`/team/${encodeURIComponent(team)}/${encodeURIComponent(member)}`);
            }}
            onCreated={(id, type) => {
              setDialog(undefined);
              if (type === "epic") navigate(`/epic/${encodeURIComponent(id)}`);
              else if (preset?.epic) navigate(`/epic/${encodeURIComponent(preset.epic)}`);
              else navigate(`/tasks?task=${encodeURIComponent(id)}`);
            }}
          />
        </Suspense>
      )}
      {dialog === "palette" && (
        <Suspense fallback={null}>
          <CommandPalette onClose={() => setDialog(undefined)} actions={actions} />
        </Suspense>
      )}
    </div>
  );
}

type ShellContext = { onNew: (preset?: NewTaskPreset) => void };

function TasksRoute() {
  return <TasksPage />;
}

function BoardRoute() {
  return <BoardPage />;
}

/** The task list with a preset's statuses, "mine" narrowing it to the viewer. */
function presetPath(id: PresetId, mine?: boolean): string {
  const sp = new URLSearchParams();
  if (id !== "open") sp.set("status", PRESETS.find((p) => p.id === id)!.statuses.join(","));
  if (mine) sp.set("who", "me");
  return `/tasks${sp.size ? `?${sp}` : ""}`;
}

/** Old addresses (`/active`, `/decisions?task=…`, `?layout=board`) lead to the board or the task list with the same filter. */
function LegacyView() {
  const view = LEGACY_VIEWS[useParams().view ?? ""];
  const [sp] = useSearchParams();
  const next = new URLSearchParams();
  for (const k of ["task", "epic"]) if (sp.get(k)) next.set(k, sp.get(k)!);
  if (!view || sp.get("layout") === "board") return <Navigate to={{ pathname: "/board", search: next.size ? `?${next}` : "" }} replace />;
  if (view.statuses) next.set("status", view.statuses.join(","));
  if (view.who) next.set("who", view.who);
  return <Navigate to={{ pathname: "/tasks", search: next.size ? `?${next}` : "" }} replace />;
}

function EpicsRoute() {
  const { onNew } = useOutletContext<ShellContext>();
  return <EpicsPage onNew={onNew} />;
}

function EpicRoute() {
  return <EpicPage onNew={useOutletContext<ShellContext>().onNew} />;
}

/** Login when needed; a friendly note when the user has no project yet. */
function Gate() {
  const session = useSession();
  const online = useLiveUpdates();
  if (session.isLoading) return <div className="auth-page" />;
  if (session.error) return <LoginPage note={`Сервер недоступен: ${session.error.message}`} />;
  if (!session.data) return <LoginPage />;
  if (!session.data.projects.length) {
    if (session.data.user.isAdmin)
      return (
        <Suspense fallback={<PageFallback />}>
          <FirstProjectPage />
        </Suspense>
      );
    return <LoginPage note="У вас пока нет доступа ни к одному проекту — попросите приглашение у администратора." />;
  }
  // One live-updates stream for both modes: the work shell and the settings one read the flag.
  return (
    <OnlineProvider value={online}>
      <Outlet />
    </OnlineProvider>
  );
}

/** The server section of the settings: server administrators only. A route-level policy, so it lives with the routes. */
function RequireServerAdmin({ children }: { children: ReactNode }) {
  const session = useSession();
  if (session.isLoading) return <div className="empty"><LoadingState label="Загрузка" lines={2} width={320} /></div>;
  if (!session.data?.user.isAdmin) return <main className="main"><div className="empty">Раздел доступен администраторам сервера.</div></main>;
  return children;
}

function RequireProjectAdmin({ children }: { children: ReactNode }) {
  const { data, isLoading } = useSession();
  if (isLoading) return <PageFallback />;
  const role = data?.projects.find((p) => p.slug === data.project)?.role;
  if (!canManageProject(!!data?.user.isAdmin, role))
    return <main className="main"><div className="empty">Раздел доступен администраторам проекта и сервера.</div></main>;
  return children;
}

function LegacyManagement({ scope, legacySection }: { scope: "project" | "server"; legacySection?: string }) {
  const { section: routeSection, caseId } = useParams();
  const section = legacySection ?? routeSection ?? (scope === "project" ? "general" : "projects");
  const { search, hash } = useLocation();
  const pathname = caseId ? `/manage/agents/automations/observation/${encodeURIComponent(caseId)}` : managementDestination(scope, section, new URLSearchParams(search).get("tab"));
  return <Navigate to={{ pathname, search, hash }} replace />;
}

/** `/agents?tab=…&id=…` — the agent catalog before it moved into the management hub. */
function LegacyAgents() {
  const [sp] = useSearchParams();
  const tab = sp.get("tab");
  const pathname = tab === "history" ? "/manage/server/history" : `/manage/agents/${tab === "templates" || tab === "skills" || tab === "mcp" ? tab : "roles"}`;
  // The selected item and its view (`id`, `view`) carry over; the tab is the path now.
  const next = new URLSearchParams(sp);
  next.delete("tab");
  return <Navigate to={{ pathname, search: next.size ? `?${next}` : "" }} replace />;
}

/** «Управление» opens on the first page this person may use. */
function ManageHome() {
  const { data } = useSession();
  if (!data) return <PageFallback />;
  const role = data.projects.find((p) => p.slug === data.project)?.role;
  return <Navigate to={hubHome({ serverAdmin: !!data.user.isAdmin, projectAdmin: canManageProject(!!data.user.isAdmin, role) })} replace />;
}

/** `/profile/…` — the personal settings before the settings mode (PD21); old notifications still carry these links. */
function LegacyProfile() {
  const { tab } = useParams();
  const section = tab === "appearance" || tab === "security" || tab === "telegram" || tab === "keys" || tab === "tokens" ? tab : "account";
  return <Navigate to={`/settings/me/${section}`} replace />;
}

/** `/project/…` — the project settings before the settings mode (PD21); old links keep working. */
function LegacyProject() {
  return <LegacyManagement scope="project" legacySection={useParams().tab ?? "general"} />;
}

/** Old server administration links retain the selected editor and query. */
function LegacyServer() {
  return <LegacyManagement scope="server" legacySection={useParams().tab ?? "health"} />;
}

/** Preserve bookmarks and notification links when operational tools move into management. */
function LegacyWorkTool({ path }: { path: string }) {
  const { caseId } = useParams();
  const { search, hash } = useLocation();
  return <Navigate to={{ pathname: `/manage/agents/automations/${path}${caseId ? `/${encodeURIComponent(caseId)}` : ""}`, search, hash }} replace />;
}

export const router = createBrowserRouter([
  { path: "/invite", element: <InvitePage /> },
  // `/answer` is outside the shell, so its page brings its own loading boundary.
  {
    path: "/answer",
    element: (
      <Suspense fallback={<PageFallback />}>
        <AnswerPage />
      </Suspense>
    ),
  },
  {
    path: "/",
    element: <Gate />,
    children: [
      { index: true, element: <Navigate to="/board" replace /> },
      // The work mode: the sidebar, the task drawer, the palette. Everything people use daily.
      {
        element: <Shell />,
        children: [
          { path: "board", element: <BoardRoute /> },
          { path: "tasks", element: <TasksRoute /> },
          { path: "task/:taskId/:tab?", element: <TaskPage /> },
          { path: "team/:teamId", element: <TeamView /> },
          { path: "team/:teamId/:member", element: <AgentChat /> },
          { path: "epics", element: <EpicsRoute /> },
          { path: "epic/:epicId", element: <EpicRoute /> },
          { path: "chat", element: <ProjectChat /> },
          { path: "docs", element: <DocsPage /> },
          { path: "docs/edit", element: <DocsPage /> },
          { path: "docs/proposals", element: <ProposalsPage /> },
          { path: "automations", element: <LegacyWorkTool path="runs" /> },
          { path: "observations", element: <LegacyWorkTool path="observations" /> },
          { path: "observation/:caseId", element: <LegacyWorkTool path="observation" /> },
          { path: "agents", element: <LegacyAgents /> },
          { path: "notifications", element: <NotificationsPage /> },
          { path: "statistics", element: <Navigate to="/manage/server/stats" replace /> },
          // «Управление»: the project hub, under the same top bar as the work pages (see shared/lib/management.ts).
          {
            path: "manage", element: <ManageShell />,
            children: [
              { index: true, element: <ManageHome /> },
              {
                path: "project",
                children: [
                  { index: true, element: <Navigate to="general" replace /> },
                  // Sections that moved to «Агенты» or «Администрирование» keep their old links.
                  ...["tools", "people", "automations", "automation-runs", "observations", "observation", "agents", "decisions", "telegram"].map((section) => ({ path: section, element: <LegacyManagement scope="project" legacySection={section} /> })),
                  { path: "observation/:caseId", element: <LegacyManagement scope="project" /> },
                  { path: ":section", element: <RequireProjectAdmin><ProjectSettingsPage /></RequireProjectAdmin> },
                ],
              },
              {
                path: "agents",
                children: [
                  { index: true, element: <Navigate to="roles" replace /> },
                  { path: "roles", element: <AgentsPage tab="roles" /> },
                  { path: "templates", element: <AgentsPage tab="templates" /> },
                  ...(["skills", "mcp"] as const).map((kind) => ({
                    path: kind, element: <HubTabsSection section={kind} label={kind === "mcp" ? "MCP-серверы" : "Навыки"} />,
                    children: [
                      { index: true, element: <AgentsPage tab={kind} /> },
                      { path: "project", element: <RequireProjectAdmin><ProjectSettingsPage section="tools" tab={kind} /></RequireProjectAdmin> },
                    ],
                  })),
                  {
                    path: "automations", element: <HubTabsSection section="automations" label="Разделы автоматизаций" />,
                    children: [
                      { index: true, element: <RequireProjectAdmin><AutomationsSettingsPage /></RequireProjectAdmin> },
                      { path: "runs", element: <AutomationsPage /> },
                      { path: "observations", element: <ObservationsPage /> },
                      { path: "observation/:caseId", element: <ObservationPage /> },
                      { path: "sources", element: <RequireProjectAdmin><ProjectSettingsPage section="observation" /></RequireProjectAdmin> },
                    ],
                  },
                  { path: "jev", element: <RequireProjectAdmin><ProjectSettingsPage section="tools" tab="jev" /></RequireProjectAdmin> },
                  { path: "work", element: <RequireProjectAdmin><ProjectSettingsPage section="work" /></RequireProjectAdmin> },
                ],
              },
              {
                path: "server", element: <RequireServerAdmin><Outlet /></RequireServerAdmin>,
                children: [
                  { index: true, element: <Navigate to="projects" replace /> },
                  ...["roles", "templates", "skills", "agents", "mcp"].map((section) => ({ path: section, element: <LegacyManagement scope="server" legacySection={section} /> })),
                  { path: "stats", element: <ServerStatisticsPage /> },
                  { path: "history", element: <AgentsPage tab="history" /> },
                  { path: ":section", element: <ServerSettingsPage /> },
                ],
              },
            ],
          },
          { path: ":view", element: <LegacyView /> },
        ],
      },
      {
        path: "settings", element: <SettingsShell />,
        children: [
          { index: true, element: <Navigate to="/settings/me/account" replace /> },
          { path: "me/:section?", element: <MeSettingsPage /> },
        ],
      },
      { path: "settings/stats", element: <Navigate to="/manage/server/stats" replace /> },
      { path: "settings/project/:section?", element: <LegacyManagement scope="project" /> },
      { path: "settings/server/:section?", element: <LegacyManagement scope="server" /> },
      { path: "profile/:tab?", element: <LegacyProfile /> },
      { path: "project/:tab?", element: <LegacyProject /> },
      { path: "server/:tab?", element: <LegacyServer /> },
    ],
  },
]);
