import { expect, test } from "@playwright/test";
import { activePane, crumbs, expectPath, HOME, item, keys, labels, message, mock, openApp, status } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

test("lists home with folders first and hidden items hidden", async ({ page }) => {
  await expect(labels(activePane(page))).toHaveText([
    "Desktop", "Documents", "Downloads", "Music", "Pictures", "Videos", "archive.zip", "readme.md",
  ]);
  await expect(crumbs(page)).toHaveText(["Home"]);
  await expect(page.locator("#status-right")).toHaveText("3 hidden"); // .bashrc, .config, .local
  await expect(page.locator(".place.active")).toHaveText("Home");
});

test("folder item counts fill in after the listing", async ({ page }) => {
  await expect(item(activePane(page), "Documents").locator("td").nth(1)).toHaveText("3 items");
  await expect(item(activePane(page), "readme.md").locator("td").nth(1)).toHaveText("2.0 KiB");
});

test("double-click opens a folder; back and up select where we came from", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "Documents").dblclick();
  await expectPath(page, `${HOME}/Documents`);
  await expect(crumbs(page)).toHaveText(["Home", "Documents"]);
  await item(pane, "Projects").dblclick();
  await expect(crumbs(page)).toHaveText(["Home", "Documents", "Projects"]);

  await page.keyboard.press("Backspace");
  await expectPath(page, `${HOME}/Documents`);
  await expect(item(pane, "Projects")).toHaveClass(/selected/);

  await page.keyboard.press(keys.back); // back to Projects
  await expectPath(page, `${HOME}/Documents/Projects`);
  await page.keyboard.press(keys.back);
  await page.keyboard.press(keys.back);
  await expectPath(page, HOME);
  await expect(item(pane, "Documents")).toHaveClass(/selected/);
  await page.keyboard.press(keys.forward);
  await expectPath(page, `${HOME}/Documents`);
});

test("sidebar places and breadcrumbs navigate", async ({ page }) => {
  await page.locator(".place", { hasText: "Pictures" }).click();
  await expectPath(page, `${HOME}/Pictures`);
  await expect(page.locator(".place.active")).toHaveText("Pictures");
  await crumbs(page).first().click();
  await expectPath(page, HOME);
});

test("typing a path into the location bar", async ({ page }) => {
  await page.locator("#location").click({ position: { x: 600, y: 10 } });
  const input = page.locator("#location-input");
  await expect(input).toBeVisible();
  await expect(input).toHaveValue(HOME);
  await input.fill("~/Documents/Projects");
  await input.press("Enter");
  await expectPath(page, `${HOME}/Documents/Projects`);
  await expect(input).toBeHidden();

  // A bad path keeps the current listing and explains why.
  await page.keyboard.press(keys.mod("l"));
  await input.fill("/nope");
  await input.press("Enter");
  await expect(message(page)).toContainText("does not exist");
  await expectPath(page, `${HOME}/Documents/Projects`);
});

test("hidden files toggle", async ({ page }) => {
  await page.keyboard.press(keys.hidden);
  await expect(labels(activePane(page)).filter({ hasText: /^\.(bashrc|config|local)$/ })).toHaveCount(3);
  await expect(page.locator("#status-right")).toHaveText("");
  await page.locator("#btn-hidden").click();
  await expect(labels(activePane(page)).filter({ hasText: ".bashrc" })).toHaveCount(0);
});

test("sorting keeps folders on top in both directions", async ({ page }) => {
  const pane = activePane(page);
  await pane.locator("th[data-key=name]").click();
  await expect(labels(pane)).toHaveText([
    "Videos", "Pictures", "Music", "Downloads", "Documents", "Desktop", "readme.md", "archive.zip",
  ]);
  await pane.locator("th[data-key=size]").click();
  await expect(labels(pane).last()).toHaveText("archive.zip");
  await expect(labels(pane).first()).not.toHaveText(/\./);
});

test("type-ahead and keyboard selection", async ({ page }) => {
  const pane = activePane(page);
  await pane.locator(".scroller").focus();
  await page.keyboard.type("dow");
  await expect(item(pane, "Downloads")).toHaveClass(/selected/);
  await page.keyboard.press("Shift+ArrowDown");
  await page.keyboard.press("Shift+ArrowDown");
  await expect(status(page)).toHaveText(/3 items selected/);
  await page.keyboard.press("Escape");
  await expect(pane.locator(".selected")).toHaveCount(0);
});

test("opening files hands them to the system, and reports failures", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "readme.md").dblclick();
  await expect.poll(() => mock(page, () => window.__mock.callsTo("open_path").length)).toBe(1);

  await page.locator(".place", { hasText: "Pictures" }).click();
  await item(pane, "photo-copy.unknown").dblclick();
  await expect(message(page)).toContainText("no application knows how to open it");
});

test("live updates: external changes appear and the selection survives", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "readme.md").click();
  await mock(page, () => { window.__mock.write(`${window.__mock.HOME}/new.txt`, 1); window.__mock.touched(window.__mock.HOME); });
  await expect(item(pane, "new.txt")).toBeVisible();
  await expect(item(pane, "readme.md")).toHaveClass(/selected/);
});

test("when the open folder is deleted, the view walks up to one that exists", async ({ page }) => {
  await page.keyboard.press(keys.mod("l"));
  await page.locator("#location-input").fill("~/Documents/Projects/sonar");
  await page.locator("#location-input").press("Enter");
  await expectPath(page, `${HOME}/Documents/Projects/sonar`);
  await mock(page, () => {
    window.__mock.remove(`${window.__mock.HOME}/Documents/Projects`);
    window.__mock.touched(`${window.__mock.HOME}/Documents/Projects/sonar`);
  });
  await expectPath(page, `${HOME}/Documents`);
  await expect(message(page)).toContainText("no longer exists");
});
