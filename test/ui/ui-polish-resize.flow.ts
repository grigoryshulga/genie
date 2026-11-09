import { expect, test } from "@playwright/test";

for (const theme of ["ink", "paper"]) {
  test(`textarea height supports touch, keyboard and announced values (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.addInitScript((value) => localStorage.setItem("genie.appearance.theme", value), theme);
    await page.goto("/tasks");
    await page.getByRole("button", { name: "Новая задача", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Новая задача", exact: true });
    const grip = dialog.getByRole("separator", { name: "Изменить высоту поля", exact: true }).first();
    const field = page.locator(`[id="${await grip.getAttribute("aria-controls")}"]`);
    await field.fill("Текст сохраняется при изменении размера.");
    await grip.scrollIntoViewIfNeeded();
    const box = (await grip.boundingBox())!;
    expect(box.height).toBeGreaterThanOrEqual(40);
    expect(box.width).toBeGreaterThanOrEqual(40);
    const before = (await field.boundingBox())!.height;
    await grip.focus();
    await page.keyboard.press("ArrowDown");
    await expect.poll(async () => (await field.boundingBox())!.height).toBeCloseTo(before + 16, 0);
    expect(Number(await grip.getAttribute("aria-valuenow"))).toBeCloseTo((await field.boundingBox())!.height, 0);
    const bounds = (await grip.boundingBox())!;
    await page.mouse.move(bounds.x + 5, bounds.y + 5);
    await page.mouse.down();
    await page.mouse.move(bounds.x + 5, bounds.y + 37);
    await page.mouse.up();
    await expect.poll(async () => (await field.boundingBox())!.height).toBeGreaterThan(before + 16);
    await grip.focus();
    await page.keyboard.press("Home");
    await expect(grip).toHaveAttribute("aria-valuenow", "96");
    await page.keyboard.press("End");
    await expect(grip).toHaveAttribute("aria-valuenow", "480");
    expect((await field.boundingBox())!.height).toBeCloseTo(480, 0);
    await expect(field).toHaveValue("Текст сохраняется при изменении размера.");
  });

  test(`phone personal settings names its exit (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto("/settings/me/security");
    const back = page.getByRole("button", { name: "Вернуться к работе", exact: true });
    await expect(back.getByText("К работе", { exact: true })).toBeVisible();
    await back.click();
    await expect(page.locator(".g-topbar-nav")).toBeVisible();
  });
}
