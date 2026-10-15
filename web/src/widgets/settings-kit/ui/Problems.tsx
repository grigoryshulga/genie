// Problems of a configuration item: what the server's check says, in plain words.

import { type Problem } from "@/entities/agent-config";

export function Problems({ items }: { items: Problem[] }) {
  if (!items.length) return null;
  return (
    <ul className="ag-problems" aria-label="Проблемы">
      {items.map((p, i) => (
        <li key={i} className={p.level}>
          <b>{p.level === "error" ? "Ошибка" : "Предупреждение"}</b>
          <span>
            {p.message}
            {p.path ? <span className="muted"> · {p.path}</span> : null}
          </span>
        </li>
      ))}
    </ul>
  );
}
