// Server statistics in the web: several projects' days and counts added up for the charts and tiles.

import assert from "node:assert/strict";
import { test } from "node:test";
import { costDays, type ProjectStats, statsTotals, sumDays } from "../web/src/entities/project/model.ts";
import { chatTitle, mergeSpends, moneyText, type Spend, spendText, tokensText, topItems } from "../web/src/entities/usage/model.ts";

const day = (d: string, created: number, done: number, runs = 0, runsFailed = 0) => ({ day: d, created, done, runs, runsFailed });

test("the days of several projects add up day by day, oldest first", () => {
  const shop = { daily: [day("2026-09-29", 2, 1, 10, 1), day("2026-09-30", 1, 0, 4)] };
  const wms = { daily: [day("2026-09-30", 3, 2, 6, 2), day("2026-09-29", 0, 1)] };
  assert.deepEqual(sumDays([shop, wms]), [day("2026-09-29", 2, 2, 10, 1), day("2026-09-30", 4, 2, 10, 2)]);
  assert.deepEqual(sumDays([]), []);
  assert.deepEqual(sumDays([{ daily: undefined as unknown as [] }]), [], "an older server sends no days");
});

test("the counts add up, and projects with open tasks are counted", () => {
  const p = (open: number, done: number) => ({ created: 3, createdByPeople: 2, done, open, decisions: 1, returns: 0, runs: 5, runsFailed: 1 }) as ProjectStats;
  const t = statsTotals([p(4, 2), p(0, 1)]);
  assert.deepEqual(t, { created: 6, createdByPeople: 4, done: 3, open: 4, decisions: 2, returns: 0, runs: 10, runsFailed: 2, openProjects: 1 });
});

test("the cost of several projects adds up by day and by model", () => {
  const d = (date: string, cost: number, byModel: Record<string, number>) => ({ ...day(date, 0, 0), cost, tokens: 100, costByModel: byModel });
  const shop = { daily: [d("2026-09-30", 3, { "a/opus": 2, "b/sol": 1 })] };
  const wms = { daily: [d("2026-09-30", 1, { "a/opus": 1 }), d("2026-09-29", 0.5, { "b/sol": 0.5 })] };
  assert.deepEqual(costDays([shop, wms]), [
    { day: "2026-09-29", cost: 0.5, tokens: 100, byModel: { "b/sol": 0.5 } },
    { day: "2026-09-30", cost: 4, tokens: 200, byModel: { "a/opus": 3, "b/sol": 1 } },
  ]);
  assert.deepEqual(costDays([{ daily: [day("2026-09-30", 1, 1)] }]), [{ day: "2026-09-30", cost: 0, tokens: 0, byModel: {} }], "an older server sends no cost");
});

const tokens = (input: number) => ({ input, output: 0, cacheRead: 0, cacheWrite: 0 });
const spend = (models: [string, number | null, number][]): Spend => ({
  calls: models.length,
  tokens: tokens(models.reduce((n, m) => n + m[2], 0)),
  cost: models.reduce((n, m) => n + (m[1] ?? 0), 0),
  unpricedTokens: models.filter((m) => m[1] === null).reduce((n, m) => n + m[2], 0),
  models: models.map(([model, cost, n]) => ({ model, calls: 1, tokens: tokens(n), cost, unpricedTokens: cost === null ? n : 0, price: null })),
});

test("spends merge by model, the most expensive first, unpriced models last", () => {
  const all = mergeSpends([spend([["a/opus", 2, 100], ["c/qwen", null, 50]]), spend([["b/sol", 3, 10], ["a/opus", 1, 20]])]);
  assert.equal(all.cost, 6);
  assert.equal(all.unpricedTokens, 50);
  assert.deepEqual(
    all.models.map((m) => [m.model, m.cost, m.tokens.input]),
    [["a/opus", 3, 120], ["b/sol", 3, 10], ["c/qwen", null, 50]],
  );
  assert.equal(mergeSpends([]).calls, 0);
});

test("money and tokens read short", () => {
  assert.equal(moneyText(12.4), "$12,40");
  assert.equal(moneyText(0.0423), "$0,042");
  assert.equal(moneyText(0), "$0");
  assert.equal(tokensText(1_400_000), "1,4 млн");
  assert.equal(tokensText(2_000_000), "2 млн");
  assert.equal(tokensText(12_345), "12 тыс.");
  assert.equal(spendText(spend([["c/qwen", null, 1500]])), "2 тыс. токенов · цена модели не указана");
  assert.deepEqual(chatTitle("SHOP-4/bender"), { name: "bender", sub: "команда SHOP-4" });
  assert.deepEqual(chatTitle("job/7"), { name: "Задание #7", sub: "разовое задание" });
  const top = topItems([{ project: "a", items: [{ id: "A-1", title: "", spend: spend([["m", 1, 1]]) }] }, { project: "b", items: [{ id: "B-1", title: "", spend: spend([["m", 5, 1]]) }] }], 1);
  assert.deepEqual(top.map((i) => [i.project, i.id]), [["b", "B-1"]]);
});
