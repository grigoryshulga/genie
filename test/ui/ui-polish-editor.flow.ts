import { expect, test } from "@playwright/test";
import { FLOW_DOC } from "./seed.ts";

for (const theme of ["ink", "paper"]) {
  for (const [kind, content] of [["list", "- X"], ["table", "| A |\n| --- |\n| X |"]]) {
    test(`editor focus and navigation preserve clean Markdown (${theme}, ${kind})`, async ({ page }) => {
      await page.setViewportSize({ width: 390, height: 844 });
      await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
      const original = await (await page.request.get(`/api/docs/page?path=${encodeURIComponent(FLOW_DOC.path)}`)).json();
      await page.route("**/api/docs/page?*", (route) => route.fulfill({ json: { ...original, content } }));
      await page.goto(`/docs/edit?page=${encodeURIComponent(FLOW_DOC.path)}`);
      const editor = page.getByRole("textbox", { name: "Текст статьи", exact: true });
      const state = page.locator(".doc-editor-footer .doc-chip-state").first();
      await expect(editor).toBeVisible();
      await expect(state).toHaveText("Сохранено");
      await editor.focus();
      await page.keyboard.press("ArrowRight");
      await expect(state).toHaveText("Сохранено", { timeout: 1000 });
      await page.getByRole("button", { name: "Markdown", exact: true }).click();
      await expect(page.getByRole("textbox", { name: "Markdown статьи", exact: true })).toHaveValue(content);
      await page.getByRole("button", { name: "Статья", exact: true }).click();
      await editor.focus();
      await page.keyboard.press("Control+End");
      await page.keyboard.type("Добавлено");
      await expect(state).toContainText("Не сохранено");
      await page.keyboard.press("Control+z");
      await expect(state).toHaveText("Сохранено", { timeout: 1000 });
      await page.keyboard.press("Control+Shift+z");
      await expect(state).toContainText("Не сохранено");
      await page.getByRole("button", { name: "Markdown", exact: true }).click();
      await expect(page.getByRole("textbox", { name: "Markdown статьи", exact: true })).toHaveValue(/Добавлено/);
    });
  }
}

for (const theme of ["ink", "paper"]) {
  test(`rich editor keeps wide tables local and has full phone controls and focus (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    const original = await (await page.request.get(`/api/docs/page?path=${encodeURIComponent(FLOW_DOC.path)}`)).json();
    const content = "# Статья\n\nОбычный текст.\n\n| A | B | C | D |\n| --- | --- | --- | --- |\n| very_long_unbreakable_value_1 | very_long_unbreakable_value_2 | very_long_unbreakable_value_3 | very_long_unbreakable_value_4 |";
    await page.route("**/api/docs/page?*", (route) => route.fulfill({ json: { ...original, content } }));
    await page.goto(`/docs/edit?page=${encodeURIComponent(FLOW_DOC.path)}`);
    const editor = page.getByRole("textbox", { name: "Текст статьи", exact: true });
    const table = editor.locator(".tableWrapper");
    await expect(table).toBeVisible();
    const scroll = await table.evaluate((node) => ({ client: node.clientWidth, content: node.scrollWidth }));
    expect(scroll.content).toBeGreaterThan(scroll.client);
    const prose = await page.locator(".doc-rich-editor").evaluate((node) => ({ client: node.clientWidth, content: node.scrollWidth }));
    expect(prose.content).toBeLessThanOrEqual(prose.client + 1);
    const before = (await editor.locator("h1").boundingBox())!.x;
    await table.evaluate((node) => { node.scrollLeft = 200; });
    expect((await editor.locator("h1").boundingBox())!.x).toBeCloseTo(before, 0);
    await editor.focus();
    await expect(editor).toHaveCSS("outline-style", "solid");
    await expect(editor).toHaveCSS("outline-width", "2px");
    for (const button of await page.locator(".doc-format-toolbar button:visible").all()) {
      const bounds = (await button.boundingBox())!;
      expect(bounds.width).toBeGreaterThanOrEqual(40);
      expect(bounds.height).toBeGreaterThanOrEqual(40);
      expect(bounds.x).toBeGreaterThanOrEqual(0);
      expect(bounds.x + bounds.width).toBeLessThanOrEqual(390);
    }
  });

  test(`editor read error offers retry and a return to Knowledge (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    let failed = true;
    await page.route("**/api/docs/page?*", (route) => failed ? route.fulfill({ status: 503, json: { error: "Temporary read failure" } }) : route.continue());
    await page.goto(`/docs/edit?page=${encodeURIComponent(FLOW_DOC.path)}`);
    const error = page.getByRole("alert");
    await expect(error).toContainText("Не удалось загрузить статью для редактирования");
    await expect(page.getByRole("link", { name: "К знаниям", exact: true })).toHaveAttribute("href", "/docs");
    failed = false;
    await error.getByRole("button", { name: "Повторить", exact: true }).click();
    await expect(page.getByRole("textbox", { name: "Текст статьи", exact: true })).toBeVisible();
    await expect(error).toBeHidden();
  });
}
