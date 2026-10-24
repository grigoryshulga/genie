import { defineConfig } from "@playwright/test";
const port = Number(process.env.GENIE_UI_PREVIEW_PORT ?? 5186);
if (!Number.isInteger(port) || port < 1024 || port > 65535) throw new Error("Invalid UI preview test port");
const baseURL = `http://127.0.0.1:${port}`;

export default defineConfig({
  testDir: ".", testMatch: "ui-preview.browser.ts", workers: 1,
  outputDir: "../test-results/ui-preview",
  use: { baseURL, viewport: { width: 1600, height: 1000 } },
  webServer: {
    command: `npm run preview:ui -- --port ${port} --strictPort`,
    url: `${baseURL}/api/auth/me`, timeout: 15_000, reuseExistingServer: false,
  },
});
