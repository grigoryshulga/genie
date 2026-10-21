// Touch coverage at phone and tablet widths: the palette's own entry point and the board's move
// menu, driven by real taps. `locator.tap()` only works in a context that has touch, so these tests
// have their own projects (`touch-phone`, `touch-tablet`); every one of them also proves that the
// events the page saw carried `pointerType: "touch"`, so a mouse-driven pass cannot stand in for it.

import { expect, type Locator, type Page, test } from "@playwright/test";
import { storedTask } from "./api.ts";

/** Records the pointer type of every pointer event the page sees, before the app has had a chance to run. */
async function watchPointers(page: Page) {
  await page.addInitScript(() => {
    const seen: string[] = [];
    for (const type of ["pointerdown", "pointerup", "click"]) {
      document.addEventListener(
        type,
        (event) => {
          if (event instanceof PointerEvent) seen.push(`${type}:${event.pointerType}`);
        },
        true,
      );
    }
    (window as Window & { __touchPointers?: string[] }).__touchPointers = seen;
  });
}

async function expectTouch(page: Page, what: string) {
  const seen = await page.evaluate(() => (window as Window & { __touchPointers?: string[] }).__touchPointers ?? []);
  expect(seen, `${what} heard a touch down`).toContain("pointerdown:touch");
  // A mouse driving the same steps would announce itself here; Chromium reports the click it derives
  // from a touch as "mouse" on some emulated devices, so the pointer itself is the evidence.
  expect(seen, `${what} was not driven by a mouse`).not.toContain("pointerdown:mouse");
}

/**
 * The opener whose menu has the furthest to hang: the first card of a lane that has another card
 * under it. That is where a menu clipped by its own card loses the items past the card's edge.
 */
async function worstCaseMoveButton(page: Page): Promise<{ opener: Locator; id: string }> {
  for (const lane of await page.locator(".g-lane[data-lane]").all()) {
    const cards = lane.locator(".bd-card");
    if ((await cards.count()) < 2) continue;
    const opener = cards.first().getByRole("button", { name: /^Переместить / });
    const id = (await opener.getAttribute("aria-label"))!.replace("Переместить ", "");
    return { opener, id };
  }
  throw new Error("no lane holds two cards, so there is no menu hanging over a neighbour");
}

test("the sections sheet opens the command palette with a tap", async ({ page }) => {
  await watchPointers(page);
  await page.goto("/tasks");
  await page.getByRole("button", { name: "Открыть меню разделов" }).tap();
  const sheet = page.getByRole("dialog", { name: "Навигация" });
  await expect(sheet).toBeVisible();

  // The row is a thumb-sized target, like the destinations it sits beside.
  const entry = sheet.getByRole("button", { name: "Команды" });
  await expect(entry).toBeVisible();
  expect((await entry.boundingBox())!.height).toBeGreaterThanOrEqual(44);

  await entry.tap();
  await expect(sheet).toBeHidden();
  const palette = page.getByRole("dialog", { name: "Команды" });
  await expect(palette).toBeVisible();
  await expect(palette.getByLabel("Поиск команд, задач и документации")).toBeFocused();

  // A row of the palette does what it says when it is tapped: the palette closes on the board.
  await palette.getByRole("option").filter({ hasText: "Доска" }).first().tap();
  await expect(palette).toBeHidden();
  await expect(page).toHaveURL(/\/board$/);
  await expect(page.getByRole("main", { name: "Доска" })).toBeVisible();
  await expectTouch(page, "the palette");
});

test("the keyboard shortcut still opens the palette on a touch screen", async ({ page }) => {
  await page.goto("/board");
  // The shortcut is heard once the shell is up; pressing before it renders would test the login gate.
  await expect(page.getByRole("main", { name: "Доска" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Открыть меню разделов" })).toBeVisible();
  await page.keyboard.press("Control+k");
  const palette = page.getByRole("dialog", { name: "Команды" });
  await expect(palette).toBeVisible();
  await expect(palette.getByLabel("Поиск команд, задач и документации")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(palette).toBeHidden();
  await expect(page.getByRole("main", { name: "Доска" })).toBeVisible();
});

test("a card's move menu is fully hittable and a tap moves the task", async ({ page }) => {
  await watchPointers(page);
  await page.goto("/board");
  await expect(page.getByRole("main", { name: "Доска" })).toBeVisible();

  // Folded lanes keep their cards off the board; the menu needs the neighbour that used to cover it.
  for (const lane of await page.locator(".g-lane[data-lane]").all()) {
    const toggle = lane.locator(".g-lane-toggle");
    if ((await toggle.getAttribute("aria-expanded")) === "false") await toggle.tap();
  }

  const { opener, id } = await worstCaseMoveButton(page);
  const before = await storedTask(page.request, id);

  await opener.tap();
  await expect(opener).toHaveAttribute("aria-expanded", "true");
  const menu = page.getByRole("menu", { name: `Переместить ${id} в` });
  await expect(menu).toBeVisible();
  // The menu leaves the card and the lane scroller: a native popover, not a clipped child.
  expect(await menu.evaluate((node) => node.parentElement!.matches(":popover-open"))).toBe(true);

  // Every item answers the pointer at its own centre: an item the cards painted later cover would
  // hit-test to them instead, which is exactly how the menu lost its lower half before.
  const items = menu.getByRole("menuitem");
  const count = await items.count();
  expect(count).toBeGreaterThan(0);
  for (let i = 0; i < count; i += 1) {
    const covered = await items.nth(i).evaluate((node) => {
      const rect = node.getBoundingClientRect();
      const hit = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
      return node.contains(hit) ? null : (hit?.outerHTML.slice(0, 80) ?? "nothing");
    });
    expect(covered, `item ${i} of ${count} is under the pointer`).toBeNull();
  }

  // The last item is the one furthest from the opener, so it is the one a clipped menu loses.
  await items.last().tap();
  await expect(menu).toBeHidden();
  const after = await storedTask(page.request, id);
  expect(after.status, `«${id}» moved to the status that was tapped`).not.toBe(before.status);
  await expectTouch(page, `the «Переместить ${id}» menu`);
});
