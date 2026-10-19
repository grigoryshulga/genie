// Screenshots of the key screens, compared with the committed baselines on desktop and mobile.
// A layout change shows up as a pixel difference; update the baselines deliberately with
// `npm run test:ui -- --update-snapshots` and review the new images in the diff.

import { expect, type Page, test } from "@playwright/test";

const seededAt = Date.parse(process.env.GENIE_UI_SEEDED_AT ?? "");

// The page's clock stands an hour after seeding, so "an hour ago" reads the same on every run.
test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(seededAt + 60 * 60 * 1000);
});

async function settle(page: Page) {
  await page.waitForLoadState("networkidle");
  await page.evaluate(() => document.fonts.ready);
}

test("task list", async ({ page }) => {
  await page.goto("/tasks");
  await settle(page);
  await expect(page).toHaveScreenshot("tasks.png", { fullPage: true });
});

test("board", async ({ page }) => {
  await page.goto("/board");
  await settle(page);
  await expect(page).toHaveScreenshot("board.png", { fullPage: true });
});

test("task", async ({ page }) => {
  await page.goto("/task/DM-2");
  await settle(page);
  await expect(page).toHaveScreenshot("task.png", { fullPage: true });
});

test("epics", async ({ page }) => {
  await page.goto("/epics");
  await settle(page);
  await expect(page).toHaveScreenshot("epics.png", { fullPage: true });
});
