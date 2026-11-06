import { readFile } from "node:fs/promises";
import { expect, test } from "@playwright/test";
import { ROLE_FILE, ROLE_GRANTS } from "./role-data.ts";
import { parseRoleTransfer } from "../../web/src/entities/agent-config/role-transfer.ts";

const id = "role-layout-test";
const importedId = "role-layout-import";
const headers = { "x-genie": "1" };

test("role workspace saves one draft across tabs and provides a paginated audit trail and portable files", async ({ page, request }) => {
  const created = await request.put(`/api/roles/${id}`, { headers, data: { content: ROLE_FILE, baseHash: "" } });
  expect(created.ok(), await created.text()).toBe(true);
  try {
    await page.goto(`/manage/agents/roles?id=${id}`);
    await expect(page.getByRole("region", { name: "Промпт роли", exact: true })).toContainText("Investigate the task");
    await page.getByRole("tab", { name: "Настройки", exact: true }).click();
    const connections = page.locator(".ag-mcp-connection");
    await expect(connections).toHaveCount(3);
    await expect(connections.first().locator("li").first()).not.toBeVisible();
    await connections.first().locator("summary").click();
    await expect(connections.first().locator("li")).toHaveCount(48);

    await page.getByRole("button", { name: "Редактировать роль", exact: true }).click();
    await page.getByLabel("Название", { exact: true }).fill("Несохранённый черновик");
    await page.getByRole("button", { name: /Документатор/ }).click();
    await expect(page.getByRole("dialog", { name: "Отменить изменения роли?" })).toBeVisible();
    await page.getByRole("dialog").getByRole("button", { name: "Отмена", exact: true }).click();
    await expect(page.getByLabel("Название", { exact: true })).toHaveValue("Несохранённый черновик");
    await page.getByRole("button", { name: "Отмена", exact: true }).click();
    await page.getByRole("button", { name: "Редактировать роль", exact: true }).click();
    await expect(page.getByLabel("Название", { exact: true })).toHaveValue("Аналитик интеграций");
    await page.getByLabel("Описание", { exact: true }).fill("Обновлённое описание роли");
    await page.getByLabel("Размышление", { exact: true }).selectOption("high");
    await page.getByRole("tab", { name: "Промпт", exact: true }).click();
    await page.getByLabel("Текст промпта").fill("Investigate the task, verify evidence and prepare a plan.");
    await page.getByRole("tab", { name: "Настройки", exact: true }).click();
    await expect(page.getByLabel("Описание", { exact: true })).toHaveValue("Обновлённое описание роли");
    await page.getByRole("button", { name: "Сохранить", exact: true }).click();
    await expect(page.locator(".pane-head")).toContainText("Обновлённое описание роли");
    const stored = await (await request.get(`/api/roles/${id}`)).json();
    expect(stored.role.mcp).toEqual(ROLE_GRANTS);
    expect(stored.role.thinking).toBe("high");
    expect(stored.file.content).toContain("verify evidence");
    await page.reload();
    await expect(page.locator(".pane-head")).toContainText("Обновлённое описание роли");

    const downloading = page.waitForEvent("download");
    await page.getByRole("button", { name: "Экспорт роли", exact: true }).click();
    const download = await downloading;
    const portable = parseRoleTransfer(await readFile((await download.path())!, "utf8"));
    expect(portable.prompt).toContain("verify evidence");
    expect(portable.mcp).toEqual(ROLE_GRANTS);
    await page.getByRole("button", { name: "Импорт роли", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Импорт роли", exact: true });
    const schemaDownload = page.waitForEvent("download");
    await dialog.getByRole("button", { name: "Скачать JSON Schema", exact: true }).click();
    const schemaFile = await schemaDownload;
    const schema = JSON.parse(await readFile((await schemaFile.path())!, "utf8"));
    expect(schema.properties.prompt).toBeDefined();
    const file = dialog.getByLabel("Файл роли (.json)", { exact: true });
    await file.setInputFiles({ name: "invalid.json", mimeType: "application/json", buffer: Buffer.from('{"bad":true}') });
    await expect(dialog.getByRole("alert")).toBeVisible();
    await expect(dialog.getByRole("button", { name: "Импортировать", exact: true })).toBeDisabled();
    await file.setInputFiles({ name: "portable.json", mimeType: "application/json", buffer: Buffer.from(JSON.stringify(portable)) });
    await expect(dialog.getByRole("checkbox", { name: /Заменить существующую/ })).not.toBeChecked();
    await expect(dialog.getByRole("button", { name: "Импортировать", exact: true })).toBeDisabled();
    await dialog.getByRole("checkbox", { name: /Заменить существующую/ }).check();
    await expect(dialog.getByRole("button", { name: "Импортировать", exact: true })).toBeEnabled();
    const concurrent = await request.put(`/api/roles/${id}`, { headers, data: { content: stored.file.content.replace("Обновлённое описание роли", "External change") } });
    expect(concurrent.ok()).toBe(true);
    await dialog.getByRole("button", { name: "Импортировать", exact: true }).click();
    await expect(dialog.getByRole("alert")).toBeVisible();
    expect((await (await request.get(`/api/roles/${id}`)).json()).role.description).toBe("External change");
    await dialog.getByLabel("Идентификатор роли", { exact: true }).fill(importedId);
    await dialog.getByRole("button", { name: "Импортировать", exact: true }).click();
    await expect(dialog).not.toBeVisible();
    const imported = await (await request.get(`/api/roles/${importedId}`)).json();
    expect(imported.role.prompt).toBe(stored.role.prompt);
    expect(imported.role.capabilities).toEqual(stored.role.capabilities);
    expect(imported.role.mcp).toEqual(ROLE_GRANTS);

    // More than one page, including an unrelated role, to check item/cursor filtering.
    for (let i = 0; i < 23; i++) {
      const res = await request.put(`/api/roles/${id}`, { headers, data: { content: stored.file.content.replace("Обновлённое описание роли", `History edit ${i}`) } });
      expect(res.ok()).toBe(true);
    }
    const first = await (await request.get(`/api/agent-config/history?item=role:${id}&limit=21`)).json();
    expect(first).toHaveLength(21);
    const older = await (await request.get(`/api/agent-config/history?item=role:${id}&limit=21&before=${first[19].id}`)).json();
    expect(older).toHaveLength(6);
    expect(older.every((row: { item: string; id: number }) => row.item === `role:${id}` && row.id < first[19].id)).toBe(true);
    expect((await request.get(`/api/agent-config/history?before=0`)).status()).toBe(400);
    await page.goto(`/manage/agents/roles?id=${id}&view=history`);
    await expect(page.locator(".config-history-table > tbody > tr")).toHaveCount(20);
    await page.getByRole("button", { name: "Старее", exact: true }).click();
    await expect(page.locator(".config-history-pagination")).toContainText("Страница 2");
    await expect(page.locator(".config-history-table > tbody > tr")).toHaveCount(6);
    await expect(page.getByRole("button", { name: "Старее", exact: true })).toBeDisabled();
  } finally {
    await request.delete(`/api/roles/${importedId}`, { headers });
    await request.delete(`/api/roles/${id}`, { headers });
  }
});
