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
