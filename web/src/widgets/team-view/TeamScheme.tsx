import { Link } from "react-router";
import { flowStages, type LiveState, liveTeam, TemplateGraph, useAgentConfig } from "@/entities/agent-config";
import { displayName, ROLE_TITLE_RU } from "@/entities/member";
import { PartyAvatar, type TeamDetail } from "@/entities/team";
import { Button, Icon, WorkingArcs } from "@/shared/ui";

const STATE_TEXT: Record<LiveState, string> = { working: "работает", waiting: "ждёт", idle: "свободен", error: "ошибка", stopped: "остановлен" };

/**
 * How a running team works, at a glance: one line of its members in handoff
 * order, each saying whether it works now or whom it waits for, each opening its
 * chat. The full graph of the team's relations (the snapshot it took from its
 * template) opens below on demand.
 */
export function TeamScheme({ team, open, onToggle }: { team: TeamDetail; open: boolean; onToggle: () => void }) {
  const cfg = useAgentConfig().data;
  const spec = team.spec;
  if (!spec) return null;
  const active = team.state === "active";
  const { live, pending, done } = liveTeam(spec, team.members, team.taskInfo?.status ?? "ready", active, displayName);
  const href = (name: string) => `/team/${encodeURIComponent(team.id)}/${encodeURIComponent(name)}`;
  const roleTitle = (role: string) => ROLE_TITLE_RU[role] ?? cfg?.roles.find((r) => r.id === role)?.title ?? role;
  const stages = flowStages(spec.members.map((m) => m.key), spec.relations);
  // Members added after the start are not in the snapshot: they join the line at its end.
  const extra = team.members.filter((m) => !spec.members.some((sm) => sm.name === m.name));
  return (
    <section className="tv-scheme" aria-label="Схема команды">
      <div className="tv-flow">
        <ol className="tv-flow-line" aria-label="Порядок работы">
          {[...stages, ...(extra.length ? [[]] : [])].map((keys, i) => (
            <li key={i} className="tv-flow-stage">
              {i > 0 && (
                <span className="tv-flow-arrow" aria-hidden="true">
                  <Icon.chevron size={10} />
                </span>
              )}
              {keys.map((key) => {
                const sm = spec.members.find((m) => m.key === key)!;
                const member = team.members.find((m) => m.name === sm.name);
                const l = live[key] ?? { state: "stopped" as const };
                const text = l.state === "waiting" && l.note ? l.note : STATE_TEXT[l.state];
                return (
                  <FlowMember key={key} href={member ? href(member.name) : undefined} name={sm.name} role={sm.role} roleTitle={roleTitle(sm.role)} state={l.state} text={text} note={l.state !== "waiting" ? l.note : undefined} />
                );
              })}
              {i === stages.length &&
                extra.map((m) => {
                  const state: LiveState = !active || m.state === "stopped" ? "stopped" : m.activity === "working" ? "working" : m.activity === "error" ? "error" : "idle";
                  return <FlowMember key={m.name} href={href(m.name)} name={m.name} role={m.role} roleTitle={roleTitle(m.role)} state={state} text={STATE_TEXT[state]} />;
                })}
            </li>
          ))}
        </ol>
        <Button variant="ghost" sm className="tv-flow-toggle" aria-expanded={open} onClick={onToggle}>
          {open ? "Скрыть связи" : "Схема связей"}
        </Button>
      </div>
      {open && (
        <TemplateGraph
          members={spec.members.map((m) => ({ key: m.key, role: m.role, name: displayName(m.name) }))}
          relations={spec.relations}
          roles={cfg?.roles ?? []}
          cards={Object.fromEntries(
            spec.members.flatMap((sm) => {
              const member = team.members.find((m) => m.name === sm.name);
              if (!member) return [];
              const state = live[sm.key]?.state;
              const session = active ? team.sessions?.[member.name] : undefined;
              const activity =
                state === "stopped"
                  ? "Агент остановлен"
                  : member.state === "paused"
                    ? "На паузе"
                    : state === "working" && session?.tool
                      ? `${session.tool.name}: ${session.tool.args}`
                      : state === "waiting"
                        ? (live[sm.key].note ?? "Ожидает передачи работы")
                        : member.status && member.status !== "starting"
                          ? member.status
                          : state === "working"
                            ? "Выполняет задачу"
                            : state === "error"
                              ? "Ошибка агента"
                              : "Ожидает поручения";
              return [[sm.key, { href: href(member.name), role: roleTitle(sm.role), activity }]];
            }),
          )}
          live={live}
          pending={pending}
          done={done}
        />
      )}
    </section>
  );
}

/** One member in the line: who it is and what it does now; it opens the chat with the agent. */
function FlowMember({ href, name, role, roleTitle, state, text, note }: { href?: string; name: string; role: string; roleTitle: string; state: LiveState; text: string; note?: string }) {
  const body = (
    <>
      <PartyAvatar role={role} name={name} sm activity={state === "working" ? "working" : state === "error" ? "error" : undefined} state={state === "stopped" ? "stopped" : undefined} />
      <span className="tv-flow-name">{displayName(name)}</span>
      {text && (
        <span className="tv-flow-state">
          {state === "working" && <WorkingArcs size={11} label="Работает" />}
          {text}
        </span>
      )}
    </>
  );
  const title = `${displayName(name)} — ${roleTitle}${text ? `: ${text}` : ""}${note ? `\n${note}` : ""}`;
  const cls = `tv-flow-member ${state}`;
  return href ? (
    <Link to={href} className={cls} title={title} aria-label={`${title}. Открыть разговор с агентом`}>
      {body}
    </Link>
  ) : (
    <span className={cls} title={title}>
      {body}
    </span>
  );
}
