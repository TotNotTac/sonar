import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, item, keys, labels, message, mock, openApp, status } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

const exists = (page, p) => mock(page, (path) => window.__mock.exists(path), p);

test("inline rename selects the name without its extension", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "readme.md").click();
  await page.keyboard.press("F2");
  const input = pane.locator(".rename-input");
  await expect(input).toBeFocused();
  const selected = await input.evaluate((i) => i.value.slice(i.selectionStart, i.selectionEnd));
  expect(selected).toBe("readme");
  await page.keyboard.type("README");
  await page.keyboard.press("Enter");
  await expect(item(pane, "README.md")).toHaveClass(/selected/);
  expect(await exists(page, `${HOME}/README.md`)).toBe(true);
});

test("rename clash keeps the field open; Escape cancels", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "readme.md").click();
  await page.keyboard.press("F2");
  await pane.locator(".rename-input").fill("archive.zip");
  await page.keyboard.press("Enter");
  await expect(message(page)).toContainText("already exists");
  await expect(pane.locator(".rename-input")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(pane.locator(".rename-input")).toHaveCount(0);
  await expect(item(pane, "readme.md")).toBeVisible();
});

test("new folder goes straight into rename", async ({ page }) => {
  await page.keyboard.press(keys.newFolder);
  const input = activePane(page).locator(".rename-input");
  await expect(input).toHaveValue("New Folder");
  await page.keyboard.type("Stuff");
  await page.keyboard.press("Enter");
  await expect(item(activePane(page), "Stuff")).toHaveClass(/selected/);
});

test("trash keeps the cursor in place, and undo restores", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "Music").click();
  await page.keyboard.press(keys.trash);
  await expect(item(pane, "Music")).toHaveCount(0);
  await expect(item(pane, "Pictures")).toHaveClass(/selected/);
  await expect(status(page)).toContainText("Moved “Music” to the Trash");
  expect(await exists(page, `${HOME}/.local/share/Trash/files/Music`)).toBe(true);

  await page.keyboard.press(keys.mod("z"));
  await expect(item(pane, "Music")).toBeVisible();
  await expect(status(page)).toContainText("Undone: Move to Trash");
});

test("permanent delete asks first", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "archive.zip").click();
  await page.keyboard.press(keys.deletePermanently);
  const dialog = page.locator("dialog[open]");
  await expect(dialog.locator(".dialog-title")).toHaveText("Permanently delete “archive.zip”?");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  expect(await exists(page, `${HOME}/archive.zip`)).toBe(true);

  await page.keyboard.press(keys.deletePermanently);
  await page.locator("dialog[open]").getByRole("button", { name: "Delete" }).click();
  await expect(item(pane, "archive.zip")).toHaveCount(0);
  expect(await exists(page, `${HOME}/archive.zip`)).toBe(false);
});

test("deleting inside the Trash is always permanent", async ({ page }) => {
  await item(activePane(page), "readme.md").click();
  await page.keyboard.press(keys.trash);
  await page.locator(".place", { hasText: "Trash" }).click();
  await item(activePane(page), "readme.md").click();
  await page.keyboard.press(keys.trash);
  await expect(page.locator("dialog[open] .dialog-title")).toHaveText("Permanently delete “readme.md”?");
});

test("copy and paste into the same folder makes a numbered duplicate", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "readme.md").click();
  await page.keyboard.press(keys.mod("c"));
  await page.keyboard.press(keys.mod("v"));
  await expect(item(pane, "readme (1).md")).toBeVisible();
  await expect(status(page)).toContainText("Copied “readme.md” to “Home”");
  await page.keyboard.press(keys.mod("z"));
  await expect(item(pane, "readme (1).md")).toHaveCount(0);
});

test("paste conflict: dialog with Skip / Keep Both / Replace and apply-to-all", async ({ page }) => {
  await mock(page, () => {
    const m = window.__mock;
    m.write(`${m.HOME}/Documents/readme.md`, 5);
    m.write(`${m.HOME}/Documents/archive.zip`, 5);
  });
  const pane = activePane(page);
  await item(pane, "archive.zip").click();
  await item(pane, "readme.md").click({ modifiers: ["Shift"] });
  await page.keyboard.press(keys.mod("c"));
  await page.locator(".place", { hasText: "Documents" }).click();
  await expectPath(page, `${HOME}/Documents`);
  await page.keyboard.press(keys.mod("v"));

  const dialog = page.locator("dialog[open]");
  await expect(dialog.locator(".dialog-title")).toHaveText("“archive.zip” already exists in “Documents”");
  await expect(dialog.locator(".btn")).toHaveText(["Cancel", "Skip", "Keep Both", "Replace"]);
  await expect(dialog.locator("dd")).toHaveText([/^5 B/, /^5\.0 MiB/]);
  await dialog.getByLabel("Apply to all conflicts").check();
  await dialog.getByRole("button", { name: "Keep Both" }).click();

  // The second conflict was answered by "apply to all".
  await expect(item(pane, "archive (1).zip")).toBeVisible();
  await expect(item(pane, "readme (1).md")).toBeVisible();
  await expect(page.locator("dialog[open]")).toHaveCount(0);
});

test("cut and paste moves, and empties the clipboard", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "readme.md").click();
  await page.keyboard.press(keys.mod("x"));
  await expect(item(pane, "readme.md")).toHaveClass(/cut/);
  await item(pane, "Documents").dblclick();
  await page.keyboard.press(keys.mod("v"));
  await expect(item(pane, "readme.md")).toBeVisible();
  expect(await exists(page, `${HOME}/readme.md`)).toBe(false);
  await expect.poll(() => page.evaluate(() => window.sonar.clipboard)).toBeNull();
  await expect(labels(pane)).toHaveText(["Projects", "notes.txt", "readme.md", "report.pdf"]);
});
