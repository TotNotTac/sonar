import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, item, mock, openApp, tabLabels } from "./helpers.js";

test("folders requested at startup open in tabs after the restored session", async ({ page }) => {
  await page.addInitScript((home) => {
    // Before the app starts: e.g. `sonar ~/Music ~/Documents/report.pdf` or Finder's "open".
    const ready = setInterval(() => {
      if (!window.__mock) return;
      clearInterval(ready);
      window.__mock.openRequests.push({ folder: `${home}/Music`, select: null }, { folder: `${home}/Documents`, select: "report.pdf" });
    }, 0);
  }, HOME);
  await openApp(page, { session: { tabs: [{ paths: [`${HOME}/Pictures`] }], active: 0 } });
  await expect(tabLabels(page)).toHaveText(["Pictures", "Music", "Documents"]);
  await expect(page.locator(".tab.active")).toHaveText("Documents");
  await expect(item(activePane(page), "report.pdf")).toHaveClass(/selected/);
});

test("without a session, a startup request replaces the default Home tab", async ({ page }) => {
  await page.addInitScript((home) => {
    const ready = setInterval(() => {
      if (!window.__mock) return;
      clearInterval(ready);
      window.__mock.openRequests.push({ folder: `${home}/Downloads`, select: null });
    }, 0);
  }, HOME);
  await openApp(page);
  await expect(tabLabels(page)).toHaveText(["Downloads"]);
});

test("requests while running open a new tab and switch to it", async ({ page }) => {
  await openApp(page);
  await mock(page, (home) => window.__mock.requestOpen([{ folder: `${home}/Pictures`, select: "photo3.png" }]), HOME);
  await expect(tabLabels(page)).toHaveText(["Home", "Pictures"]);
  await expectPath(page, `${HOME}/Pictures`);
  await expect(item(activePane(page), "photo3.png")).toHaveClass(/selected/);
});
