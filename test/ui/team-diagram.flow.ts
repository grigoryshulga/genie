// The team page shows its members as one line in handoff order; the graph of relations opens on demand,
// its live cards separate identity from activity and open the corresponding conversation.
import { expect, test } from "@playwright/test";
import type { TaskSummary } from "../../web/src/shared/api/types.ts";

test("the team line shows who works and who waits; the graph opens on demand and its cards open the agent chat", async ({ page }) => {
  const response = await page.request.get("/api/tasks?closed=1");
  expect(response.ok()).toBe(true);
  const tasks: TaskSummary[] = await response.json();
  const task = tasks[0];
  const at = new Date().toISOString();
  const team = {
    id: "diagram-test", task: task.id, cwd: "/tmp", state: "active", created: at, updated: at,
    members: [
      { name: "Бендер", role: "executor", state: "active", activity: "working", status: "Проверяет тесты", statusAt: at },
      { name: "Лиза", role: "reviewer", state: "active", activity: "idle", status: "", statusAt: at },
    ],
    spec: {
      stage: "delivery", workspace: "scratch", mail: "flow",
      members: [{ key: "executor", name: "Бендер", role: "executor" }, { key: "reviewer", name: "Лиза", role: "reviewer" }],
      relations: [{ from: "executor", to: ["reviewer"], type: "handoff", on: "review" }],
    },
    taskInfo: { id: task.id, title: task.title, status: "in_progress" },
    pending: {}, mail: [], log: [],
  };
  await page.route("**/api/tasks?*", (route) => route.fulfill({ json: tasks.map((t) => t.id === task.id ? { ...t, team: team.id } : t) }));
  await page.route("**/api/teams?*", (route) => route.fulfill({ json: [team] }));
  await page.route(`**/api/teams/${team.id}`, (route) => route.fulfill({ json: team }));
  await page.goto(`/task/${task.id}/team`);
  const line = page.getByRole("list", { name: "Порядок работы" });
  await expect(line.locator(".tv-flow-member")).toHaveText([/Бендер.*работает/, /Лиза.*ждёт Бендер/]);
  // Hovering the line changes no row's height.
  const before = (await line.boundingBox())!.height;
  await line.locator(".tv-flow-member").first().hover();
  expect((await line.boundingBox())!.height).toBe(before);
  await expect(page.locator(".team-graph")).toHaveCount(0);
  await page.getByRole("button", { name: "Схема связей" }).click();
  const reviewer = page.locator(".team-graph a").filter({ hasText: "Лиза" });
  await expect(reviewer.locator("text.nr")).toHaveText("ревьюер");
  await expect(reviewer.locator("text").filter({ hasText: /^ждёт Бендер$/ })).toHaveCount(1);
  const executor = page.locator(".team-graph a").filter({ hasText: "Бендер" }).filter({ has: page.locator("text.nr", { hasText: "исполнитель" }) });
  await expect(executor.locator("text.nr")).toHaveText("исполнитель");
  await expect(executor.locator("text.ns")).toHaveText("Проверяет тесты");
  await expect(page.locator(".team-graph text.nl").filter({ hasText: /^Менеджер проекта$/ })).toBeVisible();
  await expect(page.locator(".team-graph text.nl").filter({ hasText: /^Менеджер задачи$/ })).toBeVisible();
  const executorBox = await executor.boundingBox();
  const reviewerBox = await reviewer.boundingBox();
  expect(executorBox!.x).toBeLessThan(reviewerBox!.x);
  const graph = page.locator(".team-graph svg");
  const originalWidth = (await graph.boundingBox())!.width;
  await page.getByRole("button", { name: "Отдалить схему" }).click();
  expect((await graph.boundingBox())!.width).toBeLessThan(originalWidth);
  await page.getByRole("button", { name: "Приблизить схему" }).click();
  expect((await graph.boundingBox())!.width).toBeCloseTo(originalWidth, 0);
  await page.getByRole("button", { name: "Развернуть схему", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Развёрнутая схема команды" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Вписать целиком" }).click();
  expect((await graph.boundingBox())!.width).toBeLessThanOrEqual((await dialog.boundingBox())!.width);
  await page.keyboard.press("Escape");
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Развернуть схему", exact: true })).toBeFocused();
  await reviewer.click();
  await expect(page).toHaveURL(new RegExp(`/team/${team.id}/${encodeURIComponent("Лиза")}$`));
});
