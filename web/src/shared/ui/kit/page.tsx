import type { ReactNode } from "react";
import { cx } from "./cx.ts";

/**
 * The top row of a screen: its title on the left, the screen's toolbar pushed to the right edge.
 * Every top-level screen uses it, so the title sits in the same place and at the same size everywhere.
 */
export function PageHeader({ title, toolbar, className }: { title: ReactNode; toolbar?: ReactNode; className?: string }) {
  return (
    <header className={cx("topbar page-head", className)}>
      <h1 className="g-page-title">{title}</h1>
      <span className="grow" />
      {toolbar && (
        <div className="page-toolbar" role="toolbar" aria-label="Инструменты страницы">
          {toolbar}
        </div>
      )}
    </header>
  );
}
