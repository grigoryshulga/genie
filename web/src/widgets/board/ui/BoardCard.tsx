import { useDraggable } from "@dnd-kit/core";
import { useLayoutEffect, useRef, useState } from "react";
import { displayName } from "@/entities/member";
import { EpicIcon, moveTargets, PriorityIcon, type Status, STATUS_NAME, type TaskSummary, useEpicMap } from "@/entities/task";
import type { Team } from "@/entities/team";
import { Avatar, cx, Icon, Segments, Tag, WorkingArcs, ActionMenu, type MenuEntry } from "@/shared/ui";

/** The exact status as a sub-status tag, in the sheet's own colours. */
function statusTone(status: Status): "accent" | "signal" | "outline" {
  if (status === "changes_requested") return "signal";
  if (status === "approved") return "accent";
  return "outline";
}

/** What a card says about its task: id, tags, title, the open question or wait, criteria, epic, labels, people. */
export function CardBody({ t, team, showStatus }: { t: TaskSummary; team?: Team; showStatus: boolean }) {
  const epic = useEpicMap().get(t.parent ?? "");
  const working = !!team?.members.some((m) => m.activity === "working");
  const members = team?.members ?? [];
  const shown = members.slice(0, 3);
  const rest = members.length - shown.length;
  return (
    <>
      <span className="bd-meta">
        <span className="bd-id">{t.id}</span>
        {t.priority === 0 && <Tag tone="solid-signal">срочно</Tag>}
        {t.priority === 1 && <PriorityIcon priority={1} size={14} />}
        {showStatus && <Tag tone={statusTone(t.status)}>{STATUS_NAME[t.status].toLowerCase()}</Tag>}
        {working && (
          <span className="bd-working">
            <WorkingArcs size={20} label="Агент работает" />
          </span>
        )}
      </span>
      <span className="bd-title" title={t.title}>
        {t.type === "epic" && (
          <span className="bd-epic-mark">
            <EpicIcon size={12} />
            эпик{" "}
          </span>
        )}
        {t.title}
      </span>
      {t.needsOwner ? (
        <span className="bd-ask" title={t.needsOwner.question}>
          <span className="bd-question">{t.needsOwner.question}</span>
          <span className="bd-answer">Ответить →</span>
        </span>
      ) : t.blocked ? (
        <span className="bd-note" title={t.blocked.reason}>
          Заблокировано: {t.blocked.reason}
        </span>
      ) : (
        t.openDeps.length > 0 && <span className="bd-note">Ждёт {t.openDeps.join(", ")}</span>
      )}
      {t.acceptanceTotal > 0 && (
        <span className="bd-ac">
          <Segments total={t.acceptanceTotal} done={t.acceptanceDone} label="Критерии приёмки" />
          <b>
            {t.acceptanceDone}/{t.acceptanceTotal}
          </b>
        </span>
      )}
      {(epic || t.labels.length > 0 || members.length > 0 || t.assignee) && (
        <span className="bd-foot">
          {epic && (
            <span className="bd-epic" title={`Эпик ${epic.id}: ${epic.title}`}>
              ↳ {epic.title}
            </span>
          )}
          {t.labels.slice(0, 2).map((l) => (
            <Tag key={l} tone="outline">
              {l}
            </Tag>
          ))}
          {t.labels.length > 2 && <span className="bd-more">+{t.labels.length - 2}</span>}
          <span className="bd-who">
            {shown.map((m) => (
              <Avatar key={m.name} kind={m.role === "human" ? "human" : "agent"} name={displayName(m.name)} sm />
            ))}
            {rest > 0 && <span className="bd-more">+{rest}</span>}
            {t.assignee && <Avatar kind="human" name={t.assignee} sm />}
          </span>
        </span>
      )}
    </>
  );
}

/** The "Move to" menu: every exact status the card can be moved to, for phones and keyboards. */
function MoveMenu({ t, offline, onMove }: { t: TaskSummary; offline: boolean; onMove: (status: Status) => void }) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const panel = popup.current;
    const button = trigger.current;
    if (!open || !panel || !button) return;
    // Native popovers escape the lane scroller and transformed card stacking contexts.
    panel.showPopover();
    const place = () => {
      const anchor = button.getBoundingClientRect();
      const box = panel.getBoundingClientRect();
      panel.style.left = `${Math.max(8, Math.min(anchor.right - box.width, innerWidth - box.width - 8))}px`;
      panel.style.top = `${Math.max(8, Math.min(anchor.bottom + 4, innerHeight - box.height - 8))}px`;
    };
    place();
    window.addEventListener("resize", place);
    document.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      document.removeEventListener("scroll", place, true);
      panel.hidePopover();
    };
  }, [open]);
  const close = () => {
    setOpen(false);
    trigger.current?.focus();
  };
  const items: MenuEntry[] = moveTargets(t.status).map(({ lane, status }) => ({
    id: status,
    label: STATUS_NAME[status],
    hint: lane.statuses.length > 1 || lane.name !== STATUS_NAME[status] ? lane.name : undefined,
    disabled: offline,
    onSelect: () => onMove(status),
  }));
  return (
    <span className="bd-move g-popover-host">
      <button
        ref={trigger}
        type="button"
        className="g-icon-btn"
        aria-label={`Переместить ${t.id}`}
        aria-haspopup="menu"
        aria-expanded={open}
        disabled={offline}
        title={offline ? "Нет связи с сервером" : "Переместить в…"}
        onClick={() => setOpen(!open)}
      >
        <Icon.dots size={16} />
      </button>
      {open && (
        <div ref={popup} popover="auto" className="bd-move-popup" onToggle={(event) => {
          if (event.newState === "closed") setOpen(false);
        }}>
          <ActionMenu label={`Переместить ${t.id} в`} items={items} onClose={close} />
        </div>
      )}
    </span>
  );
}

export function BoardCard({ t, team, selected, showStatus, offline, onSelect, onMove }: {
  t: TaskSummary;
  team?: Team;
  selected: boolean;
  showStatus: boolean;
  offline: boolean;
  onSelect: () => void;
  onMove: (status: Status) => void;
}) {
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({ id: t.id, disabled: offline });
  return (
    <div ref={setNodeRef} className={cx("g-sheet interactive bd-card", t.needsOwner && "bd-decision", selected && "selected", isDragging && "dragging")} data-status={t.status}>
      <button type="button" className="bd-main" onClick={onSelect} {...attributes} {...listeners} aria-pressed={selected} aria-roledescription="карточка задачи">
        <CardBody t={t} team={team} showStatus={showStatus} />
      </button>
      <MoveMenu t={t} offline={offline} onMove={onMove} />
    </div>
  );
}
