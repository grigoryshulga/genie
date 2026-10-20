// Photographs every route in CAPTURE_ROUTES for `npm run ui:capture`: one PNG per route, theme and
// viewport in test-results/captures. Nothing is compared; the images are evidence for a redesign
// task (before and after). GENIE_UI_THEMES lists the data-theme values to capture, comma
// separated; "current" leaves the page as the application renders it.

import { mkdirSync } from "node:fs";
import { resolve } from "node:path";
import { type Page, test } from "@playwright/test";
import { CAPTURE_ROUTES } from "./routes.ts";

const seededAt = Date.parse(process.env.GENIE_UI_SEEDED_AT ?? "");
const themes = (process.env.GENIE_UI_THEMES ?? "current").split(",").map((t) => t.trim()).filter(Boolean);
const out = resolve(import.meta.dirname, "../../test-results/captures");
mkdirSync(out, { recursive: true });

async function settle(page: Page) {
  await page.waitForLoadState("networkidle");
  await page.evaluate(() => document.fonts.ready);
}

for (const theme of themes) {
  for (const route of CAPTURE_ROUTES) {
    test(`${route.name} (${theme})`, async ({ page }, info) => {
      await page.clock.setFixedTime(seededAt + 60 * 60 * 1000);
      if (theme !== "current") {
        await page.addInitScript((value) => {
          document.documentElement.dataset.theme = value;
          localStorage.setItem("genie.theme", value);
        }, theme);
      }
      await page.goto(route.path);
      await settle(page);
      const size = info.project.name === "capture-phone" ? "390" : "1440";
      await page.screenshot({ path: resolve(out, `${route.name}-${theme}-${size}.png`), fullPage: true, animations: "disabled", caret: "hide" });
    });
  }
}
