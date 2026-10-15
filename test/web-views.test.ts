// View helpers of the web that turn stored data into words: automation rules,
// a doc page's title and summary, status notes of the task activity.

import assert from "node:assert/strict";
import { test } from "node:test";
import { bodyUnderTitle, firstParagraph } from "../web/src/entities/doc/model.ts";
import { statusNote } from "../web/src/entities/task/model.ts";
import { condition, cronText, describeStep, describeTrigger, duration, untemplate } from "../web/src/entities/platform/model.ts";

test("a rule's trigger reads as words, not as its JSON", () => {
  assert.equal(
    describeTrigger({ on: { event: "task.status_changed", where: { "task.labels": { not_contains: "no-docs" }, "task.type": ["task", "bug"], to: "done" } } }),
    "Событие: статус → «Готово», без метки no-docs, тип: задача или баг",
  );
  assert.equal(describeTrigger({ on: { event: "task.created", where: { "actor.role": "human", status: "inbox" } } }), "Событие: задача создана человеком, в статусе «Входящие»");
  assert.equal(describeTrigger({ on: { schedule: "0 17 * * 5", tz: "Europe/Moscow" } }, false), "Расписание: по пятницам в 17:00, Europe/Moscow · выключено");
  assert.equal(describeTrigger({ on: { webhook: true } }, true, true), "Webhook: по вызову извне · пробный режим");
  assert.equal(describeTrigger({}), "Вручную");
  assert.equal(
    describeTrigger({ on: { event: "task.*", where: { "task.status": ["draft", "refining", "done"] } } }),
    "Событие: любое событие задачи, в статусе «Черновик», «Уточнение» или «Готово»",
  );
  assert.equal(
    describeTrigger({ on: { event: "task.status_changed", where: { to: "ready", "task.type": { not: "epic" } } } }),
    "Событие: статус → «Готово к работе», кроме эпиков",
  );
  assert.equal(condition("task.priority", { gt: 2 }), "task.priority > 2", "an unknown condition stays as written");
  assert.equal(condition("to", { not: ["done", "cancelled"] }), "статус → не «Готово» или «Отменено»");
});

test("common cron schedules read as words, others stay as written", () => {
  assert.equal(cronText("30 9 * * *"), "каждый день в 09:30");
  assert.equal(cronText("0 9 * * 1-5"), "по будням в 09:00");
  assert.equal(cronText("0 10 * * 1,4"), "по понедельникам и четвергам в 10:00");
  assert.equal(cronText("*/15 * * * *"), "каждые 15 мин");
  assert.equal(cronText("5 * * * *"), "каждый час в :05");
  assert.equal(cronText("0 0 1 * *"), "0 0 1 * *");
});

test("a rule's steps say who does what, placeholders say what they stand for", () => {
  const roles = { role: (id: string) => ({ documenter: "Документатор" })[id], template: (id: string) => ({ full: "Полная" })[id] };
  const agent = describeStep({ id: "docs", timeout: "45m", agent: { role: "documenter", workspace: "read-only", goal: "Task {{ event.task.id }} is done." } }, roles);
  assert.deepEqual(agent, { kind: "agent", title: "Документатор", note: "задание агенту · только чтение · до 45 мин", hint: "Task ‹№ задачи› is done." });
  assert.deepEqual(describeStep({ notify: { to: ["task.author", "project.admins"], title: "{{ event.task.id }} готова" } }), {
    kind: "notify",
    title: "Уведомление автору задачи и админам проекта",
    note: "‹№ задачи› готова",
  });
  assert.equal(describeStep({ "task.status": { to: "refining" } }).title, "Статус → «Уточнение»");
  assert.equal(describeStep({ "task.ready": {} }).title, "Статус → «Готово к работе», если задачу ничего не держит");
  assert.equal(describeStep({ ask: { to: ["event.actor"], from: "аналитика", remindAfter: "24h", timeout: "72h" } }).note, "от аналитика · напомнить через 24 ч · ждать 72 ч");
  assert.equal(describeStep({ team: { template: "full" } }, roles).title, "Команда «Полная»");
  assert.equal(describeStep({ http: { url: "https://hooks.example.com/x?y=1" } }).note, "hooks.example.com");
  assert.equal(untemplate("{{ steps.docs.output.changelog.text }}"), "‹changelog.text из шага docs›");
  assert.equal(duration("2d"), "2 дн");
  assert.equal(duration("soon"), "soon");
});

test("a doc page shows its title and summary once", () => {
  assert.equal(bodyUnderTitle("# Цены в возвратах\n\nКредит-нота.\n", "Цены в возвратах"), "Кредит-нота.\n");
  assert.equal(bodyUnderTitle("# Другое\n\nТекст\n", "Цены"), "# Другое\n\nТекст\n", "a heading other than the title stays");
  assert.equal(firstParagraph("# Заголовок\n\nПервая строка\nвторая строка\n\nДальше"), "Первая строка вторая строка");
  assert.equal(firstParagraph("\n\n"), undefined);
});

test("a status note of the activity is recognised", () => {
  assert.deepEqual(statusNote("[in_progress → review] Сдано на ревью"), { from: "in_progress", to: "review", note: "Сдано на ревью" });
  assert.equal(statusNote("Просто комментарий"), undefined);
});
