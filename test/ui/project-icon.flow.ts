import { expect, test } from "@playwright/test";

for (const theme of ["ink", "paper"]) {
  for (const width of [1440, 390]) {
  test(`project icon persists and clearing restores its monogram (${theme}, ${width})`, async ({ page }) => {
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/board");
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const header = page.locator(".g-topbar-project");
    await expect(header.locator(".project-icon")).toHaveText("ДЕ");
    await page.goto("/manage/project/general");
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
    await expect(page.getByRole("button", { name: "Заменить иконку", exact: true })).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await page.screenshot({ path: `test-results/project-icon-settings-${theme}-${width}.png`, animations: "disabled" });
    await page.goto("/board");
    await expect(header.locator(".project-icon img")).toBeVisible();
    expect(await header.locator(".project-icon img").evaluate((node: HTMLImageElement) => node.complete && node.naturalWidth > 0)).toBe(true);
    await page.reload();
    await expect(header.locator(".project-icon img")).toBeVisible();
    await page.screenshot({ path: `test-results/project-icon-${theme}-${width}.png`, animations: "disabled" });
    await header.click();
    await expect(page.getByRole("menu", { name: "Проекты", exact: true }).locator(".project-icon img")).toBeVisible();
    await page.keyboard.press("Escape");
    await page.goto("/manage/project/general");
    await page.getByRole("button", { name: "Удалить иконку", exact: true }).click();
    await expect(page.getByRole("button", { name: "Загрузить иконку", exact: true })).toBeVisible();
    await page.goto("/board");
    await expect(header.locator(".project-icon")).toHaveText("ДЕ");
    await expect(header.locator(".project-icon img")).toHaveCount(0);
    await page.reload();
    await expect(header.locator(".project-icon")).toHaveText("ДЕ");
  });
}
}

for (const theme of ["ink", "paper"]) {
  for (const width of [390, 768]) {
    test(`a long project name keeps phone controls in view (${theme}, ${width})`, async ({ page }) => {
      await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
      await page.setViewportSize({ width, height: 900 });
      // The session's project label is the fixture; routing and the rest of the application use the real seed.
      await page.route("**/api/auth/me", async (route) => {
        const response = await route.fetch();
        const session: { projects: { name: string }[] } = await response.json();
        session.projects = session.projects.map((project) => ({ ...project, name: "Genie — redesign с очень длинным названием проекта" }));
        await route.fulfill({ response, json: session });
      });
      await page.goto("/board");
      const project = page.getByRole("button", { name: "Проект Genie — redesign с очень длинным названием проекта", exact: true });
      await expect(project).toBeVisible();
      const geometry = await page.locator(".g-topbar").evaluate((bar) => ({
        pageWidth: document.documentElement.scrollWidth,
        viewport: innerWidth,
        controls: [...bar.querySelectorAll("button")].filter((button) => button.getBoundingClientRect().width > 0).map((button) => {
          const rect = button.getBoundingClientRect();
          return { left: rect.left, right: rect.right };
        }),
      }));
      expect(geometry.pageWidth).toBeLessThanOrEqual(geometry.viewport);
      for (const control of geometry.controls) {
        expect(control.left).toBeGreaterThanOrEqual(0);
        expect(control.right).toBeLessThanOrEqual(width);
      }
      await project.click();
      await expect(page.getByRole("menu", { name: "Проекты", exact: true })).toBeVisible();
      await page.keyboard.press("Escape");
      await expect(project).toBeFocused();
    });
  }
}
