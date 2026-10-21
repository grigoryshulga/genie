// The phone docs drawer is a dialog: it takes focus, keeps Tab inside, closes on Escape and hands
// focus back to the control that opened it. Its filter and tree keep working underneath.
import { expect, test } from "@playwright/test";
import { focusInside } from "./keyboard.ts";
import { FLOW_DOC } from "./seed.ts";

const STOPS = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

test.describe("docs drawer on a phone", () => {
  test.use({ viewport: { width: 390, height: 860 } });

  test("takes focus, traps Tab, closes on Escape and restores focus", async ({ page }) => {
    await page.goto(`/docs?page=${FLOW_DOC.path}`);
    const opener = page.getByRole("button", { name: "Открыть дерево документации" });
    await opener.click();
    const drawer = page.getByRole("dialog", { name: "Навигация по документации" });
    await expect(drawer).toBeVisible();
    await expect(drawer).toHaveAttribute("aria-modal", "true");
    expect(await focusInside(drawer)).toBe(true);

    const count = await drawer.evaluate(
      (root, selector) => Array.from(root.querySelectorAll<HTMLElement>(selector)).filter((node) => node.getClientRects().length > 0).length,
      STOPS,
    );
    const index = () =>
      drawer.evaluate(
        (root, selector) =>
          Array.from(root.querySelectorAll<HTMLElement>(selector))
            .filter((node) => node.getClientRects().length > 0)
            .indexOf(document.activeElement as HTMLElement),
        STOPS,
      );
    expect(count).toBeGreaterThan(3);
    expect(await index()).toBe(0);

    // Tab walks every stop and never leaves the drawer.
    for (let step = 1; step < count; step += 1) {
      await page.keyboard.press("Tab");
      expect(await index()).toBe(step);
      expect(await focusInside(drawer)).toBe(true);
    }
    // From the last stop Tab wraps to the first, and Shift+Tab wraps back.
    await page.keyboard.press("Tab");
    expect(await index()).toBe(0);
    await page.keyboard.press("Shift+Tab");
    expect(await index()).toBe(count - 1);

    await page.screenshot({ path: "test-results/docs-drawer-focus.png", animations: "disabled" });
    await page.keyboard.press("Escape");
    await expect(drawer).toBeHidden();
    await expect(opener).toBeFocused();
  });

  test("filters the tree and closes when a page is picked", async ({ page }) => {
    await page.goto(`/docs?page=${FLOW_DOC.path}`);
    await page.getByRole("button", { name: "Открыть дерево документации" }).click();
    const drawer = page.getByRole("dialog", { name: "Навигация по документации" });
    await expect(drawer).toBeVisible();
    // Articles only: the space overview is a row too, but it is not a page.
    const rows = drawer.locator("button.doc-row:not(.doc-space-home)");
    const filter = drawer.getByRole("textbox", { name: "Фильтр статей" });

    await filter.fill("ни одной такой статьи");
    await expect(drawer.locator(".doc-hint")).toHaveText("Ничего не найдено");
    await expect(rows).toHaveCount(0);

    await filter.fill("Как запустить");
    await expect(drawer.locator(".doc-hint")).toHaveCount(0);
    const article = rows.filter({ hasText: "Как запустить сервер" });
    await expect(article).toHaveCount(1);
    await article.click();
    await expect(drawer).toBeHidden();
    await expect(page).toHaveURL(new RegExp(`page=${encodeURIComponent(FLOW_DOC.path)}`));
    await expect(page.getByRole("heading", { name: /Как запустить/ })).toBeVisible();
  });
});
