// The accessibility audit of the screens the seed can reach, in both themes: the rules in a11y.ts run
// against the rendered page and the test fails on anything critical or serious. Moderate findings are
// printed, not failing, so a reviewer sees them in the CI log without the build turning red on taste.
//
// The same test walks the screens again with prefers-reduced-motion: nothing may still be animating
// and no transition may still move a shape, which is what the redesign promises.

import { expect, type Page, test } from "@playwright/test";
import { auditPage, type Finding, focusVisibility, motionReport } from "./a11y.ts";
import { focusInside, tabTo } from "./keyboard.ts";

const seededAt = Date.parse(process.env.GENIE_UI_SEEDED_AT ?? "");
const THEMES = ["ink", "paper"] as const;

// Every screen the standard seed (seed.ts) fills. The docs pages, the proposals queue and the
// task waiting for the owner only exist in the capture seed, so those are photographed and audited
// by `npm run ui:capture` instead of here.
const ROUTES = [
  { name: "tasks", path: "/tasks" },
  { name: "board", path: "/board" },
  { name: "task-panel", path: "/tasks?task=DM-2" },
  { name: "task", path: "/task/DM-2" },
  { name: "epics", path: "/epics" },
  { name: "epic", path: "/epic/DM-1" },
  { name: "docs", path: "/docs" },
  { name: "automations", path: "/manage/agents/automations/runs" },
  { name: "observations", path: "/manage/agents/automations/observations" },
  { name: "agents", path: "/manage/agents/roles" },
  { name: "notifications", path: "/notifications" },
  { name: "statistics", path: "/manage/server/stats" },
  { name: "settings-me", path: "/settings/me" },
  { name: "manage-project", path: "/manage/project/general" },
  { name: "manage-server", path: "/manage/server/projects" },
];

test.beforeEach(async ({ page }) => {
  await page.clock.setFixedTime(seededAt + 60 * 60 * 1000);
});

async function settle(page: Page) {
  await page.waitForLoadState("networkidle");
  await page.evaluate(() => document.fonts.ready);
}

const show = (findings: Finding[]) => findings.map((f) => `[${f.rule}] ${f.target} — ${f.detail}`).join("\n");
const blocking = (findings: Finding[]) => findings.filter((f) => f.severity === "critical" || f.severity === "serious");

for (const theme of THEMES) {
  for (const route of ROUTES) {
    test(`${route.name} audit (${theme})`, async ({ page }) => {
      await page.addInitScript((value) => {
        // The application reads this preference at start (shared/lib/appearance.ts).
        localStorage.setItem("genie.appearance.theme", value);
      }, theme);
      await page.goto(route.path);
      await settle(page);

      const { findings, checked } = await page.evaluate(auditPage);
      const moderate = findings.filter((f) => f.severity === "moderate");
      if (moderate.length > 0) console.log(`${route.name} (${theme}): ${moderate.length} moderate\n${show(moderate)}`);
      expect(blocking(findings), `critical or serious findings on ${route.path} in ${theme} (${checked} elements checked)`).toEqual([]);
    });
  }
}

// Motion is the same in both themes, so the reduced-motion pass runs once per screen.
for (const route of ROUTES) {
  test(`${route.name} reduced motion`, async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await page.goto(route.path);
    await settle(page);
    // A frame of the reduced page: entrance animations that are still running would show up here.
    await page.waitForTimeout(250);
    const { animations, transitions } = await page.evaluate(motionReport);
    expect(animations, `animations still running on ${route.path} under prefers-reduced-motion`).toEqual([]);
    expect(transitions, `transitions that still move a shape on ${route.path} under prefers-reduced-motion`).toEqual([]);
  });
}

// Every stop on the keyboard road shows where it is: the design system promises a visible focus ring,
// and a rule that removes the outline must replace it with something a person can see.
for (const path of ["/tasks", "/board"]) {
  test(`focus is visible at every stop on ${path}`, async ({ page }) => {
    await page.goto(path);
    await settle(page);
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    const missing: string[] = [];
    let stops = 0;
    for (let i = 0; i < 40; i += 1) {
      await page.keyboard.press("Tab");
      const seen = await page.evaluate(focusVisibility);
      if (seen.target === "document") break;
      stops += 1;
      if (!seen.visible) missing.push(seen.target);
    }
    expect(stops, "the page has keyboard stops to walk").toBeGreaterThan(3);
    expect(missing, `stops without a visible focus indicator on ${path}`).toEqual([]);
  });
}

test("the filter sheet holds the keyboard on a phone", async ({ page }, info) => {
  test.skip(info.project.name !== "mobile", "the filter sheet is the phone layout of the task list");
  await page.goto("/tasks");
  const chip = page.getByRole("button", { name: "Фильтры" });
  await tabTo(page, chip, "the filters chip", 240);
  await page.keyboard.press("Enter");

  const sheet = page.getByRole("dialog", { name: "Фильтры" });
  await expect(sheet).toBeVisible();
  // Opening it moves the focus inside, so the next Tab walks the filters and not the page behind.
  expect(await focusInside(sheet), "focus moves into the sheet when it opens").toBe(true);
  for (let i = 1; i <= 20; i += 1) {
    await page.keyboard.press("Tab");
    expect(await focusInside(sheet), `Tab stays inside the sheet after ${i} presses`).toBe(true);
  }

  await page.keyboard.press("Escape");
  await expect(sheet).toBeHidden();
  expect(await chip.evaluate((el) => el === document.activeElement), "focus returns to the chip that opened the sheet").toBe(true);
});
