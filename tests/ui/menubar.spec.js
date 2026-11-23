import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, item, keys, mock, openApp, panes, status, tabLabels } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

test("☰ menu (no native menu bar) drives the same actions", async ({ page }) => {
  await page.locator("#btn-menu").click();
  await expect(page.locator(".menu").first().locator(".menu-label")).toHaveText([
    "New Tab", "New Folder…", "Reopen Closed Tab", "View Mode", "Show Hidden Files", "Split View", "Filter…",
    "Go", "Copy Location", "Close Window",
  ]);
  await page.locator(".menu-item", { hasText: "Split View" }).click();
  await expect(panes(page)).toHaveCount(2);

  await page.locator("#btn-menu").click();
  await page.locator(".menu-item", { hasText: "Go" }).hover();
  await page.locator(".menu").nth(1).locator(".menu-item", { hasText: "Downloads" }).click();
  await expectPath(page, `${HOME}/Downloads`);
});

test("native menu bar items run actions", async ({ page }) => {
  await mock(page, () => window.__mock.menu("new-tab"));
  await expect(tabLabels(page)).toHaveCount(2);
  await mock(page, () => window.__mock.menu("go-documents"));
  await expectPath(page, `${HOME}/Documents`);
  await mock(page, () => window.__mock.menu("view-icons"));
  await expect(activePane(page).locator(".grid")).toBeVisible();
  await mock(page, () => window.__mock.menu("back"));
  await expectPath(page, HOME);
});

test("a shortcut handled by the page doesn't also run from the menu", async ({ page }) => {
  // The key press and its echo through the menu bar, in the same tick.
  await page.evaluate((mac) => {
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "t", metaKey: mac, ctrlKey: !mac, bubbles: true, cancelable: true }));
    window.__mock.menu("new-tab");
  }, process.platform === "darwin");
  await expect(tabLabels(page)).toHaveCount(2);
  await page.waitForTimeout(200);
  await mock(page, () => window.__mock.menu("new-tab")); // a real, later menu click
  await expect(tabLabels(page)).toHaveCount(3);
});

test("menu actions don't touch files while typing in a text field", async ({ page }) => {
  await item(activePane(page), "readme.md").click();
  await page.keyboard.press("F2");
  await expect(activePane(page).locator(".rename-input")).toBeFocused();
  await page.waitForTimeout(150); // past the shortcut-echo guard
  await mock(page, () => window.__mock.menu("trash"));
  await mock(page, () => window.__mock.menu("select-all"));
  const sel = await activePane(page).locator(".rename-input").evaluate((i) => [i.selectionStart, i.selectionEnd, i.value.length]);
  expect(sel).toEqual([0, 9, 9]); // Select All selected the text, not the files
  await page.keyboard.press("Escape");
  await expect(item(activePane(page), "readme.md")).toBeVisible();
  expect(await mock(page, () => window.__mock.exists(`${window.__mock.HOME}/readme.md`))).toBe(true);
});

test("Edit ▸ Copy/Paste from the menu bar arrive as clipboard events", async ({ page }) => {
  await item(activePane(page), "readme.md").click();
  await page.evaluate(() => document.dispatchEvent(new ClipboardEvent("copy", { bubbles: true, cancelable: true })));
  await expect.poll(() => mock(page, () => window.__mock.clipboard)).toEqual({ mode: "copy", paths: [`${HOME}/readme.md`] });
  await page.locator(".place", { hasText: "Music" }).click();
  await page.evaluate(() => document.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true })));
  await expect(item(activePane(page), "readme.md")).toBeVisible();
  await expect(status(page)).toContainText("Copied “readme.md” to “Music”");
});

test("menu check marks follow the active pane", async ({ page }) => {
  await page.evaluate(() => { window.__mock.nativeMenu = true; window.sonar.nativeMenu = true; });
  await page.keyboard.press("F3");
  await expect.poll(() => mock(page, () => window.__mock.menuState)).toMatchObject({ split: true, viewMode: "details", canGoUp: true });
  await page.keyboard.press(keys.icons);
  await expect.poll(() => mock(page, () => window.__mock.menuState?.viewMode)).toBe("icons");
});
