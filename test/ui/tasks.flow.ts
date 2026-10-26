// The task workflow from the owner's side: create, find, and work a task through its detail page.

import { expect, test } from "@playwright/test";
import { storedTask } from "./api.ts";
import { SEED_TASKS } from "./seed.ts";

test("the new-task dialog creates a task with its criteria and labels", async ({ page }) => {
  await page.goto("/tasks");
  await page.getByRole("button", { name: /Новая задача/ }).click();

  const dialog = page.getByRole("dialog", { name: "Новая задача", exact: true });
  await expect(dialog.getByText("Новая задача во входящие")).toBeVisible();
  const create = dialog.getByRole("button", { name: /Создать/ });
  await expect(create).toBeDisabled();

  await dialog.getByLabel("Название").fill("Проверить тёмную тему");
  await dialog.getByLabel("Описание").fill("Все экраны читаются в тёмной теме.");
  await dialog.getByLabel(/Критерии приёмки/).fill("Контраст текста не ниже AA\nИконки видны на тёмном фоне");
  await dialog.getByLabel("Метки").fill("ui, theme");
  await create.click();

  await expect(dialog).toBeHidden();
  const taskDialog = page.getByRole("dialog", { name: /^Задача DM-/ });
  await expect(taskDialog).toBeVisible();
  const id = new URL(page.url()).searchParams.get("task");
  expect(id, "the task query names the created task").toMatch(/^DM-\d+$/);

  // Creation opens the task above the list; closing reveals the saved row.
  await expect(page).toHaveURL(new RegExp(`[?&]task=${id}`));
  await expect(taskDialog.getByRole("heading", { name: "Проверить тёмную тему" })).toBeVisible();

  const stored = await storedTask(page.request, id!);
  expect(stored).toMatchObject({ title: "Проверить тёмную тему", status: "inbox", labels: ["ui", "theme"] });
  expect(stored.acceptance.map((a) => a.text)).toEqual(["Контраст текста не ниже AA", "Иконки видны на тёмном фоне"]);
  await taskDialog.getByRole("button", { name: "Закрыть", exact: true }).click();
  await expect(page.getByRole("listitem").filter({ hasText: "Проверить тёмную тему" })).toBeVisible();
});

test("cancelling the new-task dialog creates nothing", async ({ page }) => {
  await page.goto("/tasks");
  const rows = page.getByRole("list", { name: "Задачи" }).getByRole("listitem");
  await expect(rows.filter({ hasText: SEED_TASKS[3].title })).toBeVisible();
  const before = await rows.count();
  await page.getByRole("button", { name: /Новая задача/ }).click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Название").fill("Эта задача не должна появиться");
  await dialog.getByRole("button", { name: "Отмена" }).click();

  await expect(dialog).toBeHidden();
  await expect(page.getByText("Эта задача не должна появиться")).toHaveCount(0);
  await expect(rows).toHaveCount(before);
});

test("the search narrows the task list and keeps the query in the address", async ({ page }) => {
  await page.goto("/tasks");
  const list = page.getByRole("list", { name: "Задачи" });
  await expect(list.getByText(SEED_TASKS[3].title)).toBeVisible();

  await page.getByRole("button", { name: "Открыть поиск" }).click();
  const search = page.getByRole("searchbox", { name: "Поиск задач" });
  await search.fill("фильтруется");
  await expect(page).toHaveURL(/[?&]q=/);
  await expect(list.getByRole("listitem")).toHaveCount(1);
  await expect(list.getByText(SEED_TASKS[2].title)).toBeVisible();

  // The address alone restores the search.
  await page.reload();
  await page.getByRole("button", { name: "Открыть поиск" }).click();
  await expect(search).toHaveValue("фильтруется");
  await expect(list.getByRole("listitem")).toHaveCount(1);

  await search.press("Escape");
  await expect(page).not.toHaveURL(/[?&]q=/);
  await expect(list.getByText(SEED_TASKS[3].title)).toBeVisible();
});

test("the task rail folds its cards and keeps the team reachable", async ({ page }) => {
  await page.goto("/task/DM-2");
  await expect(page.getByRole("heading", { name: SEED_TASKS[1].title })).toBeVisible();
  await expect(page.locator(".tp-tabs")).toHaveCount(0);

  // Folding is remembered across a reload; expanding brings the properties back.
  const properties = page.locator(".tp-properties");
  const toggle = properties.locator(".tp-card-toggle");
  await expect(properties.getByText("Статус", { exact: true })).toBeVisible();
  await toggle.click();
  await expect(properties.getByText("Статус", { exact: true })).toBeHidden();
  await page.reload();
  await expect(properties.getByText("Статус", { exact: true })).toBeHidden();
  await toggle.click();
  await expect(properties.getByText("Статус", { exact: true })).toBeVisible();

  // The team opens from the rail and comes back through the arrow beside the title.
  await page.locator(".tp-team-card").getByRole("link", { name: /Перейти к команде/ }).click();
  await expect(page).toHaveURL(/\/task\/DM-2\/team$/);
  await expect(page.getByRole("link", { name: "Перейти к команде" })).toHaveCount(0);
  await page.getByRole("link", { name: "К описанию задачи" }).click();
  await expect(page).toHaveURL(/\/task\/DM-2$/);
  await expect(page.getByRole("heading", { name: SEED_TASKS[1].title })).toBeVisible();
});

test("ticking a criterion and changing the status on the task page are saved", async ({ page }) => {
  await page.goto("/task/DM-2");
  await expect(page.getByRole("heading", { name: SEED_TASKS[1].title })).toBeVisible();

  const criterion = page.getByRole("checkbox", { name: SEED_TASKS[1].acceptance![0] });
  const criteria = page.getByRole("button", { name: /Критерии приёмки/ });
  await expect(criteria).toContainText("0 из 2");
  // The checkbox follows the server's answer, not the click: wait for the saved state.
  await criterion.click();
  await expect(criterion).toBeChecked();
  await expect(criteria).toContainText("1 из 2");

  const status = page.getByLabel("Статус", { exact: true });
  await status.selectOption("review");
  await expect(page.getByText("В работе → На ревью")).toBeVisible();

  const stored = await storedTask(page.request, "DM-2");
  expect(stored.status).toBe("review");
  expect(stored.acceptance.map((a) => a.done)).toEqual([true, false]);

  // What the page shows after a reload comes from the server.
  await page.reload();
  await expect(criterion).toBeChecked();
  await expect(status).toHaveValue("review");
});

for (const width of [1440, 390]) {
  test(`create form animates on opening while preserving focus at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/tasks");
    const plus = page.locator(".g-topbar-create");
    const dialog = page.getByRole("dialog", { name: "Новая задача", exact: true });
    const surface = dialog;
    await plus.click();
    await expect(dialog).toBeVisible();
    await expect(surface).toHaveCSS("animation-name", "g-new-task-appear");
    await expect.poll(() => surface.evaluate((node) => node.scrollWidth <= node.clientWidth)).toBe(true);
    await expect.poll(() => surface.evaluate((node) => getComputedStyle(node).transform)).toBe("none");
    const bounds = (await dialog.boundingBox())!;
    for (const button of await dialog.locator(".mh button, .mf button").all()) {
      const box = (await button.boundingBox())!;
      expect(box.x).toBeGreaterThanOrEqual(bounds.x);
      expect(box.x + box.width).toBeLessThanOrEqual(bounds.x + bounds.width);
    }
    await expect.poll(() => dialog.evaluate((node) => node.contains(document.activeElement))).toBe(true);
    await dialog.getByLabel("Название", { exact: true }).fill("Проверить появление формы");
    // Entry must end without a persistent transform/containing block over form popups.
    await expect.poll(() => surface.evaluate((node) => getComputedStyle(node).transform)).toBe("none");
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(plus).toBeFocused();
    await plus.click();
    await expect(surface).toHaveCSS("animation-name", "g-new-task-appear");
    await dialog.getByRole("button", { name: "Отмена", exact: true }).click();
    await expect(plus).toBeFocused();
    await page.emulateMedia({ reducedMotion: "reduce" });
    await plus.click();
    await expect(surface).toHaveCSS("animation-name", "none");
    expect(await dialog.evaluate((node) => getComputedStyle(node, "::backdrop").animationName)).toBe("none");
    await page.keyboard.press("Escape");
    await expect(plus).toBeFocused();
  });
}
