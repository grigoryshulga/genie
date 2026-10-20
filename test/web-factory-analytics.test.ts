import assert from "node:assert/strict";
import { test } from "node:test";
import { cohortOptions, metricBasis, metricValue } from "../web/src/pages/settings/ui/factory-model.ts";
import type { FactoryCohort } from "../web/src/shared/api/generated/FactoryCohort.ts";
import type { Metric } from "../web/src/shared/api/generated/Metric.ts";
const cohort = (dimension: string, key: string): FactoryCohort => ({ dimension, key, tasks: ["GNE-1"], metrics: {}, smallSample: true });

test("factory groups keep identifiers separate from readable choices and explain comparable conditions", () => {
  const hash = "abcdef".repeat(10);
  const options = cohortOptions([cohort("policy_version", hash), cohort("model", "provider/model")], "policy_version");
  assert.equal(options.length, 1);
  assert.equal(options[0]?.label, "Вариант 1");
  assert.equal(options[0]?.cohort.key, hash, "retain the exact filter key and audit identifier");
  const comparable = cohortOptions([cohort("comparable_routing", "solo; complexity=unknown; ambiguity=clear; blast_radius=high; verification=partial")], "comparable_routing")[0];
  assert.equal(comparable?.label, "Один исполнитель · условия 1");
  assert.deepEqual(comparable?.conditions, ["Сложность: нет оценки", "Ясность требований: требования ясны", "Масштаб последствий: высокая", "Проверяемость: проверяется частично"]);
});

test("factory cost coverage does not misrepresent accepted-delivery denominator as observed tasks", () => {
  const metric: Metric = { value: null, numerator: 3, denominator: 1, eligible: 3, missing: 1, unit: "USD" };
  assert.equal(metricBasis("costPerAcceptedTask", metric), "Стоимость известна для 2 из 3 задач; принято 1");
  assert.equal(metricValue(metric), "Недостаточно данных");
  assert.equal(metricValue({ ...metric, value: 0.001, unit: "hours" }), "< 1 мин");
  assert.equal(metricBasis("escalationRate", { ...metric, value: 0.5, unit: "fraction", numerator: 1, denominator: 2, eligible: 2, missing: 0 }), "1 из 2 попыток с известным результатом");
});
