// Folders with thousands of items: only the rows near the viewport are in the
// DOM, but selection, keyboard movement, rename and the layout behave as if
// every item were there.

import { expect, test } from "@playwright/test";
import { activePane, HOME, item, keys, openApp, settle, status } from "./helpers.js";

const N = 5000;
const name = (i) => `file${String(i).padStart(5, "0")}.txt`;

test.beforeEach(async ({ page }) => {
  await openApp(page);
  await page.evaluate(({ HOME, N }) => {
    window.__mock.mkdir(`${HOME}/big`);
    for (let i = 0; i < N; i++) window.__mock.write(`${HOME}/big/file${String(i).padStart(5, "0")}.txt`, 100);
  }, { HOME, N });
  await page.evaluate((path) => window.sonar.pane.navigate(path), `${HOME}/big`);
  await settle(page);
});

const rendered = (page) => activePane(page).locator("[data-i]").count();
const focus = (page) => page.evaluate(() => window.sonar.pane.focus);

test("only rows near the viewport are rendered, and End/Home reach the ends", async ({ page }) => {
  expect(await rendered(page)).toBeLessThan(200);
  const pane = activePane(page);
  await item(pane, name(0)).click();
  await page.keyboard.press("End");
  await expect(item(pane, name(N - 1))).toBeInViewport();
  await expect(item(pane, name(N - 1))).toHaveClass(/selected/);
  await expect(item(pane, name(0))).toHaveCount(0); // scrolled far away: not in the DOM
  expect(await rendered(page)).toBeLessThan(200);

  await page.keyboard.press("Home");
  await expect(item(pane, name(0))).toBeInViewport();
  await page.keyboard.press("PageDown");
  const paged = await focus(page);
  expect(paged).toBeGreaterThan(10);
  await expect(item(pane, name(paged))).toBeInViewport();
});

test("selections cover items that aren't rendered", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, name(0)).click();
  await page.keyboard.press("Shift+End");
  await expect(status(page)).toContainText(`${N} items selected`);
  await page.keyboard.press("Home");
  await page.keyboard.press(keys.mod("a"));
  await expect(status(page)).toContainText(`${N} items selected`);
  // Scrolling to new rows shows them selected too.
  await page.evaluate(() => { window.sonar.pane.scroller.scrollTop = 40000; });
  const middle = activePane(page).locator("[data-i]").nth(30);
  await expect(middle).toHaveClass(/selected/);
});

test("a rename survives scrolling away and back", async ({ page }) => {
  const pane = activePane(page);
  await page.evaluate(() => { window.sonar.pane.scroller.scrollTop = 26 * 2000; });
  await item(pane, name(2010)).click();
  await page.keyboard.press("F2");
  const input = pane.locator(".rename-input");
  await expect(input).toBeVisible();
  await page.evaluate(() => { window.sonar.pane.scroller.scrollTop = 0; });
  await expect(item(pane, name(0))).toBeVisible();
  await expect(input).toHaveCount(1); // still there, off screen
  await page.evaluate(() => window.sonar.pane.viewImpl.reveal(2010));
  await input.fill("renamed.txt");
  await page.keyboard.press("Enter");
  await expect(item(pane, "renamed.txt")).toBeInViewport(); // moved to the end, and shown there
});

test("icons view: rows of tiles, arrow keys and resizing", async ({ page }) => {
  await page.keyboard.press(keys.icons);
  const pane = activePane(page);
  await expect(pane.locator(".grid")).toBeVisible();
  expect(await rendered(page)).toBeLessThan(400);
  const cols = await page.evaluate(() => window.sonar.pane.viewImpl.columns());
  expect(cols).toBeGreaterThan(1);
  // The computed columns match the CSS grid.
  const tops = await pane.locator(".grid > [data-i]").evaluateAll((els) => els.slice(0, 20).map((e) => e.offsetTop));
  expect(tops.filter((t) => t === tops[0]).length).toBe(cols);

  await item(pane, name(0)).click();
  await page.keyboard.press("ArrowDown");
  expect(await focus(page)).toBe(cols);
  await page.keyboard.press("End");
  await expect(item(pane, name(N - 1))).toBeInViewport();

  await page.setViewportSize({ width: 700, height: 700 });
  await expect.poll(() => page.evaluate(() => window.sonar.pane.viewImpl.columns())).toBeLessThan(cols);
  await page.keyboard.press("Home");
  await expect(item(pane, name(0))).toBeInViewport();
  const narrow = await page.evaluate(() => window.sonar.pane.viewImpl.columns());
  const tops2 = await pane.locator(".grid > [data-i]").evaluateAll((els) => els.slice(0, 20).map((e) => e.offsetTop));
  expect(tops2.filter((t) => t === tops2[0]).length).toBe(narrow);
});
