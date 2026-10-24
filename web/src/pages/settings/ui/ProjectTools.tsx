import { useState } from "react";
import { useProjectTools, useSaveProjectTools } from "@/entities/agent-config";
import type { ProjectTools } from "@/shared/api/generated/ProjectTools.ts";
import { useAct } from "@/shared/ui";
import { JevSettings } from "./JevSettings.tsx";
import { SettingsLoading } from "@/widgets/settings-shell";

export function ProjectToolsSection({ slug, tab }: { slug: string; tab: string }) {
  const query = useProjectTools(slug);
  return (
    <>
      <div hidden={tab === "jev"}>
        <div className="st-body">
          <div className="st-head">
            <h2>Инструменты проекта</h2>
            <p>Выберите навыки и MCP из каталога сервера. Разрешения ролей и ограничения сервера продолжают действовать.</p>
          </div>
          {query.data ? (
            <Selection key={`${slug}:${query.data.hash}`} slug={slug} data={query.data} kind={tab === "mcp" ? "mcp" : "skills"} />
          ) : (
            query.error ? <p role="status">{query.error.message}</p> : <SettingsLoading />
          )}
        </div>
      </div>
      <div hidden={tab !== "jev"}>
        <JevSettings key={slug} slug={slug} admin />
      </div>
    </>
  );
}

function Selection({ slug, data, kind }: { slug: string; kind: "skills" | "mcp"; data: NonNullable<ReturnType<typeof useProjectTools>["data"]> }) {
  const [settings, set] = useState<ProjectTools>(data.settings);
  const [busy, setBusy] = useState(false);
  const save = useSaveProjectTools(slug);
  const act = useAct();
  const dirty = JSON.stringify(settings) !== JSON.stringify(data.settings);
  return (
    <form
      className="st-col"
      onSubmit={(e) => {
        e.preventDefault();
        setBusy(true);
        void act(() => save({ settings, baseHash: data.hash }), "Инструменты проекта сохранены").finally(() => setBusy(false));
      }}
    >
      {[kind].map((kind) => {
        const items = kind === "skills" ? data.skills : data.mcp;
        const selected = settings[kind];
        return (
          <section className="st-card" key={kind}>
            <div className="st-card-head">
              <h3>{kind === "skills" ? "Навыки проекта" : "MCP-серверы проекта"}</h3>
            </div>
            <div className="st-card-body">
              <label className="pj-check">
                <input
                  type="checkbox"
                  checked={selected == null}
                  onChange={(e) => set({ ...settings, [kind]: e.target.checked ? null : items.map((s) => s.id) })}
                />
                Все доступные на сервере, включая новые
              </label>
              {items.map((item) => (
                <label className="st-row" key={item.id}>
                  <span className="lbl">
                    <b>{item.id}</b>
                    <span>{item.description}</span>
                  </span>
                  <input
                    type="checkbox"
                    aria-label={`${kind === "skills" ? "Навык" : "MCP"} ${item.id}`}
                    checked={selected == null || selected.includes(item.id)}
                    onChange={(e) => {
                      const current = selected ?? items.map((s) => s.id);
                      set({ ...settings, [kind]: e.target.checked ? [...current, item.id] : current.filter((id) => id !== item.id) });
                    }}
                  />
                </label>
              ))}
              {!items.length && <p className="g-muted">В каталоге сервера пока нет доступных инструментов.</p>}
              {selected
                ?.filter((id) => !items.some((item) => item.id === id))
                .map((id) => (
                  <p key={id}>
                    {id} — недоступен{" "}
                    <button type="button" className="g-btn ghost" onClick={() => set({ ...settings, [kind]: selected.filter((s) => s !== id) })}>
                      Убрать из проекта
                    </button>
                  </p>
                ))}
            </div>
          </section>
        );
      })}
      <div className="st-foot">
        <span className="grow" />
        <button type="button" className="g-btn secondary" disabled={!dirty || busy} onClick={() => set(data.settings)}>
          Отменить
        </button>
        <button className="g-btn secondary" disabled={!dirty || busy}>
          Сохранить
        </button>
      </div>
    </form>
  );
}
