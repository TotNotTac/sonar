// Picker mode: the window is a file chooser for another app (the portal's
// Open / Save dialogs). The request comes from `picker_request`; the answer
// goes to `picker_respond`.

import { expect, test } from "@playwright/test";
import { activePane, HOME, item, mock, openApp, settle, status } from "./helpers.js";

const request = (r) => ({
  id: 1, save: false, title: "", acceptLabel: null, multiple: false, directory: false,
  filters: [], currentFilter: null, choices: [], folder: null, name: null, ...r,
});
const answers = (page) => mock(page, () => window.__mock.pickerAnswers);
const bar = (page) => page.locator(".picker-bar");
const button = (page, name) => bar(page).getByRole("button", { name, exact: true });

test("Open: filters by type, multiple selection, and the answer", async ({ page }) => {
  await openApp(page, { picker: request({
    title: "Upload photos", acceptLabel: "Upload", multiple: true, folder: `${HOME}/Pictures`,
    filters: [{ name: "Images", patterns: [[1, "image/*"]] }, { name: "All files", patterns: [[0, "*"]] }],
    currentFilter: 0,
  }) });
  await settle(page);
  await expect(page).toHaveTitle("Upload photos");
  await expect(page.locator("#tabbar")).toBeHidden();
  const pane = activePane(page);
  await expect(item(pane, "photo1.png")).toBeVisible();
  await expect(item(pane, "photo-copy.unknown")).toHaveCount(0); // not an image

  await page.locator(".picker-filter").selectOption({ label: "All files" });
  await expect(item(pane, "photo-copy.unknown")).toBeVisible();

  await item(pane, "photo1.png").click();
  await item(pane, "photo2.png").click({ modifiers: [process.platform === "darwin" ? "Meta" : "Control"] });
  await button(page, "Upload").click();
  await expect.poll(() => answers(page)).toEqual([{
    id: 1, ok: true, paths: [`${HOME}/Pictures/photo1.png`, `${HOME}/Pictures/photo2.png`], choices: [], filter: 1,
  }]);
});

test("Open: double-click chooses a file, and goes into folders", async ({ page }) => {
  await openApp(page, { picker: request({}) });
  await settle(page);
  const pane = activePane(page);
  await item(pane, "Documents").dblclick();
  await settle(page);
  await item(pane, "report.pdf").dblclick();
  await expect.poll(() => answers(page)).toEqual([
    { id: 1, ok: true, paths: [`${HOME}/Documents/report.pdf`], choices: [], filter: null },
  ]);
  // Choosing a file doesn't open it in an app, and the tabs aren't saved.
  expect(await mock(page, () => window.__mock.opened)).toEqual([]);
  expect(await page.evaluate(() => localStorage.getItem("sonar.session"))).toBeNull();
});

test("Open next time starts in the folder chosen from last time", async ({ page }) => {
  await openApp(page, { picker: request({}) });
  await settle(page);
  await item(activePane(page), "Documents").dblclick();
  await settle(page);
  await item(activePane(page), "notes.txt").dblclick();
  await expect.poll(async () => (await answers(page)).length).toBe(1);
  await page.reload();
  await settle(page);
  expect(await page.evaluate(() => window.sonar.pane.path)).toBe(`${HOME}/Documents`);
});

test("Save: suggested name, replacing asks first, a new name is joined to the folder", async ({ page }) => {
  await openApp(page, { picker: request({ save: true, folder: `${HOME}/Documents`, name: "notes.txt" }) });
  await settle(page);
  const name = bar(page).getByLabel("File name");
  await expect(name).toHaveValue("notes.txt");
  await expect(name).toBeFocused();

  await button(page, "Save").click();
  const dialog = page.locator("#dialog[open]");
  await expect(dialog.locator(".dialog-title")).toHaveText("Replace “notes.txt”?");
  await dialog.getByRole("button", { name: "Cancel" }).click();
  expect(await answers(page)).toEqual([]);

  await name.fill("new notes.txt");
  await name.press("Enter");
  await expect.poll(() => answers(page)).toEqual([
    { id: 1, ok: true, paths: [`${HOME}/Documents/new notes.txt`], choices: [], filter: null },
  ]);
});

test("Save: clicking a file takes its name; a folder's name goes into it", async ({ page }) => {
  await openApp(page, { picker: request({ save: true, name: "x.txt" }) });
  await settle(page);
  const name = bar(page).getByLabel("File name");
  await item(activePane(page), "readme.md").click();
  await expect(name).toHaveValue("readme.md");
  await name.fill("Music");
  await name.press("Enter");
  await expect.poll(() => page.evaluate(() => window.sonar.pane.path)).toBe(`${HOME}/Music`);
  expect(await answers(page)).toEqual([]);
});

test("Folders: files are hidden; the selection or the current folder is chosen", async ({ page }) => {
  await openApp(page, { picker: request({ directory: true }) });
  await settle(page);
  const pane = activePane(page);
  await expect(button(page, "Select Folder")).toBeVisible();
  await expect(item(pane, "readme.md")).toHaveCount(0);
  await item(pane, "Music").click();
  await button(page, "Select Folder").click();
  await expect.poll(() => answers(page)).toEqual([{ id: 1, ok: true, paths: [`${HOME}/Music`], choices: [], filter: null }]);
});

test("Extra options are answered; Cancel and Escape cancel", async ({ page }) => {
  await openApp(page, { picker: request({ choices: [
    { id: "ro", label: "Open read-only", options: [], selected: "false" },
    { id: "enc", label: "Encoding", options: [["utf8", "UTF-8"], ["latin1", "Latin-1"]], selected: "utf8" },
  ] }) });
  await settle(page);
  await bar(page).getByLabel("Open read-only").check();
  await bar(page).getByLabel("Encoding").selectOption("latin1");
  await item(activePane(page), "readme.md").dblclick();
  await expect.poll(async () => (await answers(page))[0]?.choices).toEqual([["ro", "true"], ["enc", "latin1"]]);

  await button(page, "Cancel").click();
  await activePane(page).locator(".scroller").focus();
  await page.keyboard.press("Escape"); // clears the selection
  await page.keyboard.press("Escape"); // nothing selected: cancels
  await expect.poll(async () => (await answers(page)).slice(1).map((a) => a.ok)).toEqual([false, false]);
});

test("Items inside archives can't be chosen", async ({ page }) => {
  await openApp(page, { picker: request({}) });
  await settle(page);
  await page.evaluate((p) => window.sonar.pane.navigate(p), `${HOME}/Downloads/photos.zip/photos`);
  await settle(page);
  await item(activePane(page), "readme.txt").dblclick();
  await expect(status(page)).toContainText("can't be chosen");
  expect(await answers(page)).toEqual([]);
});

test("a request that is gone closes the window", async ({ page }) => {
  // Not openApp: it waits for a folder, and none ever loads.
  await page.addInitScript({ path: "tests/ui/mock-backend.js" });
  await page.goto("/index.html?picker=99");
  await expect.poll(() => mock(page, () => window.__mock.windowCloses)).toBe(1);
});
