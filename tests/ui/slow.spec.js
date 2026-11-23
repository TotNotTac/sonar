import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, item, openApp, status } from "./helpers.js";

test("a folder that doesn't answer says so, and other folders still open", async ({ page }) => {
  await page.clock.install();
  await openApp(page);
  await page.evaluate((p) => window.__mock.slow.set(p, Infinity), `${HOME}/Music`);
  await item(activePane(page), "Music").dblclick();
  await page.clock.runFor(600);
  await expect(status(page)).toHaveText("Loading “Music”…");
  await page.clock.runFor(5000);
  await expect(activePane(page).locator(".message")).toContainText("“Music” is not responding");
  await item(activePane(page), "Videos").dblclick();
  await expectPath(page, `${HOME}/Videos`);
  await expect(activePane(page).locator(".message")).toBeHidden();
});

test("a slow folder shows Loading, then opens", async ({ page }) => {
  await page.clock.install();
  await openApp(page);
  await page.evaluate((p) => window.__mock.slow.set(p, 2000), `${HOME}/Music`);
  await item(activePane(page), "Music").dblclick();
  await page.clock.runFor(600);
  await expect(status(page)).toHaveText("Loading “Music”…");
  await page.clock.runFor(1600);
  await expectPath(page, `${HOME}/Music`);
  await expect(activePane(page).locator(".message")).toBeHidden();
});
