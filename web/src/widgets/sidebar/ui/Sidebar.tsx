import { NavLink, useLocation } from "react-router";
import { useAgentConfig } from "@/entities/agent-config";
import { BookIcon, useDocsTree } from "@/entities/doc";
import { initials, useMeta } from "@/entities/project";
import { useLogout, useSession, useSwitchProject } from "@/entities/session";
import { useNotifications, useProposals } from "@/entities/platform";
import { EpicIcon, inTaskViews, OPEN, useTasks } from "@/entities/task";
import { useTeams } from "@/entities/team";
import { timeAgo, useTick } from "@/shared/lib";
import { Icon } from "@/shared/ui";

export function Sidebar({ onNew, online }: { onNew: () => void; online: boolean }) {
  useTick();
  const meta = useMeta().data;
  const session = useSession().data;
  const switchProject = useSwitchProject();
  const logout = useLogout();
  const unread = useNotifications().data?.unread ?? 0;
  const proposals = useProposals().data?.length ?? 0;
  const teams = useTeams().data?.filter((t) => t.state === "active") ?? [];
  const { pathname } = useLocation();
  const tasks = useTasks().data;
  const docs = useDocsTree().data;
  const docsCount = docs?.pages.length ?? 0;
  const agents = useAgentConfig();
  // The list's badges: tasks waiting for a decision (when any) and all open ones.
  const listed = tasks?.filter(inTaskViews);
  const open = listed ? listed.filter((t) => OPEN.includes(t.status)).length : OPEN.reduce((n, st) => n + (meta?.counts[st] ?? 0), 0);
  const waiting = listed ? listed.filter((t) => t.status === "needs_owner").length : (meta?.counts.needs_owner ?? 0);
  const openEpics = tasks?.filter((t) => t.type === "epic" && t.status !== "done" && t.status !== "cancelled").length ?? 0;

  const project = session?.projects.find((p) => p.slug === session.project);
  const many = (session?.projects.length ?? 0) > 1;
  const me = session?.mode === "users" ? session.user.name || session.user.login : "Локальный режим";

  return (
    <nav className="sidebar" aria-label="Навигация">
      <div className={`project${many ? " switch" : ""}`} title={project ? (project.hasRepo ? "Проект с репозиторием кода" : "Проект без кода") : undefined}>
        <span className="logo-mark">
          <Icon.mark size={15} />
        </span>
        <span className="txt">
          <span className="name">{project?.name ?? meta?.project ?? "genie"}</span>
          <span className="sub">{project ? `${project.slug} · ${ROLE_NAME[project.role] ?? project.role}` : "genie"}</span>
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

      <button type="button" className="new-task" onClick={onNew}>
        <Icon.plus size={14} />
        <span className="grow" style={{ textAlign: "left" }}>
          Новая задача
        </span>
        <kbd>C</kbd>
      </button>

      <div className="nav-group">
        <NavLink to="/board" className={({ isActive }) => `nav-item${isActive ? " on" : ""}`}>
          <Icon.board size={15} />
          <span className="grow">Доска</span>
        </NavLink>
        <NavLink to="/tasks" className={({ isActive }) => `nav-item${isActive ? " on" : ""}`}>
          <Icon.list size={15} />
          <span className="grow">Задачи</span>
          {waiting > 0 && (
            <span className="count alert" title="ждут вашего решения">
              {waiting}
            </span>
          )}
          <span className="count">{open || ""}</span>
        </NavLink>
        <NavLink to="/epics" className={({ isActive }) => `nav-item${isActive || pathname.startsWith("/epic/") ? " on" : ""}`}>
          <EpicIcon size={15} />
          <span className="grow">Эпики</span>
          <span className="count">{openEpics || ""}</span>
        </NavLink>
      </div>

      <div className="nav-group">
        <span className="nav-label">Знания</span>
        <NavLink
          to="/docs"
          className={({ isActive }) => `nav-item${(isActive || pathname.startsWith("/docs")) && !pathname.startsWith("/docs/proposals") ? " on" : ""}`}
        >
          <BookIcon size={15} />
          <span className="grow">Документация</span>
          <span className="count">{docs ? docsCount : ""}</span>
        </NavLink>
        <NavLink to="/docs/proposals" className={({ isActive }) => `nav-item${isActive ? " on" : ""}`}>
          <Icon.proposal size={15} />
          <span className="grow">Предложения</span>
          <span className="count">{proposals || ""}</span>
        </NavLink>
      </div>

      <div className="nav-group">
        <span className="nav-label">Команда и правила</span>
        {agents.isSuccess && (
          <NavLink to="/agents" className={({ isActive }) => `nav-item${isActive ? " on" : ""}`}>
            <Icon.userPlus size={15} />
            <span className="grow">Агенты</span>
          </NavLink>
        )}
        <NavLink to="/automations" className={({ isActive }) => `nav-item${isActive ? " on" : ""}`}>
          <Icon.bolt size={15} />
          <span className="grow">Автоматизации</span>
        </NavLink>
        <NavLink to="/notifications" className={({ isActive }) => `nav-item${isActive ? " on" : ""}`}>
          <Icon.bell size={15} />
          <span className="grow">Уведомления</span>
          {unread > 0 && <span className="unread-dot" role="img" aria-label={`непрочитанных: ${unread}`} />}
        </NavLink>
        <NavLink to="/project" className={({ isActive }) => `nav-item${isActive || pathname.startsWith("/project/") ? " on" : ""}`}>
          <Icon.folder size={15} />
          <span className="grow">Проект и люди</span>
        </NavLink>
        {session?.user.isAdmin && (
          <NavLink to="/server" className={({ isActive }) => `nav-item${isActive ? " on" : ""}`}>
            <Icon.server size={15} />
            <span className="grow">Сервер</span>
          </NavLink>
        )}
      </div>

      {teams.length > 0 && (
        <div className="nav-group">
          <span className="nav-label">Работают сейчас</span>
          {teams.map((t) => {
            const status = t.taskInfo?.status;
            const working = t.members.some((m) => m.activity === "working");
            const waiting = status === "needs_owner";
            return (
              <NavLink key={t.id} to={`/team/${encodeURIComponent(t.id)}`} className={({ isActive }) => `team-link${isActive ? " on" : ""}`}>
                {waiting ? <span className="dot-amber" /> : working ? <span className="spin" /> : <span className="dot-idle" />}
                <span className="mono">{t.id}</span>
                <span className={`st${waiting ? " amber" : ""}`}>{waiting ? "ждёт вас" : status ? statusShort(status) : ""}</span>
                <span className="when">{timeAgo(t.created)}</span>
              </NavLink>
            );
          })}
        </div>
      )}

      <div className="side-foot">
        {session?.mode === "users" && session.user.avatar ? (
          <span className="me photo" aria-hidden="true">
            <img src={session.user.avatar} alt="" />
          </span>
        ) : (
          <span className="me" aria-hidden="true">
            {initials(me)}
          </span>
        )}
        <span className="who">
          <span className="nm">{me}</span>
          <span className={`live${online ? "" : " off"}`} title={meta?.tailnet ?? location.host}>
            <i />
            {online ? "онлайн" : "нет связи"}
          </span>
        </span>
        <NavLink to="/profile" className="icon-btn" aria-label="Профиль и каналы" title="Профиль и каналы">
          <Icon.gear size={14} />
        </NavLink>
        {session?.mode === "users" && (
          <button type="button" className="icon-btn" aria-label="Выйти" title={`Выйти (${session.user.login})`} onClick={() => void logout()}>
            <Icon.logout size={14} />
          </button>
        )}
      </div>
    </nav>
  );
}

const ROLE_NAME: Record<string, string> = { owner: "владелец", admin: "админ", member: "участник", viewer: "только чтение" };

function statusShort(s: string): string {
  return ({ inbox: "входящие", draft: "черновик", refining: "уточнение", ready: "готово", in_progress: "в работе", changes_requested: "доработка", review: "ревью", approved: "одобрено", done: "принято" } as Record<string, string>)[s] ?? s;
}
