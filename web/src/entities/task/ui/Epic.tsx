import { Link } from "react-router";
import { useEpicMap } from "../api.ts";
import { PROGRESS, progressOf, type TaskSummary } from "../model.ts";

/** Epic mark: a diamond, filled once the epic is closed. */
export function EpicIcon({ size = 14, filled, empty }: { size?: number; filled?: boolean; empty?: boolean }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" aria-hidden="true" style={{ flex: "none" }}>
      <path d="M8 1.8L14.2 8 8 14.2 1.8 8z" stroke="var(--violet)" strokeWidth="1.5" strokeLinejoin="round" fill={filled ? "var(--violet)" : "none"} />
      {!filled && !empty && <path d="M8 5.2L10.8 8 8 10.8 5.2 8z" fill="var(--violet)" />}
    </svg>
  );
}

/** Small link to the epic a task belongs to; nothing when its parent is not an epic. */
export function EpicChip({ id, full, text }: { id?: string; full?: boolean; /** Plain text (inside another button). */ text?: boolean }) {
  const epic = useEpicMap().get(id ?? "");
  if (!epic) return null;
  if (text) {
    return (
      <span className="epic-chip" title={`Эпик ${epic.id}: ${epic.title}`}>
        <EpicIcon size={11} />
        {full ? `${epic.id} · ${epic.title}` : epic.id}
      </span>
    );
  }
  return (
    <Link to={`/epic/${encodeURIComponent(epic.id)}`} className="epic-chip" title={`Эпик ${epic.id}: ${epic.title}`} onClick={(e) => e.stopPropagation()}>
      <EpicIcon size={11} />
      {full ? `${epic.id} · ${epic.title}` : epic.id}
    </Link>
  );
}

export function progressCounts(tasks: TaskSummary[]) {
  return PROGRESS.map((p) => ({ ...p, n: tasks.filter((c) => progressOf(c.status) === p.id).length })).filter((p) => p.n > 0);
}

/**
 * One segment per task of the epic, coloured by where it stands. With many tasks, or
 * when `compact`, the segments merge into one bar per state, so it never overflows.
 */
export function EpicProgress({ tasks, big, legend, compact }: { tasks: TaskSummary[]; big?: boolean; legend?: "inline" | "list"; compact?: boolean }) {
  const closed = tasks.filter((c) => progressOf(c.status) === "closed").length;
  const order = PROGRESS.map((p) => p.id);
  const sorted = [...tasks].sort((a, b) => order.indexOf(progressOf(a.status)) - order.indexOf(progressOf(b.status)));
  const counts = progressCounts(tasks);
  const merged = compact || tasks.length > 24;
  return (
    <div className={`epic-progress${big ? " big" : ""}`}>
      <span className={merged ? "segs merged" : "segs"} role="progressbar" aria-label={`Закрыто ${closed} из ${tasks.length}`} aria-valuemin={0} aria-valuemax={tasks.length} aria-valuenow={closed}>
        {merged
          ? counts.map((p) => <span key={p.id} title={`${p.name}: ${p.n}`} style={{ flexGrow: p.n, background: p.color }} />)
          : sorted.map((c) => <span key={c.id} title={`${c.id}: ${c.title}`} style={{ background: PROGRESS.find((p) => p.id === progressOf(c.status))!.color }} />)}
      </span>
      {legend === "inline" && (
        <span className="legend">
          <b>
            {closed} из {tasks.length} закрыто
          </b>
          {counts
            .filter((p) => p.id !== "closed")
            .map((p) => (
              <span key={p.id}>
                <i style={{ background: p.color }} />
                {p.n} {p.name.toLowerCase()}
              </span>
            ))}
        </span>
      )}
      {legend === "list" && (
        <span className="legend-list">
          {counts.map((p) => (
            <span key={p.id}>
              <i style={{ background: p.color }} />
              <span className="grow">{p.name}</span>
              <span className="muted">{p.n}</span>
            </span>
          ))}
        </span>
      )}
    </div>
  );
}
