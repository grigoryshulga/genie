import { useDraggable } from "@dnd-kit/core";
import { useLayoutEffect, useRef, useState } from "react";
import { displayName } from "@/entities/member";
import { EpicChip, EpicIcon, epicTone, moveTargets, PriorityIcon, type Status, STATUS_NAME, type TaskSummary, useEpicMap } from "@/entities/task";
import type { Team } from "@/entities/team";
import { Avatar, cx, Icon, Segments, Tag, WorkingArcs, ActionMenu, type MenuEntry } from "@/shared/ui";

/** The exact status as a sub-status tag, in the sheet's own colours. */
function statusTone(status: Status): "accent" | "signal" | "outline" {
  if (status === "changes_requested") return "signal";
  if (status === "approved") return "accent";
  return "outline";
}

/** The epic link and move menu stay outside the draggable task button. */
export function CardBody({ t, team, showStatus, onSelect, selected, drag }: {
  t: TaskSummary;
  team?: Team;
  showStatus: boolean;
  onSelect?: () => void;
  selected?: boolean;
  drag?: Pick<ReturnType<typeof useDraggable>, "attributes" | "listeners">;
}) {
  const working = !!team?.members.some((m) => m.activity === "working");
  const members = team?.members ?? [];
  const shown = members.slice(0, 2);
  const rest = members.length - shown.length;
  const content = (
    <>
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
      ) : (
        <span className="bd-context">
          {showStatus && <span className="bd-status"><Tag tone={statusTone(t.status)}>{STATUS_NAME[t.status].toLowerCase()}</Tag></span>}
          {t.blocked ? (
            <span className="bd-note" title={t.blocked.reason}>Заблокировано: {t.blocked.reason}</span>
          ) : t.openDeps.length > 0 && <span className="bd-note" title={`Ждёт ${t.openDeps.join(", ")}`}>Ждёт {t.openDeps.join(", ")}</span>}
        </span>
      )}
    </>
  );
  return (
    <>
      <div className="bd-meta">
        <span className="bd-id">{t.id}</span>
        <EpicChip id={t.parent} titleOnly text={!onSelect} />
        {t.priority === 0 && <Tag tone="solid-signal">срочно</Tag>}
        {t.priority === 1 && <PriorityIcon priority={1} size={14} />}
        {working && (
          <span className="bd-working">
            <WorkingArcs size={16} label="Агент работает" />
          </span>
        )}
      </div>
      {onSelect ? (
        <button type="button" className="bd-main" onClick={onSelect} {...drag?.attributes} {...drag?.listeners} aria-label={`${t.id}: ${t.title}`} aria-description={t.needsOwner?.question ?? t.blocked?.reason} aria-pressed={selected} aria-roledescription="карточка задачи">
          {content}
        </button>
      ) : <div className="bd-main">{content}</div>}
      {(t.acceptanceTotal > 0 || t.comments > 0 || t.labels.length > 0 || members.length > 0 || t.assignee) && <div className="bd-foot">
        {t.acceptanceTotal > 0 && (
          <span className="bd-ac">
            <Segments total={t.acceptanceTotal} done={t.acceptanceDone} label="Критерии приёмки" />
            <b>{t.acceptanceDone}/{t.acceptanceTotal}</b>
          </span>
        )}
        {t.comments > 0 && <span className="bd-comments" title={`Комментариев: ${t.comments}`}>{t.comments} комм.</span>}
        <span className="bd-labels" title={t.labels.join(", ")}>
          {t.labels.slice(0, 1).map((l) => <Tag key={l} tone="outline">{l}</Tag>)}
          {t.labels.length > 1 && <span className="bd-more">+{t.labels.length - 1}</span>}
        </span>
        <span className="bd-who">
          {shown.map((m) => <Avatar key={m.name} kind={m.role === "human" ? "human" : "agent"} name={displayName(m.name)} sm />)}
          {rest > 0 && <span className="bd-more">+{rest}</span>}
          {t.assignee && <Avatar kind="human" name={t.assignee} sm />}
        </span>
      </div>}
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
  const epic = useEpicMap().get(t.parent ?? "");
  return (
    <div ref={setNodeRef} className={cx("g-sheet interactive bd-card", t.needsOwner && "bd-decision", selected && "selected", isDragging && "dragging")} data-task-id={t.id} data-status={t.status} data-epic-tone={epic ? epicTone(epic.id) : undefined}
      onClick={(event) => {
        if (isDragging || (event.target instanceof Element && event.target.closest("a, button"))) return;
        event.currentTarget.querySelector<HTMLButtonElement>(".bd-main")?.focus();
        onSelect();
      }}
    >
      <CardBody t={t} team={team} showStatus={showStatus} onSelect={onSelect} selected={selected} drag={{ attributes, listeners }} />
      <MoveMenu t={t} offline={offline} onMove={onMove} />
    </div>
  );
}
