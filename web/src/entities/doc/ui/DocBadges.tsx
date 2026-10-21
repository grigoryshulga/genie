// Doc badges: type / status / stale / diagnostics, plus the tree legend (D1).

import type { DocPage, DocStatus } from "../model.ts";
import { DOC_STATUS_NAME, DOC_TYPE_NAME, DOC_TYPE_SHORT } from "../model.ts";
import { Tag } from "@/shared/ui";
import { DeprecatedIcon, DiagIcon, DraftIcon, OkIcon, StaleIcon, UnknownIcon } from "./icons.tsx";

// Tag tones carry the status: green means current, orange means "check me", red means the
// frontmatter is broken, and a plain outline is a lifecycle fact rather than a call to act.
const STATUS_TONE = { current: "signal", draft: "outline", deprecated: "outline" } as const;

/** The page's type when the frontmatter sets one (a plain page shows nothing). */
export function DocTypeBadge({ type, long }: { type: DocPage["type"]; long?: boolean }) {
  if (!type) return null;
  return (
    <Tag tone="outline" className={`doc-type t-${type}`} title={`Тип: ${DOC_TYPE_NAME[type]}`}>
      {long ? DOC_TYPE_NAME[type].toLowerCase() : DOC_TYPE_SHORT[type]}
    </Tag>
  );
}

export function DocStatusIcon({ status, size = 12 }: { status: DocStatus | null; size?: number }) {
  if (status === "current") return <OkIcon size={size} className="doc-ic ok" aria-hidden />;
  if (status === "draft") return <DraftIcon size={size} className="doc-ic draft" aria-hidden />;
  if (status === "deprecated") return <DeprecatedIcon size={size} className="doc-ic deprecated" aria-hidden />;
  return <UnknownIcon size={size} className="doc-ic none" aria-hidden />;
}

export function DocStatusBadge({ status }: { status: DocStatus | null }) {
  const name = status ? DOC_STATUS_NAME[status] : "Статус не указан";
  return (
    <Tag tone={status ? STATUS_TONE[status] : "outline"} className={`doc-status s-${status ?? "none"}`} title={name}>
      <DocStatusIcon status={status} />
      {name}
    </Tag>
  );
}

export function DocStaleIcon({ size = 12 }: { size?: number }) {
  return <StaleIcon size={size} className="doc-ic stale" aria-hidden />;
}

export function DocDiagIcon({ size = 12 }: { size?: number }) {
  return <DiagIcon size={size} className="doc-ic diag" aria-hidden />;
}

export function DocStaleBadge({ count }: { count?: number }) {
  return (
    <Tag tone="accent" className="doc-status s-stale" title="Код, который описывает страница, менялся после её проверки">
      <DocStaleIcon />
      Возможно устарела{count && count > 1 ? ` · ${count}` : ""}
    </Tag>
  );
}

export function DocDiagBadge({ count }: { count?: number }) {
  return (
    <Tag tone="danger" className="doc-status s-diag" title="Frontmatter прочитан с ошибками, использованы значения по умолчанию">
      <DocDiagIcon />
      Ошибка frontmatter{count && count > 1 ? ` · ${count}` : ""}
    </Tag>
  );
}

/** Tree/search marks: diagnostics win over staleness, as in the mockups. */
export function DocMarks({ page, size = 12 }: { page: Pick<DocPage, "status" | "stale" | "diagnostics">; size?: number }) {
  return (
    <span className="doc-marks">
      {page.status && <DocStatusIcon status={page.status} size={size} />}
      {page.stale && <DocStaleIcon size={size} />}
      {page.diagnostics.length > 0 && <DocDiagIcon size={size} />}
    </span>
  );
}

/** Status text used by the mobile drawer's flat tree. */
export function docStatusText(page: Pick<DocPage, "status" | "stale" | "diagnostics">): { text: string; cls: string } {
  if (page.diagnostics.length) return { text: "ошибка", cls: "diag" };
  if (page.status === "draft") return { text: "черновик", cls: "draft" };
  if (page.stale) return { text: "устарела?", cls: "stale" };
  if (page.status === "current") return { text: "актуальна", cls: "ok" };
  if (page.status === "deprecated") return { text: "устарела", cls: "deprecated" };
  return { text: "без статуса", cls: "none" };
}

/** The four-item legend at the bottom of the tree column, folded until asked for. */
export function DocLegend() {
  return (
    <details className="doc-legend-wrap">
      <summary>Обозначения</summary>
      <div className="doc-legend">
      <span className="doc-status s-current">
        <DocStatusIcon status="current" /> {DOC_STATUS_NAME.current}
      </span>
      <span className="doc-status s-draft">
        <DocStatusIcon status="draft" /> {DOC_STATUS_NAME.draft}
      </span>
      <span className="doc-status s-stale">
        <DocStaleIcon />
        <span>
          Возможно
          <br />
          устарела
        </span>
      </span>
      <span className="doc-status s-diag">
        <DocDiagIcon />
        Ошибка frontmatter
      </span>
      </div>
    </details>
  );
}
