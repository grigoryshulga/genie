// Keep configuration failures beside their section, with details on demand.
import { type Problem } from "@/entities/agent-config";

export function Problems({ items }: { items: Problem[] }) {
  if (!items.length) return null;
  const errors = items.filter((p) => p.level === "error").length;
  return (
    <details className={`ag-problem-summary${errors ? " error" : ""}`}>
      <summary>{errors ? `Ошибки: ${errors}` : `Предупреждения: ${items.length}`}{errors && items.length > errors ? ` · предупреждения: ${items.length - errors}` : ""}</summary>
      <ul className="ag-problems" aria-label="Проблемы">
        {items.map((p, i) => (
          <li key={`${p.item}-${i}`} className={p.level}>
            <b>{p.level === "error" ? "Ошибка" : "Предупреждение"}</b>
            <span>{p.message}<small className="muted">{p.path ?? p.item}</small></span>
          </li>
        ))}
      </ul>
      {errors > 0 && <p className="muted ag-note">До исправления действует последняя рабочая версия.</p>}
    </details>
  );
}
