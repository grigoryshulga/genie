// The floating bar must stay inside the window at every width: the project button and the pill never
// overlap or leave the screen, the page never scrolls sideways, and on a phone the bar rests at the
// bottom edge. The project name no longer sits in the bar, so a long one is checked where it shows:
// the switcher's menu, which ellipsizes it instead of widening past the window.
import { expect, test } from "@playwright/test";

const LONG = Array(3).fill("genie-platform-integration-observatory-and-observability-console").join("-");

/** Reads the bar's geometry in one pass so a refetch cannot split it. */
const measure = () => {
  const box = (selector: string) => document.querySelector<HTMLElement>(selector)!.getBoundingClientRect();
  const bar = document.querySelector<HTMLElement>(".g-topbar")!;
  const project = box(".g-topbar-project");
  const pill = box(".g-topbar-pill");
  return {
    inner: window.innerWidth,
    innerHeight: window.innerHeight,
    docScroll: document.documentElement.scrollWidth,
    bar: bar.clientWidth,
    barBottom: bar.getBoundingClientRect().bottom,
    projectLeft: project.left,
    projectRight: project.right,
    pillLeft: pill.left,
    pillRight: pill.right,
    controls: [...bar.querySelectorAll("button, a")]
      .map((node) => node.getBoundingClientRect())
      .filter((rect) => rect.width > 0)
      .map((rect) => ({ left: rect.left, right: rect.right })),
  };
};

for (const width of [1440, 1280, 1181, 1000, 901, 900, 768, 430, 390, 360]) {
  test(`the floating bar stays inside the window at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 860 });
    await page.route("**/api/auth/me", async (route) => {
      const response = await route.fetch();
      const session: { projects: { name: string }[] } = await response.json();
      session.projects = session.projects.map((project) => ({ ...project, name: LONG }));
      await route.fulfill({ response, json: session });
    });
    await page.goto("/tasks");
    await page.locator("main.g-tasks-page").waitFor();
    await page.locator(".g-topbar-project").waitFor({ state: "visible" });

    const geometry = await page.evaluate(measure);
    expect(geometry.docScroll).toBeLessThanOrEqual(geometry.inner);
    expect(geometry.bar).toBe(geometry.inner);
    for (const control of geometry.controls) {
      expect(control.left).toBeGreaterThanOrEqual(0);
      expect(control.right).toBeLessThanOrEqual(width);
    }
    if (width > 900) {
      // Side by side: the project button ends before the pill begins, and the pill is centred.
      expect(geometry.projectRight).toBeLessThanOrEqual(geometry.pillLeft);
      expect(Math.abs((geometry.pillLeft + geometry.pillRight) / 2 - width / 2)).toBeLessThan(2);
    } else {
      expect(geometry.projectRight).toBeLessThanOrEqual(geometry.pillLeft);
      // Under the thumb: the bar rests on the bottom edge.
      expect(geometry.barBottom).toBeCloseTo(geometry.innerHeight, 0);
    }

    await page.getByRole("button", { name: `Проект ${LONG}`, exact: true }).click();
    const menu = page.getByRole("menu", { name: "Проекты", exact: true });
    await expect(menu).toBeVisible();
    const box = (await menu.boundingBox())!;
    expect(box.x).toBeGreaterThanOrEqual(0);
    expect(box.x + box.width).toBeLessThanOrEqual(width);
    expect(box.y).toBeGreaterThanOrEqual(0);
    expect(box.y + box.height).toBeLessThanOrEqual(860);
    const name = menu.locator(".g-topbar-project-name").first();
    expect(await name.evaluate((node) => node.scrollWidth > node.clientWidth)).toBe(true);
  });
}
