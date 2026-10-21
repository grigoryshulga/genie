import { expect, test } from "@playwright/test";

for (const theme of ["ink", "paper"]) {
  test(`board cards keep their size as lanes fold and the viewport changes (${theme})`, async ({ page }) => {
    await page.goto("/board");
    await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
    const lanes = page.locator(".g-lane[data-lane]");
    for (const lane of await lanes.all()) {
      const toggle = lane.locator(".g-lane-toggle");
      if (await toggle.getAttribute("aria-expanded") === "false") await toggle.click();
    }
    const board = page.locator(".g-board");
    for (const width of [1440, 1920, 390]) {
      await page.setViewportSize({ width, height: 900 });
      const sizes = await board.locator(".bd-card").evaluateAll((nodes) => nodes.map((node) => {
        const box = node.getBoundingClientRect();
        return { width: box.width, height: box.height };
      }));
      expect(sizes.length).toBeGreaterThan(0);
      for (const size of sizes) {
        expect(size.width).toBeCloseTo(288, 2);
        expect(size.height).toBeCloseTo(224, 2);
      }
      expect(await board.evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(true);
      await board.evaluate((node) => { node.scrollLeft = node.scrollWidth; });
      expect(await board.evaluate((node) => node.scrollLeft)).toBeGreaterThan(0);
    }
    for (const lane of await lanes.all()) {
      if (await lane.getAttribute("data-lane") !== "work") await lane.locator(".g-lane-toggle").click();
    }
    const working = page.locator('[data-lane="work"]');
    for (const width of [1440, 1920, 390]) {
      await page.setViewportSize({ width, height: 900 });
      expect((await working.boundingBox())!.width).toBeCloseTo(344, 2);
      const card = working.locator(".bd-card").first();
      expect((await card.boundingBox())!.width).toBeCloseTo(288, 2);
      expect((await card.boundingBox())!.height).toBeCloseTo(224, 2);
      await page.screenshot({ path: `test-results/board-fixed-${theme}-${width}.png`, animations: "disabled" });
    }
  });
}

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
