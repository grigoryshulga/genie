import { expect, test } from "@playwright/test";

for (const theme of ["ink", "paper"]) {
  test(`project icon persists and clearing restores its monogram (${theme})`, async ({ page }) => {
    await page.goto("/manage/project/general");
    await page.evaluate((value) => { document.documentElement.dataset.theme = value; }, theme);
    const header = page.locator(".g-topbar-project");
    await expect(header.locator(".project-icon")).toHaveText("ДЕ");
    const png = await page.evaluate(() => {
      const canvas = document.createElement("canvas");
      canvas.width = canvas.height = 64;
      const context = canvas.getContext("2d")!;
      context.fillStyle = "#009879";
      context.fillRect(0, 0, 64, 64);
      context.fillStyle = "white";
      context.font = "bold 40px sans-serif";
      context.fillText("G", 15, 48);
      return canvas.toDataURL("image/png").split(",")[1];
    });
    await page.getByLabel("Загрузить иконку проекта", { exact: true }).setInputFiles({ name: "project.png", mimeType: "image/png", buffer: Buffer.from(png, "base64") });
    await expect(header.locator(".project-icon img")).toBeVisible();
    expect(await header.locator(".project-icon img").evaluate((node: HTMLImageElement) => node.complete && node.naturalWidth > 0)).toBe(true);
    await page.reload();
    await expect(header.locator(".project-icon img")).toBeVisible();
    await page.screenshot({ path: `test-results/project-icon-${theme}.png`, animations: "disabled" });
    await header.click();
    await expect(page.getByRole("menu", { name: "Проекты", exact: true }).locator(".project-icon img")).toBeVisible();
    await page.keyboard.press("Escape");
    await page.getByRole("button", { name: "Удалить иконку", exact: true }).click();
    await expect(header.locator(".project-icon")).toHaveText("ДЕ");
    await expect(header.locator(".project-icon img")).toHaveCount(0);
    await page.reload();
    await expect(header.locator(".project-icon")).toHaveText("ДЕ");
  });
}
