import { PRIORITY_NAME, type Status, STATUS_NAME } from "../model.ts";

export function StatusIcon({ status, size = 14 }: { status: Status; size?: number }) {
  const common = { width: size, height: size, viewBox: "0 0 16 16", fill: "none", role: "img", "aria-label": STATUS_NAME[status] } as const;
  switch (status) {
    case "inbox":
      return (
        <svg {...common} stroke="var(--status-inbox)" strokeWidth="1.5" strokeLinejoin="round">
          <path d="M2 9h3l1 2h4l1-2h3" />
          <path d="M3.6 3h8.8L14 9v4H2V9z" />
        </svg>
      );
    case "draft":
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="6" stroke="var(--status-draft)" strokeWidth="1.6" strokeDasharray="2.4 2" />
        </svg>
      );
    case "refining":
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="6" stroke="var(--status-refining)" strokeWidth="1.6" strokeDasharray="2.4 2" />
        </svg>
      );
    case "ready":
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="6" stroke="var(--status-ready)" strokeWidth="1.6" />
        </svg>
      );
    case "in_progress":
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="6" stroke="var(--status-in-progress)" strokeWidth="1.6" />
          <path d="M8 4a4 4 0 0 1 0 8z" fill="var(--status-in-progress)" />
        </svg>
      );
    case "changes_requested":
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="6" stroke="var(--status-changes-requested)" strokeWidth="1.6" />
          <path d="M10.5 6.5A3 3 0 1 0 11 9" stroke="var(--status-changes-requested)" strokeWidth="1.5" strokeLinecap="round" />
          <path d="M11.3 4.6v2.3H9" stroke="var(--status-changes-requested)" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "review":
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="6" stroke="var(--status-review)" strokeWidth="1.6" />
          <path d="M8 4a4 4 0 1 1-4 4h4z" fill="var(--status-review)" />
        </svg>
      );
    case "approved":
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="6" stroke="var(--status-approved)" strokeWidth="1.6" />
          <circle cx="8" cy="8" r="4" fill="var(--status-approved)" />
        </svg>
      );
    case "needs_owner":
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="6.3" fill="var(--status-needs-owner)" />
          <path d="M8 4.6v4.2" stroke="var(--bg)" strokeWidth="1.7" strokeLinecap="round" />
          <circle cx="8" cy="11.2" r="0.95" fill="var(--bg)" />
        </svg>
      );
    case "done":
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="6.3" fill="var(--status-done)" />
          <path d="M5.4 8.2l1.8 1.8 3.5-3.7" stroke="var(--bg)" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      );
    case "cancelled":
      return (
        <svg {...common}>
          <circle cx="8" cy="8" r="6" stroke="var(--status-cancelled)" strokeWidth="1.6" />
          <path d="M6 6l4 4M10 6l-4 4" stroke="var(--status-cancelled)" strokeWidth="1.5" strokeLinecap="round" />
        </svg>
      );
  }
}

export function PriorityIcon({ priority, size = 14 }: { priority: number; size?: number }) {
  const label = PRIORITY_NAME[priority] ?? "";
  if (priority === 0) {
    return (
      <svg width={size} height={size} viewBox="0 0 16 16" role="img" aria-label={label}>
        <rect x="1.5" y="1.5" width="13" height="13" rx="3" fill="var(--amber)" />
        <path d="M8 4.5v4.3" stroke="var(--bg)" strokeWidth="1.7" strokeLinecap="round" />
        <circle cx="8" cy="11.3" r="0.95" fill="var(--bg)" />
      </svg>
    );
  }
  const n = priority === 1 ? 3 : priority === 2 ? 2 : priority === 3 ? 1 : 0;
  const c = (i: number) => (n >= i ? "var(--text-2)" : "var(--off)");
  return (
    <svg width={size} height={size} viewBox="0 0 16 16" role="img" aria-label={label}>
      <rect x="2" y="9" width="3" height="5" rx="1" fill={c(1)} />
      <rect x="6.5" y="6" width="3" height="8" rx="1" fill={c(2)} />
      <rect x="11" y="3" width="3" height="11" rx="1" fill={c(3)} />
    </svg>
  );
}
