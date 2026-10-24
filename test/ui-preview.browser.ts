import { expect, test } from "@playwright/test";

for (const theme of ["ink", "paper"]) for (const width of [1440, 390]) {
  test(`${theme} ${width}: real board and task detail render from fixtures`, async ({ page }) => {
    const errors: string[] = []; page.on("pageerror", e => errors.push(e.message));
    await page.goto(`/__preview?theme=${theme}&width=${width}&route=/board`);
    const app = page.frameLocator("#app");
    await expect(app.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(app.locator("body")).toContainText("DM-2");
    expect(await app.locator("html").evaluate(el => el.clientWidth)).toBe(width);
    await page.getByLabel("Route").fill("/task/DM-2");
    await page.getByRole("button", { name: "Apply" }).click();
    await expect(app.locator("body")).toContainText("DM-2");
    expect(errors).toEqual([]);
    await page.screenshot({ path: `test-results/ui-preview-${theme}-${width}.png` });
  });
}

test("state controls show empty, pending and failure states and recover", async ({ page }) => {
  await page.goto("/__preview?route=/board");
  const app = page.frameLocator("#app");
  await expect(app.locator("body")).toContainText("DM-2");
  for (const state of ["empty", "loading", "error", "populated"]) {
    await page.getByLabel("State").selectOption(state);
    await page.getByRole("button", { name: "Apply" }).click();
    if (state === "populated") await expect(app.locator("body")).toContainText("DM-2");
    else {
      await expect(app.locator("body")).not.toContainText("DM-2");
      if (state === "empty") await expect(app.getByRole("heading", { name: "Здесь тихо" })).toBeVisible();
      if (state === "loading") await expect(app.locator('.g-board-skeleton[aria-busy="true"]')).not.toHaveCount(0);
      if (state === "error") await expect(app.locator("body")).toContainText("simulated server unavailable");
    }
  }
});

test("recorded docs, catalogue and settings render without frontend exceptions", async ({ page }) => {
  const errors: string[] = []; page.on("pageerror", e => errors.push(e.message));
  for (const route of ["/docs?page=demo/guides/deploy.md", "/agents", "/settings/me"]) {
    await page.goto(`/__preview?route=${encodeURIComponent(route)}`);
    await expect(page.frameLocator("#app").locator("main")).toBeVisible();
    await expect(page.frameLocator("#app").locator("body")).not.toContainText("No UI fixture");
  }
  expect(errors).toEqual([]);
});
