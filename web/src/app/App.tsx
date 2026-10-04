import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createBrowserRouter, Navigate, Outlet, useLocation, useNavigate, useOutletContext, useSearchParams } from "react-router";
import { useTasks, type ViewId } from "@/entities/task";
import { useTeamMap } from "@/entities/team";
import { CommandPalette, type PaletteActions } from "@/features/command-palette";
import { NewTaskDialog, type NewTaskPreset } from "@/features/create-task";
import { AgentsPage } from "@/pages/agents";
import { EpicPage } from "@/pages/epic";
import { EpicsPage } from "@/pages/epics";
import { DocsPage } from "@/pages/docs";
import { InvitePage, LoginPage } from "@/pages/auth";
import { FirstProjectPage, ProjectPage, ServerPage } from "@/pages/project";
import { AnswerPage, AutomationsPage, NotificationsPage, ProposalsPage } from "@/pages/platform";
import { ProfilePage } from "@/pages/profile";
import { useSession } from "@/entities/session";
import { TaskPage } from "@/pages/task";
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
  const taskRoute = location.pathname.startsWith("/task/");
  const docsRoute = location.pathname.startsWith("/docs");
  const platformRoute = ["/automations", "/agents", "/notifications", "/profile", "/project", "/server"].some((p) => location.pathname.startsWith(p));
  // Pages without a task list: palette actions that need one go to "active".
  const ownPage = teamRoute || taskRoute || location.pathname.startsWith("/epic") || docsRoute || platformRoute;
  const openTaskId = teamRoute || taskRoute ? undefined : (sp.get("task") ?? undefined);
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
      go: (v: ViewId) => navigate({ pathname: `/${v}`, search: sp.get("layout") ? `?layout=${sp.get("layout")}` : "" }),
      layout: (l) => navigate({ pathname: ownPage ? "/active" : location.pathname, search: `?layout=${l}` }),
      openTask: (id) => navigate({ pathname: ownPage ? "/active" : location.pathname, search: `?${new URLSearchParams({ ...(sp.get("layout") ? { layout: sp.get("layout")! } : {}), task: id })}` }),
      openTeam: (id) => navigate(`/team/${encodeURIComponent(id)}`),
      openDoc: (path) => navigate(`/docs?page=${encodeURIComponent(path)}`),
      openDocs: () => navigate("/docs"),
      newDoc: (kind, title) => navigate(`/docs?new=${kind}${title ? `&title=${encodeURIComponent(title)}` : ""}`),
      searchDocs: (q) => navigate(`/docs?q=${encodeURIComponent(q)}`),
    }),
    [navigate, sp, location.pathname, ownPage, newTask],
  );

  // Global shortcuts (Linear-style): C, /, ⌘K, Esc, G then M/I/D/A/P/C/E
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
        const map: Record<string, ViewId> = { m: "mine", i: "inbox", d: "decisions", a: "active", p: "prep", c: "done" };
        gPending.current = 0;
        if (e.key === "e") {
          navigate("/epics");
          e.preventDefault();
          return;
        }
        if (map[e.key]) {
          actions.go(map[e.key]);
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
            else navigate(`/inbox?task=${encodeURIComponent(id)}`);
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
      { index: true, element: <Navigate to="/active" replace /> },
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
      { path: "profile/:tab?", element: <ProfilePage /> },
      { path: "project/:tab?", element: <ProjectPage /> },
      { path: "server/:tab?", element: <ServerPage /> },
      { path: ":view", element: <TasksRoute /> },
    ],
  },
]);
