import { expect, test } from "@playwright/test";

for (const theme of ["ink", "paper"]) {
  for (const width of [1440, 390]) {
    test(`task section controls align and remain usable (${theme}, ${width})`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto("/tasks?task=DM-5");
      await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
      const modal = page.getByRole("dialog", { name: "Задача DM-5", exact: true });
      const criteria = modal.locator(".sec").filter({ has: page.getByRole("button", { name: "Критерии приёмки 0 из 0" }) });
      const heading = criteria.locator(".sec-head");
      const count = criteria.locator(".n");
      await expect(count).toHaveText("0 из 0");
      expect(Math.abs((await heading.boundingBox())!.x + (await heading.boundingBox())!.width - (await count.boundingBox())!.x - (await count.boundingBox())!.width)).toBeLessThan(2);
      const observation = modal.locator(".sec").filter({ has: page.getByRole("button", { name: "Наблюдение", exact: true }) });
      const add = observation.getByRole("button", { name: "Создать наблюдение", exact: true });
      await expect(add).toHaveText("");
      const row = (await observation.locator(".sec-head").boundingBox())!;
      const control = (await add.boundingBox())!;
      expect(Math.abs(control.x + control.width - row.x - row.width)).toBeLessThan(2);
      expect(control.height).toBeGreaterThanOrEqual(40);
      await modal.getByRole("button", { name: "Изменить описание", exact: true }).click();
      const editor = modal.getByRole("textbox", { name: "Описание", exact: true });
      await editor.focus();
      await expect(editor).toHaveCSS("outline-style", "none");
      await expect(editor.locator("..")).toHaveCSS("outline-style", "solid");
      await editor.fill("Проверка единой рамки фокуса");
      await editor.locator("..").getByRole("button", { name: "Отмена", exact: true }).click();
      const bar = (await modal.locator(".td-bar").boundingBox())!;
      const fullPage = (await modal.getByRole("link", { name: "Открыть страницу" }).boundingBox())!;
      expect(fullPage.y - bar.y).toBeGreaterThanOrEqual(6);
      expect(bar.y + bar.height - fullPage.y - fullPage.height).toBeGreaterThanOrEqual(6);
      await add.click();
      await expect(observation.getByRole("form", { name: "Создать наблюдение" })).toBeVisible();
      await expect(observation.getByRole("button", { name: "Наблюдение", exact: true })).toHaveAttribute("aria-expanded", "true");
      await observation.getByRole("button", { name: "Отмена" }).click();
      await add.focus();
      await page.keyboard.press("Enter");
      await expect(observation.getByRole("form", { name: "Создать наблюдение" })).toBeVisible();
      await page.screenshot({ path: `test-results/task-sections-${theme}-${width}.png`, animations: "disabled" });
    });
  }

  test(`board move menu stays above following cards and returns focus (${theme})`, async ({ page }) => {
    await page.goto("/board");
    await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
    const lane = page.locator('[data-lane="maybe"]');
    await lane.locator(".g-lane-toggle").click();
    const opener = lane.getByRole("button", { name: "Переместить DM-1", exact: true });
    await opener.click();
    const menu = page.getByRole("menu", { name: "Переместить DM-1 в", exact: true });
    await expect(menu).toBeVisible();
    expect(await menu.evaluate((node) => node.parentElement!.matches(":popover-open"))).toBe(true);
    const last = menu.getByRole("menuitem").last();
    await last.scrollIntoViewIfNeeded();
    expect(await last.evaluate((node) => {
      const rect = node.getBoundingClientRect();
      return node.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
    })).toBe(true);
    await page.keyboard.press("Escape");
    await expect(menu).toBeHidden();
    await expect(opener).toBeFocused();
  });
}

for (const theme of ["ink", "paper"]) {
  test(`themed native select keeps keyboard selection and persists (${theme})`, async ({ page }) => {
    await page.goto("/tasks?task=DM-5");
    await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
    const select = page.getByRole("combobox", { name: "Приоритет", exact: true });
    await expect(select).toHaveCSS("appearance", "base-select");
    await select.focus();
    await page.keyboard.press("Space");
    await page.keyboard.press("Home");
    await page.keyboard.press("Enter");
    await expect(select).toHaveValue("0");
    await page.reload();
    await expect(select).toHaveValue("0");
    await select.focus();
    await page.keyboard.press("Space");
    await page.keyboard.press("End");
    await page.keyboard.press("Escape");
    await expect(select).toHaveValue("0");
    await select.selectOption("3");
    await expect(select).toHaveValue("3");
  });
}
