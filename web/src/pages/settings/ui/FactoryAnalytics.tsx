import "./FactoryAnalytics.css";
import { useState } from "react";
import { useNavigate } from "react-router";
import type { ProjectStats } from "@/entities/project";
import { useSession, useSwitchProject } from "@/entities/session";
import type { FactoryAnalytics as Analytics } from "@/shared/api/generated/FactoryAnalytics.ts";

import { cohortOptions, DIMENSIONS, TECHNICAL_DIMENSIONS, METRIC_GROUPS, TEAM_NAMES, ROLE_NAMES, metricBasis, metricValue } from "./factory-model.ts";

const MISSING: Record<string, string> = { completed_first_review: "нет вердикта первого ревью", execution_attempts: "нет истории попыток", routing_history: "нет истории выбора команды", complete_priced_usage: "не все расходы известны", per_attempt_cost_attribution: "расходы не полностью привязаны к попыткам" };
const RESULTS: Record<string, string> = { accepted: "Принята", cancelled: "Отменена", failed: "Неудача", rejected: "Отклонена", unknown: "Результат неизвестен" };

export function FactoryAnalytics({ list }: { list: ProjectStats[] }) {
  return <div className="factory-analytics">
    <p className="muted">Как завершались задачи, сколько заняли времени и во что обошлись. Выберите группу, чтобы посмотреть её результаты.</p>
    {list.map((p) => p.factory ? <ProjectFactory key={p.project} project={p.project} name={p.name} data={p.factory} /> : <p key={p.project}>{p.name}: история выполнения пока недоступна.</p>)}
  </div>;
}
function ProjectFactory({ project, name, data }: { project: string; name: string; data: Analytics }) {
  const [dimension, setDimension] = useState("");
  const [selection, setSelection] = useState("");
  const [technical, setTechnical] = useState(false);
  const [metricGroup, setMetricGroup] = useState("result");
  const [error, setError] = useState("");
  const options = cohortOptions(data.cohorts, dimension);
  const option = options.find((o) => o.cohort.key === selection);
  const selected = option?.cohort;
  const group = METRIC_GROUPS.find((g) => g.id === metricGroup) ?? METRIC_GROUPS[0]!;
  const dimensions = [...DIMENSIONS, ...(technical ? TECHNICAL_DIMENSIONS : [])].filter((d) => data.cohorts.some((c) => c.dimension === d.id));
  const metrics = selected?.metrics ?? data.metrics;
  const tasks = selected ? data.tasks.filter((t) => selected.tasks.includes(t.task)) : data.tasks;
  const session = useSession().data;
  const switchProject = useSwitchProject();
  const navigate = useNavigate();
  const open = async (task: string) => {
    try { if (session?.project !== project) await switchProject(project); navigate(`/task/${encodeURIComponent(task)}`); }
    catch (e) { setError(e instanceof Error ? e.message : String(e)); }
  };
  return <section className="st-card" aria-label={`Аналитика ${name}`}>
    <div className="st-card-head"><h3>{name}</h3><p>Задачи, завершённые с {new Date(data.since).toLocaleDateString("ru-RU")} по {new Date(data.until).toLocaleDateString("ru-RU")}. Повторно завершённая задача учитывается один раз, по последнему результату.</p></div>
    <div className="factory-filters">
      <label>Группировать по<select className="st-input" value={dimension} onChange={(e) => { setDimension(e.target.value); setSelection(""); }}>
        <option value="">Все завершённые задачи</option>
        {dimensions.map((d) => <option key={d.id} value={d.id}>{d.label}</option>)}
      </select></label>
      {dimension && <label>Показать группу<select className="st-input" value={selection} onChange={(e) => setSelection(e.target.value)}>
        <option value="">Выберите группу</option>
        {options.map((o) => <option key={o.cohort.key} value={o.cohort.key}>{o.label} · задач: {o.cohort.tasks.length}</option>)}
      </select></label>}
    </div>
    <label className="factory-technical"><input type="checkbox" checked={technical} onChange={(e) => { setTechnical(e.target.checked); if (!e.target.checked && TECHNICAL_DIMENSIONS.some((d) => d.id === dimension)) { setDimension(""); setSelection(""); } }} />Показать версии и причины решений</label>
    {option && <div className="factory-selection"><strong>{option.label}</strong>{option.conditions.map((condition) => <span key={condition}>{condition}</span>)}
      {(["policy_version", "profile_version", "policy_by_signals", "router_version", "reason"].includes(dimension)) && <details><summary>Идентификатор для проверки истории</summary><code>{selected?.key.split("; ")[0]}</code></details>}
    </div>}
    <div className="factory-sample"><strong>Задач в выборке: {tasks.length}</strong>
      {tasks.length < 20 ? <span>Данных пока мало. Используйте результаты для разбора задач, а выводы о лучшей команде или политике отложите до накопления истории.</span> : <span>Сравнение описывает прошлые результаты и само по себе не доказывает преимущество команды или модели.</span>}
    </div>
    <div className="factory-highlights">{["deliveredTasks", "firstPassAcceptance", "executionHoursMedian", "costPerAcceptedTask"].map((key) => {
      const m = metrics[key];
      const title = { deliveredTasks: "Принято", firstPassAcceptance: "С первого ревью", executionHoursMedian: "Обычный срок", costPerAcceptedTask: "На принятую задачу" }[key];
      return m ? <div key={key}><span>{title}</span><strong>{metricValue(m)}</strong><small>{key === "deliveredTasks" ? `из ${m.eligible} завершённых задач` : m.value === null ? "Не хватает истории или сведений о расходах" : metricBasis(key, m)}</small></div> : null;
    })}</div>
    <div className="factory-sections" aria-label="Раздел показателей">{METRIC_GROUPS.map((g) => <button type="button" className="btn ghost sm" key={g.id} aria-pressed={metricGroup === g.id} onClick={() => setMetricGroup(g.id)}>{g.label}</button>)}</div>
    <div className="factory-scroll"><table className="factory-table"><caption>{group.label}</caption><thead><tr><th>Показатель</th><th>Результат</th><th>На каких данных</th></tr></thead><tbody>
      {group.metrics.map(([key = "", title, explanation]) => {
        const m = metrics[key];
        return m ? <tr key={key}><th scope="row">{title}<small>{explanation}</small></th><td>{metricValue(m)}</td><td>{metricBasis(key, m)}{m.missing > 0 && <small>Без данных: {m.missing}. Они исключены из расчёта; полная стоимость при пропусках не показывается.</small>}</td></tr> : null;
      })}
    </tbody></table></div>
    <details><summary>Задачи и попытки · {tasks.length}</summary>
      {error && <p role="alert">{error}</p>}
      {!tasks.length && <p className="muted">В выбранном периоде нет завершённых задач.</p>}
      {tasks.map((t) => <details key={t.task} className="factory-task"><summary>{t.task} · {t.title} · {RESULTS[t.result] ?? "Результат неизвестен"}</summary>
        <button type="button" className="btn ghost sm" onClick={() => void open(t.task)}>Открыть задачу и историю решений</button>
        <dl><dt>Попытки</dt><dd>{t.attempts.length || "История не записана"}</dd><dt>Команда</dt><dd>{TEAM_NAMES[t.initialTopology ?? "unknown"] ?? t.initialTopology} → {TEAM_NAMES[t.finalTopology ?? "unknown"] ?? t.finalTopology}</dd>
          <dt>Стоимость всей работы</dt><dd>{t.costUsd === null ? `Неполные данные; известно $${t.knownCostUsd.toFixed(4)}` : `$${t.costUsd.toFixed(4)}`}</dd>
          <dt>Покрытие</dt><dd>{t.missing.map((m) => MISSING[m] ?? "неполная история").join("; ") || "Полная история"}</dd></dl>
        <details><summary>Расходы по попыткам и моделям · {t.attemptCostCoverage === "complete" ? "полная история" : "неполная привязка к попыткам"}</summary>
          <div className="factory-scroll"><table className="factory-table"><thead><tr><th>Попытка</th><th>Стадия</th><th>Роль / модель</th><th>Вызовов</th><th>Стоимость</th></tr></thead><tbody>
            {t.costs.map((c, i) => <tr key={i}><td>{c.attemptId ?? "Вне попытки"}</td><td>{{ delivery: "Выполнение", refinement: "Подготовка", unknown: "Стадия неизвестна" }[c.stage] ?? c.stage}</td><td>{ROLE_NAMES[c.role ?? ""] ?? c.role ?? "Роль неизвестна"} / {c.model}</td><td>{c.calls}</td><td>{c.costUsd === null ? `Неполные данные: $${c.knownCostUsd.toFixed(4)}` : `$${c.costUsd.toFixed(4)}`}</td></tr>)}
          </tbody></table></div>
        </details>
      </details>)}
    </details>
    <details><summary>Границы измерений</summary><ul>{data.limitations.map((limitation) => <li key={limitation}>{limitation}</li>)}</ul></details>
  </section>;
}
