// An empty state carries one character, and a slow page announces skeletons instead of printing a
// bare "Загрузка…" row.
import { expect, test } from "@playwright/test";
import { FLOW_DOC } from "./seed.ts";

test("the empty team tab shows one character and keeps its call to action", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/task/DM-2/team");
  const empty = page.locator(".g-empty");
  await expect(empty).toBeVisible();
  await expect(empty.getByRole("heading", { name: "Команда не собрана" })).toBeVisible();
  const character = empty.getByRole("img");
  await expect(character).toHaveCount(1);
  await expect(character).toHaveAttribute("alt", "Команда ещё не собрана");
  // One per view, not one per block (the top bar's agent status is a different surface).
  await expect(page.locator("main .g-character")).toHaveCount(1);
  await expect(empty.getByRole("button", { name: "Собрать команду" })).toBeVisible();
  await page.screenshot({ path: "test-results/empty-team-character.png", animations: "disabled" });
});

test("a slow page shows skeleton rows under one status instead of bare loading text", async ({ page }) => {
  await page.route("**/api/docs/page?path=*", async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 1200));
    await route.continue();
  });
  await page.goto(`/docs?page=${FLOW_DOC.path}`);
  const status = page.getByRole("status").filter({ hasText: "Загрузка страницы" });
  await expect(status).toBeVisible();
  await expect(status.locator(".g-skeleton").first()).toBeVisible();
  // The old bare row is gone.
  await expect(page.getByText("Загрузка страницы…", { exact: true })).toHaveCount(0);
  await expect(status).toBeHidden({ timeout: 20000 });
  await expect(page.getByRole("heading", { name: /Как запустить/ })).toBeVisible();
});

test("an empty task list uses the shared heading", async ({ page }) => {
  await page.route("**/api/tasks?*", (route) => route.fulfill({ json: [] }));
  await page.goto("/tasks");
  await expect(page.locator(".g-tasks-scroll .g-empty").getByRole("heading", { name: "Здесь пусто", exact: true })).toBeVisible();
});

test("a failed task list announces the error and retries", async ({ page }) => {
  let failed = true;
  await page.route("**/api/tasks?*", async (route) => {
    if (failed) {
      await route.fulfill({ status: 503, json: { error: "Temporary fixture failure" } });
    } else await route.continue();
  });
  await page.goto("/tasks");
  const error = page.locator(".g-tasks-scroll").getByRole("alert");
  await expect(error).toContainText("Не удалось загрузить задачи");
  failed = false;
  await error.getByRole("button", { name: "Повторить", exact: true }).click();
  await expect(error).toBeHidden();
  await expect(page.getByRole("list", { name: "Задачи", exact: true })).toBeVisible();
});

for (const [path, api, label] of [
  ["/observations", "**/api/observation/cases?*", "Загрузка наблюдений"],
  ["/observation/O-1", "**/api/observation/cases/1", "Загрузка наблюдения"],
]) {
  test(`${path} announces a skeleton while its data is pending`, async ({ page }) => {
    let release = () => {};
    const held = new Promise<void>((resolve) => { release = resolve; });
    await page.route(api, async (route) => {
      await held;
      await route.continue();
    });
    await page.goto(path);
    const loading = page.getByRole("status").filter({ hasText: label });
    await expect(loading).toBeVisible();
    await expect(loading.locator(".g-skeleton").first()).toBeVisible();
    release();
    await expect(loading).toBeHidden();
  });
}
