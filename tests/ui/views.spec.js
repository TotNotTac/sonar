import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, item, keys, labels, openApp, panes, status } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

test("icons view: grid layout and arrow keys move by tile and row", async ({ page }) => {
  await page.locator(".place", { hasText: "Pictures" }).click();
  await expectPath(page, `${HOME}/Pictures`);
  await page.keyboard.press(keys.icons);
  const pane = activePane(page);
  await expect(pane.locator(".grid > [data-i]")).toHaveCount(13);
  await expect(page.locator("#btn-view-icons")).toHaveAttribute("aria-pressed", "true");
  const cols = await page.evaluate(() => window.sonar.pane.viewImpl.columns());
  expect(cols).toBeGreaterThan(1);

  await pane.locator(".scroller").focus();
  await page.keyboard.press("ArrowRight"); // nothing focused yet -> first tile
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowDown");
  await expect.poll(() => page.evaluate(() => window.sonar.pane.focus)).toBe(1 + cols);
  // Moving down past the last row does nothing (rather than jumping to the end).
  for (let i = 0; i < 5; i++) await page.keyboard.press("ArrowDown");
  const last = await page.evaluate(() => window.sonar.pane.focus);
  expect(last).toBeLessThan(13);
  expect(last % cols).toBe(1);

  await page.keyboard.press(keys.details);
  await expect(pane.locator("table.details")).toBeVisible();
});

test("view mode and sort order are remembered per folder", async ({ page }) => {
  await page.keyboard.press(keys.icons);
  await page.locator(".place", { hasText: "Documents" }).click();
  await expectPath(page, `${HOME}/Documents`);
  await expect(activePane(page).locator("table.details")).toBeVisible();

  await page.keyboard.press(keys.back);
  await expectPath(page, HOME);
  await expect(activePane(page).locator(".grid")).toBeVisible();

  // Survives a restart.
  await page.reload();
  await page.waitForFunction(() => window.sonar?.pane?.view);
  await expect(activePane(page).locator(".grid")).toBeVisible();
});

test("filter bar narrows the listing live", async ({ page }) => {
  await page.locator(".place", { hasText: "Pictures" }).click();
  await expectPath(page, `${HOME}/Pictures`);
  await page.keyboard.press(keys.filter);
  const input = activePane(page).locator(".filterbar input");
  await expect(input).toBeFocused();
  await input.fill("photo1");
  await expect(labels(activePane(page))).toHaveText(["photo1.png", "photo10.png", "photo11.png", "photo12.png"]);
  await expect(activePane(page).locator(".filter-count")).toHaveText("4 of 13");
  await expect(status(page)).toContainText("filtered from 13 items");

  await input.press("Escape");
  await expect(activePane(page).locator(".filterbar")).toBeHidden();
  await expect(labels(activePane(page))).toHaveCount(13);
});

test("the filter resets when changing folder", async ({ page }) => {
  await page.keyboard.press(keys.filter);
  await activePane(page).locator(".filterbar input").fill("doc");
  await expect(labels(activePane(page))).toHaveText(["Documents"]);
  await item(activePane(page), "Documents").dblclick();
  await expectPath(page, `${HOME}/Documents`);
  await expect(labels(activePane(page))).toHaveCount(3);
});

test("narrow split panes drop the Type column to keep names readable", async ({ page }) => {
  await page.keyboard.press("F3");
  await expect(panes(page)).toHaveCount(2);
  const type = panes(page).first().locator("th[data-key=type]");
  await expect(type).toBeHidden();
  const nameWidth = await panes(page).first().locator("th[data-key=name]").evaluate((th) => th.getBoundingClientRect().width);
  expect(nameWidth).toBeGreaterThan(150);
  await page.keyboard.press("F3");
  await expect(activePane(page).locator("th[data-key=type]")).toBeVisible();
});
