import { expect, test } from "@playwright/test";
import { activePane, crumbs, expectPath, HOME, item, keys, labels, mock, openApp, panes, status } from "./helpers.js";

const DL = `${HOME}/Downloads`;
const exists = (page, p) => mock(page, (path) => window.__mock.exists(path), p);

test.beforeEach(async ({ page }) => {
  await openApp(page);
  await page.locator(".place", { hasText: "Downloads" }).click();
  await expectPath(page, DL);
});

test("archives open like read-only folders", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "photos.zip").dblclick();
  await expectPath(page, `${DL}/photos.zip`);
  await expect(crumbs(page)).toHaveText(["Home", "Downloads", "photos.zip"]);
  await expect(labels(pane)).toHaveText(["photos"]);
  await expect(page.locator("#status-right")).toContainText("Read-only archive");

  await item(pane, "photos").dblclick();
  await expect(labels(pane)).toHaveText(["2025", "readme.txt"]);
  await expect(item(pane, "2025").locator("td").nth(1)).toHaveText("2 items");
  await page.keyboard.press("Backspace");
  await page.keyboard.press("Backspace");
  await expectPath(page, DL);
  await expect(item(pane, "photos.zip")).toHaveClass(/selected/);
});

test("nothing inside an archive can be changed", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "photos.zip").dblclick();
  await item(pane, "photos").click();
  await page.keyboard.press("F2");
  await expect(pane.locator(".rename-input")).toHaveCount(0);
  await expect(status(page)).toContainText("“photos.zip” is read-only");
  await page.keyboard.press(keys.trash);
  await page.keyboard.press(keys.newFolder);
  await expect(labels(pane)).toHaveText(["photos"]);
  expect(await mock(page, () => window.__mock.callsTo("trash_entries").length)).toBe(0);

  await item(pane, "photos").click({ button: "right" });
  await expect(page.locator(".menu .menu-label")).toHaveText(["Open", "Extract to “Downloads”", "Copy", "Copy Location", "Properties"]);
});

test("copy items out of an archive", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "photos.zip").dblclick();
  await item(pane, "photos").dblclick();
  await item(pane, "readme.txt").click();
  await item(pane, "2025").click({ modifiers: ["Shift"] });
  await page.keyboard.press(keys.mod("c"));
  await page.locator(".place", { hasText: "Documents" }).click();
  await page.keyboard.press(keys.mod("v"));
  await expect(item(pane, "readme.txt")).toBeVisible();
  await expect(item(pane, "2025")).toBeVisible();
  expect(await exists(page, `${HOME}/Documents/2025/beach.jpg`)).toBe(true);
});

test("Extract to … from inside the archive", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "photos.zip").dblclick();
  await item(pane, "photos").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Extract to" }).click();
  await expect.poll(() => exists(page, `${DL}/photos/2025/sunset.jpg`)).toBe(true);
  await expect(status(page)).toContainText("Copied “photos” to “Downloads”");
});

test("Extract Here: one top folder as is, several items into a new folder, never overwriting", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "photos.zip").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Extract Here" }).click();
  await expect(item(pane, "photos")).toHaveClass(/selected/);
  await expect(status(page)).toContainText("Extracted “photos.zip”");
  expect(await exists(page, `${DL}/photos/2025/beach.jpg`)).toBe(true);

  await item(pane, "bundle.tar.gz").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Extract Here" }).click();
  await expect(item(pane, "bundle")).toHaveClass(/selected/);
  expect(await exists(page, `${DL}/bundle/a.txt`)).toBe(true);

  await item(pane, "photos.zip").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Extract Here" }).click();
  await expect(item(pane, "photos (1)")).toBeVisible();

  await page.keyboard.press(keys.mod("z"));
  await expect(item(pane, "photos (1)")).toHaveCount(0);
  await expect(status(page)).toContainText("Undone: Extract");
});

test("Extract to the other side of a split view", async ({ page }) => {
  await page.keyboard.press("F3");
  await page.locator(".place", { hasText: "Music" }).click();
  const left = panes(page).first();
  await item(left, "bundle.tar.gz").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Extract to “Music”" }).click();
  await expect(item(panes(page).nth(1), "bundle")).toBeVisible();
});

test("Compress the selection", async ({ page }) => {
  await page.locator(".place", { hasText: "Home" }).click();
  const pane = activePane(page);
  await item(pane, "archive.zip").click();
  await item(pane, "readme.md").click({ modifiers: ["Shift"] });
  await item(pane, "readme.md").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Compress" }).hover();
  await expect(page.locator(".menu").nth(1).locator(".menu-label")).toHaveText(["as ZIP", "as 7-Zip", "as tar.gz", "as tar.xz", "as tar.bz2"]);
  await page.locator(".menu").nth(1).locator(".menu-item", { hasText: "as ZIP" }).click();
  await expect(item(pane, "Archive.zip")).toHaveClass(/selected/);
  await expect(status(page)).toContainText("Created “Archive.zip”");
  const entries = await mock(page, (p) => window.__mock.archiveEntries(p).map((e) => e.path), `${HOME}/Archive.zip`);
  expect(entries).toEqual(expect.arrayContaining(["archive.zip", "readme.md"]));

  await item(pane, "readme.md").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Compress" }).hover();
  await page.locator(".menu").nth(1).locator(".menu-item", { hasText: "as 7-Zip" }).click();
  await expect(item(pane, "readme.md.7z")).toHaveClass(/selected/);
});

test("archives aren't drop targets, and their items can't be dragged to other apps", async ({ page }) => {
  await page.keyboard.press("F3");
  await panes(page).nth(1).locator(".scroller").click({ position: { x: 300, y: 300 } });
  await item(panes(page).nth(1), "photos.zip").dblclick();
  const [left, right] = [panes(page).first(), panes(page).nth(1)];
  const from = await item(left, "big.iso").locator(".name-label").boundingBox();
  const to = await right.locator(".scroller").boundingBox();
  await page.mouse.move(from.x + 5, from.y + 5);
  await page.mouse.down();
  await page.mouse.move(to.x + 100, to.y + 200, { steps: 8 });
  await expect(page.locator(".drop-target")).toHaveCount(0);
  await page.mouse.up();
  await expect(page.locator(".menu")).toHaveCount(0);

  const inner = await item(right, "photos").locator(".name-label").boundingBox();
  await page.mouse.move(inner.x + 5, inner.y + 5);
  await page.mouse.down();
  await page.mouse.move(inner.x + 30, inner.y + 10, { steps: 3 });
  await page.mouse.move(-20, 200, { steps: 4 });
  await expect(status(page)).toContainText("extract them first");
  expect(await mock(page, () => window.__mock.dragOuts.length)).toBe(0);
  await page.mouse.up();
});

test("opening a file inside an archive", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "photos.zip").dblclick();
  await item(pane, "photos").dblclick();
  await item(pane, "readme.txt").dblclick();
  await expect.poll(() => mock(page, () => window.__mock.opened)).toEqual([`${DL}/photos.zip/photos/readme.txt`]);
});

test("password-protected archives ask for the password, again if it's wrong", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "secret.zip").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Extract Here" }).click();
  const dlg = page.locator(".password-dialog");
  await expect(dlg.locator(".dialog-title")).toHaveText("“secret.zip” is password protected");
  await dlg.locator("input").fill("nope");
  await page.keyboard.press("Enter");
  await expect(dlg.locator(".dialog-message")).toHaveText("That password is not correct. Try again.");
  await dlg.locator("input").fill("hunter2");
  await dlg.locator(".btn.primary").click();
  await expect(item(pane, "secret")).toHaveClass(/selected/);
  expect(await exists(page, `${DL}/secret/plan.txt`)).toBe(true);
});

test("cancelling the password prompt cancels the job", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "secret.zip").dblclick(); // listing needs no password
  await item(pane, "plan.txt").dblclick();
  await expect(page.locator(".password-dialog")).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.locator(".password-dialog")).toHaveCount(0);
  expect(await mock(page, () => window.__mock.opened)).toEqual([]);
  await expect(pane.locator(".message:visible")).toHaveCount(0);

  await item(pane, "plan.txt").click();
  await page.keyboard.press(keys.mod("c"));
  await page.locator(".place", { hasText: "Documents" }).click();
  await page.keyboard.press(keys.mod("v"));
  await page.locator(".password-dialog .btn", { hasText: "Cancel" }).click();
  await expect(status(page)).toHaveText("Copy cancelled");
  expect(await exists(page, `${HOME}/Documents/plan.txt`)).toBe(false);
});

test("archives inside archives open like folders, after the outer password", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "secret.zip").dblclick();
  await item(pane, "inner.tar.gz").dblclick();
  await page.locator(".password-dialog input").fill("hunter2");
  await page.keyboard.press("Enter");
  await expectPath(page, `${DL}/secret.zip/inner.tar.gz`);
  await expect(crumbs(page)).toHaveText(["Home", "Downloads", "secret.zip", "inner.tar.gz"]);
  await item(pane, "deep").dblclick();
  await item(pane, "treasure.txt").dblclick();
  await expect.poll(() => mock(page, () => window.__mock.opened)).toEqual([`${DL}/secret.zip/inner.tar.gz/deep/treasure.txt`]);
  await expect(page.locator(".password-dialog")).toHaveCount(0); // remembered
});
