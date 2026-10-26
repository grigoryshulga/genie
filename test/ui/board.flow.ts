import { expect, test } from "@playwright/test";
import { SEED_TASKS } from "./seed.ts";

for (const theme of ["ink", "paper"]) {
  test(`board epic links are separate controls with stable stripes (${theme})`, async ({ page }) => {
    await page.goto("/board");
    await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
    const card = page.locator('[data-task-id="DM-2"]');
    const sibling = page.locator('[data-task-id="DM-3"]');
    const tone = await card.getAttribute("data-epic-tone");
    expect(tone).toMatch(/^[1-4]$/);
    await expect(sibling).toHaveAttribute("data-epic-tone", tone!);
    const stripe = await card.evaluate((node) => {
      const style = getComputedStyle(node, "::before");
      return { width: style.width, color: style.backgroundColor, shadow: getComputedStyle(node).boxShadow };
    });
    expect(stripe.width).toBe("4px");
    expect(stripe.color).not.toBe("rgba(0, 0, 0, 0)");
    expect(stripe.shadow).toContain("0px 1px 0px");
    await expect(card.locator(".bd-main a")).toHaveCount(0);
    const link = card.getByRole("link", { name: SEED_TASKS[0].title });
    await expect(link).toHaveAttribute("href", "/epic/DM-1");
    await link.focus();
    await page.keyboard.press("Space");
    await expect(page.locator(".bd-ghost")).toHaveCount(0);
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/\/epic\/DM-1$/);
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await expect(page.getByRole("heading", { name: SEED_TASKS[0].title })).toBeVisible();
    const epicMark = page.locator(".ep-id .epic-icon");
    await expect(epicMark).toHaveAttribute("data-epic-tone", tone!);

    await page.goto("/board?q=Форма");
    await expect(card).toHaveAttribute("data-epic-tone", tone!);
    await card.getByRole("link").click();
    await expect(page).toHaveURL(/\/epic\/DM-1$/);
    await page.goto("/board?q=Форма");
    await page.reload();
    await expect(card).toHaveAttribute("data-epic-tone", tone!);
    await card.locator(".bd-ac").click();
    const modal = page.getByRole("dialog", { name: "Задача DM-2", exact: true });
    await expect(modal).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(modal).toHaveCount(0);
    await expect(card.locator(".bd-main")).toBeFocused();
  });
}

for (const reducedMotion of ["no-preference", "reduce"] as const) {
  for (const width of [1440, 390]) {
    test(`board task transition preserves native dialog and focus (${reducedMotion}, ${width})`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.emulateMedia({ reducedMotion });
      await page.addInitScript(() => {
        const animate = Element.prototype.animate;
        Element.prototype.animate = function (frames, options) {
          if (this.matches(".task-modal")) {
            const log = JSON.parse(document.documentElement.dataset.taskMotion ?? "[]");
            log.push({ frames, options });
            document.documentElement.dataset.taskMotion = JSON.stringify(log);
          }
          return animate.call(this, frames, options);
        };
      });
      await page.goto("/board?q=Форма");
      const opener = page.locator('[data-task-id="DM-2"] .bd-main');
      await opener.click();
      const modal = page.getByRole("dialog", { name: "Задача DM-2", exact: true });
      await expect(modal).toBeVisible();
      await expect(modal.getByRole("heading", { name: SEED_TASKS[1].title })).toBeVisible();
      expect(await modal.evaluate((node) => node.matches(":modal"))).toBe(true);
      await page.keyboard.press("Escape");
      await expect(modal).toHaveCount(0);
      await expect(page).toHaveURL(/\/board\?q=/);
      await expect(page).not.toHaveURL(/[?&]task=/);
      await expect(opener).toBeFocused();
      const motions: { frames: { transform: string }[]; options: { duration: number } }[] = await page.evaluate(() => JSON.parse(document.documentElement.dataset.taskMotion ?? "[]"));
      if (reducedMotion === "reduce") expect(motions).toEqual([]);
      else {
        expect(motions.map((motion) => motion.options.duration)).toEqual([200, 150]);
        expect(motions[0].frames[0].transform).toContain("translate(");
        expect(motions[1].frames[1].transform).toContain("scale(");
      }
      await opener.click();
      await expect(modal).toBeVisible();
      await modal.getByRole("button", { name: width === 390 ? "Назад" : "Закрыть", exact: true }).click();
      await expect(modal).toHaveCount(0);
      await expect(opener).toBeFocused();
    });
  }
}

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
        expect(size.height).toBeCloseTo(144, 2);
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
      expect((await card.boundingBox())!.height).toBeCloseTo(144, 2);
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
      await expect(page.getByRole("toolbar", { name: "Инструменты страницы" })).toBeVisible();

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
