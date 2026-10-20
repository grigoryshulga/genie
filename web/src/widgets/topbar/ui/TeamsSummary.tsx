import type { Team } from "@/entities/team";
import { plural } from "@/shared/lib";

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
