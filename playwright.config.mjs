// UI tests: the real frontend (ui/) against an in-memory fake backend
// (tests/ui/mock-backend.js), in Chromium and in WebKit — the engine Tauri
// uses on macOS (WKWebView) and Linux (WebKitGTK).

import { defineConfig, devices } from "@playwright/test";

const PORT = 4173;

export default defineConfig({
  testDir: "tests/ui",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  use: {
    baseURL: `http://127.0.0.1:${PORT}`,
    viewport: { width: 1200, height: 700 },
    trace: "retain-on-failure",
  },
  projects: [
    { name: "webkit", use: { ...devices["Desktop Safari"], viewport: { width: 1200, height: 700 } } },
    { name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1200, height: 700 } } },
  ],
  webServer: {
    command: `node tests/ui/serve.mjs`,
    env: { PORT: String(PORT) },
    url: `http://127.0.0.1:${PORT}/index.html`,
    reuseExistingServer: !process.env.CI,
  },
});
