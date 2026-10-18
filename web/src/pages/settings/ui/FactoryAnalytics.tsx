import "./FactoryAnalytics.css";
import { useState } from "react";
import { useNavigate } from "react-router";
import type { ProjectStats } from "@/entities/project";
import { useSession, useSwitchProject } from "@/entities/session";
import type { FactoryAnalytics as Analytics } from "@/shared/api/generated/FactoryAnalytics.ts";
import type { Metric } from "@/shared/api/generated/Metric.ts";

const NAMES: Record<string, string> = {
  humanEscalationRate: "Усиления по решению человека", attempt_accepted_rate: "Принятые попытки", attempt_rejected_rate: "Отклонённые попытки", attempt_failed_rate: "Неудачные попытки", attempt_cancelled_rate: "Отменённые попытки",
  attemptTelemetryCoverage: "Покрытие истории попыток", deliveredTasks: "Доставлено задач", firstPassAcceptance: "Принятие с первого ревью", retryRate: "Повторные попытки",
  escalationRate: "Усиление команды", humanInterventionRate: "Вмешательство людей", ownerClarificationRate: "Вопросы владельцу",
  acceptanceRate: "Доля принятых", executionHoursMedian: "Выполнение: медиана", executionHoursP90: "Выполнение: P90",
  activeHoursMedian: "Активное время: медиана", waitingHoursMedian: "Ожидание: медиана", reviewRoundsMedian: "Завершённые раунды ревью: медиана",
  costPerAcceptedTask: "Стоимость принятой задачи", failedWorkCost: "Стоимость неудачной работы",
};
const DIMENSIONS: Record<string, string> = { initial_topology: "Начальная команда", final_topology: "Итоговая команда", model: "Модель", role: "Роль", policy_version: "Версия политики", router_version: "Версия маршрутизатора", reason: "Причина решения", profile_version: "Версия профиля", comparable_routing: "Команды при сопоставимых сигналах", policy_by_signals: "Политики при сопоставимых сигналах" };
function value(m: Metric) {
  if (m.value === null) return "Нет данных";
  if (m.unit === "fraction") return `${(m.value * 100).toFixed(1)}%`;
  if (m.unit === "USD") return `$${m.value.toFixed(4)}`;
  return `${m.value.toLocaleString("ru-RU", { maximumFractionDigits: 2 })}${m.unit === "hours" ? " ч" : ""}`;
}
export function FactoryAnalytics({ list }: { list: ProjectStats[] }) {
  return <div className="factory-analytics">
    <p className="muted">Наблюдаемые результаты задач. Сравнения описывают выборку и не доказывают преимущество команды или модели.</p>
    {list.map((p) => p.factory ? <ProjectFactory key={p.project} project={p.project} name={p.name} data={p.factory} /> : <p key={p.project}>{p.name}: история выполнения пока недоступна.</p>)}
  </div>;
}
function ProjectFactory({ project, name, data }: { project: string; name: string; data: Analytics }) {
  const [selection, setSelection] = useState("");
  const [error, setError] = useState("");
  const selected = data.cohorts.find((c) => `${c.dimension}:${c.key}` === selection);
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
    <div className="st-card-head"><h3>{name}</h3><p>{new Date(data.since).toLocaleString("ru-RU")} — {new Date(data.until).toLocaleString("ru-RU")}. Когорта: последнее наблюдаемое завершение каждой задачи за период; стоимость включает предшествующие попытки.</p></div>
    <label>Когорта <select className="st-input" value={selection} onChange={(e) => setSelection(e.target.value)}>
      <option value="">Все завершённые задачи</option>
      {data.cohorts.map((c) => <option key={`${c.dimension}:${c.key}`} value={`${c.dimension}:${c.key}`}>{DIMENSIONS[c.dimension] ?? c.dimension}: {c.key} · N={c.tasks.length}</option>)}
    </select></label>
    {tasks.length < 20 && <p className="muted">Малая выборка: N={tasks.length}. Для изменения политики нужны дополнительные данные.</p>}
    <div className="factory-scroll"><table className="factory-table"><thead><tr><th>Показатель</th><th>Значение</th><th>Наблюдений / всего</th><th>Нет данных</th></tr></thead><tbody>
      {Object.entries(metrics).map(([key, m]) => <tr key={key}><th scope="row">{NAMES[key] ?? key}</th><td>{value(m)}</td><td>{m.denominator} / {m.eligible}</td><td>{m.missing}</td></tr>)}
    </tbody></table></div>
    <details><summary>Задачи и попытки · {tasks.length}</summary>
      {error && <p role="alert">{error}</p>}
      {!tasks.length && <p className="muted">В выбранном периоде нет завершённых задач.</p>}
      {tasks.map((t) => <details key={t.task} className="factory-task"><summary>{t.task} · {t.title} · {t.result}</summary>
        <button type="button" className="btn ghost sm" onClick={() => void open(t.task)}>Открыть задачу и историю решений</button>
        <dl><dt>Попытки</dt><dd>{t.attemptIds.join(", ") || "Не измерено"}</dd><dt>Команда</dt><dd>{t.initialTopology ?? "Неизвестно"} → {t.finalTopology ?? "Неизвестно"}</dd>
          <dt>Стоимость всей работы</dt><dd>{t.costUsd === null ? `Неполные данные; известно $${t.knownCostUsd.toFixed(4)}` : `$${t.costUsd.toFixed(4)}`}</dd>
          <dt>Покрытие</dt><dd>{t.missing.join(", ") || "Полное"}</dd></dl>
        <details><summary>Стоимость по попыткам, стадиям и моделям · покрытие {t.attemptCostCoverage}</summary>
          <div className="factory-scroll"><table className="factory-table"><thead><tr><th>Попытка</th><th>Стадия</th><th>Роль / модель</th><th>Вызовов</th><th>Стоимость</th></tr></thead><tbody>
            {t.costs.map((c, i) => <tr key={i}><td>{c.attemptId ?? "Вне попытки"}</td><td>{c.stage}</td><td>{c.role ?? "Не измерено"} / {c.model}</td><td>{c.calls}</td><td>{c.costUsd === null ? `Неполные данные: $${c.knownCostUsd.toFixed(4)}` : `$${c.costUsd.toFixed(4)}`}</td></tr>)}
          </tbody></table></div>
        </details>
      </details>)}
    </details>
    <details><summary>Границы измерений</summary><ul>{data.limitations.map((limitation) => <li key={limitation}>{limitation}</li>)}</ul></details>
  </section>;
}
