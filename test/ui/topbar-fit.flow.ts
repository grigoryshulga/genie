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
  const pill = box(window.innerWidth > 900 ? ".g-topbar-pill" : ".g-topbar-nav");
  // «Знания» is the middle of the three destinations: its pill (label and chevron) centres the dock.
  const board = box('.g-topbar-nav .g-tab-split:has(a[href="/docs"])');
  const create = box(".g-topbar-create");
  return {
    inner: window.innerWidth,
    innerHeight: window.innerHeight,
    docScroll: document.documentElement.scrollWidth,
    bar: bar.clientWidth,
    barBottom: bar.getBoundingClientRect().bottom,
    projectTop: project.top,
    projectBottom: project.bottom,
    dockTop: pill.top,
    dockBottom: pill.bottom,
    boardMiddle: (board.left + board.right) / 2,
    createTop: create.top,
    createBottom: create.bottom,
    links: [...bar.querySelectorAll(".g-topbar-nav a")].map((node) => node.getAttribute("href")),
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

    await expect(page.locator(".g-topbar-nav a")).toHaveCount(3);
    await page.evaluate(() => document.fonts.ready);
    const geometry = await page.evaluate(measure);
    expect(geometry.docScroll).toBeLessThanOrEqual(geometry.inner);
    expect(geometry.bar).toBe(geometry.inner);
    for (const control of geometry.controls) {
      expect(control.left).toBeGreaterThanOrEqual(0);
      expect(control.right).toBeLessThanOrEqual(width);
    }
    if (width > 900) {
      // The project sits immediately beside the pill; their group is centred.
      expect(geometry.projectRight).toBeLessThanOrEqual(geometry.pillLeft);
      expect(Math.abs((geometry.projectLeft + geometry.pillRight) / 2 - width / 2)).toBeLessThan(2);
      expect(geometry.pillLeft - geometry.projectRight).toBeLessThanOrEqual(12);
    } else {
      expect(geometry.links).toEqual(["/tasks", "/docs", "/manage/project/general"]);
      expect(geometry.boardMiddle).toBeCloseTo(width / 2, 0);
      expect(geometry.projectTop).toBeLessThan(64);
      expect(geometry.projectBottom).toBeLessThan(geometry.dockTop);
      expect(geometry.createBottom).toBeLessThan(geometry.dockTop);
      expect(geometry.createTop).toBeGreaterThan(geometry.innerHeight / 2);
      expect(geometry.innerHeight - geometry.dockBottom).toBeLessThanOrEqual(16);
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

for (const path of ["/board", "/tasks", "/epics", "/docs", "/manage/agents/roles"]) {
  test(`phone top strip keeps its intrinsic height on ${path}`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 1000 });
    await page.goto(path);
    await page.waitForLoadState("networkidle");
    await expect(page.locator(".g-topbar-project")).toBeVisible();
    const before = await page.locator(".g-topbar").boundingBox();
    expect(before!.height).toBeLessThanOrEqual(64);
    const project = (await page.locator(".g-topbar-project").boundingBox())!;
    expect(project.y).toBeLessThanOrEqual(16);
    await page.getByRole("button", { name: "Открыть поиск", exact: true }).click();
    const during = (await page.locator(".g-topbar").boundingBox())!;
    expect(during.height).toBeCloseTo(before!.height, 0);
    await page.getByRole("button", { name: "Закрыть поиск", exact: true }).click();
    await page.screenshot({ path: `test-results/mobile-top-strip-${path.slice(1).replaceAll("/", "-")}.png`, animations: "disabled" });
  });
}
