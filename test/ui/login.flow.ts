// Signing in: a wrong password is explained without losing the input, the right one opens the app.

import { expect, test } from "@playwright/test";
import { OWNER } from "./seed.ts";

test.use({ storageState: { cookies: [], origins: [] } });

test("a wrong password shows an error and keeps the form, the right one signs in", async ({ page }) => {
  await page.goto("/tasks");
  const login = page.getByLabel("Логин");
  const password = page.getByLabel("Пароль");
  await login.fill(OWNER.login);
  await password.fill("not-the-password");
  await page.getByRole("button", { name: "Войти" }).click();

  await expect(page.getByText("Неверный логин или пароль")).toBeVisible();
  await expect(login).toHaveValue(OWNER.login);

  await password.fill(OWNER.password);
  await page.getByRole("button", { name: "Войти" }).click();
  await expect(page.getByRole("list", { name: "Задачи" })).toBeVisible();
  await expect(page.getByText(OWNER.name)).toBeVisible();
});
