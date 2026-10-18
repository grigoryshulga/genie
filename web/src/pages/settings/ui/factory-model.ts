import type { FactoryCohort } from "../../../shared/api/generated/FactoryCohort.ts";
import type { Metric } from "../../../shared/api/generated/Metric.ts";

export const TEAM_NAMES: Record<string, string> = {
  solo: "Один исполнитель", pair: "Исполнитель и ревьюер", standard: "Аналитик, исполнитель и ревьюер",
  full: "Полная команда", custom: "Изменённая команда", unknown: "Команда не записана",
};
export const ROLE_NAMES: Record<string, string> = { executor: "Исполнитель", reviewer: "Ревьюер", analyst: "Аналитик", tester: "Тестировщик", documenter: "Документатор", orchestrator: "Координатор" };
export const REASONS: Record<string, string> = {
  delivery_dispatch: "Начало выполнения", scope_expansion: "Расширилась область изменений", topology_upgrade: "Команда усилена",
  sensitive_change: "Изменение затрагивает защищённую область", irreversible_change: "Трудно отменить изменение", verification_gap: "Не хватает проверок",
  review_return: "Возврат с ревью", requirement_conflict: "Противоречие в требованиях", owner_decision_required: "Нужно решение владельца",
  required_reviewer_capacity_exceeded: "Нет места для обязательного ревьюера", manual_member_removed: "Участник удалён вручную", manual_model_change: "Модель изменена вручную",
};
const SIGNALS: Record<string, string> = { complexity: "Сложность", ambiguity: "Ясность требований", blast_radius: "Масштаб последствий", verification: "Проверяемость", scope: "Область изменений", novelty: "Новизна", sensitivity: "Чувствительность", reversibility: "Обратимость" };
const VALUES: Record<string, string> = { unknown: "нет оценки", partially_ambiguous: "есть неясности", component: "один компонент", routine: "простая", moderate: "средняя", substantial: "сложная", extensive: "очень сложная", ordinary: "обычные изменения", security: "безопасность", privacy: "персональные данные", financial: "финансы", strong: "хорошо проверяется", partial: "проверяется частично", familiar: "знакомый подход", low: "низкая", medium: "средняя", high: "высокая", clear: "требования ясны", bounded: "изменения ограничены", testable: "есть проверка", needs_owner_decision: "нужно уточнение владельца", local: "локальная", cross_component: "несколько компонентов", protected: "защищённая область", missing: "проверок не хватает", known: "известный подход", novel: "новый подход" };
export const DIMENSIONS = [
  { id: "initial_topology", label: "Команда на старте" }, { id: "final_topology", label: "Команда к завершению" },
  { id: "model", label: "Модель" }, { id: "role", label: "Роль участника" },
  ...Object.entries(SIGNALS).map(([id, label]) => ({ id: `signal:${id}`, label })),
  { id: "comparable_routing", label: "Команды при одинаковых условиях" }, { id: "policy_by_signals", label: "Политики при одинаковых условиях" },
];
export const TECHNICAL_DIMENSIONS = [{ id: "policy_version", label: "Версия политики" }, { id: "profile_version", label: "Версия профиля проекта" }, { id: "router_version", label: "Версия маршрутизатора" }, { id: "reason", label: "Причина изменения команды" }];
export function cohortOptions(cohorts: FactoryCohort[], dimension: string) {
  const entries = cohorts.filter((c) => c.dimension === dimension);
  const heads = [...new Set(entries.map((c) => c.key.split("; ")[0]))];
  const conditions = [...new Set(entries.map((c) => c.key.split("; ").slice(1).join("; ")))];
  return entries.map((cohort, index) => {
    const [head = "", ...details] = cohort.key.split("; ");
    let label = cohort.key;
    if (dimension.endsWith("topology")) label = TEAM_NAMES[cohort.key] ?? cohort.key;
    if (dimension === "role") label = ROLE_NAMES[cohort.key] ?? cohort.key;
    if (dimension === "reason") label = REASONS[cohort.key] ?? "Другая причина";
    if (dimension.startsWith("signal:")) label = VALUES[cohort.key] ?? cohort.key;
    if (["profile_version", "policy_version"].includes(dimension)) label = `Вариант ${index + 1}`;
    if (dimension === "comparable_routing") label = `${TEAM_NAMES[head] ?? head} · условия ${conditions.indexOf(details.join("; ")) + 1}`;
    if (dimension === "policy_by_signals") label = `Политика ${heads.indexOf(head) + 1} · условия ${conditions.indexOf(details.join("; ")) + 1}`;
    return { cohort, label, conditions: details.map((part) => {
      const [name = "", value = ""] = part.split("=");
      return `${SIGNALS[name] ?? name}: ${VALUES[value] ?? value}`;
    }) };
  });
}
export function metricValue(m: Metric) {
  if (m.value === null) return "Недостаточно данных";
  if (m.unit === "fraction") return `${(m.value * 100).toLocaleString("ru-RU", { maximumFractionDigits: 1 })}%`;
  if (m.unit === "USD") return `$${m.value.toFixed(4)}`;
  if (m.unit === "hours" && m.value > 0 && m.value < 1 / 60) return "< 1 мин";
  if (m.unit === "hours" && m.value > 0 && m.value < 1) return `${Math.round(m.value * 60)} мин`;
  return `${m.value.toLocaleString("ru-RU", { maximumFractionDigits: 2 })}${m.unit === "hours" ? " ч" : ""}`;
}
export function metricBasis(key: string, m: Metric) {
  const unit = key === "escalationRate" || key === "humanEscalationRate" || key.startsWith("attempt_") ? "попыток" : "задач";
  // Cost per accepted task divides all known work by accepted deliveries; its
  // denominator is not a telemetry count (cancelled work remains in the cost).
  if (key === "costPerAcceptedTask") return `Стоимость известна для ${m.eligible - m.missing} из ${m.eligible} задач; принято ${m.denominator}`;
  if (m.unit === "fraction") return `${m.numerator} из ${m.denominator} ${unit} с известным результатом`;
  return `Измерено для ${m.denominator} из ${m.eligible} ${unit}`;
}
export const METRIC_GROUPS = [
  { id: "result", label: "Результаты", metrics: [
    ["deliveredTasks", "Принято задач", "Задачи, завершённые со статусом «Готово»."],
    ["acceptanceRate", "Доля принятых задач", "Принятые задачи среди всех завершённых, включая отменённые."],
    ["firstPassAcceptance", "Принято с первого ревью", "Первое завершённое ревью одобрено и задача принята. Без вердикта ревью результат неизвестен."],
    ["retryRate", "Задачи с повторной попыткой", "После завершения одной попытки задача снова запускалась в работу. Возврат с ревью сам по себе не создаёт новую попытку."],
    ["reviewRoundsMedian", "Обычно раундов ревью", "Медиана числа завершённых раундов для задач с известным первым вердиктом."],
    ["attemptTelemetryCoverage", "Задачи с историей выполнения", "Показывает, у какой части задач записана хотя бы одна попытка. У старых задач история может отсутствовать."],
  ] },
  { id: "time", label: "Время", metrics: [
    ["executionHoursMedian", "Обычно от старта до принятия", "Медиана времени принятой задачи с начала первой попытки; включает ожидание."],
    ["executionHoursP90", "Срок для 90% принятых задач", "90% измеренных задач уложились в это время (P90)."],
    ["activeHoursMedian", "Обычно в активной работе", "Медиана времени в статусе «В работе». Это календарное время, а не сумма часов всех агентов."],
    ["waitingHoursMedian", "Обычно в ожидании", "Медиана остального времени до принятия: ревью, уточнения и другие паузы."],
  ] },
  { id: "cost", label: "Стоимость", metrics: [
    ["costPerAcceptedTask", "Затраты на одну принятую задачу", "Все затраты выбранных задач, включая отмены, ревью и повторы, делятся на число принятых. Учитываются и затраты до начала периода."],
    ["failedWorkCost", "Затраты на непринятые попытки", "Стоимость отклонённых, неудачных и отменённых попыток. Нужна полная привязка расходов к попыткам."],
  ] },
  { id: "people", label: "Команда и люди", metrics: [
    ["escalationRate", "Попытки с усилением команды", "Доля попыток, где команде добавили участника после обнаружения новых обстоятельств."],
    ["humanEscalationRate", "Усиление по решению человека", "Доля попыток, в которых усиление команды запросил человек."],
    ["humanInterventionRate", "Задачи с участием человека", "Человек выполнял отслеживаемые действия с задачей; это не обязательно исправление ошибки агента."],
    ["ownerClarificationRate", "Задачи с вопросами владельцу", "В ходе работы был записан хотя бы один вопрос владельцу."],
  ] },
  { id: "attempts", label: "Исходы попыток", metrics: [
    ["attempt_accepted_rate", "Принятые попытки", "Завершились принятием задачи."],
    ["attempt_rejected_rate", "Отклонённые попытки", "Результат попытки был отклонён."],
    ["attempt_failed_rate", "Неудачные попытки", "Записан неуспешный исход выполнения."],
    ["attempt_cancelled_rate", "Отменённые попытки", "Выполнение было отменено."],
  ] },
];
