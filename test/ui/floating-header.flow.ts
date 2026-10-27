import { expect, test } from "@playwright/test";

for (const width of [1440, 390]) {
  for (const theme of ["ink", "paper"]) {
    test(`search replaces navigation and preserves utilities (${width}, ${theme})`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
      await page.goto("/board");
      const bar = page.locator(".g-topbar");
      const toggle = bar.getByRole("button", { name: "Открыть поиск" });
      await expect(page.getByRole("searchbox")).toHaveCount(0);
      await page.waitForLoadState("networkidle");
      await page.evaluate(() => document.fonts.ready);
      const surface = width > 900 ? bar.locator(".g-topbar-pill") : bar;
      const initial = (await surface.boundingBox())!;
      await toggle.click();
      const search = bar.getByRole("searchbox", { name: "Поиск на доске" });
      await expect(search).toBeFocused();
      const opened = (await surface.boundingBox())!;
      expect(opened.width).toBeCloseTo(initial.width, 0);
      expect(opened.height).toBeCloseTo(initial.height, 0);
      expect(opened.x).toBeCloseTo(initial.x, 0);
      expect(opened.y).toBeCloseTo(initial.y, 0);
      if (width > 900) await expect(bar.getByRole("navigation")).toBeHidden();
      else await expect(bar.getByRole("navigation")).toBeVisible();
      await expect(bar.getByRole("button", { name: /^Проект / })).toBeHidden();
      await expect(bar.getByRole("button", { name: "Новая задача", exact: true })).toBeVisible();
      await expect(bar.getByRole("button", { name: /^Уведомления/ })).toBeVisible();
      await expect(bar.getByRole("button", { name: /^Профиль:/ })).toBeVisible();
      const fieldBox = (await bar.locator(".g-topbar-search").boundingBox())!;
      const closeBox = (await bar.getByRole("button", { name: "Закрыть поиск" }).boundingBox())!;
      expect(fieldBox.x + fieldBox.width).toBeLessThanOrEqual(closeBox.x);
      await expect(bar.locator(".g-topbar-search")).toHaveCSS("outline-style", "none");
      await search.fill("фильтруется");
      await expect(page).toHaveURL(/[?&]q=/);
      await page.screenshot({ path: `test-results/search-${theme}-${width}.png`, animations: "disabled" });
      await search.press("Escape");
      await expect(bar.getByRole("navigation")).toBeVisible();
      await expect(toggle).toBeFocused();
      await expect(page).not.toHaveURL(/[?&]q=/);
      await page.keyboard.press("/");
      await expect(search).toBeFocused();
      await bar.getByRole("button", { name: "Закрыть поиск" }).click();
      await expect(toggle).toBeFocused();
      await page.keyboard.press("/");
      await expect(search).toBeFocused();
      await page.emulateMedia({ reducedMotion: "reduce" });
      await expect(bar.locator(".g-topbar-search-mode")).toHaveCSS("animation-name", "none");
    });
  }
}

test("project gear switches the selected project before opening its management", async ({ page }) => {
  let selected = "demo";
  let release = () => {};
  const held = new Promise<void>((resolve) => { release = resolve; });
  await page.route("**/api/auth/me", async (route) => {
    const response = await route.fetch();
    const session = await response.json();
    selected = selected === "demo" ? session.project : selected;
    await route.fulfill({ json: { ...session, project: selected, projects: [...session.projects, { ...session.projects[0], slug: "other", name: "Другой проект" }] } });
  });
  await page.route("**/api/session/project", async (route) => {
    await held;
    selected = route.request().postDataJSON().project;
    await route.fulfill({ json: { ok: true } });
  });
  await page.goto("/board");
  await page.getByRole("button", { name: /^Проект / }).click();
  const menu = page.getByRole("menu", { name: "Проекты", exact: true });
  const first = menu.getByRole("menuitemcheckbox").first();
  await expect(first).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menu.getByRole("menuitem", { name: /^Управление проектом/ }).first()).toBeFocused();
  await menu.getByRole("menuitem", { name: "Управление проектом Другой проект", exact: true }).click();
  await expect(page).toHaveURL(/\/board$/);
  release();
  await expect(page).toHaveURL(/\/manage\/project\/general$/);
  await expect(page.locator(".st-rail-head")).toContainText("Другой проект");
});

test("a failed project switch keeps the person in the current project", async ({ page }) => {
  await page.route("**/api/auth/me", async (route) => {
    const response = await route.fetch();
    const session = await response.json();
    await route.fulfill({ json: { ...session, projects: [...session.projects, { ...session.projects[0], slug: "other", name: "Другой проект" }] } });
  });
  await page.route("**/api/session/project", (route) => route.fulfill({ status: 503, json: { error: "Переключение недоступно" } }));
  await page.goto("/board");
  await page.getByRole("button", { name: /^Проект / }).click();
  await page.getByRole("menuitem", { name: "Управление проектом Другой проект", exact: true }).click();
  await expect(page.locator(".g-toast-region")).toContainText("Переключение недоступно");
  await expect(page).toHaveURL(/\/board$/);
});

for (const role of ["owner", "member", "viewer"]) {
  test(`management retains access to operational tools for ${role}`, async ({ page }) => {
    await page.route("**/api/auth/me", async (route) => {
      const response = await route.fetch();
      const session = await response.json();
      await route.fulfill({ json: { ...session, user: { ...session.user, isAdmin: false }, projects: session.projects.map((p: object) => ({ ...p, role })) } });
    });
    await page.goto("/automations?rule=1&q=demo#runs");
    await expect(page).toHaveURL(/\/manage\/agents\/automations\/runs\?rule=1&q=demo#runs$/);
    // The hub stays under the top bar: «Управление» is the current destination, not a separate mode.
    await expect(page.getByRole("navigation", { name: "Навигация", exact: true }).getByRole("link", { name: "Управление" })).toHaveAttribute("aria-current", "page");
    const rail = page.getByRole("navigation", { name: "Управление", exact: true });
    await expect(rail.getByRole("link", { name: "Автоматизации", exact: true })).toHaveAttribute("aria-current", "page");
    await page.locator(".st-tabs-band").getByRole("link", { name: "Наблюдения", exact: true }).click();
    await expect(page).toHaveURL(/\/manage\/agents\/automations\/observations$/);
    await expect(page.getByRole("heading", { name: "Наблюдения", exact: true })).toBeVisible();
    await page.goto("/observation/O-1?tab=contract#evidence");
    await expect(page).toHaveURL(/\/manage\/agents\/automations\/observation\/O-1\?tab=contract#evidence$/);
  });
}

for (const [name, width, height] of [["wide", 160, 40], ["tall", 40, 160], ["square", 100, 100]] as const) {
  test(`the entire ${name} project image fits inside its circle`, async ({ page }) => {
    const icon = `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}"><rect width="100%" height="100%" fill="orange"/></svg>`)}`;
    await page.route("**/api/auth/me", async (route) => {
      const response = await route.fetch();
      const session = await response.json();
      await route.fulfill({ json: { ...session, projects: session.projects.map((p: object) => ({ ...p, icon })) } });
    });
    await page.goto("/board");
    const image = page.locator(".g-topbar-project img");
    await expect(image).toHaveCSS("object-fit", "contain");
    await expect.poll(() => image.evaluate((node) => node instanceof HTMLImageElement && node.complete && node.naturalWidth > 0)).toBe(true);
    const geometry = await image.evaluate((node) => {
      if (!(node instanceof HTMLImageElement)) throw new Error("Expected a project image");
      const style = getComputedStyle(node);
      const box = node.getBoundingClientRect();
      const width = box.width - parseFloat(style.paddingLeft) - parseFloat(style.paddingRight);
      const height = box.height - parseFloat(style.paddingTop) - parseFloat(style.paddingBottom);
      const scale = Math.min(width / node.naturalWidth, height / node.naturalHeight);
      return { diagonal: Math.hypot(node.naturalWidth * scale, node.naturalHeight * scale), diameter: box.width };
    });
    expect(geometry.diagonal).toBeLessThan(geometry.diameter);
  });
}

for (const width of [1440, 390]) {
  test(`the active pill slides between destinations and stays aligned at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/board");
    await page.waitForLoadState("networkidle");
    await page.evaluate(() => document.fonts.ready);
    const nav = page.getByRole("navigation", { name: "Навигация", exact: true });
    const indicator = nav.locator(".g-tabs-indicator");
    // Each destination is a label-and-chevron pair; the pill covers the whole pair.
    const active = nav.locator(".g-tab-split[data-current]");
    const aligned = async () => {
      const fill = (await indicator.boundingBox())!;
      const tab = (await active.boundingBox())!;
      return Math.max(Math.abs(fill.x - tab.x), Math.abs(fill.y - tab.y), Math.abs(fill.width - tab.width), Math.abs(fill.height - tab.height));
    };
    await expect.poll(aligned).toBeLessThan(0.5);
    await nav.getByRole("link", { name: /Знания/ }).click();
    await expect.poll(() => indicator.evaluate((node) => node.getAnimations().filter((animation) => animation.playState === "running").length)).toBeGreaterThan(0);
    await expect.poll(aligned).toBeLessThan(0.5);
    // Rapid changes must settle under the final destination, not an earlier animation's target.
    await nav.getByRole("link", { name: /Управление/ }).click();
    await nav.getByRole("link", { name: /Задачи/ }).click();
    await expect.poll(aligned).toBeLessThan(0.5);
    await page.setViewportSize({ width: width > 900 ? 1000 : 360, height: 900 });
    await expect.poll(aligned).toBeLessThan(0.5);
    await nav.getByRole("link", { name: /Задачи/ }).focus();
    await page.keyboard.press("ArrowRight");
    await expect(nav.getByRole("link", { name: /Знания/ })).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/\/docs$/);
    await expect.poll(aligned).toBeLessThan(0.5);
    await page.emulateMedia({ reducedMotion: "reduce" });
    await expect(indicator).toHaveCSS("transition-duration", "0s");
    await nav.getByRole("link", { name: /Задачи/ }).click();
    await expect.poll(aligned).toBeLessThan(0.5);
  });
}
