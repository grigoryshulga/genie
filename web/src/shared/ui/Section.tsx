// A titled section and a small badge — the common parts of every settings page
// (the tabs of the agent catalogue, the server page, the settings mode).

import { type ReactNode } from "react";

export function Section({ title, note, aside, children }: { title: string; note?: ReactNode; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="ag-section">
      <div className="ag-section-head">
        <h3>{title}</h3>
        <span className="ag-section-note">{note}</span>
        {aside}
      </div>
      {children}
    </section>
  );
}

export function Badge({ children, tone }: { children: ReactNode; tone?: "accent" | "amber" | "green" | "red" | "muted" }) {
  return <span className={`ag-badge${tone ? ` ${tone}` : ""}`}>{children}</span>;
}
