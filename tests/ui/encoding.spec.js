// File names that aren't valid UTF-8 reach the UI with each undecodable byte
// as a character in U+10FF00–U+10FFFF (sonar-core's uipath.rs). The UI shows
// them as U+FFFD but must send the encoded path back unchanged.

import { expect, test } from "@playwright/test";
import { activePane, HOME, isMac, item, keys, mock, openApp, settle } from "./helpers.js";

const RAW = "caf\u{10FFE9}.txt"; // "café.txt" in Latin-1, as the backend encodes it
const SHOWN = "caf\uFFFD.txt";

test.beforeEach(async ({ page }) => {
  await openApp(page);
  await page.evaluate(({ HOME, RAW }) => {
    window.__mock.write(`${HOME}/Documents/${RAW}`, 10);
    window.__mock.mkdir(`${HOME}/Documents/dir\u{10FFE9}`);
  }, { HOME, RAW });
  await item(activePane(page), "Documents").dblclick();
  await settle(page);
});

test("undecodable bytes show as U+FFFD", async ({ page }) => {
  const pane = activePane(page);
  await expect(item(pane, SHOWN)).toBeVisible();
  await item(pane, SHOWN).click();
  await page.keyboard.press(keys.properties);
  await expect(page.locator("dialog.props .dialog-title")).toHaveText(SHOWN);
});

test("operations get the encoded path back", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, SHOWN).click();
  await page.keyboard.press(keys.trash);
  await expect(item(pane, SHOWN)).toHaveCount(0);
  expect(await mock(page, () => window.__mock.callsTo("trash_entries"))).toEqual([{ paths: [`${HOME}/Documents/${RAW}`] }]);
});

test("renaming starts from the shown name and sends the real path", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, SHOWN).click();
  await page.keyboard.press("F2");
  const input = pane.locator(".rename-input");
  await expect(input).toHaveValue(SHOWN);
  await page.keyboard.press("Enter"); // unchanged: nothing to do
  expect(await mock(page, () => window.__mock.callsTo("rename_entry"))).toEqual([]);

  await page.keyboard.press("F2");
  await input.fill("cafe.txt");
  await page.keyboard.press("Enter");
  await expect(item(pane, "cafe.txt")).toBeVisible();
  expect(await mock(page, () => window.__mock.callsTo("rename_entry"))).toEqual([
    { path: `${HOME}/Documents/${RAW}`, newName: "cafe.txt" },
  ]);
});

test("copied locations and the location bar use the shown text", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, SHOWN).click();
  await page.keyboard.press(isMac ? "Meta+Alt+c" : "Control+Alt+c");
  await expect.poll(() => mock(page, () => window.__mock.clipboard?.text)).toBe(`${HOME}/Documents/${SHOWN}`);

  // A folder with such a name: the location bar shows U+FFFD, and Enter stays put.
  await item(pane, "dir\uFFFD").dblclick();
  await settle(page);
  await page.keyboard.press(keys.mod("l"));
  await expect(page.locator("#location-input")).toHaveValue(`${HOME}/Documents/dir\uFFFD`);
  await page.keyboard.press("Enter");
  await expect(page.locator("#location-input")).toBeHidden();
  expect(await page.evaluate(() => window.sonar.pane.path)).toBe(`${HOME}/Documents/dir\u{10FFE9}`);
  expect(await mock(page, () => window.__mock.callsTo("resolve_path"))).toEqual([]);
});
