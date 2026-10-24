// Keyboard-only navigation for the accessibility tests: reaching a control the way a person does, with
// Tab, so a control that is only reachable through a pointer (or through focus() in a test) fails.

import { expect, type Locator, type Page } from "@playwright/test";

/**
 * Presses Tab until `target` has focus, then fails with what it was looking for. Keyboard order is part
 * of the contract, so this walks it instead of jumping with focus().
 */
export async function tabTo(page: Page, target: Locator, what: string, max = 160) {
  for (let i = 0; i < max; i += 1) {
    const focused = await target.evaluate((el) => el === document.activeElement).catch(() => false);
    if (focused) return;
    await page.keyboard.press("Tab");
  }
  expect(await target.evaluate((el) => el === document.activeElement), `Tab reaches ${what}`).toBe(true);
}

/** Whether focus is somewhere inside `target`, read in the page. */
export async function focusInside(target: Locator): Promise<boolean> {
  return target.evaluate((el) => el.contains(document.activeElement));
}
