import { PRIORITY_NAME, type Status, STATUS_NAME } from "../model.ts";
import { priorityBars, STATUS_STYLE, type StatusShape } from "../status-style.ts";

/** 16px, 1.6 stroke, currentColor; the colour role comes from the st-* class (components.css). */
function ShapeArt({ shape }: { shape: StatusShape }) {
  switch (shape) {
    case "tray":
      return (
        <g strokeLinejoin="round">
          <path d="M2 9h3l1 2h4l1-2h3" />
          <path d="M3.6 3h8.8L14 9v4H2V9z" />
        </g>
      );
    case "ring-dashed":
      return <circle cx="8" cy="8" r="6" strokeDasharray="2.4 2" />;
    case "ring-dashed-dot":
      return (
        <>
          <circle cx="8" cy="8" r="6" strokeDasharray="2.4 2" />
          <circle cx="8" cy="8" r="1.8" fill="currentColor" stroke="none" />
        </>
      );
    case "ring":
      return <circle cx="8" cy="8" r="6" />;
    case "ring-half":
      return (
        <>
          <circle cx="8" cy="8" r="6" />
          <path d="M8 4a4 4 0 0 1 0 8z" fill="currentColor" stroke="none" />
        </>
      );
    case "return-arrow":
      return (
        <g strokeLinecap="round" strokeLinejoin="round">
          <path d="M8 2a6 6 0 1 1-5.4 3.4" />
          <path d="M2 2v3.6h3.6" />
        </g>
      );
    case "ring-dot":
      return (
        <>
          <circle cx="8" cy="8" r="6" />
          <circle cx="8" cy="8" r="2.2" fill="currentColor" stroke="none" />
        </>
      );
    case "disc-check":
      return (
        <>
          <circle cx="8" cy="8" r="7" fill="currentColor" stroke="none" />
          <path className="st-knock" d="M5 8.2l2.2 2.2L11 6.2" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
        </>
      );
    case "disc-alert":
      return (
        <>
          <circle cx="8" cy="8" r="7" fill="currentColor" stroke="none" />
          <path className="st-knock" d="M8 4.4v4.2" strokeWidth="1.9" strokeLinecap="round" />
          <circle cx="8" cy="11.4" r="1.05" fill="var(--g-bg)" stroke="none" />
        </>
      );
    case "ring-slash":
      return (
        <g strokeLinecap="round">
          <circle cx="8" cy="8" r="6" />
          <path d="M4.2 11.8l7.6-7.6" />
        </g>
      );
  }
}

export function StatusIcon({ status, size = 14 }: { status: Status; size?: number }) {
  const { shape, role } = STATUS_STYLE[status];
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" role="img" aria-label={STATUS_NAME[status]} className={`st-${role}`}>
      <ShapeArt shape={shape} />
    </svg>
  );
}

export function PriorityIcon({ priority, size = 14 }: { priority: number; size?: number }) {
  const label = PRIORITY_NAME[priority] ?? "";
  const bars = priorityBars(priority);
  if (bars === "urgent") {
    return (
      <svg width={size} height={size} viewBox="0 0 16 16" role="img" aria-label={label}>
        <rect x="1.5" y="1.5" width="13" height="13" rx="4" fill="var(--g-signal)" />
        <path d="M8 4.6v4.2" stroke="var(--g-on-signal)" strokeWidth="1.9" strokeLinecap="round" />
        <circle cx="8" cy="11.4" r="1.05" fill="var(--g-on-signal)" />
      </svg>
    );
  }
  if (bars === "none") {
    return (
      <svg width={size} height={size} viewBox="0 0 16 16" role="img" aria-label={label} className="st-neutral" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round">
        <path d="M3 8h2M7 8h2M11 8h2" />
      </svg>
    );
  }
  const dim = { opacity: 0.28 };
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" role="img" aria-label={label} className="st-neutral" fill="currentColor">
      <rect x="2" y="9" width="3" height="5" rx="1" />
      <rect x="6.5" y="5.5" width="3" height="8.5" rx="1" style={bars >= 2 ? undefined : dim} />
      <rect x="11" y="2" width="3" height="12" rx="1" style={bars >= 3 ? undefined : dim} />
    </svg>
  );
}
