import { NavLink, useLocation, useNavigate } from "react-router";
import { inTaskViews, useTasks } from "@/entities/task";
import { type Team, useTeams } from "@/entities/team";
import { plural } from "@/shared/lib";

/** On phones the sidebar is hidden: its task sections and the working teams become a row of pills. */
export function MobileNav() {
  const navigate = useNavigate();
  const { pathname } = useLocation();
  const waiting = useTasks().data?.filter((t) => inTaskViews(t) && t.status === "needs_owner").length ?? 0;
  const teams = useTeams().data?.filter((t) => t.state === "active") ?? [];
  const cls = (on: boolean, amber = false) => [on ? "on" : "", amber ? "amber" : ""].filter(Boolean).join(" ");
  return (
    <div className="m-only m-nav" role="group" aria-label="Разделы">
      <NavLink to="/board" className={cls(pathname === "/board")}>
        Доска
      </NavLink>
      <NavLink to="/tasks" className={cls(pathname === "/tasks", waiting > 0)}>
        Задачи {waiting || ""}
      </NavLink>
      <NavLink to="/epics" className={cls(pathname === "/epics")}>
        Эпики
      </NavLink>
      {teams.map((t) => (
        <button key={t.id} type="button" onClick={() => navigate(`/team/${encodeURIComponent(t.id)}`)}>
          Команда {t.id}
        </button>
      ))}
    </div>
  );
}

export function TeamsSummary({ teams }: { teams: Map<string, Team> }) {
  const active = [...teams.values()].filter((t) => t.state === "active");
  const agents = active.reduce((n, t) => n + t.members.filter((m) => m.activity === "working").length, 0);
  if (!active.length) return <span>Нет активных команд</span>;
  return (
    <span>
      {active.length} {plural(active.length, "команда", "команды", "команд")} · {agents} {plural(agents, "агент работает", "агента работают", "агентов работают")}
    </span>
  );
}
