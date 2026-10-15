import { type LiveState, liveTeam, type TeamSpecView, TemplateGraph, useAgentConfig } from "@/entities/agent-config";
import { displayName } from "@/entities/member";
import type { TeamDetail } from "@/entities/team";

/** What the team is busy with, in one line: who works, who waits for whom. */
function summary(spec: TeamSpecView, live: Record<string, { state: LiveState; note?: string }>, active: boolean): { text: string; tone: "" | "working" | "waiting" | "error" } {
  if (!active) return { text: "Команда остановлена", tone: "" };
  const name = (key: string) => displayName(spec.members.find((m) => m.key === key)?.name ?? key);
  const of = (s: LiveState) => Object.keys(live).filter((k) => live[k].state === s);
  const [working, waiting, failed] = [of("working"), of("waiting"), of("error")];
  const parts = [
    working.length ? `${working.map(name).join(", ")} ${working.length > 1 ? "работают" : "работает"}` : "",
    ...waiting.map((k) => `${name(k)} ${live[k].note}`),
    failed.length ? `ошибка у ${failed.map(name).join(", ")}` : "",
  ].filter(Boolean);
  if (!parts.length) return { text: "Сейчас никто не работает", tone: "" };
  return { text: parts.join(" · "), tone: failed.length ? "error" : waiting.length ? "waiting" : "working" };
}

/**
 * How a running team works, live: its members and relations as a graph, who
 * works now and who waits for whose handoff. The team keeps the snapshot it
 * took from its template. Folded, only the line of who does what stays.
 */
export function TeamScheme({ team, open, onToggle }: { team: TeamDetail; open: boolean; onToggle: () => void }) {
  const cfg = useAgentConfig().data;
  const spec = team.spec;
  if (!spec) return null;
  const active = team.state === "active";
  const { live, pending, done } = liveTeam(spec, team.members, team.taskInfo?.status ?? "ready", active, displayName);
  const now = summary(spec, live, active);
  const head = (
    <>
      <span className={`team-now ${now.tone}`}>
        {now.tone === "working" ? <span className="spin" /> : <i />}
        {now.text}
      </span>
      <button type="button" className="btn ghost sm" aria-expanded={open} onClick={onToggle}>
        {open ? "Свернуть схему" : "Показать схему"}
      </button>
    </>
  );
  return (
    <section className="team-scheme" aria-label="Схема команды">
      {open ? (
        <TemplateGraph
          members={spec.members.map((m) => ({ key: m.key, role: m.role, name: displayName(m.name) }))}
          relations={spec.relations}
          roles={cfg?.roles ?? []}
          live={live}
          pending={pending}
          done={done}
          head={head}
        />
      ) : (
        <div className="team-scheme-folded">{head}</div>
      )}
    </section>
  );
}
