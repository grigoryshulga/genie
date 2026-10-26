import { expect, test } from "@playwright/test";

for (const theme of ["ink", "paper"]) for (const width of [1440, 390]) {
  test(`${theme} ${width}: real board and task detail render from fixtures`, async ({ page }) => {
    const errors: string[] = []; page.on("pageerror", e => errors.push(e.message));
    await page.goto(`/__preview?theme=${theme}&width=${width}&route=/board`);
    const app = page.frameLocator("#app");
    await expect(app.locator("html")).toHaveAttribute("data-theme", theme);
    await expect(app.locator("body")).toContainText("DM-2");
    expect(await app.locator("html").evaluate(el => el.clientWidth)).toBe(width);
    await page.getByLabel("Route").fill("/task/DM-2");
    await page.getByRole("button", { name: "Apply" }).click();
    await expect(app.locator("body")).toContainText("DM-2");
    expect(errors).toEqual([]);
    await page.screenshot({ path: `test-results/ui-preview-${theme}-${width}.png` });
  });
}

test("state controls show empty, pending and failure states and recover", async ({ page }) => {
  await page.goto("/__preview?route=/board");
  const app = page.frameLocator("#app");
  await expect(app.locator("body")).toContainText("DM-2");
  for (const state of ["empty", "loading", "error", "populated"]) {
    await page.getByLabel("State").selectOption(state);
    await page.getByRole("button", { name: "Apply" }).click();
    if (state === "populated") await expect(app.locator("body")).toContainText("DM-2");
    else {
      await expect(app.locator("body")).not.toContainText("DM-2");
      if (state === "empty") await expect(app.getByRole("heading", { name: "Здесь тихо" })).toBeVisible();
      if (state === "loading") await expect(app.locator('.g-board-skeleton[aria-busy="true"]')).not.toHaveCount(0);
      if (state === "error") await expect(app.locator("body")).toContainText("simulated server unavailable");
    }
  }
});

test("recorded docs, catalogue and settings render without frontend exceptions", async ({ page }) => {
  const errors: string[] = []; page.on("pageerror", e => errors.push(e.message));
  for (const route of ["/docs?page=demo/guides/deploy.md", "/agents", "/settings/me"]) {
    await page.goto(`/__preview?route=${encodeURIComponent(route)}`);
    await expect(page.frameLocator("#app").locator("main")).toBeVisible();
    await expect(page.frameLocator("#app").locator("body")).not.toContainText("No UI fixture");
  }
  expect(errors).toEqual([]);
});

for (const theme of ["ink", "paper"]) for (const width of [1440, 390]) {
  test(`role layout ${theme} ${width}: grouped grants and inline editing`, async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.route("**/api/agent-config", async (route) => {
      const response = await route.fetch();
      const cfg = await response.json();
      cfg.mcp.find((s: { id: string }) => s.id === "knowledge").tools = Array.from({ length: 60 }, (_, i) => ({ name: `read_document_${i + 1}` }));
      await route.fulfill({ response, json: cfg });
    });
    await page.goto(`/__preview?theme=${theme}&width=${width}&route=${encodeURIComponent("/agents?id=integration-analyst")}`);
    const app = page.frameLocator("#app");
    await expect(app.getByRole("heading", { name: "Аналитик интеграций", exact: true })).toBeVisible();
    await expect(app.locator(".ag-mcp-connection")).toHaveCount(3);
    await expect(app.locator(".ag-mcp-connection li").first()).not.toBeVisible();
    await app.getByRole("button", { name: "Редактировать роль", exact: true }).click();
    await expect(app.getByRole("dialog")).toHaveCount(0);
    await expect(app.getByLabel("Название", { exact: true })).toHaveValue("Аналитик интеграций");
    const tools = app.locator(".ag-mcp-tool-picker");
    await expect(tools).toHaveCount(1);
    await expect(tools.getByRole("checkbox").first()).not.toBeVisible();
    await tools.locator("summary").click();
    await app.getByRole("searchbox", { name: "Поиск инструментов knowledge" }).fill("read_document_48");
    await expect(tools.getByRole("checkbox")).toHaveCount(1);
    await expect(tools.getByRole("checkbox")).toBeChecked();
    await tools.getByRole("checkbox").uncheck();
    // The fixture server rejects writes explicitly; the draft must remain editable.
    await app.getByLabel("Описание", { exact: true }).fill("Черновик");
    await app.getByRole("button", { name: "Сохранить", exact: true }).click();
    await expect(app.getByRole("alert")).toContainText("read-only");
    await expect(app.getByLabel("Описание", { exact: true })).toHaveValue("Черновик");
    expect(await app.locator("html").evaluate((el) => el.scrollWidth <= el.clientWidth)).toBe(true);
    await app.getByRole("button", { name: "Отмена", exact: true }).click();
    await expect(app.locator(".ag-role-editor")).toHaveCount(0);
    expect(errors).toEqual([]);
  });
}
