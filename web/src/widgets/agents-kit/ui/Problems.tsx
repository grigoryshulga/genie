// Keep configuration failures beside their section, with details on demand.
import { Icon } from "@/shared/ui";
import { type Problem } from "@/entities/agent-config";

function describe(message: string) {
  const writable = message.match(/^`([^`]+)` may write files and works in the main working copy \(`workspace: repo`\)$/);
  return writable ? `Участник «${writable[1]}» может изменять файлы в основной рабочей копии (workspace: repo). Выберите отдельную рабочую копию, чтобы изолировать изменения.` : message;
}

export function Problems({ items }: { items: Problem[] }) {
  if (!items.length) return null;
  const errors = items.filter((p) => p.level === "error").length;
  return (
    <details className={`ag-problem-summary${errors ? " error" : ""}`}>
      <summary><Icon.alert size={14} />{errors ? `Ошибки: ${errors}` : `Предупреждения: ${items.length}`}{errors && items.length > errors ? ` · предупреждения: ${items.length - errors}` : ""}</summary>
      <ul className="ag-problems" aria-label="Проблемы">
        {items.map((p, i) => (
          <li key={`${p.item}-${i}`} className={p.level}>
            <b>{p.level === "error" ? "Ошибка" : "Предупреждение"}</b>
            <span>{describe(p.message)}<small className="g-muted">{p.path ?? p.item}</small></span>
          </li>
        ))}
      </ul>
      {errors > 0 && <p className="g-muted ag-note">До исправления действует последняя рабочая версия.</p>}
    </details>
  );
}
