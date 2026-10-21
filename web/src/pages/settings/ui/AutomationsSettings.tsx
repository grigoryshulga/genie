// «Автоматизации» — the rules of the current project: the list, the JSON editor,
// playbooks, enabling and deleting. Running a rule and watching its runs is the
// work side (/automations); defining rules is here.

import { SettingsTabs } from "./SettingsTabs.tsx";
import { useState } from "react";
import { useSearchParams } from "react-router";
import { useAgentConfig } from "@/entities/agent-config";
import { type Automation, describeStep, describeTrigger, RUN_STATUS, useAutomations, usePlaybooks } from "@/entities/platform";
import { request } from "@/shared/api";
import { timeAgo, useTick } from "@/shared/lib";
import { Icon, Modal, useRefreshingAct } from "@/shared/ui";
import "@/shared/ui/settings.css";
import { SettingsLoading } from "@/widgets/settings-shell";
// The settings pages share the split-pane rules with the platform pages, then the responsive and
// centring layers.
import "@/app/styles/pages.css";
import "./settings-sheets.css";

export function AutomationsSettingsPage() {
  useTick();
  const rules = useAutomations();
  const playbooks = usePlaybooks();
  const [sp, setSp] = useSearchParams();
  const [editing, setEditing] = useState<{ id?: number; text: string }>();
  const [showPlaybooks, setShowPlaybooks] = useState(false);
  const act = useRefreshingAct();
  const installed = new Set((rules.data ?? []).map((r) => r.name));
  const selected = Number(sp.get("rule") ?? "") || rules.data?.[0]?.id;
  const current = rules.data?.find((r) => r.id === selected);
  return (
    <main className="main">
      <header className="topbar">
        <h1>Автоматизации</h1>
        <span className="sub">правила проекта</span>
        <span className="grow" />
        {!!playbooks.data?.length && (
          <button type="button" className="g-btn secondary" onClick={() => setShowPlaybooks(true)}>
            Плейбуки
          </button>
        )}
        <button type="button" className="g-btn secondary" onClick={() => setEditing({ text: JSON.stringify(EMPTY_RULE, null, 2) })}>
          <Icon.plus size={13} />
          <span className="d-only">Новое правило</span>
        </button>
      </header>
        <SettingsTabs scope="project" section="tools" current="automations" />
      <div className="split">
        <section className="split-list" aria-label="Правила">
          {rules.data?.map((r) => (
            <button type="button" key={r.id} className={`pick${r.id === current?.id ? " on" : ""}`} onClick={() => setSp({ rule: String(r.id) })}>
              <span className="t">
                <span className={`state-dot ${r.enabled ? "ok" : "off"}`} />
                <span>{r.name}</span>
              </span>
              <span className="s">{describeTrigger(r.spec, r.enabled, r.dryRun)}</span>
              <span className="s">
                {r.lastRun ? `последний запуск ${timeAgo(r.lastRun.started)} · ${(RUN_STATUS[r.lastRun.status] ?? r.lastRun.status).toLowerCase()}` : "ещё не запускалось"}
              </span>
            </button>
          ))}
          {rules.isSuccess && !rules.data.length && (
            <p className="g-muted" style={{ margin: 0, padding: "var(--g-space-1) var(--g-space-4) var(--g-space-2)" }}>
              Правил пока нет — начните с плейбука: готового правила для частого случая.
            </p>
          )}
        </section>
        <section className="split-main" aria-label="Правило">
          {current ? (
            <RuleView key={current.id} rule={current} onEdit={() => setEditing({ id: current.id, text: JSON.stringify(current.spec, null, 2) })} />
          ) : (
            <div className="pane-empty">
              <Icon.bolt size={22} />
              {rules.isPending ? <SettingsLoading /> : "Выберите правило или добавьте плейбук"}
            </div>
          )}
        </section>
      </div>
      {editing && <RuleEditor initial={editing} onClose={() => setEditing(undefined)} />}
      {showPlaybooks && (
        <Modal label="Плейбуки" onClose={() => setShowPlaybooks(false)}>
          <div className="mh">
            Плейбуки — готовые правила для частых случаев
            <span className="grow" />
            <button type="button" className="g-icon-btn" onClick={() => setShowPlaybooks(false)} aria-label="Закрыть">
              <Icon.close />
            </button>
          </div>
          <ul className="mb playbooks">
            {playbooks.data?.map((p) => {
              const added = installed.has(specName(p.spec));
              return (
                <li key={p.id}>
                  <span className="t">{p.title}</span>
                  <span className="s">{describeTrigger(p.spec)}</span>
                  {added ? (
                    <span className="g-muted">добавлен</span>
                  ) : (
                    <button type="button" className="g-btn secondary" onClick={() => void act(() => request("POST", `/api/automations/playbooks/${p.id}`), "Правило добавлено")}>
                      Добавить
                    </button>
                  )}
                </li>
              );
            })}
          </ul>
        </Modal>
      )}
    </main>
  );
}

const EMPTY_RULE = {
  name: "Новое правило",
  on: { event: "task.status_changed", where: { to: "done" } },
  steps: [{ id: "tell", notify: { to: ["task.author"], title: "{{ event.task.id }} готова", text: "{{ event.task.title }}" } }],
};

/** The name of a rule spec — parsed JSON, so the field is read only after checking it is there. */
function specName(spec: unknown): string {
  if (spec && typeof spec === "object" && "name" in spec && typeof spec.name === "string") return spec.name;
  return "";
}

/** The steps of a rule spec — parsed JSON with an optional `steps` array. */
function specSteps(spec: unknown): Record<string, unknown>[] {
  if (spec && typeof spec === "object" && "steps" in spec && Array.isArray(spec.steps)) return spec.steps;
  return [];
}

function RuleView({ rule, onEdit }: { rule: Automation; onEdit: () => void }) {
  const act = useRefreshingAct();
  const cfg = useAgentConfig().data;
  const steps = specSteps(rule.spec).map((st) =>
    describeStep(st, {
      role: (id) => cfg?.roles.find((r) => r.id === id)?.title,
      template: (id) => cfg?.teams.find((t) => t.id === id)?.title,
    }),
  );
  return (
    <>
      <div className="pane-head">
        <div className="ttl">
          <h2>{rule.name}</h2>
          <span className="sub">{describeTrigger(rule.spec, rule.enabled, rule.dryRun)}</span>
        </div>
        <label className="toggle">
          <input type="checkbox" checked={rule.enabled} onChange={(e) => void act(() => request("POST", `/api/automations/${rule.id}/enabled`, { enabled: e.target.checked }))} />
          Включено
        </label>
        <button type="button" className="g-btn secondary" onClick={onEdit}>
          Изменить
        </button>
      </div>
      <div className="pane-body">
        <p className="g-muted" style={{ marginTop: 0 }}>
          Запустить правило и посмотреть его запуски — на странице «Автоматизации» в рабочей части.
        </p>
        {steps.length > 0 && (
          <ol className="steps" aria-label="Шаги">
            {steps.map((st, i) => (
              <li key={i} title={st.hint}>
                <span className="k">{st.kind}</span>
                <span className="d">{st.title}</span>
                {st.note && <span className="n">{st.note}</span>}
              </li>
            ))}
          </ol>
        )}
      </div>
    </>
  );
}

function RuleEditor({ initial, onClose }: { initial: { id?: number; text: string }; onClose: () => void }) {
  const agents = useAgentConfig().data;
  const [text, setText] = useState(initial.text);
  const [error, setError] = useState<string>();
  const act = useRefreshingAct();
  const save = () =>
    void act(async () => {
      const spec = JSON.parse(text);
      if (initial.id) await request("PUT", `/api/automations/${initial.id}`, { spec });
      else await request("POST", "/api/automations", { spec });
      onClose();
    }, "Правило сохранено");
  const remove = () => void act(() => request("DELETE", `/api/automations/${initial.id}`), "Правило удалено").then((ok) => ok && onClose());
  return (
    <Modal label={initial.id ? "Правило (JSON)" : "Новое правило (JSON)"} onClose={onClose} wide>
      <div className="mb">
        <p className="g-muted" style={{ margin: 0 }}>
          Триггер: <code>event</code> (+ <code>where</code>), <code>schedule</code> (cron, <code>tz</code>), <code>manual</code>, <code>webhook</code>. Шаги: task.status, task.comment, task.create,
          task.update, task.get, task.ready, notify, agent, team, ask, wait, wake_orchestrator, changelog.add, release, http. Подстановки: <code>{"{{ event.task.id }}"}</code>,{" "}
          <code>{"{{ steps.<id>.output.… }}"}</code>.
        </p>
        {agents && (
          <p className="g-muted" style={{ margin: 0 }}>
            Шаблоны для <code>team.template</code>: {agents.teams.map((t) => t.id).join(", ")}. Роли для <code>agent.role</code>:{" "}
            {agents.roles
              .filter((r) => r.class !== "orchestrator")
              .map((r) => r.id)
              .join(", ")}
            .
          </p>
        )}
        <textarea className="g-mono code-edit" rows={22} value={text} onChange={(e) => setText(e.target.value)} spellCheck={false} />
        {error && <div className="auth-error">{error}</div>}
      </div>
      <div className="mf">
        {initial.id && (
          <button type="button" className="g-btn danger" onClick={remove}>
            Удалить
          </button>
        )}
        <span className="grow" />
        <button type="button" className="g-btn secondary" onClick={onClose}>
          Отмена
        </button>
        <button type="button" className="g-btn secondary" onClick={save}>
          Сохранить
        </button>
      </div>
    </Modal>
  );
}
