import type { LiveState } from "@/entities/agent-config";
import { timeAgo } from "@/shared/lib";
import type { LiveSession, Team } from "./model.ts";

/** The same activity label on a team's member list and the task's compact roster. */
export function memberActivity(team: Pick<Team, "state">, member: Team["members"][number], live?: { state: LiveState; note?: string }, session?: LiveSession): { state: LiveState; text: string } {
  const active = team.state === "active";
  const paused = active && member.state === "paused";
  const state: LiveState = !active || member.state === "stopped" ? "stopped" : paused ? "waiting" : member.activity === "error" || member.state === "error" ? "error" : member.activity === "working" ? "working" : (live?.state ?? "idle");
  const text = state === "working"
    ? session?.tool ? `работает · ${session.tool.name} ${timeAgo(session.tool.since)}` : "работает"
    : paused ? "на паузе"
    : state === "waiting" ? (live?.note ?? "ждёт")
    : state === "error" ? "ошибка"
    : state === "stopped" ? "остановлен" : "свободен";
  return { state, text };
}
