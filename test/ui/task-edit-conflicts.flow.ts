import { expect, test } from "@playwright/test";
import { storedTask, type StoredTask } from "./api.ts";
import { PROJECT } from "./seed.ts";

for (const type of ["task", "epic"] as const) {
  for (const theme of ["ink", "paper"] as const) {
    test(`a ${type} title retains its text and version through refresh, conflict and refocus in ${theme}`, async ({ page }) => {
      const created = await page.request.post(`/api/tasks?project=${PROJECT.id}`, {
        headers: { "x-genie": "1" }, data: { title: "Исходное название", type },
      });
      expect(created.ok()).toBe(true);
      const task: StoredTask = await created.json();
      const path = `/api/tasks/${task.id}?project=${PROJECT.id}`;
      await page.goto(`/${type}/${task.id}`);
      await page.evaluate((theme) => { document.documentElement.dataset.theme = theme; }, theme);
      const heading = page.locator("h1.title");
      await expect(heading).toHaveText("Исходное название");
      await heading.fill("Мой черновик");
      // Consecutive input must preserve the caret while the draft state changes.
      await heading.press("Home");
      await heading.pressSequentially("Автор: ");
      const draft = "Автор: Мой черновик";
      await expect(heading).toHaveText(draft);
      const version = (await storedTask(page.request, task.id)).editVersion;
      const refreshed = page.waitForResponse((r) => r.request().method() === "GET"
        && new URL(r.url()).pathname === `/api/tasks/${task.id}` && r.ok());
      expect((await page.request.patch(path, {
        headers: { "x-genie": "1" }, data: { title: "Название другого клиента", expectedVersion: version },
      })).ok()).toBe(true);
      await refreshed;
      await expect(heading).toHaveText(draft);
      for (let attempt = 0; attempt < 2; attempt++) {
        await expect(heading).toHaveAttribute("contenteditable", "true");
        await heading.focus();
        const response = page.waitForResponse((r) => r.request().method() === "PATCH"
          && new URL(r.url()).pathname === `/api/tasks/${task.id}`);
        await heading.blur();
        const conflict = await response;
        expect(conflict.status()).toBe(409);
        expect(conflict.request().postDataJSON().expectedVersion).toBe(version);
        await expect(heading).toHaveText(draft);
      }
      await page.screenshot({ path: test.info().outputPath(`title-conflict-${type}-${theme}.png`) });
      expect((await storedTask(page.request, task.id)).title).toBe("Название другого клиента");
      // Escape deliberately adopts the current server title and a new edit base.
      await expect(heading).toHaveAttribute("contenteditable", "true");
      await heading.focus();
      await heading.press("Escape");
      await expect(heading).toHaveText("Название другого клиента");
      await heading.fill("Согласованное название");
      await heading.press("Enter");
      await expect.poll(async () => (await storedTask(page.request, task.id)).title).toBe("Согласованное название");
      await expect(heading).toHaveText("Согласованное название");
      const current = await storedTask(page.request, task.id);
      expect((await page.request.patch(path, {
        headers: { "x-genie": "1" }, data: { title: "Следующее название с сервера", expectedVersion: current.editVersion },
      })).ok()).toBe(true);
      await expect(heading).toHaveText("Следующее название с сервера");
      expect((await page.request.delete(path, { headers: { "x-genie": "1" } })).ok()).toBe(true);
    });
  }
}

for (const scenario of [
  { type: "task", field: "description", edit: "Изменить описание", label: "Описание", theme: "ink" },
  { type: "epic", field: "description", edit: "Изменить: цель", label: "Цель", theme: "paper" },
  { type: "epic", field: "plan", edit: "Изменить: дорожная карта", label: "Дорожная карта", theme: "ink" },
] as const) {
  test(`a ${scenario.type} ${scenario.field} draft keeps its original version across a live refresh`, async ({ page }) => {
    const created = await page.request.post(`/api/tasks?project=${PROJECT.id}`, {
      headers: { "x-genie": "1" },
      data: { title: "Совместное редактирование", type: scenario.type, description: "Исходные требования", plan: "Исходный план" },
    });
    expect(created.ok()).toBe(true);
    const task: StoredTask = await created.json();
    const path = `/api/tasks/${task.id}?project=${PROJECT.id}`;
    await page.goto(`/${scenario.type}/${task.id}`);
    await page.evaluate((theme) => { document.documentElement.dataset.theme = theme; }, scenario.theme);
    await page.getByRole("button", { name: scenario.edit, exact: true }).click();
    const editor = page.getByRole("textbox", { name: scenario.label, exact: true });
    const draft = "Мой черновик до чужого изменения";
    await editor.fill(draft);
    const oldVersion = (await storedTask(page.request, task.id)).editVersion;
    // Another authenticated client changes the same requirements while this editor remains open.
    const refreshed = page.waitForResponse((response) => response.request().method() === "GET"
      && new URL(response.url()).pathname === `/api/tasks/${task.id}` && response.ok());
    const changed = await page.request.patch(path, {
      headers: { "x-genie": "1" }, data: { [scenario.field]: "Новые требования другого клиента", expectedVersion: oldVersion },
    });
    expect(changed.ok()).toBe(true);
    await refreshed;
    await expect(editor).toHaveValue(draft);
    const response = page.waitForResponse((r) => r.request().method() === "PATCH"
      && new URL(r.url()).pathname === `/api/tasks/${task.id}`);
    await editor.locator("..").getByRole("button", { name: "Сохранить", exact: true }).click();
    const conflict = await response;
    expect(conflict.status()).toBe(409);
    expect(conflict.request().postDataJSON().expectedVersion).toBe(oldVersion);
    await expect(editor).toBeVisible();
    await expect(editor).toHaveValue(draft);
    expect((await storedTask(page.request, task.id))[scenario.field]).toBe("Новые требования другого клиента");
    await page.screenshot({ path: test.info().outputPath(`conflict-preserves-draft-${scenario.theme}.png`) });
    // Reopening deliberately reads the new requirements; a reconciled save then succeeds.
    await editor.locator("..").getByRole("button", { name: "Отмена", exact: true }).click();
    await page.getByRole("button", { name: scenario.edit, exact: true }).click();
    await expect(editor).toHaveValue("Новые требования другого клиента");
    await editor.fill("Новые требования другого клиента\nСогласованное дополнение");
    await editor.locator("..").getByRole("button", { name: "Сохранить", exact: true }).click();
    await expect(editor).toBeHidden();
    expect((await storedTask(page.request, task.id))[scenario.field]).toBe("Новые требования другого клиента\nСогласованное дополнение");
    const removed = await page.request.delete(path, { headers: { "x-genie": "1" } });
    expect(removed.ok()).toBe(true);
  });
}

for (const theme of ["ink", "paper"] as const) {
  test(`an integration draft survives same-field live refresh, repeated conflict and cancellation in ${theme}`, async ({ page }) => {
    const created = await page.request.post(`/api/tasks?project=${PROJECT.id}`, {
      headers: { "x-genie": "1" }, data: { title: "Совместная интеграция", mergeStrategy: "Исходная интеграция" },
    });
    expect(created.ok()).toBe(true);
    const task: StoredTask = await created.json();
    const path = `/api/tasks/${task.id}?project=${PROJECT.id}`;
    await page.goto(`/task/${task.id}`);
    await page.evaluate((theme) => { document.documentElement.dataset.theme = theme; }, theme);
    const input = page.getByRole("textbox", { name: "Интеграция", exact: true });
    await input.fill("Мой черновик");
    await input.press("Home");
    await input.pressSequentially("Автор: ");
    const draft = "Автор: Мой черновик";
    const version = (await storedTask(page.request, task.id)).editVersion;
    const refreshed = page.waitForResponse((r) => r.request().method() === "GET"
      && new URL(r.url()).pathname === `/api/tasks/${task.id}` && r.ok());
    expect((await page.request.patch(path, {
      headers: { "x-genie": "1" }, data: { mergeStrategy: "Интеграция другого клиента", expectedVersion: version },
    })).ok()).toBe(true);
    await refreshed;
    await expect(input).toHaveValue(draft);
    for (let attempt = 0; attempt < 2; attempt++) {
      await input.focus();
      const response = page.waitForResponse((r) => r.request().method() === "PATCH"
        && new URL(r.url()).pathname === `/api/tasks/${task.id}`);
      await input.blur();
      const conflict = await response;
      expect(conflict.status()).toBe(409);
      expect(conflict.request().postDataJSON().expectedVersion).toBe(version);
      await expect(input).toHaveValue(draft);
    }
    await page.screenshot({ path: test.info().outputPath(`integration-conflict-${theme}.png`) });
    await input.focus();
    await input.press("Escape");
    await expect(input).toHaveValue("Интеграция другого клиента");
    await input.fill("Согласованная интеграция");
    await input.press("Enter");
    await expect.poll(async () => (await storedTask(page.request, task.id)).mergeStrategy).toBe("Согласованная интеграция");
    const current = await storedTask(page.request, task.id);
    expect((await page.request.patch(path, {
      headers: { "x-genie": "1" }, data: { mergeStrategy: "Следующая интеграция с сервера", expectedVersion: current.editVersion },
    })).ok()).toBe(true);
    await expect(input).toHaveValue("Следующая интеграция с сервера");
    expect((await page.request.delete(path, { headers: { "x-genie": "1" } })).ok()).toBe(true);
  });
}

test("refocusing a conflicted inline edit cannot silently grant it a new version", async ({ page }) => {
  const created = await page.request.post(`/api/tasks?project=${PROJECT.id}`, {
    headers: { "x-genie": "1" }, data: { title: "Конфликт интеграции", mergeStrategy: "Исходная интеграция" },
  });
  expect(created.ok()).toBe(true);
  const task: StoredTask = await created.json();
  const path = `/api/tasks/${task.id}?project=${PROJECT.id}`;
  await page.goto(`/task/${task.id}`);
  const input = page.getByRole("textbox", { name: "Интеграция", exact: true });
  await input.fill("Мой черновик интеграции");
  const version = (await storedTask(page.request, task.id)).editVersion;
  const refreshed = page.waitForResponse((response) => response.request().method() === "GET"
    && new URL(response.url()).pathname === `/api/tasks/${task.id}` && response.ok());
  const changed = await page.request.patch(path, {
    headers: { "x-genie": "1" }, data: { description: "Новые требования", expectedVersion: version },
  });
  expect(changed.ok()).toBe(true);
  await refreshed;
  for (let attempt = 0; attempt < 2; attempt++) {
    await input.focus();
    const response = page.waitForResponse((r) => r.request().method() === "PATCH"
      && new URL(r.url()).pathname === `/api/tasks/${task.id}`);
    await input.blur();
    const conflict = await response;
    expect(conflict.status()).toBe(409);
    expect(conflict.request().postDataJSON().expectedVersion).toBe(version);
    await expect(input).toHaveValue("Мой черновик интеграции");
  }
  expect((await storedTask(page.request, task.id)).description).toBe("Новые требования");
  expect((await page.request.delete(path, { headers: { "x-genie": "1" } })).ok()).toBe(true);
});
