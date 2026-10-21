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
// Touch screens at the two widths H30 audits: a phone and a tablet, both with a real touch layer and
// both on Chromium, the engine the rest of the suite and the app's native pickers are checked on (the
// iPad descriptor alone would start WebKit).
const phone = { ...devices["Pixel 7"], viewport: { width: 390, height: 844 }, browserName: "chromium" };
const tablet = { ...devices["iPad Mini"], viewport: { width: 768, height: 1024 }, browserName: "chromium" };

// `npm run ui:capture` swaps the test projects for the capture ones (test/ui/screens.capture.ts).
const capturing = process.env.GENIE_UI_CAPTURE === "1";

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
  projects: capturing ? captureProjects() : [
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
    // Touch last: these taps move a card, and the flows above must see their own seed first.
    {
      name: "touch-phone",
      testMatch: "touch.ts",
      dependencies: ["login-flows"],
      use: { ...phone, baseURL: flowsURL, storageState: FLOWS_AUTH },
    },
    {
      name: "touch-tablet",
      testMatch: "touch.ts",
      dependencies: ["login-flows"],
      use: { ...tablet, baseURL: flowsURL, storageState: FLOWS_AUTH },
    },
  ],
});

function captureProjects() {
  return [
    { name: "login-visual", testMatch: "auth.setup.ts", use: { ...desktop, baseURL: visualURL, storageState: undefined }, metadata: { auth: VISUAL_AUTH } },
    { name: "capture-desktop", testMatch: "*.capture.ts", dependencies: ["login-visual"], use: { ...desktop, baseURL: visualURL, storageState: VISUAL_AUTH } },
    {
      name: "capture-phone",
      testMatch: "*.capture.ts",
      dependencies: ["login-visual"],
      use: { ...devices["Pixel 7"], viewport: { width: 390, height: 844 }, baseURL: visualURL, storageState: VISUAL_AUTH },
    },
  ];
}
