import { expect, test } from "@playwright/test";
import { auditPage } from "./a11y.ts";

test("contrast checks account for the indicator behind an animated tab", async ({ page }) => {
  await page.setContent(`<main><div class="g-tabs-motion" style="position:relative;background:#111">
    <span class="g-tabs-indicator" aria-hidden="true" style="position:absolute;inset:0;background:#fff"></span>
    <a aria-current="page" href="#" style="position:relative;color:#000">Readable</a>
  </div></main>`);
  const findings = () => page.evaluate(auditPage).then((result) => result.findings.filter((item) => item.rule === "color-contrast"));
  expect(await findings()).toEqual([]);
  await page.locator("a").evaluate((el) => { el.style.color = "white"; });
  expect(await findings()).toHaveLength(1);
});
