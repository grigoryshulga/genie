import { expect, test } from "@playwright/test";
import { ROLE_FILE, ROLE_GRANTS } from "./role-data.ts";

const id = "role-layout-test";
const headers = { "x-genie": "1" };

test("role editing stays on the page, cancels drafts and saves without changing grants or prompt", async ({ page, request }) => {
  const created = await request.put(`/api/roles/${id}`, { headers, data: { content: ROLE_FILE, baseHash: "" } });
  expect(created.ok(), await created.text()).toBe(true);
  try {
    await page.goto(`/agents?id=${id}`);
    await expect(page.getByRole("heading", { name: "Аналитик интеграций", exact: true })).toBeVisible();
    const connections = page.locator(".ag-mcp-connection");
    await expect(connections).toHaveCount(3);
    await expect(connections.first().locator("summary")).toContainText("48");
    await expect(connections.first().locator("li").first()).not.toBeVisible();
    await connections.first().locator("summary").click();
    await expect(connections.first().locator("li")).toHaveCount(48);
    await expect(connections.first().locator("li").first()).toBeVisible();

    await page.getByRole("button", { name: "Редактировать роль", exact: true }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByLabel("Название", { exact: true }).fill("Несохранённый черновик");
    await page.getByRole("button", { name: /Документатор/ }).click();
    await expect(page.getByRole("dialog", { name: "Отменить изменения роли?" })).toBeVisible();
    await page.getByRole("dialog").getByRole("button", { name: "Отмена", exact: true }).click();
    await expect(page.getByLabel("Название", { exact: true })).toHaveValue("Несохранённый черновик");
    await page.getByRole("button", { name: "Отмена", exact: true }).click();
    await page.getByRole("button", { name: "Редактировать роль", exact: true }).click();
    await expect(page.getByLabel("Название", { exact: true })).toHaveValue("Аналитик интеграций");
    await page.getByLabel("Описание", { exact: true }).fill("Обновлённое описание роли");
    await page.getByRole("button", { name: "Сохранить", exact: true }).click();
    await expect(page.locator(".pane-head")).toContainText("Обновлённое описание роли");
    const stored = await (await request.get(`/api/roles/${id}`)).json();
    expect(stored.role.mcp).toEqual(ROLE_GRANTS);
    expect(stored.file.content).toContain("Investigate the task and prepare a plan.");
    await page.reload();
    await expect(page.locator(".pane-head")).toContainText("Обновлённое описание роли");
  } finally {
    expect((await request.delete(`/api/roles/${id}`, { headers })).ok()).toBe(true);
  }
});
