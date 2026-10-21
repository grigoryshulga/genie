import { initial, type Member, memberLabel, ROLE_LETTER } from "../model.ts";
import "./avatar.css";
// Legacy avatar primitives (`.av`, `.avatars`, `.more`) still live in the task-list sheet.
import "@/app/styles/list.css";

export function Avatar({ role, name, activity, state, size }: { role: string; name?: string; activity?: string; state?: string; size?: "md" | "lg" | "solo" }) {
  // Shape, not colour, says which one this is: a rounded square is an agent, a circle is a person.
  const shape = role === "human" ? "human" : "agent";
  const cls = ["av", shape, size ?? "", activity === "working" ? "working" : "", activity === "error" ? "error" : "", state === "stopped" ? "stopped" : "", state === "lost" ? "lost" : ""].filter(Boolean).join(" ");
  const person = role === "human" || role === "orchestrator" || !name;
  const label = `${person ? (name ?? role) : memberLabel(name, role)}${activity === "working" ? " — работает" : activity === "error" ? " — ошибка" : ""}`;
  return (
    <span className={cls} title={label} role="img" aria-label={label}>
      {person ? (ROLE_LETTER[role] ?? role.slice(0, 1).toUpperCase()) : initial(name, role)}
    </span>
  );
}

/** Overlapping avatars; past `max` the rest are counted (`+2`). */
export function Avatars({ members, max }: { members: Member[]; max?: number }) {
  const shown = max && members.length > max ? members.slice(0, max - 1) : members;
  const rest = members.length - shown.length;
  return (
    <span className="avatars">
      {shown.map((m) => (
        <Avatar key={m.name} role={m.role} name={m.name} activity={m.activity} state={m.state} />
      ))}
      {rest > 0 && (
        <span className="av more" title={members.slice(shown.length).map((m) => m.name).join(", ")}>
          +{rest}
        </span>
      )}
    </span>
  );
}
