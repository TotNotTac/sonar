import { expect, test } from "@playwright/test";
import { activePane, crumbs, expectPath, HOME, item, keys, mock, openApp, panes } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

test("F3 splits; toolbar and breadcrumbs follow the active side", async ({ page }) => {
  await page.keyboard.press("F3");
  await expect(panes(page)).toHaveCount(2);
  await expect(page.locator("#btn-split")).toHaveAttribute("aria-pressed", "true");
  await expect(panes(page).nth(1)).toHaveClass(/active/);

  await page.locator(".place", { hasText: "Downloads" }).click();
  await expect(crumbs(page)).toHaveText(["Home", "Downloads"]);
  await expect(page.locator(".pane-header")).toHaveText(["Home", "Downloads"]);
  await expect(page.locator(".tab.active")).toHaveText("Home | Downloads");

  await item(panes(page).first(), "Music").click();
  await expect(panes(page).first()).toHaveClass(/active/);
  await expect(crumbs(page)).toHaveText(["Home"]);

  // Closing the split keeps the side being worked in.
  await page.keyboard.press("F3");
  await expect(panes(page)).toHaveCount(1);
  await expectPath(page, HOME);
});

test("each side gets its own live updates and item counts", async ({ page }) => {
  await page.keyboard.press("F3");
  await page.locator(".place", { hasText: "Documents" }).click();
  const [left, right] = [panes(page).first(), panes(page).nth(1)];
  await expect(item(right, "Projects").locator("td").nth(1)).toHaveText("1 item");
  await expect(item(left, "Documents").locator("td").nth(1)).toHaveText("3 items");

  await mock(page, () => { window.__mock.write(`${window.__mock.HOME}/Documents/fresh.txt`, 1); window.__mock.touched(`${window.__mock.HOME}/Documents`); });
  await expect(item(right, "fresh.txt")).toBeVisible();
  await expect(item(left, "Documents").locator("td").nth(1)).toHaveText("3 items");
});

test("copy on one side, paste on the other", async ({ page }) => {
  await page.keyboard.press("F3");
  await page.locator(".place", { hasText: "Downloads" }).click();
  await item(panes(page).first(), "readme.md").click();
  await page.keyboard.press(keys.mod("c"));
  await item(panes(page).nth(1), "big.iso").click();
  await page.keyboard.press(keys.mod("v"));
  await expect(item(panes(page).nth(1), "readme.md")).toBeVisible();
  await expect(item(panes(page).first(), "readme.md")).toBeVisible();
});

test("Open in Split View from a folder's context menu", async ({ page }) => {
  await item(activePane(page), "Pictures").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Open in Split View" }).click();
  await expect(panes(page)).toHaveCount(2);
  await expectPath(page, `${HOME}/Pictures`);
});
