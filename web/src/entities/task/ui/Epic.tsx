import { Link } from "react-router";
import { useEpicMap } from "../api.ts";
import { PROGRESS, progressOf, type TaskSummary } from "../model.ts";
import { epicTone } from "../epic-tone.ts";
import "./epic-color.css";

/** Epic mark: the brand arcs in miniature; the closed epic is drawn solid, an empty one is outlined by its lighter inner arc. */
export function EpicIcon({ size = 14, filled, empty, id }: { size?: number; filled?: boolean; empty?: boolean; id?: string }) {
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="currentColor" aria-hidden="true" className="st-accent epic-icon" data-epic-tone={id ? epicTone(id) : undefined}>
      <path d="M1 15A14 14 0 0 1 15 1v3.2A10.8 10.8 0 0 0 4.2 15z" />
      {!empty && <path d="M5.8 15A9.2 9.2 0 0 1 15 5.8V9a6 6 0 0 0-6 6z" opacity={filled ? 1 : 0.6} />}
    </svg>
  );
}

/** Small link to the epic a task belongs to; nothing when its parent is not an epic. */
export function EpicChip({ id, full, titleOnly, text }: { id?: string; full?: boolean; titleOnly?: boolean; /** Plain text (inside another button). */ text?: boolean }) {
  const epic = useEpicMap().get(id ?? "");
  if (!epic) return null;
  const label = titleOnly ? epic.title : full ? `${epic.id} · ${epic.title}` : epic.id;
  if (text) {
    return (
      <span className="epic-chip" title={`Эпик ${epic.id}: ${epic.title}`}>
        <EpicIcon size={11} id={titleOnly ? epic.id : undefined} />
        <span>{label}</span>
      </span>
    );
  }
  return (
    <Link to={`/epic/${encodeURIComponent(epic.id)}`} className="epic-chip" title={`Эпик ${epic.id}: ${epic.title}`} onClick={(e) => e.stopPropagation()}>
      <EpicIcon size={11} id={titleOnly ? epic.id : undefined} />
      <span>{label}</span>
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
          ? counts.map((p) => <span key={p.id} className={`p-${p.id}`} title={`${p.name}: ${p.n}`} style={{ flexGrow: p.n }} />)
          : sorted.map((c) => <span key={c.id} className={`p-${progressOf(c.status)}`} title={`${c.id}: ${c.title}`} />)}
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
                <i className={`p-${p.id}`} />
                {p.n} {p.name.toLowerCase()}
              </span>
            ))}
        </span>
      )}
      {legend === "list" && (
        <span className="legend-list">
          {counts.map((p) => (
            <span key={p.id}>
              <i className={`p-${p.id}`} />
              <span className="grow">{p.name}</span>
              <span className="muted">{p.n}</span>
            </span>
          ))}
        </span>
      )}
    </div>
  );
}

/**
 * The arcs as a progress motif: the outer arc fills in proportion to the closed tasks, the
 * inner two stay as quiet tracks. Decoration for an epic card; the numbers beside it carry the data.
 */
export function EpicArcs({ done, total, size = 64 }: { done: number; total: number; size?: number }) {
  const share = total ? Math.round((done / total) * 100) : 0;
  return (
    <svg className="epic-arcs" width={size} height={size} viewBox="0 0 16 16" fill="none" role="img" aria-label={`Закрыто ${done} из ${total}`}>
      <path className="track" d="M2.6 15A12.4 12.4 0 0 1 15 2.6" strokeWidth="3.2" pathLength="100" />
      <path className="fill" d="M2.6 15A12.4 12.4 0 0 1 15 2.6" strokeWidth="3.2" pathLength="100" strokeDasharray={`${share} 100`} />
      <path className="track" d="M7.4 15A7.6 7.6 0 0 1 15 7.4" strokeWidth="3.2" />
      <path className="track" d="M12.4 15A2.6 2.6 0 0 1 15 12.4" strokeWidth="3.2" />
    </svg>
  );
}
