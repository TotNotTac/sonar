import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, item, keys, openApp, status } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

const selectedNames = (page) => page.evaluate(() => window.sonar.pane.selectedEntries().map((e) => e.name));
const center = async (locator) => {
  const b = await locator.boundingBox();
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
};

async function band(page, from, to, modifiers = []) {
  for (const m of modifiers) await page.keyboard.down(m);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move((from.x + to.x) / 2, (from.y + to.y) / 2, { steps: 4 });
  await page.mouse.move(to.x, to.y, { steps: 4 });
  await expect(page.locator(".rubberband")).toBeVisible();
  await page.mouse.up();
  for (const m of modifiers) await page.keyboard.up(m);
  await expect(page.locator(".rubberband")).toHaveCount(0);
}

test("rubber band from empty space selects the rows it touches", async ({ page }) => {
  const pane = activePane(page);
  const scroller = await pane.locator(".scroller").boundingBox();
  const pictures = await center(item(pane, "Pictures").locator("td").nth(2));
  await band(page, { x: pictures.x, y: scroller.y + scroller.height - 30 }, pictures);
  expect(await selectedNames(page)).toEqual(["Pictures", "Videos", "archive.zip", "readme.md"]);
  await expect(status(page)).toContainText("4 items selected");
});

test("rubber band can start on a row outside its name", async ({ page }) => {
  const pane = activePane(page);
  const from = await center(item(pane, "Desktop").locator("td").nth(2));
  const to = await center(item(pane, "Downloads").locator("td").nth(3));
  await band(page, from, to);
  expect(await selectedNames(page)).toEqual(["Desktop", "Documents", "Downloads"]);
});

test("Ctrl/⌘ rubber band toggles against the existing selection", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "Desktop").click();
  await item(pane, "Documents").click({ modifiers: [keys.mod("a").split("+")[0]] });
  const from = await center(item(pane, "Documents").locator("td").nth(2));
  const to = await center(item(pane, "Downloads").locator("td").nth(2));
  await band(page, from, to, [keys.mod("a").split("+")[0]]);
  // Documents was selected and got toggled off; Downloads got added.
  expect(await selectedNames(page)).toEqual(["Desktop", "Downloads"]);
});

test("rubber band in icons view", async ({ page }) => {
  await page.locator(".place", { hasText: "Pictures" }).click();
  await expectPath(page, `${HOME}/Pictures`);
  await page.keyboard.press(keys.icons);
  const pane = activePane(page);
  const tiles = pane.locator(".grid > [data-i]");
  const names = await tiles.locator(".tile-label").allTextContents();
  const grid = await pane.locator(".grid").boundingBox();
  const second = await tiles.nth(1).boundingBox();
  // Start in the grid's padding above-left of the first tile, end inside the second.
  await band(page, { x: grid.x + 3, y: grid.y + 3 }, { x: second.x + 10, y: second.y + 10 });
  expect(await selectedNames(page)).toEqual(names.slice(0, 2));
});

test("a plain click on part of a multi-selection collapses it on release", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "Desktop").click();
  await item(pane, "Music").click({ modifiers: ["Shift"] });
  expect((await selectedNames(page)).length).toBe(4);
  await item(pane, "Documents").locator(".name-label").click();
  expect(await selectedNames(page)).toEqual(["Documents"]);
});
