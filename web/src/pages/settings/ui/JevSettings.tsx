import { useState } from "react";
import { Link } from "react-router";
import { useAgentConfig } from "@/entities/agent-config";
import { type DecisionMode, type DecisionPolicy, type DecisionSettings, useDecisionSettings, useDecisions, useSaveDecisionPolicy } from "@/entities/decision";
import { useModels } from "@/entities/team";
import { useAct } from "@/shared/ui";

const MODES: { id: DecisionMode; title: string; hint: string }[] = [
  { id: "off", title: "Выключено", hint: "Jev не участвует в этом выборе." },
  { id: "observe", title: "Наблюдение", hint: "Оценка остаётся в журнале; менеджер действует как обычно." },
  { id: "advise", title: "Совет", hint: "Менеджер может запросить рекомендацию до запуска. Оценка при запуске также приходит ему сообщением." },
  { id: "joint", title: "Совместное решение", hint: "Уверенное расхождение требует пересмотра или записанной причины отклонения." },
  { id: "select", title: "Автоматический выбор", hint: "Jev выбирает среди разрешённых вариантов; сервер проверяет ограничения." },
];
const LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];
const toggle = (values: string[], id: string) => values.includes(id) ? values.filter((v) => v !== id) : [...values, id];

export function JevSettings({ slug, admin }: { slug: string; admin: boolean }) {
  const settings = useDecisionSettings(slug);
  return <div className="st-body">
    <div className="st-head"><h2>Решения с Jev</h2><p>Участие в подборе новой команды и инструмент, к которому агенты могут обратиться за дополнительной оценкой.</p></div>
    {settings.data ? <PolicyForm key={`${slug}:${JSON.stringify(settings.data.policy)}`} slug={slug} admin={admin} settings={settings.data} />
      : <div className="empty">{settings.error?.message ?? "Загрузка…"}</div>}
    <DecisionHistory slug={slug} />
  </div>;
}

function PolicyForm({ slug, admin, settings }: { slug: string; admin: boolean; settings: DecisionSettings }) {
  const [p, set] = useState(settings.policy);
  const save = useSaveDecisionPolicy(slug);
  const act = useAct();
  const catalogue = useAgentConfig().data;
  const models = useModels().data;
  const roles = catalogue?.roles ?? [];
  const dirty = JSON.stringify(p) !== JSON.stringify(settings.policy);
  const field = <K extends keyof DecisionPolicy>(key: K, value: DecisionPolicy[K]) => set((v) => ({ ...v, [key]: value }));
  const profile = (i: number, patch: Partial<DecisionPolicy["profiles"][number]>) => field("profiles", p.profiles.map((v, j) => i === j ? { ...v, ...patch } : v));
  return <form onSubmit={(e) => { e.preventDefault(); void act(() => save.mutateAsync(p), "Настройки Jev сохранены"); }}>
    {!settings.available && <p className="muted">Обращения к Jev отключены администратором сервера.</p>}
    <fieldset disabled={!admin || save.isPending} style={{ border: 0, padding: 0, margin: 0 }}>
      <legend className="sr-only">Настройки Jev</legend>
      <section className="st-card">
        <div className="st-card-head"><h3>Подключение</h3><p>Jev оценивает переданные ему факты и варианты. Подключение к совместимому System One API задаётся в конфигурации сервера.</p></div>
        <div className="st-row"><label className="lbl" htmlFor="jev-external"><b>Передавать контекст сервису решений</b><span>Разрешает внешние обращения для этого проекта.</span></label><div className="ctl"><input id="jev-external" type="checkbox" style={{ alignSelf: "flex-start" }} checked={p.allowExternal} onChange={(e) => field("allowExternal", e.target.checked)} /></div></div>
        <div className="st-row"><div className="lbl"><b>Подключённая модель</b><span>{settings.model} · ${settings.inputPrice} за миллион входных токенов.<br />{settings.endpoint ?? "Некорректный endpoint в конфигурации сервера"}</span></div><div className="ctl">{settings.serverKey ? <span className="muted">Ключ задан на сервере</span> : <Link to="/settings/me/keys">Ключ провайдера {settings.keyProvider}</Link>}</div></div>
      </section>
      <section className="st-card" style={{ marginTop: 20 }}>
        <div className="st-card-head"><h3>Влияние на новую команду</h3><p>Применяется к delivery-командам, которые запускает менеджер или автоматизация. Выбор человека сохраняется.</p></div>
        {([ ["team", "Состав команды"], ["model", "Модель участника"], ["thinking", "Уровень усилий"] ] as const).map(([key, label]) => <div className="st-row" key={key}>
          <label className="lbl" htmlFor={`jev-${key}`}><b>{label}</b><span>{MODES.find((m) => m.id === p[key])?.hint}</span></label>
          <div className="ctl"><select id={`jev-${key}`} value={p[key]} onChange={(e) => field(key, e.target.value as DecisionMode)}>{MODES.map((m) => <option key={m.id} value={m.id}>{m.title}</option>)}</select></div>
        </div>)}
        <div className="st-row"><div className="lbl"><b>Альтернативные шаблоны</b><span>Jev сравнит их с предложением менеджера. Сохраняются рабочее место и независимое ревью.</span></div><div className="ctl">
          {(catalogue?.teams ?? []).filter((t) => t.stage === "delivery").map((t) => <label key={t.id}><input type="checkbox" checked={p.templates.includes(t.id)} onChange={() => field("templates", toggle(p.templates, t.id))} /> {t.title || t.id}</label>)}
          {!p.templates.length && <span className="muted">Без альтернативных шаблонов состав остаётся прежним.</span>}
        </div></div>
      </section>
      <section className="st-card" style={{ marginTop: 20 }}>
        <div className="st-card-head"><h3>Разрешённые профили моделей</h3><p>Добавьте проверенные пары модели и усилий. В описании укажите возможности и случаи применения: Jev использует эти сведения при выборе.</p></div>
        <datalist id="jev-models">{models?.models.map((m) => <option key={m.id} value={m.id} />)}</datalist>
        {p.profiles.map((v, i) => <div key={i} className="st-row">
          <div className="lbl"><b>Профиль {i + 1}</b><span>Проверьте доступ к модели и поддержку выбранного уровня усилий.</span>{admin && <button type="button" className="btn ghost sm" onClick={() => field("profiles", p.profiles.filter((_, j) => j !== i))}>Удалить профиль</button>}</div>
          <div className="ctl">
            <label>Название<input className="st-input" value={v.id} maxLength={64} required onChange={(e) => profile(i, { id: e.target.value })} /></label>
            <label>Модель<input className="st-input" list="jev-models" value={v.model} placeholder="provider/model" required onChange={(e) => profile(i, { model: e.target.value })} /></label>
            <label>Усилия<select value={v.thinking ?? "off"} onChange={(e) => profile(i, { thinking: e.target.value })}>{LEVELS.map((l) => <option key={l}>{l}</option>)}</select></label>
            <label>Когда подходит<textarea rows={2} value={v.description} maxLength={2000} required onChange={(e) => profile(i, { description: e.target.value })} /></label>
            <span>Для ролей:</span>{roles.filter((r) => r.class !== "orchestrator").map((r) => <label key={r.id}><input type="checkbox" checked={v.roles.includes(r.id)} onChange={() => profile(i, { roles: toggle(v.roles, r.id) })} /> {r.title || r.id}</label>)}
          </div>
        </div>)}
        <div className="st-foot">{!p.profiles.length && <span className="muted">Без профилей модели и усилия остаются прежними.</span>}<span className="grow" />{admin && <button type="button" className="btn ghost" onClick={() => field("profiles", [...p.profiles, { id: "", model: "", thinking: "medium", description: "", roles: [] }])}>Добавить профиль</button>}</div>
      </section>
      <section className="st-card" style={{ marginTop: 20 }}>
        <div className="st-card-head"><h3>Инструмент для агентов</h3><p>Агент передаёт вопрос и варианты, получает оценку и сам решает, как её использовать. Инструмент не изменяет команду.</p></div>
        <div className="st-row"><label className="lbl" htmlFor="jev-tool"><b>Разрешить обращения к Jev</b><span>Работает независимо от автоматического подбора команды.</span></label><div className="ctl"><input id="jev-tool" type="checkbox" style={{ alignSelf: "flex-start" }} checked={p.toolEnabled} onChange={(e) => field("toolEnabled", e.target.checked)} /></div></div>
        <div className="st-row"><div className="lbl"><b>Разрешённые роли</b></div><div className="ctl">{roles.map((r) => <label key={r.id}><input type="checkbox" checked={p.toolRoles.includes(r.id)} onChange={() => field("toolRoles", toggle(p.toolRoles, r.id))} /> {r.title || r.id}</label>)}</div></div>
      </section>
      <section className="st-card" style={{ marginTop: 20 }}>
        <div className="st-card-head"><h3>Пороги и расход</h3><p>При неопределённости или недоступности Jev сохраняется предложение менеджера. Уверенность в выборе не доказывает успешность выполнения.</p></div>
        {([
          ["minConfidence", "Минимальная уверенность", 0, 1, 0.05], ["minMargin", "Минимальный отрыв от второго варианта", 0, 1, 0.05],
          ["maxCallsPerTask", "Вызовов на задачу", 1, 1000, 1], ["maxCostPerTaskUsd", "Лимит Jev на задачу, $", 0.001, 100, 0.001],
        ] as const).map(([key, label, min, max, step]) => <div className="st-row" key={key}><label className="lbl" htmlFor={`jev-${key}`}><b>{label}</b></label><div className="ctl"><input id={`jev-${key}`} className="st-input" type="number" required min={min} max={max} step={step} value={p[key]} onChange={(e) => field(key, e.target.valueAsNumber)} /></div></div>)}
      </section>
      {admin && <div className="st-foot"><span className="grow" /><button type="button" className="btn ghost" disabled={!dirty} onClick={() => set(settings.policy)}>Отменить</button><button className="btn primary" disabled={!dirty || save.isPending}>Сохранить настройки Jev</button></div>}
    </fieldset>
  </form>;
}

function DecisionHistory({ slug }: { slug: string }) {
  const history = useDecisions(slug);
  const names: Record<string, string> = { ok: "Оценка получена", uncertain: "Неопределённость", unavailable: "Недоступен", pending: "Выполняется" };
  const applied: Record<string, string> = { preview: "Предварительная оценка", prepared: "План подготовлен", spawned: "Команда запущена", "needs-reconsideration": "Нужен пересмотр" };
  return <section className="st-card" style={{ marginTop: 24 }}>
    <div className="st-card-head"><h3>История решений</h3><p>Последние 100 оценок проекта, включая обращения инструментом и решения при сборке команды.</p><button type="button" className="btn ghost sm" onClick={() => void history.refetch()}>Обновить</button></div>
    {history.error && <p role="alert">{history.error.message}</p>}
    {!history.data?.length && <div className="st-row"><span className="muted">{history.isPending ? "Загрузка…" : "Обращений к Jev пока нет."}</span></div>}
    {history.data?.map((r) => <details key={r.id} className="st-row" style={{ display: "block" }}>
      <summary>#{r.id} · {r.task} · {names[r.status] ?? r.status} · {r.costUsd == null ? `резерв $${r.reservedUsd.toFixed(6)}` : `$${r.costUsd.toFixed(6)}`} · {r.elapsedMs} мс</summary>
      <p>{r.actor} · {new Date(r.created).toLocaleString()} · {r.purpose}{r.application.state ? ` · ${applied[r.application.state] ?? r.application.state}` : ""}</p>
      {r.result.error && <p>{r.result.error}</p>}
      {Object.entries(r.result.answers ?? {}).map(([id, a]) => <div key={id}><b>{r.request.questions?.[id]?.instructions ?? id}</b><p>{a.choice ?? a.score ?? a.probability}{a.confidence == null ? "" : ` · уверенность ${(a.confidence * 100).toFixed(0)}%`}{a.qualified === false ? " · ниже порога применения" : ""}</p>{a.probabilities && <ul>{Object.entries(a.probabilities).map(([v, n]) => <li key={v}>{v}: {(n * 100).toFixed(1)}%</li>)}</ul>}</div>)}
      {r.application.override && <p>Причина отклонения: {r.application.override}</p>}
      {r.application.team && <Link to={`/team/${encodeURIComponent(r.application.team)}`}>Открыть команду</Link>}
      <details><summary>Контекст и запись решения</summary><pre style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere", fontSize: 12 }}>{JSON.stringify({ request: r.request, result: r.result, application: r.application }, null, 2)}</pre></details>
    </details>)}
  </section>;
}
