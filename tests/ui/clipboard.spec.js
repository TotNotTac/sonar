import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, isMac, item, keys, mock, openApp, status } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

const clipboard = (page) => mock(page, () => window.__mock.clipboard);
const setClipboard = (page, clip) => mock(page, (c) => { window.__mock.clipboard = c; }, clip);
const exists = (page, p) => mock(page, (path) => window.__mock.exists(path), p);

test("copy and cut put files on the system clipboard", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "archive.zip").click();
  await item(pane, "readme.md").click({ modifiers: ["Shift"] });
  await page.keyboard.press(keys.mod("c"));
  await expect.poll(() => clipboard(page)).toEqual({ mode: "copy", paths: [`${HOME}/archive.zip`, `${HOME}/readme.md`] });
  await expect(status(page)).toContainText("Copied 2 items");

  await page.keyboard.press(keys.mod("x"));
  await expect.poll(() => clipboard(page)).toEqual({ mode: "cut", paths: [`${HOME}/archive.zip`, `${HOME}/readme.md`] });
  await expect(item(pane, "readme.md")).toHaveClass(/cut/);
});

test("paste files copied in another app", async ({ page }) => {
  await setClipboard(page, { mode: "copy", paths: [`${HOME}/Downloads/big.iso`] });
  await page.keyboard.press(keys.mod("v"));
  await expect(item(activePane(page), "big.iso")).toBeVisible();
  expect(await exists(page, `${HOME}/Downloads/big.iso`)).toBe(true);
});

test("files cut in another file manager are moved on paste", async ({ page }) => {
  await setClipboard(page, { mode: "cut", paths: [`${HOME}/Downloads/big.iso`] });
  await page.keyboard.press(keys.mod("v"));
  await expect(item(activePane(page), "big.iso")).toBeVisible();
  expect(await exists(page, `${HOME}/Downloads/big.iso`)).toBe(false);
});

test("cut + paste moves, then clears the clipboard", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "readme.md").click();
  await page.keyboard.press(keys.mod("x"));
  await item(pane, "Documents").dblclick();
  await expectPath(page, `${HOME}/Documents`);
  await page.keyboard.press(keys.mod("v"));
  await expect(item(pane, "readme.md")).toBeVisible();
  await expect.poll(() => clipboard(page)).toBeNull();
});

test("pasting with no files on the clipboard says so", async ({ page }) => {
  await setClipboard(page, { text: "just some text" });
  await page.keyboard.press(keys.mod("v"));
  await expect(status(page)).toHaveText("The clipboard has no files to paste");
});

test("Copy Location puts paths on the clipboard as text", async ({ page }) => {
  await item(activePane(page), "readme.md").click();
  await page.keyboard.press(`${isMac ? "Meta" : "Control"}+Alt+KeyC`);
  await expect.poll(() => clipboard(page)).toEqual({ text: `${HOME}/readme.md` });

  await page.keyboard.press("Escape"); // nothing selected: the folder itself
  await item(activePane(page), "readme.md").click({ button: "right" });
  await page.keyboard.press("Escape");
  await page.keyboard.press(`${isMac ? "Meta" : "Control"}+Alt+KeyC`);
  await expect.poll(() => clipboard(page)).toEqual({ text: `${HOME}/readme.md` });
});

test("⌘⌥V moves what was copied (macOS, like Finder)", async ({ page }) => {
  test.skip(!isMac, "macOS shortcut");
  await setClipboard(page, { mode: "copy", paths: [`${HOME}/Downloads/big.iso`] });
  await page.keyboard.press("Meta+Alt+KeyV");
  await expect(item(activePane(page), "big.iso")).toBeVisible();
  expect(await exists(page, `${HOME}/Downloads/big.iso`)).toBe(false);
});

test("cut marks follow the clipboard when the window regains focus", async ({ page }) => {
  await setClipboard(page, { mode: "cut", paths: [`${HOME}/readme.md`] });
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(item(activePane(page), "readme.md")).toHaveClass(/cut/);
  await setClipboard(page, null);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(item(activePane(page), "readme.md")).not.toHaveClass(/cut/);
});

test("Paste in the context menu is enabled only when there are files to paste", async ({ page }) => {
  const box = await activePane(page).locator(".scroller").boundingBox();
  const bg = { x: box.x + 300, y: box.y + box.height - 40 };
  await page.mouse.click(bg.x, bg.y, { button: "right" });
  await expect(page.locator(".menu-item", { hasText: /^\s*Paste/ }).first()).toHaveClass(/disabled/);
  await page.keyboard.press("Escape");

  await setClipboard(page, { mode: "copy", paths: [`${HOME}/Downloads/big.iso`] });
  await page.mouse.click(bg.x, bg.y, { button: "right" });
  const paste = page.locator(".menu-item").filter({ has: page.locator(".menu-label", { hasText: /^Paste$/ }) });
  await expect(paste).not.toHaveClass(/disabled/);
  await paste.click();
  await expect(item(activePane(page), "big.iso")).toBeVisible();
});
