// A unified diff as the reader sees it: a quiet metadata lead, hunk headers, and
// added/removed lines in the semantic signal/danger pairs of both themes.

import { diffStats, parseUnifiedDiff } from "@/entities/doc";

export function DocDiff({ diff, empty = "Изменений нет" }: { diff: string | undefined; empty?: string }) {
  const lines = parseUnifiedDiff(diff ?? "");
  if (!lines.length) return <p className="doc-hint">{empty}</p>;
  const stats = diffStats(lines);
  return (
    <div className="doc-diff-wrap">
      <p className="doc-diff-stats">
        <span className="doc-diff-added">+{stats.added}</span>
        <span className="doc-diff-removed">−{stats.removed}</span>
      </p>
      <div className="doc-diff" role="group" aria-label="Построчная разница версий">
        {lines.map((line, index) => (
          <span key={index} className={`doc-diff-line ${line.kind}`}>
            {line.text || "\u00a0"}
          </span>
        ))}
      </div>
    </div>
  );
}
