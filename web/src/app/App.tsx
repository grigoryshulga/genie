import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createBrowserRouter, Navigate, Outlet, useLocation, useNavigate, useOutletContext, useParams, useSearchParams } from "react-router";
import { LEGACY_VIEWS, type PresetId, PRESETS, useTasks } from "@/entities/task";
import { useTeamMap } from "@/entities/team";
import { CommandPalette, type PaletteActions } from "@/features/command-palette";
import { NewTaskDialog, type NewTaskPreset } from "@/features/create-task";
import { AgentsPage } from "@/pages/agents";
import { BoardPage } from "@/pages/board";
import { EpicPage } from "@/pages/epic";
import { EpicsPage } from "@/pages/epics";
import { DocsPage } from "@/pages/docs";
import { InvitePage, LoginPage } from "@/pages/auth";
import { FirstProjectPage, ProjectPage, ServerPage } from "@/pages/project";
import { AnswerPage, AutomationsPage, NotificationsPage, ProposalsPage } from "@/pages/platform";
import { ProfilePage } from "@/pages/profile";
import { useSession } from "@/entities/session";
import { TasksPage } from "@/pages/tasks";
import { TeamView } from "@/pages/team";
import { AgentChat } from "@/pages/agent";
import { useLiveUpdates } from "@/shared/api";
import { isTyping } from "@/shared/lib";
import { Sidebar } from "@/widgets/sidebar";
import { TaskDetail } from "@/widgets/task-detail";

function Shell() {
  const online = useLiveUpdates();
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
  const docsRoute = location.pathname.startsWith("/docs");
  const platformRoute = ["/automations", "/agents", "/notifications", "/profile", "/project", "/server"].some((p) => location.pathname.startsWith(p));
  // Pages without the task list or board: opening a task from the palette goes to the list.
  const ownPage = teamRoute || location.pathname.startsWith("/epic") || docsRoute || platformRoute;
  const openTaskId = teamRoute ? undefined : (sp.get("task") ?? undefined);
  const openTask = tasks?.find((t) => t.id === openTaskId);

  const closeTask = useCallback(() => {
    const next = new URLSearchParams(sp);
    next.delete("task");
    setSp(next);
  }, [sp, setSp]);

  const actions: PaletteActions = useMemo(
    () => ({
      newTask: () => newTask(),
      newIdea: () => newTask({ idea: true }),
      go: (where: "board" | "tasks" | "epics") => navigate(`/${where}`),
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
        const sections: Record<string, "board" | "tasks" | "epics"> = { b: "board", t: "tasks", e: "epics" };
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
      <Outlet context={{ onNew: newTask, searchRef }} />
      {openTaskId && <TaskDetail key={openTaskId} id={openTaskId} team={openTask?.team ? teams.get(openTask.team) : undefined} onClose={closeTask} />}
      {dialog === "new" && (
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
      )}
      {dialog === "palette" && <CommandPalette onClose={() => setDialog(undefined)} actions={actions} />}
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
  if (session.isLoading) return <div className="auth-page" />;
  if (session.error) return <LoginPage note={`Сервер недоступен: ${session.error.message}`} />;
  if (!session.data) return <LoginPage />;
  if (!session.data.projects.length) {
    if (session.data.user.isAdmin) return <FirstProjectPage />;
    return <LoginPage note="У вас пока нет доступа ни к одному проекту — попросите приглашение у администратора." />;
  }
  return <Shell />;
}

export const router = createBrowserRouter([
  { path: "/invite", element: <InvitePage /> },
  { path: "/answer", element: <AnswerPage /> },
  {
    path: "/",
    element: <Gate />,
    children: [
      { index: true, element: <Navigate to="/board" replace /> },
      { path: "board", element: <BoardRoute /> },
      { path: "tasks", element: <TasksRoute /> },
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
      { path: "profile/:tab?", element: <ProfilePage /> },
      { path: "project/:tab?", element: <ProjectPage /> },
      { path: "server/:tab?", element: <ServerPage /> },
      { path: ":view", element: <LegacyView /> },
    ],
  },
]);
