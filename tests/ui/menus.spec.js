import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, item, openApp } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

const menuLabels = (page, level = 0) => page.locator(".menu").nth(level).locator(".menu-label");

test("item context menu for a folder", async ({ page }) => {
  await item(activePane(page), "Music").click({ button: "right" });
  await expect(item(activePane(page), "Music")).toHaveClass(/selected/);
  await expect(menuLabels(page)).toHaveText([
    "Open", "Open in New Tab", "Open in Split View", "Compress", "Cut", "Copy", "Paste Into Folder", "Copy Location",
    "Rename…", "Move to Trash", "Delete Permanently…",
  ]);
  await expect(page.locator(".menu-item", { hasText: "Paste Into Folder" })).toHaveClass(/disabled/);
  await page.locator(".menu-item", { hasText: "Rename…" }).click();
  await expect(activePane(page).locator(".rename-input")).toHaveValue("Music");
});

test("background menu with submenus, driven by keyboard", async ({ page }) => {
  const scroller = activePane(page).locator(".scroller");
  const box = await scroller.boundingBox();
  await page.mouse.click(box.x + 300, box.y + box.height - 40, { button: "right" });
  await expect(menuLabels(page)).toHaveText([
    "New Folder…", "Paste", "Undo", "View Mode", "Sort By", "Show Hidden Files",
    "Copy Location", "Search…", "Filter…", "Open in New Tab", "Split View",
  ]);
  // Down to "View Mode" (disabled items are skipped), open it, pick "Icons".
  for (let i = 0; i < 2; i++) await page.keyboard.press("ArrowDown");
  await expect(page.locator(".menu-item.hot")).toHaveText(/View Mode/);
  await page.keyboard.press("ArrowRight");
  await expect(menuLabels(page, 1)).toHaveText(["Icons", "Details"]);
  await page.keyboard.press("Enter");
  await expect(page.locator(".menu")).toHaveCount(0);
  await expect(activePane(page).locator(".grid")).toBeVisible();
});

test("menus close on Escape and outside clicks without acting", async ({ page }) => {
  await item(activePane(page), "Music").click({ button: "right" });
  await page.keyboard.press("Escape");
  await expect(page.locator(".menu")).toHaveCount(0);
  await item(activePane(page), "Music").click({ button: "right" });
  await page.mouse.click(5, 650);
  await expect(page.locator(".menu")).toHaveCount(0);
  await expectPath(page, HOME);
});

test("undo entry names the last action", async ({ page }) => {
  await item(activePane(page), "readme.md").click();
  await page.keyboard.press("Delete");
  await expect(item(activePane(page), "readme.md")).toHaveCount(0);
  const box = await activePane(page).locator(".scroller").boundingBox();
  await page.mouse.click(box.x + 300, box.y + box.height - 40, { button: "right" });
  await page.locator(".menu-item", { hasText: "Undo Move to Trash" }).click();
  await expect(item(activePane(page), "readme.md")).toBeVisible();
});
