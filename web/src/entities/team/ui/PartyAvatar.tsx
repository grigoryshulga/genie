import { displayName } from "@/entities/member";
import { Avatar, cx } from "@/shared/ui";

/**
 * Who speaks or acts in a team: agents (members and the orchestrator) are rounded squares, people are
 * circles. A working agent gets a ring in the accent colour, a failing one in the danger colour.
 */
export function PartyAvatar({ role, name, activity, state, sm }: { role: string; name?: string; activity?: string; state?: string; sm?: boolean }) {
  const human = role === "human";
  const label = human ? (name ?? "Вы") : role === "orchestrator" ? "Оркестратор" : displayName(name ?? role);
  return (
    <span className={cx("tv-av", activity === "working" && "working", activity === "error" && "error", (state === "stopped" || state === "lost") && "stopped")}>
      <Avatar kind={human ? "human" : "agent"} name={label} sm={sm} />
    </span>
  );
}
