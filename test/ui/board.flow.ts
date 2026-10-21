import { expect, test } from "@playwright/test";

for (const theme of ["ink", "paper"]) {
  for (const width of [1440, 390]) {
    test(`collapsed board lanes stay compact and reopen (${theme}, ${width})`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto("/board");
      await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);

      const lane = page.locator('[data-lane="maybe"]');
      const toggle = lane.getByRole("button", { name: /Может быть/ });
      const count = await lane.locator(".g-lane-n").textContent();
      await expect(toggle).toHaveAttribute("aria-expanded", "false");
      await expect(lane.locator(".g-lane-name")).toHaveCSS("writing-mode", "vertical-rl");
      expect((await lane.boundingBox())!.width).toBeLessThanOrEqual(72);
      await expect(page.locator(".g-board-hero")).toHaveCount(0);
      await expect(page.getByRole("region", { name: "Что происходит сейчас" })).toBeVisible();

      await toggle.click();
      await expect(toggle).toHaveAttribute("aria-expanded", "true");
      await expect(lane.locator(".g-lane-name")).toHaveCSS("writing-mode", "horizontal-tb");
      expect((await lane.boundingBox())!.width).toBeGreaterThan(72);
      await expect(lane.locator(".g-lane-n")).toHaveText(count!);

      await toggle.click();
      await expect(toggle).toHaveAttribute("aria-expanded", "false");
      await expect(lane.locator(".g-lane-name")).toHaveCSS("writing-mode", "vertical-rl");
      expect((await lane.boundingBox())!.width).toBeLessThanOrEqual(72);
    });
  }
}
