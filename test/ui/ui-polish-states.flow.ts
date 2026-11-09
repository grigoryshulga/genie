import { expect, test } from "@playwright/test";
import { FLOW_DOC } from "./seed.ts";

for (const theme of ["ink", "paper"]) {
  test(`palette exposes the active result and a keyboard reachable search scope (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/board");
    await expect(page.getByRole("button", { name: "Новая задача", exact: true })).toBeVisible();
    await page.keyboard.press("Control+k");
    const search = page.getByRole("combobox", { name: "Поиск команд, задач и документации", exact: true });
    const list = page.getByRole("listbox", { name: "Результаты поиска", exact: true });
    await expect(search).toBeFocused();
    await expect(search).toHaveAttribute("aria-controls", (await list.getAttribute("id"))!);
    const first = await search.getAttribute("aria-activedescendant");
    await page.keyboard.press("ArrowDown");
    expect(await search.getAttribute("aria-activedescendant")).not.toBe(first);
    await expect(search).toBeFocused();
    for (let i = 0; i < 20; i++) await page.keyboard.press("ArrowDown");
    const active = list.getByRole("option", { selected: true });
    await expect(active).toHaveAttribute("id", (await search.getAttribute("aria-activedescendant"))!);
    await expect(active).toBeInViewport({ ratio: 1 });
    const scope = page.getByRole("checkbox", { name: "Только документация", exact: true });
    await page.keyboard.press("Tab");
    await expect(scope).toBeFocused();
    await expect(scope).not.toBeChecked();
    await page.keyboard.press("Space");
    await expect(scope).toBeChecked();
    await page.keyboard.press("Shift+Tab");
    await expect(search).toBeFocused();
    await search.fill("Как запустить сервер");
    await expect(list.getByRole("option", { name: /^Как запустить сервер(?:\s|$)/ }).first()).toBeVisible();
    await expect(list.getByRole("option", { selected: true })).toHaveAttribute("id", (await search.getAttribute("aria-activedescendant"))!);
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(new RegExp(`page=${encodeURIComponent(FLOW_DOC.path)}`));
  });

  test(`long Knowledge drawer rows retain their complete rename target (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    const response = await page.request.get("/api/docs/tree");
    const tree = await response.json();
    const title = "Восстановление базы из резервной копии после сбоя рабочего сервера";
    await page.route("**/api/docs/tree", (route) => route.fulfill({ json: { ...tree, pages: tree.pages.map((p: { path: string }) => p.path === FLOW_DOC.path ? { ...p, title, type: "guide", status: "current", stale: true } : p) } }));
    await page.goto(`/docs?page=${encodeURIComponent(FLOW_DOC.path)}`);
    await page.getByRole("button", { name: "Открыть дерево документации", exact: true }).click();
    const drawer = page.getByRole("dialog", { name: "Навигация по документации", exact: true });
    await expect(drawer).toBeVisible();
    const edge = (await drawer.boundingBox())!;
    const rename = drawer.getByRole("button", { name: "Переименовать статью", exact: true });
    await expect(rename.first()).toBeVisible();
    for (const button of await rename.all()) {
      const bounds = (await button.boundingBox())!;
      expect(bounds.width).toBeGreaterThanOrEqual(40);
      expect(bounds.x + bounds.width).toBeLessThanOrEqual(edge.x + edge.width + 1);
    }
    await rename.first().click();
    const input = drawer.getByRole("textbox", { name: "Название статьи", exact: true });
    await expect(input).toHaveValue(title);
    expect((await input.boundingBox())!.x + (await input.boundingBox())!.width).toBeLessThanOrEqual(edge.x + edge.width);
  });

  test(`empty MCP catalogue has a named setup action for administrators (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/manage/agents/mcp");
    const configure = page.getByRole("link", { name: "Настроить подключения", exact: true });
    await expect(configure).toBeVisible();
    await configure.click();
    await expect(page).toHaveURL(/\/manage\/server\/tools\?tab=mcp$/);
  });

  test(`global creation stays quiet beside a contextual task action (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    for (const [path, action] of [["/task/DM-2", "Собрать команду"], ["/epic/DM-1", "Задача в эпик"]]) {
      await page.goto(path);
      const create = page.getByRole("button", { name: "Новая задача", exact: true });
      const contextual = page.getByRole("button", { name: action, exact: true });
      await expect(contextual).toBeVisible();
      const color = await contextual.evaluate((node) => getComputedStyle(node).backgroundColor);
      expect(await create.evaluate((node) => getComputedStyle(node).backgroundColor)).not.toBe(color);
      await create.click();
      await expect(page.getByRole("dialog", { name: "Новая задача", exact: true })).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(create).toBeFocused();
    }
  });
}

test("MCP setup is explained without offering server settings to a non-admin", async ({ page }) => {
  await page.route("**/api/auth/me", async (route) => {
    const response = await route.fetch();
    const session = await response.json();
    await route.fulfill({ json: { ...session, user: { ...session.user, isAdmin: false } } });
  });
  await page.goto("/manage/agents/mcp");
  await expect(page.getByText("Подключения MCP настраивает администратор сервера.", { exact: true })).toBeVisible();
  await expect(page.getByRole("link", { name: "Настроить подключения", exact: true })).toHaveCount(0);
});

test("Skills empty state names skills", async ({ page }) => {
  const response = await page.request.get("/api/projects/demo/tools");
  const tools = await response.json();
  await page.route("**/api/projects/demo/tools", (route) => route.fulfill({ json: { ...tools, skills: [] } }));
  await page.goto("/manage/agents/skills/project");
  await expect(page.getByText("В каталоге сервера пока нет доступных навыков.", { exact: true })).toBeVisible();
});
