import { expect, test } from "@playwright/test";
import { FLOW_DOC } from "./seed.ts";
import type { GitConnections } from "../../web/src/entities/repo/model.ts";

for (const theme of ["ink", "paper"]) for (const width of [390, 1440]) {
  test(`page actions are visible, keyboard reachable and stay bounded (${theme}, ${width})`, async ({ page }) => {
    await page.setViewportSize({ width, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto(`/docs?page=${encodeURIComponent(FLOW_DOC.path)}`);
    const trigger = page.getByRole("button", { name: "Действия страницы", exact: true });
    await trigger.focus();
    await page.keyboard.press("Enter");
    const menu = page.getByRole("menu", { name: "Действия страницы", exact: true });
    await expect(menu).toBeInViewport({ ratio: 1 });
    await expect(menu.getByRole("menuitem").first()).toBeFocused();
    await expect(menu.getByRole("menuitem").first()).toBeInViewport({ ratio: 1 });
    await page.setViewportSize({ width: width === 390 ? 360 : 1100, height: 700 });
    await expect(menu).toBeInViewport({ ratio: 1 });
    await page.keyboard.press("Escape");
    await expect(trigger).toBeFocused();
    await trigger.click();
    await menu.getByRole("menuitem", { name: /Изменения документации/ }).click();
    await expect(page).toHaveURL(/\/docs\/changes$/);
  });
}

for (const theme of ["ink", "paper"]) {
  test(`phone repository health details have a full touch target (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    const connections: GitConnections = {
      github: { configured: true, slug: "genie-dev", host: "github" }, canConnect: true,
      connections: [{ id: "health-test", provider: "github", host: "github", owner: 1, ownerLogin: "owner", accountLogin: "team-health", accountType: "Organization", installationId: 1, selection: "selected", health: "needs_attention", statusNote: "Требуется проверка доступа", checked: new Date().toISOString(), repos: 1, blocked: "Нужно подтвердить доступ" }],
    };
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.route("**/api/git/connections", (route) => route.fulfill({ json: connections }));
    await page.goto("/manage/project/repos");
    const trigger = page.getByRole("button", { name: "team-health: нужно внимание. Подробности", exact: true });
    await trigger.scrollIntoViewIfNeeded();
    const bounds = (await trigger.boundingBox())!;
    expect(bounds.width).toBeGreaterThanOrEqual(40);
    expect(bounds.height).toBeGreaterThanOrEqual(40);
    await page.mouse.click(bounds.x + bounds.width / 2, bounds.y + 5);
    await expect(page.getByRole("dialog")).toBeInViewport({ ratio: 1 });
    await page.keyboard.press("Escape");
    await expect(trigger).toBeFocused();
  });
  test(`phone formatting overflow is visible and restores focus (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto(`/docs/edit?page=${encodeURIComponent(FLOW_DOC.path)}`);
    const trigger = page.getByRole("button", { name: "Другие инструменты форматирования", exact: true });
    await trigger.click();
    const menu = page.getByRole("menu", { name: "Другие инструменты форматирования", exact: true });
    await expect(menu).toBeInViewport({ ratio: 1 });
    await expect(menu.getByRole("menuitem").first()).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(trigger).toBeFocused();
  });
  test(`expanded template drawing fills the available dialog (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/manage/agents/templates");
    await page.getByRole("button", { name: "Развернуть схему", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Развёрнутая схема команды", exact: true });
    await expect(dialog).toBeInViewport({ ratio: 1 });
    expect((await dialog.locator(".team-graph-scroll").boundingBox())!.height).toBeGreaterThan(500);
    await page.keyboard.press("Escape");
    await expect(page.getByRole("button", { name: "Развернуть схему", exact: true })).toBeFocused();
  });
}
