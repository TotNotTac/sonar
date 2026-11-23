import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, item, keys, mock, openApp, tabLabels } from "./helpers.js";

test("a single tab is shown with its own close button", async ({ page }) => {
  await openApp(page);
  await expect(page.locator("#tabbar")).toBeVisible();
  await expect(tabLabels(page)).toHaveText(["Home"]);
  await expect(page.locator(".tab .tab-close")).toBeVisible();
});

test("open, switch, close and reopen tabs", async ({ page }) => {
  await openApp(page);
  await page.keyboard.press(keys.mod("t"));
  await page.locator(".place", { hasText: "Downloads" }).click();
  await expect(tabLabels(page)).toHaveText(["Home", "Downloads"]);
  await expect(page.locator(".tab.active")).toHaveText("Downloads");

  await page.keyboard.press("Control+Tab");
  await expect(page.locator(".tab.active")).toHaveText("Home");

  // Close the inactive "Downloads" tab with its × button.
  await page.locator(".tab", { hasText: "Downloads" }).locator(".tab-close").click();
  await expect(tabLabels(page)).toHaveText(["Home"]);
  await page.keyboard.press(keys.reopenTab);
  await expect(tabLabels(page)).toHaveText(["Home", "Downloads"]);
  await expect(page.locator(".tab.active")).toHaveText("Downloads");
  await expectPath(page, `${HOME}/Downloads`);
});

test("middle-click a folder opens it in a background tab", async ({ page }) => {
  await openApp(page);
  await item(activePane(page), "Music").click({ button: "middle" });
  await expect(tabLabels(page)).toHaveText(["Home", "Music"]);
  await expect(page.locator(".tab.active")).toHaveText("Home");
  // Middle-click a tab closes it.
  await page.locator(".tab", { hasText: "Music" }).click({ button: "middle" });
  await expect(tabLabels(page)).toHaveText(["Home"]);
});

test("drag a tab to reorder", async ({ page }) => {
  await openApp(page);
  await page.keyboard.press(keys.mod("t"));
  await page.locator(".place", { hasText: "Music" }).click();
  await page.keyboard.press(keys.mod("t"));
  await page.locator(".place", { hasText: "Videos" }).click();
  await expect(tabLabels(page)).toHaveText(["Home", "Music", "Videos"]);

  const from = await page.locator(".tab", { hasText: "Videos" }).boundingBox();
  const to = await page.locator(".tab", { hasText: "Home" }).boundingBox();
  await page.mouse.move(from.x + 30, from.y + 10);
  await page.mouse.down();
  await page.mouse.move(from.x + 10, from.y + 10, { steps: 3 });
  await page.mouse.move(to.x + 5, to.y + 10, { steps: 8 });
  await page.mouse.up();
  await expect(tabLabels(page)).toHaveText(["Videos", "Home", "Music"]);
});

test("tab context menu", async ({ page }) => {
  await openApp(page);
  await page.keyboard.press(keys.mod("t"));
  await page.keyboard.press(keys.mod("t"));
  await expect(tabLabels(page)).toHaveCount(3);
  await page.locator(".tab").first().click({ button: "right" });
  await expect(page.locator(".menu .menu-label")).toHaveText([
    "New Tab", "Duplicate Tab", "Reopen Closed Tab", "Close Tab", "Close Other Tabs", "Close Tabs to the Right",
  ]);
  await page.locator(".menu-item", { hasText: "Close Tabs to the Right" }).click();
  await expect(tabLabels(page)).toHaveCount(1);
});

test("double-click empty tab bar space opens a new tab", async ({ page }) => {
  await openApp(page);
  const bar = await page.locator("#tabbar").boundingBox();
  await page.mouse.dblclick(bar.x + bar.width - 20, bar.y + 10);
  await expect(tabLabels(page)).toHaveCount(2);
});

test("closing the last tab closes the window", async ({ page }) => {
  await openApp(page);
  await page.locator(".tab .tab-close").click();
  await expect.poll(() => mock(page, () => window.__mock.windowCloses)).toBe(1);
});

test("Ctrl/⌘+W on the last tab closes its split view first", async ({ page }) => {
  await openApp(page);
  await page.keyboard.press("F3");
  await page.keyboard.press(keys.mod("w"));
  await expect(page.locator(".tab-panes:not([hidden]) .pane")).toHaveCount(1);
  expect(await mock(page, () => window.__mock.windowCloses)).toBe(0);
});

test("tabs and splits are restored on the next launch", async ({ page }) => {
  await openApp(page, {
    session: {
      tabs: [
        { paths: [`${HOME}/Documents`], active: 0 },
        { paths: [HOME, `${HOME}/Downloads`], active: 1 },
        { paths: [42] }, // corrupt entry: skipped
        { paths: [`${HOME}/gone`] }, // missing folder: falls back to Home
      ],
      active: 1,
    },
  });
  await expect(tabLabels(page)).toHaveText(["Documents", "Home | Downloads", "Home"]);
  await expect(page.locator(".tab.active")).toHaveText("Home | Downloads");
  await expectPath(page, `${HOME}/Downloads`);

  // And the current state is saved as it changes.
  await page.keyboard.press(keys.mod("t"));
  await expect.poll(() => page.evaluate(() => JSON.parse(localStorage.getItem("sonar.session")).tabs.length)).toBe(4);
});
