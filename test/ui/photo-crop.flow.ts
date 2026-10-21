// The photo crop panel is a dialog too: choosing a photo opens it with the keyboard inside, Tab stays
// there, and Escape or Отмена close it and give focus back to the button that started it.
import { expect, test, type Page } from "@playwright/test";
import { focusInside } from "./keyboard.ts";

const STOPS = 'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

// A 64x64 PNG, small enough to inline: the panel only needs a picture to crop.
const PHOTO = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAEAAAABACAIAAAAlC+aJAAAAtUlEQVR42u3PkWJCAQAAwAdBMAiCIAiCIAiCIAgGQRAEwSAIgiAIgiAIgmAwGAyCIAiCIAgGQRAEQRAEQRAEQRD0CWFyH3BwQRAKf0SisXgimUpnsrl84bNYKleqX7V6o9lqd7q9/mD4/fP7NxpPprP5Yvm/Wm+2u/3heDpfrrf74+1eQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQOCVfwKYe6FaxdUo5QAAAABJRU5ErkJggg==",
  "base64",
);

/** Opens /settings/me and the crop panel the way a person does: pick a file for the upload button. */
async function openCrop(page: Page) {
  await page.goto("/settings/me");
  const upload = page.getByRole("button", { name: /^(Загрузить фото|Заменить)$/ });
  await expect(upload).toBeVisible();
  const [chooser] = await Promise.all([page.waitForEvent("filechooser"), upload.click()]);
  await chooser.setFiles({ name: "photo.png", mimeType: "image/png", buffer: PHOTO });
  const dialog = page.getByRole("dialog", { name: "Настройка фото" });
  await expect(dialog).toBeVisible();
  return { upload, dialog };
}

for (const width of [1440, 390]) {
  test(`the photo crop takes focus, traps Tab and restores it (${width}px)`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    const { upload, dialog } = await openCrop(page);

    expect(await focusInside(dialog)).toBe(true);
    // The first stop is the zoom slider, not the panel itself.
    expect(await page.evaluate(() => document.activeElement instanceof HTMLInputElement && document.activeElement.type === "range")).toBe(true);

    const count = await dialog.evaluate(
      (root, selector) => Array.from(root.querySelectorAll<HTMLElement>(selector)).filter((node) => node.getClientRects().length > 0).length,
      STOPS,
    );
    const index = () =>
      dialog.evaluate(
        (root, selector) =>
          Array.from(root.querySelectorAll<HTMLElement>(selector))
            .filter((node) => node.getClientRects().length > 0)
            .indexOf(document.activeElement as HTMLElement),
        STOPS,
      );
    expect(count).toBeGreaterThan(4);
    for (let step = 1; step < count; step += 1) {
      await page.keyboard.press("Tab");
      expect(await index()).toBe(step);
    }
    await page.keyboard.press("Tab");
    expect(await index()).toBe(0);
    await page.keyboard.press("Shift+Tab");
    expect(await index()).toBe(count - 1);

    await page.screenshot({ path: `test-results/photo-crop-${width}.png`, animations: "disabled" });
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    await expect(upload).toBeFocused();
  });
}

test("the crop sliders work and Отмена closes the panel back to its button", async ({ page }) => {
  const { upload, dialog } = await openCrop(page);
  const zoom = dialog.getByRole("slider").first();
  await zoom.fill("1.6");
  await expect(zoom).toHaveValue("1.6");
  // The preview follows the chosen zoom instead of showing the untouched picture.
  const transform = await dialog.locator(".st-crop-preview img").evaluate((node) => getComputedStyle(node).transform);
  expect(transform).not.toBe("none");

  await dialog.getByRole("button", { name: "Отмена" }).click();
  await expect(dialog).toBeHidden();
  await expect(upload).toBeFocused();
});
