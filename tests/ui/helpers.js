import { expect } from "@playwright/test";

// The app picks shortcuts by platform (navigator.platform), which matches the
// OS the browser runs on, so tests do the same.
export const isMac = process.platform === "darwin";
const MOD = isMac ? "Meta" : "Control";

export const keys = {
  mod: (k) => `${MOD}+${k}`,
  back: isMac ? "Meta+BracketLeft" : "Alt+ArrowLeft",
  forward: isMac ? "Meta+BracketRight" : "Alt+ArrowRight",
  filter: isMac ? "Meta+Alt+KeyF" : "Control+i",
  search: isMac ? "Meta+f" : "Control+f",
  properties: isMac ? "Meta+i" : "Alt+Enter",
  icons: `${MOD}+1`,
  details: isMac ? "Meta+2" : "Control+3",
  hidden: isMac ? "Meta+Shift+Period" : "Control+h",
  newFolder: `${MOD}+Shift+N`,
  trash: "Delete",
  deletePermanently: "Shift+Delete",
  reopenTab: `${MOD}+Shift+T`,
};

export const HOME = "/home/user";

/** Load the app against the fake backend. `session` pre-seeds saved tabs. */
export async function openApp(page, { session } = {}) {
  await page.addInitScript({ path: "tests/ui/mock-backend.js" });
  if (session) {
    await page.addInitScript((s) => {
      if (!sessionStorage.getItem("seeded")) {
        localStorage.setItem("sonar.session", JSON.stringify(s));
        sessionStorage.setItem("seeded", "1");
      }
    }, session);
  }
  await page.goto("/index.html");
  await page.waitForFunction(() => window.sonar?.pane?.view);
}

/** The visible, active pane. */
export const activePane = (page) => page.locator(".tab-panes:not([hidden]) .pane.active");
export const panes = (page) => page.locator(".tab-panes:not([hidden]) .pane");
export const labels = (scope) => scope.locator(".name-label, .tile-label");
export const item = (scope, name) => scope.locator("[data-i]").filter({ has: scope.page().locator(`:is(.name-label, .tile-label)`, { hasText: new RegExp(`^${escapeRe(name)}$`) }) });
export const crumbs = (page) => page.locator("#crumbs .crumb");
export const status = (page) => page.locator("#status-left");
export const tabLabels = (page) => page.locator(".tab .tab-label");
export const message = (page) => activePane(page).locator(".message");

export async function expectPath(page, path) {
  await expect.poll(() => page.evaluate(() => window.sonar.pane.path)).toBe(path);
}

/** Wait for the app to settle after a navigation-triggering action. */
export async function settle(page) {
  await page.waitForFunction(() => {
    const p = window.sonar.pane;
    return p.view && !p.refreshing;
  });
}

export const mock = (page, fn, arg) => page.evaluate(fn, arg);

function escapeRe(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
