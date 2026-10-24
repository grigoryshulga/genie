import { type ReactNode, useMemo, useState } from "react";
import { type ModelOption, useModels } from "@/entities/team";
import { plural } from "@/shared/lib";
import { LoadingState } from "@/shared/ui";

/**
 * The search box and the models pi offers, grouped by provider, as a radio
 * list. `first` is the option above the groups (the role's or the default
 * model); `current`, the model in use now, stays listed even if pi does not
 * know it.
 */
export function ModelList({ current, pick, onPick, first, note, autoFocus }: { autoFocus?: boolean; current?: string; pick: string; onPick: (id: string) => void; first: ReactNode; note: (m: ModelOption) => string }) {
  const models = useModels();
  const [query, setQuery] = useState("");
  const groups = useMemo(() => {
    const all = [...(models.data?.models ?? [])];
    if (current && !all.some((m) => m.id === current)) {
      const [provider, name] = current.includes("/") ? current.split(/\/(.*)/s) : ["", current];
      all.push({ id: current, provider, name, listed: false });
    }
    const q = query.trim().toLowerCase();
    const by = new Map<string, ModelOption[]>();
    for (const m of all.filter((m) => !q || m.id.toLowerCase().includes(q))) by.set(m.provider, [...(by.get(m.provider) ?? []), m]);
    return [...by.entries()];
  }, [models.data, current, query]);
  const total = models.data?.models.length ?? 0;
  return (
    <>
      <input className="mm-search" type="search" autoFocus={autoFocus} placeholder={total ? `Найти среди ${total} ${plural(total, "модели", "моделей", "моделей")}` : "Найти модель"} aria-label="Найти модель" value={query} onChange={(e) => setQuery(e.target.value)} />
      <div className="mm-list" role="radiogroup" aria-label="Модель">
        {first}
        {groups.map(([provider, items]) => (
          <div key={provider} className="mm-group">
            {provider && (
              <span className="mm-provider">
                {provider} <span className="n">{items.length}</span>
              </span>
            )}
            {items.map((m) => (
              <button key={m.id} type="button" role="radio" aria-checked={pick === m.id} className={`mm-opt${pick === m.id ? " on" : ""}`} onClick={() => onPick(m.id)}>
                <span className="mm-radio" />
                <span className="mono">{m.name}</span>
                <span className="mm-note">{note(m) || (!m.listed && models.data?.catalogue ? "нет у pi" : "")}</span>
              </button>
            ))}
          </div>
        ))}
        {models.isPending && <div className="mm-empty"><LoadingState label="Загрузка моделей" lines={3} /></div>}
        {models.data && !groups.length && <p className="mm-empty">{query ? "Ничего не найдено" : "pi не назвал ни одной модели"}</p>}
        {models.data && !models.data.catalogue && <p className="mm-empty">Список pi недоступен: показаны модели из настроек агентов.</p>}
      </div>
    </>
  );
}
