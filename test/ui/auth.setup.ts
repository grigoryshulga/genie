// Signs the owner in through the login form once per server and keeps the session for the tests.

import { expect, test as setup } from "@playwright/test";
import { OWNER } from "./seed.ts";

setup("the owner signs in", async ({ page }, info) => {
  await page.goto("/");
  await page.getByLabel("Логин").fill(OWNER.login);
  await page.getByLabel("Пароль").fill(OWNER.password);
  await page.getByRole("button", { name: "Войти" }).click();
  await expect(page.getByRole("button", { name: "Войти" })).toBeHidden();
  await page.context().storageState({ path: info.project.metadata.auth });
});
