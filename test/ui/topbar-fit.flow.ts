// A long project name must not widen the bar past the window: the name ellipsizes and the icons stay
// on screen. Regression: below 900px the single grid column grew to the name's full width, the bar
// stretched with it, and the page scrolled sideways.
import { expect, test } from "@playwright/test";

// Longer than the name box at the widest viewport we test, so the clipping path is always the one
// under test: a name that happens to fit would make the overflow assertion meaningless.
const LONG = Array(3).fill("genie-platform-integration-observatory-and-observability-console").join("-");

/** Reads the bar's geometry; writes the name first so one pass cannot be split by a refetch. */
const measure = (value?: string) => {
  const bar = document.querySelector<HTMLElement>(".g-topbar")!;
  const node = document.querySelector<HTMLElement>(".g-topbar-project .name")!;
  if (value !== undefined) node.textContent = value;
  const trigger = document.querySelector<HTMLElement>(".g-topbar-project")!.getBoundingClientRect();
  const box = node.getBoundingClientRect();
  const style = getComputedStyle(node);
  return {
    inner: window.innerWidth,
    docScroll: document.documentElement.scrollWidth,
    bar: bar.clientWidth,
    rightmost: Math.max(...Array.from(bar.children).map((child) => child.getBoundingClientRect().right)),
    switcherRight: document.querySelector<HTMLElement>(".g-topbar-project-switcher")!.getBoundingClientRect().right,
    triggerRight: trigger.right,
    nameRight: box.right,
    nameClient: node.clientWidth,
    nameScroll: node.scrollWidth,
    nameHeight: node.clientHeight,
    fontSize: parseFloat(style.fontSize),
    overflow: style.textOverflow,
    wrap: style.whiteSpace,
  };
};

for (const width of [1440, 1180, 901, 900, 768, 430, 390, 360]) {
  test(`a long project name stays inside the bar at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 860 });
    await page.goto("/tasks");
    // Measure the loaded page, not the chunk fallback that precedes it.
    await page.locator("main.g-tasks-page").waitFor();
    await page.locator(".g-topbar-project .name").waitFor({ state: "visible" });

    // The everyday name is short: the fix must not clip it or start scrolling the page.
    const short = await page.evaluate(measure, undefined as string | undefined);
    expect(short.docScroll).toBeLessThanOrEqual(short.inner);
    expect(short.bar).toBe(short.inner);
    // Below 390px even the short name is clipped; what matters is that the bar stays inside the page.
    if (width >= 390) expect(short.nameScroll).toBeLessThanOrEqual(short.nameClient + 1);

    const long = await page.evaluate(measure, LONG);

    expect(long.docScroll).toBeLessThanOrEqual(long.inner);
    expect(long.bar).toBe(long.inner);
    expect(long.rightmost).toBeLessThanOrEqual(long.inner);
    expect(long.switcherRight).toBeLessThanOrEqual(long.inner);
    // The name gives way instead of pushing the trigger past the bar, and stays on one line.
    expect(long.nameRight).toBeLessThanOrEqual(long.triggerRight + 1);
    expect(long.overflow).toBe("ellipsis");
    expect(long.wrap).toBe("nowrap");
    expect(long.nameHeight).toBeLessThan(long.fontSize * 2);
    expect(long.nameScroll).toBeGreaterThan(long.nameClient);
    // Something of the name stays readable wherever the bar has room for it.
    if (width >= 390) expect(long.nameClient).toBeGreaterThan(20);
  });
}
