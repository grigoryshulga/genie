import { expect, test } from "@playwright/test";
import { SEED_TASKS } from "./seed.ts";

for (const route of ["board", "tasks"]) {
  for (const theme of ["ink", "paper"]) {
    for (const width of [1440, 390]) {
      test(`task opens above ${route} and returns focus (${theme}, ${width})`, async ({ page }) => {
        await page.setViewportSize({ width, height: 900 });
        await page.goto(`/${route}`);
        await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
        const opener = page.getByRole("button", { name: new RegExp(SEED_TASKS[1].title) });
        await opener.click();
        const modal = page.getByRole("dialog", { name: "Задача DM-2", exact: true });
        await expect(modal).toBeVisible();
        await expect(modal.getByRole("heading", { name: SEED_TASKS[1].title })).toBeVisible();
        await expect(modal).toHaveCSS("transform", "none");
        await expect(page).toHaveURL(new RegExp(`/${route}\\?task=DM-2`));
        expect(await modal.evaluate((node) => node.matches(":modal"))).toBe(true);
        const box = (await modal.boundingBox())!;
        expect(Math.abs(box.x + box.width / 2 - width / 2)).toBeLessThan(2);
        expect(box.width).toBeCloseTo(width === 390 ? 390 : 864, 2);
        expect(Math.abs(box.y + box.height / 2 - 450)).toBeLessThan(2);
        expect(await page.locator(".app").evaluate((node) => node.classList.contains("with-detail"))).toBe(false);

        // Start from loaded content: the lazy fallback itself has no focusable controls.
        await modal.getByRole("link", { name: "Открыть страницу" }).focus();
        for (const key of ["Tab", "Shift+Tab"]) {
          await page.keyboard.press(key);
          expect(await modal.evaluate((node) => node.contains(document.activeElement))).toBe(true);
        }
        // An inert background control cannot steal focus even through a programmatic request.
        await page.evaluate(() => document.querySelector<HTMLButtonElement>(".g-topbar button")?.focus());
        expect(await modal.evaluate((node) => node.contains(document.activeElement))).toBe(true);
        await page.screenshot({ path: `test-results/task-modal-${route}-${theme}-${width}.png`, animations: "disabled" });
        await page.keyboard.press("Escape");
        await expect(modal).toBeHidden();
        await expect(page).toHaveURL(new RegExp(`/${route}$`));
        await expect(opener).toBeFocused();
      });
    }
  }
}

test("task query links open the modal and retain the full-page route", async ({ page }) => {
  await page.goto("/tasks?q=Форма&task=DM-2");
  const modal = page.getByRole("dialog", { name: "Задача DM-2", exact: true });
  await expect(modal).toBeVisible();
  await modal.getByRole("button", { name: "Закрыть", exact: true }).click();
  await expect(page).toHaveURL(/\/tasks\?q=/);
  await expect(page).not.toHaveURL(/[?&]task=/);
  await page.goto("/tasks?task=DM-2");
  await page.getByRole("link", { name: "Открыть страницу" }).click();
  await expect(page).toHaveURL(/\/task\/DM-2$/);
  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("heading", { name: SEED_TASKS[1].title })).toBeVisible();
});
