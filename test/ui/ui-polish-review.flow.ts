import { expect, test } from "@playwright/test";
import { FLOW_DOC } from "./seed.ts";

for (const theme of ["ink", "paper"]) {
  test(`review decisions use existing transitions and retain failed return text (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    const task = await (await page.request.get("/api/tasks/DM-2")).json();
    let status = "review";
    let failed = true;
    await page.route("**/api/tasks/DM-2", (route) => route.fulfill({ json: { ...task, status } }));
    let body: unknown;
    await page.route("**/api/tasks/DM-2/status", async (route) => {
      body = route.request().postDataJSON();
      if (failed) await route.fulfill({ status: 503, json: { error: "Сервис временно недоступен" } });
      else { status = "changes_requested"; await route.fulfill({ json: { ...task, status } }); }
    });
    await page.goto("/task/DM-2");
    const band = page.getByRole("region", { name: "Решение по ревью" });
    await expect(band.getByRole("button", { name: "Одобрить", exact: true })).toBeInViewport();
    await band.getByRole("button", { name: "Одобрить", exact: true }).click();
    await expect(band.getByRole("alert")).toBeVisible();
    expect(body).toEqual({ status: "approved" });
    await band.getByRole("button", { name: "Вернуть на доработку", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Вернуть на доработку", exact: true });
    await expect(dialog.getByRole("textbox", { name: "Что нужно исправить", exact: true })).toBeFocused();
    await dialog.getByRole("textbox", { name: "Что нужно исправить", exact: true }).fill("Добавить обработку ошибки");
    await dialog.getByRole("button", { name: "Вернуть на доработку", exact: true }).click();
    await expect(dialog.getByRole("alert")).toBeVisible();
    expect(body).toEqual({ status: "changes_requested", note: "Добавить обработку ошибки" });
    await expect(dialog.getByRole("textbox")).toHaveValue("Добавить обработку ошибки");
    failed = false;
    await dialog.getByRole("button", { name: "Вернуть на доработку", exact: true }).click();
    await expect(band).toBeHidden();
    await expect(page.locator(".td-headline .title")).toBeFocused();
  });

  test(`work navigation names its actual destination (${theme})`, async ({ page }) => {
    for (const [path, label] of [["/board", "Доска"], ["/tasks", "Задачи"], ["/epics", "Эпики"]]) {
      await page.goto(path);
      const link = page.locator(".g-topbar-nav").getByRole("link", { name: new RegExp(label) }).first();
      await expect(link).toHaveAttribute("href", path);
      await expect(link).toHaveAttribute("aria-current", "page");
    }
  });

  test(`phone editor keeps clean save quiet and secondary formatting reachable (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto(`/docs/edit?page=${encodeURIComponent(FLOW_DOC.path)}`);
    const save = page.getByRole("button", { name: "Сохранить", exact: true });
    await expect(save).toBeDisabled();
    const toolbar = page.getByRole("toolbar", { name: "Форматирование статьи" });
    expect((await toolbar.boundingBox())!.height).toBeLessThanOrEqual(112);
    await page.getByRole("button", { name: "Другие инструменты форматирования", exact: true }).click();
    await expect(page.getByRole("menuitem", { name: "Ссылка", exact: true })).toBeVisible();
    await page.keyboard.press("Escape");
    await page.getByRole("textbox", { name: "Текст статьи", exact: true }).press("End");
    await page.keyboard.type(" новая правка");
    await expect(save).toBeEnabled();
  });
}

for (const theme of ["ink", "paper"]) {
  test(`phone task selects activate across the full visible pill (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/tasks");
    for (const name of ["Порядок задач", "Группы задач"]) {
      const select = page.getByRole("combobox", { name, exact: true });
      const bounds = (await select.boundingBox())!;
      const pill = (await select.locator("..").boundingBox())!;
      expect(bounds.height).toBeGreaterThanOrEqual(40);
      expect(bounds.width).toBeCloseTo(pill.width - 2, 0);
      await page.mouse.click(bounds.x + 5, bounds.y + bounds.height - 5);
      await expect.poll(() => select.evaluate((node) => node.matches(":open"))).toBe(true);
      await page.keyboard.press("Escape");
    }
  });
  test(`phone article starts with prose and retains its page actions (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto(`/docs?page=${encodeURIComponent(FLOW_DOC.path)}`);
    const prose = page.locator(".doc-reading .doc-body .md p").first();
    await expect(prose).toBeVisible();
    expect((await prose.boundingBox())!.y).toBeLessThan(400);
    await page.getByRole("button", { name: "Действия страницы", exact: true }).click();
    for (const name of ["Редактировать", "Клонировать", "Удалить", "История версий", "Изменения документации"]) await expect(page.getByRole("menuitem", { name, exact: true })).toBeVisible();
  });
}
