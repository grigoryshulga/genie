import { expect, test } from "@playwright/test";
import { SEED_TASKS } from "./seed.ts";
import { storedTask } from "./api.ts";

for (const theme of ["ink", "paper"]) {
  test(`board epic links keep stable markers independent of lane tint (${theme})`, async ({ page }) => {
    await page.goto("/board");
    await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
    const card = page.locator('[data-task-id="DM-2"]');
    const sibling = page.locator('[data-task-id="DM-3"]');
    const tone = await card.getAttribute("data-epic-tone");
    expect(tone).toMatch(/^[1-4]$/);
    await expect(sibling).toHaveAttribute("data-epic-tone", tone!);
    const marker = await card.evaluate((node) => {
      return { color: getComputedStyle(node.querySelector(".epic-icon")!).color, stripe: getComputedStyle(node, "::before").content, shadow: getComputedStyle(node).boxShadow };
    });
    expect(marker.stripe).toBe("none");
    expect(marker.shadow).toContain("0px 1px 0px");
    // Both tasks are in work (one in progress, one ready); the lane tint itself is covered by the next test.
    await expect(card).toHaveAttribute("data-lane-tone", "work");
    await expect(sibling).toHaveAttribute("data-lane-tone", "work");
    await expect(sibling.locator(".epic-icon")).toHaveCSS("color", marker.color);
    await expect(card.locator(".bd-main a")).toHaveCount(0);
    const link = card.getByRole("link", { name: SEED_TASKS[0].title });
    await expect(link).toHaveAttribute("href", "/epic/DM-1");
    await expect(link).toHaveCSS("color", marker.color);
    await link.hover();
    await expect(link).toHaveCSS("color", marker.color);
    await expect(card.locator(".bd-status")).toHaveCount(0);
    const label = card.locator(".bd-labels .g-tag");
    await expect(label).not.toHaveClass(/outline/);
    const labelStyle = await label.evaluate((node) => {
      const text = node.querySelector(".bd-label-text")!.getBoundingClientRect();
      const box = node.getBoundingClientRect();
      const style = getComputedStyle(node);
      return { offset: Math.abs(text.y + text.height / 2 - box.y - box.height / 2), fill: style.backgroundColor, shadow: style.boxShadow };
    });
    expect(labelStyle.offset).toBeLessThan(1);
    expect(labelStyle.fill).not.toBe("rgba(0, 0, 0, 0)");
    expect(labelStyle.shadow).toBe("none");
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

for (const theme of ["ink", "paper"]) {
  test(`lane tints stay distinct and follow a saved move without recolouring the epic (${theme})`, async ({ page, request }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    const headers = { "x-genie": "1" };
    const created = await request.post("/api/tasks?project=demo", {
      headers,
      data: { title: `Проверка цвета дорожки ${theme}`, description: "Check lane colour after a persisted move.", acceptance: ["Colour follows the lane"], plan: "Move and verify", parent: "DM-1", draft: true },
    });
    expect(created.ok()).toBe(true);
    const { id }: { id: string } = await created.json();
    try {
      await page.goto(`/board?q=${encodeURIComponent(`Проверка цвета дорожки ${theme}`)}`);
      await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
      const laneBackgrounds = await page.locator('.g-lane[data-lane]:not([data-lane="cancelled"])').evaluateAll((nodes) => nodes.map((node) => getComputedStyle(node).backgroundColor));
      expect(new Set(laneBackgrounds).size).toBe(5);
      const card = page.locator(`[data-task-id="${id}"]`);
      await expect(card).toHaveAttribute("data-lane-tone", "prep");
      const before = await card.evaluate((node) => ({ background: getComputedStyle(node).backgroundColor, epic: getComputedStyle(node.querySelector(".epic-icon")!).color }));

      await card.locator(".bd-main").focus();
      await page.keyboard.press("Space");
      const ghost = page.locator(".bd-ghost");
      await expect(ghost).toHaveAttribute("data-lane-tone", "prep");
      await expect(ghost.locator(".epic-icon")).toHaveCSS("color", before.epic);
      expect(await ghost.evaluate((node) => getComputedStyle(node).backgroundColor)).toBe(before.background);
      await page.keyboard.press("Escape");
      // dnd-kit attaches the keyboard sensor's document Escape listener in a deferred task after the
      // drag starts, so a cancel sent in the same breath is lost. Press again while the overlay is up.
      await expect(async () => {
        await page.keyboard.press("Escape");
        await expect(ghost).toHaveCount(0, { timeout: 250 });
      }).toPass({ timeout: 5_000 });

      await card.getByRole("button", { name: `Переместить ${id}` }).click();
      await page.getByRole("menuitem", { name: /^Готово к работе/ }).click();
      await expect(card).toHaveAttribute("data-lane-tone", "work");
      await expect.poll(async () => (await storedTask(request, id)).status).toBe("ready");
      await expect.poll(() => card.evaluate((node) => getComputedStyle(node).backgroundColor)).not.toBe(before.background);
      await expect(card.locator(".epic-icon")).toHaveCSS("color", before.epic);
      await page.reload();
      await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
      await expect(card).toHaveAttribute("data-lane-tone", "work");

      // The browser resolves color-mix; a one-pixel canvas reads the resulting sRGB values.
      const contrasts = await page.evaluate(() => {
        const ctx = document.createElement("canvas").getContext("2d")!;
        const luminance = (color: string) => {
          ctx.clearRect(0, 0, 1, 1); ctx.fillStyle = color; ctx.fillRect(0, 0, 1, 1);
          const rgb = [...ctx.getImageData(0, 0, 1, 1).data].slice(0, 3).map((value) => {
            const c = value / 255;
            return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
          });
          return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
        };
        const ratio = (fg: string, bg: string) => {
          const a = luminance(fg), b = luminance(bg);
          return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
        };
        return [...document.querySelectorAll<HTMLElement>('.g-lane[data-lane]:not([data-lane="cancelled"])')].flatMap((lane) => {
          const name = lane.querySelector(".g-lane-name")!, count = lane.querySelector(".g-lane-n")!;
          return [
            { label: `${lane.dataset.lane} heading`, ratio: ratio(getComputedStyle(name).color, getComputedStyle(lane).backgroundColor) },
            { label: `${lane.dataset.lane} count`, ratio: ratio(getComputedStyle(count).color, getComputedStyle(count).backgroundColor) },
          ];
        });
      });
      for (const pair of contrasts) expect(pair.ratio, pair.label).toBeGreaterThanOrEqual(4.5);
    } finally {
      expect((await request.post(`/api/tasks/${id}/status?project=demo`, { headers, data: { status: "cancelled" } })).ok()).toBe(true);
    }
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

const AWAITING_TEAM = "Ждёт запуска: команда соберётся, когда освободится место и будут готовы зависимости";

for (const theme of ["ink", "paper"]) {
  test(`a ready card in work shows a team placeholder and its hint, a running card does not (${theme})`, async ({ page }) => {
    await page.goto("/board");
    await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
    // DM-3 is seeded as ready (no team yet); DM-2 is in work. The keyboard move in a11y.flow may have
    // moved DM-2 to review on this same server, so either in-work status is accepted.
    const waiting = page.locator('[data-task-id="DM-3"]');
    const running = page.locator('[data-task-id="DM-2"]');
    await expect(waiting).toHaveAttribute("data-status", "ready");
    await expect(running).toHaveAttribute("data-status", /^(in_progress|review)$/);

    const placeholder = waiting.locator(".bd-team-dot");
    await expect(placeholder).toHaveCount(3);
    await expect(waiting.locator(".bd-team-wait")).toHaveAttribute("aria-hidden", "true");
    await expect(placeholder.first()).toHaveClass(/g-skeleton/);
    await expect(placeholder.first()).toHaveCSS("animation-name", "g-shimmer");
    // The tiles keep the small avatar size, so the card does not grow.
    expect((await placeholder.first().boundingBox())!.width).toBeCloseTo(24, 0);
    await expect(waiting.getByRole("button", { name: /^DM-3:/ })).toHaveAccessibleDescription(new RegExp(AWAITING_TEAM));
    expect((await waiting.boundingBox())!.height).toBeCloseTo(144, 2);

    await expect(running.locator(".bd-team-dot")).toHaveCount(0);
    await expect(running.getByRole("button", { name: /^DM-2:/ })).not.toHaveAccessibleDescription(/Ждёт запуска/);

    await page.emulateMedia({ reducedMotion: "reduce" });
    await expect(placeholder.first()).toHaveCSS("animation-name", "none");
  });
}
