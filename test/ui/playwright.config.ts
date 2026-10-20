// Browser tests of the web UI. Start them with `npm run test:ui`: test/ui/run.ts starts two seeded
// servers and runs this configuration inside the pinned Playwright image.

import { resolve } from "node:path";
import { defineConfig, devices } from "@playwright/test";

const visualURL = process.env.GENIE_UI_VISUAL_URL;
const flowsURL = process.env.GENIE_UI_FLOWS_URL;
if (!visualURL || !flowsURL) throw new Error("the server URLs are not set: run the browser tests with `npm run test:ui`");

// Each server has its own session, kept in its own file.
const results = resolve(import.meta.dirname, "../../test-results");
const VISUAL_AUTH = resolve(results, "ui-auth-visual.json");
const FLOWS_AUTH = resolve(results, "ui-auth-flows.json");
const desktop = { ...devices["Desktop Chrome"], viewport: { width: 1440, height: 900 } };

export default defineConfig({
  testDir: ".",
  outputDir: resolve(results, "ui"),
  snapshotPathTemplate: "{testDir}/__screenshots__/{testFileName}/{arg}-{projectName}{ext}",
  // Flows on one server change its data: one test at a time keeps them independent of order.
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never", outputFolder: resolve(results, "ui-report") }]] : "list",
  expect: {
    toHaveScreenshot: { animations: "disabled", caret: "hide", maxDiffPixelRatio: 0.002 },
  },
  use: {
    locale: "ru-RU",
    timezoneId: "Europe/Moscow",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    { name: "login-visual", testMatch: "auth.setup.ts", use: { ...desktop, baseURL: visualURL, storageState: undefined }, metadata: { auth: VISUAL_AUTH } },
    { name: "login-flows", testMatch: "auth.setup.ts", use: { ...desktop, baseURL: flowsURL, storageState: undefined }, metadata: { auth: FLOWS_AUTH } },
    {
      name: "desktop",
      testMatch: "*.visual.ts",
      dependencies: ["login-visual"],
      use: { ...desktop, baseURL: visualURL, storageState: VISUAL_AUTH },
    },
    {
      name: "mobile",
      testMatch: "*.visual.ts",
      dependencies: ["login-visual"],
      use: { ...devices["Pixel 7"], baseURL: visualURL, storageState: VISUAL_AUTH },
    },
    {
      name: "flows",
      testMatch: "*.flow.ts",
      dependencies: ["login-flows"],
      use: { ...desktop, baseURL: flowsURL, storageState: FLOWS_AUTH },
    },
  ],
});
