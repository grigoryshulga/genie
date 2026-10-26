import { expect, test } from "@playwright/test";

for (const theme of ["ink", "paper"]) {
  for (const width of [1440, 390]) {
    test(`compact shell and task header (${theme}, ${width})`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
      await page.goto("/board");
      const bar = page.locator(".g-topbar");
      await expect(bar.locator(".g-dock")).toHaveCount(0);
      await expect(bar.getByRole("button", { name: "Сменить тему" })).toHaveCount(0);
      await expect(page.getByRole("toolbar", { name: "Инструменты страницы" }).getByRole("button", { name: "Готово", exact: true })).toHaveCount(0);
      await expect(page.getByRole("toolbar", { name: "Инструменты страницы" })).toBeVisible();
      await expect(page.locator("[data-lane]").first()).toBeVisible();
      const flip = theme === "paper" ? "ink" : "paper";
      if (width > 900) {
        const create = bar.getByRole("button", { name: "Новая задача", exact: true });
        await expect(create).toHaveText("");
        await create.click();
        await expect(page.getByRole("dialog")).toBeVisible();
        await page.keyboard.press("Escape");
        const profile = bar.getByRole("button", { name: /^Профиль:/ });
        await profile.click();
        const light = page.getByRole("menuitemcheckbox", { name: "Светлая тема" });
        await expect(light).toHaveAttribute("aria-checked", String(theme === "paper"));
        await light.click();
        await expect(page.locator("html")).toHaveAttribute("data-theme", flip);
        await expect(profile).toBeFocused();
        await profile.click();
        await expect(light).toHaveAttribute("aria-checked", String(theme !== "paper"));
        await light.click();
      } else {
        // On a phone the bar sits at the bottom; creating a task and the theme live in the sheet behind «Ещё».
        const more = bar.getByRole("button", { name: "Открыть меню разделов" });
        await more.click();
        const sheet = page.getByRole("dialog", { name: "Навигация" });
        await sheet.getByRole("button", { name: "Новая задача" }).click();
        await expect(page.getByRole("dialog", { name: "Новая задача" })).toBeVisible();
        await page.keyboard.press("Escape");
        await more.click();
        await sheet.getByRole("button", { name: "Светлая тема" }).click();
        await expect(page.locator("html")).toHaveAttribute("data-theme", flip);
        await more.click();
        await sheet.getByRole("button", { name: "Светлая тема" }).click();
      }
      await page.goto("/tasks?task=DM-5");
      const modal = page.getByRole("dialog", { name: "Задача DM-5", exact: true });
      const header = modal.locator(".td-bar");
      await expect(header.getByRole("heading", { name: "Изучить визуальную регрессию" })).toBeVisible();
      await expect(modal.locator(".td-id")).toHaveCount(1);
      await expect(modal.locator(".detail-body .td-headline")).toHaveCount(0);
      const open = header.getByRole("link", { name: "Открыть страницу", exact: true });
      await expect(open).toHaveText("");
      const follow = header.getByRole("button", { name: "Следить", exact: true });
      await expect(follow).toHaveAttribute("aria-pressed", "true");
      await expect(follow).toBeDisabled(); // The seed's owner authored the task; server notifications are automatic.
      const box = (await header.boundingBox())!;
      const linkBox = (await open.boundingBox())!;
      expect(linkBox.x + linkBox.width).toBeLessThanOrEqual(box.x + box.width);
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
      await page.screenshot({ path: `test-results/header-controls-${theme}-${width}.png`, animations: "disabled" });
      await open.click();
      await expect(page).toHaveURL(/\/task\/DM-5$/);
    });
  }
}

test("follow eye toggles with the watch API and survives a task refetch", async ({ page }) => {
  // The seed's single person authored every task. Model a non-author's watch responses here;
  // the server's author-notification policy is unchanged and exercised separately above.
  let watching = false;
  const methods: string[] = [];
  await page.route("**/api/tasks/DM-5/watch?*", async (route) => {
    const method = route.request().method();
    methods.push(method);
    if (method === "PUT") watching = true;
    if (method === "DELETE") watching = false;
    await route.fulfill({ json: { author: false, watching } });
  });
  await page.goto("/tasks?task=DM-5");
  const follow = page.getByRole("dialog", { name: "Задача DM-5", exact: true }).getByRole("button", { name: "Следить", exact: true });
  await expect(follow).toHaveAttribute("aria-pressed", "false");
  await follow.click();
  await expect(follow).toHaveAttribute("aria-pressed", "true");
  await page.reload();
  await expect(follow).toHaveAttribute("aria-pressed", "true");
  await follow.click();
  await expect(follow).toHaveAttribute("aria-pressed", "false");
  expect(methods).toContain("PUT");
  expect(methods).toContain("DELETE");
});
