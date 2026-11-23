import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, isMac, item, keys, mock, openApp, panes, status } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

const exists = (page, p) => mock(page, (path) => window.__mock.exists(path), p);
const center = async (locator) => {
  const b = await locator.boundingBox();
  return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
};

/** Press on `source`, drag to `target` (a locator or point) and release, optionally holding keys. */
async function drag(page, source, target, { hold = [], release = true } = {}) {
  const from = await center(source);
  const to = target.x != null ? target : await center(target);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + 10, from.y + 4, { steps: 3 });
  await expect(page.locator(".drag-ghost")).toBeVisible();
  await page.mouse.move(to.x, to.y, { steps: 10 });
  for (const k of hold) await page.keyboard.down(k);
  if (release) await page.mouse.up();
  for (const k of hold) await page.keyboard.up(k);
}

const label = (pane, name) => item(pane, name).locator(".name-label, .tile-label");
const dropMenu = (page) => page.locator(".menu .menu-label");

test("drop on a folder asks Move or Copy", async ({ page }) => {
  const pane = activePane(page);
  await drag(page, label(pane, "readme.md"), item(pane, "Documents"), { release: false });
  await expect(item(pane, "Documents")).toHaveClass(/drop-target/);
  await page.mouse.up();
  await expect(dropMenu(page)).toHaveText(["Move Here", "Copy Here", "Cancel"]);
  await page.locator(".menu-item", { hasText: "Copy Here" }).click();
  await expect.poll(() => exists(page, `${HOME}/Documents/readme.md`)).toBe(true);
  expect(await exists(page, `${HOME}/readme.md`)).toBe(true);
  await expect(page.locator(".drop-target")).toHaveCount(0);
});

test("Shift moves and Ctrl/⌥ copies without asking", async ({ page }) => {
  const pane = activePane(page);
  await drag(page, label(pane, "readme.md"), item(pane, "Music"), { hold: ["Shift"] });
  await expect(page.locator(".menu")).toHaveCount(0);
  await expect(item(pane, "readme.md")).toHaveCount(0);
  expect(await exists(page, `${HOME}/Music/readme.md`)).toBe(true);

  await drag(page, label(pane, "archive.zip"), item(pane, "Music"), { hold: [isMac ? "Alt" : "Control"] });
  await expect(page.locator(".menu")).toHaveCount(0);
  await expect.poll(() => exists(page, `${HOME}/Music/archive.zip`)).toBe(true);
  expect(await exists(page, `${HOME}/archive.zip`)).toBe(true);
});

test("a whole multi-selection is dragged", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "archive.zip").click();
  await item(pane, "readme.md").click({ modifiers: ["Shift"] });
  await drag(page, label(pane, "archive.zip"), item(pane, "Videos"), { hold: ["Shift"] });
  await expect(page.locator(".drag-ghost")).toHaveCount(0);
  await expect.poll(() => exists(page, `${HOME}/Videos/readme.md`)).toBe(true);
  expect(await exists(page, `${HOME}/Videos/archive.zip`)).toBe(true);
});

test("drop on sidebar places and breadcrumbs", async ({ page }) => {
  const pane = activePane(page);
  await drag(page, label(pane, "readme.md"), page.locator(".place", { hasText: "Pictures" }), { hold: ["Shift"] });
  await expect.poll(() => exists(page, `${HOME}/Pictures/readme.md`)).toBe(true);

  await item(pane, "Documents").dblclick();
  await expectPath(page, `${HOME}/Documents`);
  await drag(page, label(pane, "notes.txt"), page.locator(".crumb", { hasText: "Home" }), { hold: ["Shift"] });
  await expect.poll(() => exists(page, `${HOME}/notes.txt`)).toBe(true);
});

test("pointless drops do nothing: same folder, or a folder into itself", async ({ page }) => {
  const pane = activePane(page);
  const box = await pane.locator(".scroller").boundingBox();
  await drag(page, label(pane, "readme.md"), { x: box.x + 300, y: box.y + box.height - 40 });
  await drag(page, label(pane, "Documents"), item(pane, "Documents"));
  await expect(page.locator(".menu")).toHaveCount(0);
  await expect(page.locator(".drop-target")).toHaveCount(0);
});

test("Escape cancels a drag", async ({ page }) => {
  const pane = activePane(page);
  await drag(page, label(pane, "readme.md"), item(pane, "Music"), { release: false });
  await page.keyboard.press("Escape");
  await expect(page.locator(".drag-ghost")).toHaveCount(0);
  await page.mouse.up();
  await expect(page.locator(".menu")).toHaveCount(0);
  expect(await exists(page, `${HOME}/readme.md`)).toBe(true);
});

test("drag between split panes", async ({ page }) => {
  await page.keyboard.press("F3");
  await page.locator(".place", { hasText: "Downloads" }).click();
  const [left, right] = [panes(page).first(), panes(page).nth(1)];
  await drag(page, label(left, "readme.md"), right.locator(".scroller"), { hold: ["Shift"] });
  await expect(item(right, "readme.md")).toBeVisible();
  await expect(item(left, "readme.md")).toHaveCount(0);
});

test("hovering a tab switches to it, then drop there", async ({ page }) => {
  await page.keyboard.press(keys.mod("t"));
  await page.locator(".place", { hasText: "Music" }).click();
  await page.locator(".tab", { hasText: "Home" }).click();
  await expectPath(page, HOME);
  const pane = activePane(page);
  await drag(page, label(pane, "readme.md"), page.locator(".tab", { hasText: "Music" }), { release: false });
  await expect(page.locator(".tab.active")).toHaveText("Music", { timeout: 2000 });
  const musicPane = activePane(page);
  await page.mouse.move(...Object.values(await center(musicPane.locator(".scroller"))), { steps: 5 });
  await page.keyboard.down("Shift");
  await page.mouse.up();
  await page.keyboard.up("Shift");
  await expect(item(musicPane, "readme.md")).toBeVisible();
});

test("leaving the window hands the drag to the OS", async ({ page }) => {
  const pane = activePane(page);
  await drag(page, label(pane, "readme.md"), item(pane, "Music"), { release: false });
  await page.mouse.move(-20, 200, { steps: 5 });
  await expect.poll(() => mock(page, () => window.__mock.dragOuts)).toEqual([[`${HOME}/readme.md`]]);
  await expect(page.locator(".drag-ghost")).toHaveCount(0);
  await page.mouse.up();
});

test("files dropped from another app", async ({ page }) => {
  const pane = activePane(page);
  await mock(page, () => window.__mock.write("/tmp/report from mail.pdf", 1234));
  const target = await center(item(pane, "Documents"));
  await mock(page, ({ x, y }) => window.__mock.dropFiles(["/tmp/report from mail.pdf"], x, y), target);
  await expect(dropMenu(page)).toHaveText(["Move Here", "Copy Here", "Cancel"]);
  await page.locator(".menu-item", { hasText: "Copy Here" }).click();
  await expect.poll(() => exists(page, `${HOME}/Documents/report from mail.pdf`)).toBe(true);
  await expect(status(page)).toContainText("Copied “report from mail.pdf” to “Documents”");

  // A drag that just passes over the window leaves no highlight behind.
  await mock(page, ({ x, y }) => window.__mock.dropFiles(["/tmp/x"], x, y, { leaveFirst: true }), target);
  await expect(page.locator(".drop-target")).toHaveCount(0);
});

test("pressing on a name drags; it never starts a rubber band", async ({ page }) => {
  const pane = activePane(page);
  await drag(page, label(pane, "Music"), item(pane, "Videos"), { release: false });
  await expect(page.locator(".rubberband")).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.mouse.up();
});
