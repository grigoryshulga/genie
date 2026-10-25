import { useEffect, useRef } from "react";
import { Avatars } from "@/entities/member";
import { PersonAvatar } from "@/entities/project";
import { EpicChip, EpicIcon, Labels, PriorityIcon, StatusIcon, type Status, STATUS_NAME, STATUS_ORDER, type TaskSummary } from "@/entities/task";
import type { Team } from "@/entities/team";
import { timeAgo } from "@/shared/lib";
import { cx, EmptyState, Segments } from "@/shared/ui";
// The list rows: `.tl-*` and the `.pill`/`.empty` primitives the rows and their pages reuse.

/**
 * The task list: rows grouped by status (or one flat list, in the given order). A row states the task's
 * exact status, its id and title, what is waiting on it, its criteria, who has it and its age. The
 * keyboard moves a single focus through the rows; the caller owns which row is focused and which is open.
 */
export function TaskList({
  tasks,
  statuses,
  grouped = true,
  when = "updated",
  empty = "Здесь пусто",
  teams,
  people,
  focused,
  selected,
  onOpen,
  onFocus,
}: {
  tasks: TaskSummary[];
  statuses: Status[];
  /** By status, or one flat list in the given order. */
  grouped?: boolean;
  /** Which date the last column shows. */
  when?: "updated" | "created";
  empty?: string;
  teams: Map<string, Team>;
  /** login → name of the project's people. */
  people?: Map<string, string>;
  focused?: string;
  selected?: string;
  onOpen: (id: string) => void;
  onFocus: (id: string) => void;
}) {
  const focusRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    void focusRef.current?.scrollIntoView({ block: "nearest" });
  }, [focused]);

  const groups: { status?: Status; items: TaskSummary[] }[] = grouped
    ? STATUS_ORDER.filter((s) => statuses.includes(s))
        .map((s) => ({ status: s, items: tasks.filter((t) => t.status === s) }))
        .filter((g) => g.items.length)
    : tasks.length
      ? [{ items: tasks }]
      : [];

  if (!groups.length) {
    return (
      <EmptyState className="tl-empty" title={empty} />
    );
  }

  return (
    <div role="list" aria-label="Задачи" className="tl-groups">
      {groups.map((g) => (
        <section key={g.status ?? "all"} aria-label={g.status ? STATUS_NAME[g.status] : "Задачи"} className="tl-group">
          {g.status && (
            <div className="g-group-head tl-group-head">
              <StatusIcon status={g.status} size={14} />
              <span>{STATUS_NAME[g.status]}</span>
              <span className="n">{g.items.length}</span>
            </div>
          )}
          {g.items.map((t) => {
            const team = t.team ? teams.get(t.team) : undefined;
            const note = t.needsOwner
              ? `Нужно решение: ${t.needsOwner.question}`
              : t.blocked
                ? `Заблокировано: ${t.blocked.reason}`
                : t.openDeps.length > 0
                  ? `Ждёт ${t.openDeps.join(", ")}`
                  : undefined;
            return (
              <div
                key={t.id}
                ref={focused === t.id ? focusRef : undefined}
                role="listitem"
                aria-selected={selected === t.id}
                data-focus={focused === t.id ? "true" : undefined}
                className={cx("g-row tl-row", selected === t.id && "selected")}
                onMouseEnter={() => onFocus(t.id)}
                onClick={() => onOpen(t.id)}
              >
                <StatusIcon status={t.status} size={14} />
                <span className="tl-id g-mono">{t.id}</span>
                <span className="tl-main">
                  <span className="tl-title-row">
                    <button
                      type="button"
                      className="tl-title"
                      onClick={(e) => {
                        e.stopPropagation();
                        onOpen(t.id);
                      }}
                    >
                      {t.priority <= 1 && <PriorityIcon priority={t.priority} />}
                      {t.type === "epic" && (
                        <span className="tl-epic-mark">
                          <EpicIcon size={12} />
                          эпик
                        </span>
                      )}
                      {t.title}
                    </button>
                    <span className="tl-labels d-only">
                      <Labels labels={t.labels.slice(0, 3)} />
                      {t.labels.length > 3 && <span className="tl-more g-muted">+{t.labels.length - 3}</span>}
                    </span>
                  </span>
                  {note && <span className="tl-note">{note}</span>}
                </span>
                <span className="tl-tags d-only">
                  <EpicChip id={t.parent} />
                </span>
                <span className="tl-ac">
                  {t.acceptanceTotal > 0 && (
                    <>
                      <Segments total={t.acceptanceTotal} done={t.acceptanceDone} label="Критерии приёмки" />
                      <b className="g-num">
                        {t.acceptanceDone}/{t.acceptanceTotal}
                      </b>
                    </>
                  )}
                </span>
                <span className="tl-who">
                  {team ? (
                    <Avatars members={team.members} max={t.assignee ? 3 : 4} />
                  ) : (
                    !t.assignee && <span className="tl-none">{t.status === "inbox" ? "ждёт оркестратора" : "без команды"}</span>
                  )}
                  {t.assignee && <PersonAvatar login={t.assignee} name={people?.get(t.assignee)} />}
                </span>
                <span className="tl-age" title={when === "created" ? "создана" : "обновлена"}>
                  {timeAgo(when === "created" ? t.created : t.updated)}
                </span>
              </div>
            );
          })}
        </section>
      ))}
    </div>
  );
}
