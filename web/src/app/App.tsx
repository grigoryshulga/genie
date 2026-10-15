import { Suspense, useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { createBrowserRouter, Navigate, Outlet, useLocation, useNavigate, useOutletContext, useParams, useSearchParams } from "react-router";
import { LEGACY_VIEWS, type PresetId, PRESETS, useTasks } from "@/entities/task";
import { useTeamMap } from "@/entities/team";
import type { PaletteActions } from "@/features/command-palette";
import type { NewTaskPreset } from "@/features/create-task";
import { InvitePage, LoginPage } from "@/pages/auth";
import { BoardPage } from "@/pages/board";
import { useSession } from "@/entities/session";
import { OnlineProvider, useLiveUpdates, useOnline } from "@/shared/api";
import { RequireServerAdmin, SettingsShell } from "@/widgets/settings-shell";
import { isTyping, rememberWorkRoute } from "@/shared/lib";
import { Sidebar } from "@/widgets/sidebar";
import { lazyPage } from "./lazyPage.ts";

// Only the entry screen is eager: the gate (its login page and the shell) and the board,
// which is what `/` redirects to. Every other page is a chunk fetched when its route is
// first shown; the three dialogs/panels the shell renders on demand are chunks too.
const TasksPage = lazyPage(() => import("@/pages/tasks").then((m) => m.TasksPage));
const TaskPage = lazyPage(() => import("@/pages/task").then((m) => m.TaskPage));
const TeamView = lazyPage(() => import("@/pages/team").then((m) => m.TeamView));
const AgentChat = lazyPage(() => import("@/pages/agent").then((m) => m.AgentChat));
const EpicsPage = lazyPage(() => import("@/pages/epics").then((m) => m.EpicsPage));
const EpicPage = lazyPage(() => import("@/pages/epic").then((m) => m.EpicPage));
const DocsPage = lazyPage(() => import("@/pages/docs").then((m) => m.DocsPage));
const AgentsPage = lazyPage(() => import("@/pages/agents").then((m) => m.AgentsPage));
const MeSettingsPage = lazyPage(() => import("@/pages/settings").then((m) => m.MeSettingsPage));
const ProjectSettingsPage = lazyPage(() => import("@/pages/settings").then((m) => m.ProjectSettingsPage));
const ServerSettingsPage = lazyPage(() => import("@/pages/settings").then((m) => m.ServerSettingsPage));
const FirstProjectPage = lazyPage(() => import("@/pages/settings").then((m) => m.FirstProjectPage));
const AnswerPage = lazyPage(() => import("@/pages/platform").then((m) => m.AnswerPage));
const AutomationsPage = lazyPage(() => import("@/pages/platform").then((m) => m.AutomationsPage));
const NotificationsPage = lazyPage(() => import("@/pages/platform").then((m) => m.NotificationsPage));
const ProposalsPage = lazyPage(() => import("@/pages/platform").then((m) => m.ProposalsPage));
const TaskDetail = lazyPage(() => import("@/widgets/task-detail").then((m) => m.TaskDetail));
const CommandPalette = lazyPage(() => import("@/features/command-palette").then((m) => m.CommandPalette));
const NewTaskDialog = lazyPage(() => import("@/features/create-task").then((m) => m.NewTaskDialog));

/** What a page shows while its chunk loads; the same «Загрузка…» row the pages use. */
function PageFallback() {
  return <div className="empty">Загрузка…</div>;
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
  const teams = useTeamMap();
  const tasks = useTasks().data;
  const teamRoute = location.pathname.startsWith("/team/");
  const taskRoute = location.pathname.startsWith("/task/");
  const docsRoute = location.pathname.startsWith("/docs");
  const platformRoute = ["/automations", "/agents", "/notifications"].some((p) => location.pathname.startsWith(p));
  // Pages without the task list or board: opening a task from the palette goes to the list.
  const ownPage = teamRoute || taskRoute || location.pathname.startsWith("/epic") || docsRoute || platformRoute;
  const openTaskId = teamRoute || taskRoute ? undefined : (sp.get("task") ?? undefined);
  const openTask = tasks?.find((t) => t.id === openTaskId);

  // The settings mode's «← к работе» returns here (PD21).
  useEffect(() => {
    if (!location.pathname.startsWith("/settings")) rememberWorkRoute(location.pathname, location.search);
  }, [location]);


  const closeTask = useCallback(() => {
    const next = new URLSearchParams(sp);
    next.delete("task");
    setSp(next);
  }, [sp, setSp]);

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
      if (dialog) return;
      if (e.key === "Escape" && openTaskId && !isTyping(e)) {
        closeTask();
        return;
      }
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
        searchRef.current?.focus();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [dialog, openTaskId, closeTask, actions, navigate, newTask]);

  const agentRoute = /^\/team\/[^/]+\/[^/]+/.test(location.pathname);
  const cls = agentRoute ? "app team-view agent-view" : teamRoute ? "app team-view" : openTaskId ? "app with-detail" : "app";
  return (
    <div className={cls}>
      <Sidebar onNew={() => newTask()} online={online} />
      <Suspense fallback={<PageFallback />}>
        <Outlet context={{ onNew: newTask, searchRef }} />
      </Suspense>
      {openTaskId && (
        <Suspense fallback={null}>
          <TaskDetail key={openTaskId} id={openTaskId} team={openTask?.team ? teams.get(openTask.team) : undefined} onClose={closeTask} />
        </Suspense>
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

type ShellContext = { onNew: (preset?: NewTaskPreset) => void; searchRef: React.RefObject<HTMLInputElement | null> };

function TasksRoute() {
  const ctx = useOutletContext<ShellContext>();
  return <TasksPage onNew={ctx.onNew} searchRef={ctx.searchRef} />;
}

function BoardRoute() {
  const ctx = useOutletContext<ShellContext>();
  return <BoardPage onNew={ctx.onNew} searchRef={ctx.searchRef} />;
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
  return <EpicsPage onNew={useOutletContext<ShellContext>().onNew} />;
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

/** `/profile/…` — the personal settings before the settings mode (PD21); old notifications still carry these links. */
function LegacyProfile() {
  const { tab } = useParams();
  const section = tab === "security" || tab === "telegram" || tab === "keys" || tab === "tokens" ? tab : "account";
  return <Navigate to={`/settings/me/${section}`} replace />;
}

/** `/project/…` — the project settings before the settings mode (PD21); old links keep working. */
function LegacyProject() {
  const { tab } = useParams();
  const section = tab === "agents" || tab === "repos" || tab === "people" ? tab : "general";
  return <Navigate to={`/settings/project/${section}`} replace />;
}

/** `/server/…` — the server administration before the settings mode (PD21); old links keep working. */
function LegacyServer() {
  const { tab } = useParams();
  const section = tab === "stats" || tab === "projects" || tab === "accounts" || tab === "mcp" || tab === "vault" ? tab : "health";
  return <Navigate to={`/settings/server/${section}`} replace />;
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
          { path: "docs", element: <DocsPage /> },
          { path: "docs/edit", element: <DocsPage /> },
          { path: "docs/proposals", element: <ProposalsPage /> },
          { path: "automations", element: <AutomationsPage /> },
          { path: "agents", element: <AgentsPage /> },
          { path: "notifications", element: <NotificationsPage /> },
          { path: ":view", element: <LegacyView /> },
        ],
      },
      // The settings mode (PD21): its own layout; «Личное» first, the project and server sections follow.
      {
        path: "settings",
        element: <SettingsShell />,
        children: [
          { index: true, element: <Navigate to="/settings/me/account" replace /> },
          { path: "me/:section?", element: <MeSettingsPage /> },
          { path: "project/:section?", element: <ProjectSettingsPage /> },
          { path: "server/:section?", element: <RequireServerAdmin><ServerSettingsPage /></RequireServerAdmin> },
        ],
      },
      { path: "profile/:tab?", element: <LegacyProfile /> },
      { path: "project/:tab?", element: <LegacyProject /> },
      { path: "server/:tab?", element: <LegacyServer /> },
    ],
  },
]);
