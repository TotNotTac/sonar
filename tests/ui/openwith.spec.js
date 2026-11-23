import { expect, test } from "@playwright/test";
import { activePane, HOME, item, message, mock, openApp } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

const submenu = (page) => page.locator(".menu").nth(1);

async function openWithMenu(page, name) {
  await item(activePane(page), name).click({ button: "right" });
  await page.locator(".menu-item").filter({ has: page.locator(".menu-label", { hasText: /^Open With$/ }) }).hover();
}

test("Open With lists the apps for the file, default first", async ({ page }) => {
  await openWithMenu(page, "readme.md");
  await expect(submenu(page).locator(".menu-label")).toHaveText([
    "Typora (default)", "TextEdit", "Visual Studio Code", "Other Application…",
  ]);
  await expect(submenu(page).locator(".menu-item").first().locator("img.menu-icon, .menu-check")).not.toHaveCount(0);
  await submenu(page).locator(".menu-item", { hasText: "TextEdit" }).click();
  await expect.poll(() => mock(page, () => window.__mock.openedWith)).toEqual([["/Applications/TextEdit.app", [`${HOME}/readme.md`]]]);
});

test("Other Application… can make the choice the default", async ({ page }) => {
  await openWithMenu(page, "readme.md");
  await submenu(page).locator(".menu-item", { hasText: "Other Application…" }).click();
  const dlg = page.locator("dialog.choose-app[open]");
  await expect(dlg.locator(".dialog-title")).toHaveText("Open “readme.md” with");
  await expect(dlg.locator(".app-row.chosen")).toHaveText("Typora (default)");
  await dlg.locator(".app-row", { hasText: "Visual Studio Code" }).click();
  await dlg.getByLabel("Always use this app for MD text files").check();
  await dlg.getByRole("button", { name: "Open" }).click();
  await expect.poll(() => mock(page, () => window.__mock.defaults)).toEqual([[`${HOME}/readme.md`, "/Applications/Visual Studio Code.app"]]);
  expect(await mock(page, () => window.__mock.openedWith)).toEqual([["/Applications/Visual Studio Code.app", [`${HOME}/readme.md`]]]);
});

test("a refused default is reported and nothing opens", async ({ page }) => {
  await item(activePane(page), "Documents").dblclick();
  await openWithMenu(page, "report.pdf");
  await submenu(page).locator(".menu-item", { hasText: "Other Application…" }).click();
  const dlg = page.locator("dialog.choose-app[open]");
  await dlg.locator(".app-row", { hasText: "Okular" }).click();
  await dlg.locator(".dialog-check input").check();
  await dlg.getByRole("button", { name: "Open" }).click();
  await expect(message(page)).toContainText("refused to change the default app");
  expect(await mock(page, () => window.__mock.openedWith)).toEqual([]);
});

test("no Open With when no app is known; folders can be opened with apps", async ({ page }) => {
  await item(activePane(page), "archive.zip").click({ button: "right" });
  await expect(page.locator(".menu-label", { hasText: /^Open With$/ })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await openWithMenu(page, "Music");
  await expect(submenu(page).locator(".menu-label")).toHaveText(["Visual Studio Code", "Other Application…"]);
});
