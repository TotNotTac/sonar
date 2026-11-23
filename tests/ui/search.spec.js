import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, item, keys, labels, mock, openApp, status } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

const bar = (page) => activePane(page).locator(".searchbar");

async function search(page, query) {
  await page.keyboard.press(keys.search);
  await expect(bar(page).locator("input")).toBeFocused();
  await bar(page).locator("input").fill(query);
  await expect(bar(page).locator(".search-status")).toHaveText(/result/);
}

test("search finds matches in subfolders, with their location", async ({ page }) => {
  await page.keyboard.press(keys.search);
  await expect(bar(page).locator("input")).toHaveAttribute("placeholder", "Search in “Home”…");
  await bar(page).locator("input").fill("photo1");
  await expect(bar(page).locator(".search-status")).toHaveText("4 results");
  const pane = activePane(page);
  await expect(labels(pane)).toHaveText(["photo1.png", "photo10.png", "photo11.png", "photo12.png"]);
  await expect(pane.locator('th[data-key="type"]')).toHaveText("Location");
  await expect(item(pane, "photo1.png").locator("td.location")).toHaveText("Pictures");
  await expect(status(page)).toHaveText("4 results for “photo1”");
  await expect(page.locator("#status-right")).toHaveText("in “Home”");
});

test("best matches first: shallower and tighter matches rank higher", async ({ page }) => {
  await search(page, "notes");
  await expect(labels(activePane(page)).first()).toHaveText("notes.txt");
  await search(page, "read");
  await expect(labels(activePane(page)).first()).toHaveText("readme.md");
});

test("results stream in while searching", async ({ page }) => {
  await mock(page, () => { window.__mock.searchDelay = 400; });
  await page.keyboard.press(keys.search);
  await bar(page).locator("input").fill("photo");
  await expect(bar(page).locator(".search-status")).toHaveText(/^Searching…/);
  await expect(status(page)).toContainText("Searching…");
  await expect(bar(page).locator(".search-status")).toHaveText("14 results", { timeout: 5000 }); // 13 photos + photos.zip;
});

test("Escape closes search and shows the folder again", async ({ page }) => {
  await search(page, "photo");
  await bar(page).locator("input").press("Escape");
  await expect(bar(page)).toBeHidden();
  await expect(labels(activePane(page))).toHaveCount(8);
  await expect(activePane(page).locator('th[data-key="type"]')).toHaveText("Type");
});

test("opening a folder result leaves search", async ({ page }) => {
  await search(page, "proj");
  await item(activePane(page), "Projects").dblclick();
  await expectPath(page, `${HOME}/Documents/Projects`);
  await expect(bar(page)).toBeHidden();
  await expect(labels(activePane(page))).toHaveText(["sonar"]);
});

test("Open Containing Folder selects the result there", async ({ page }) => {
  await search(page, "photo3");
  await item(activePane(page), "photo3.png").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Open Containing Folder" }).click();
  await expectPath(page, `${HOME}/Pictures`);
  await expect(item(activePane(page), "photo3.png")).toHaveClass(/selected/);
});

test("actions on results re-run the search", async ({ page }) => {
  await search(page, "report");
  const pane = activePane(page);
  await item(pane, "report.pdf").click();
  await page.keyboard.press("F2");
  await pane.locator(".rename-input").fill("report-final.pdf");
  await page.keyboard.press("Enter");
  await expect(labels(pane)).toHaveText(["report-final.pdf"]);
  await expect(bar(page)).toBeVisible();
  expect(await mock(page, (p) => window.__mock.exists(p), `${HOME}/Documents/report-final.pdf`)).toBe(true);
});

test("sorting results by a column, and hidden files", async ({ page }) => {
  await search(page, "photo1");
  const pane = activePane(page);
  await pane.locator('th[data-key="name"]').click();
  await expect(labels(pane)).toHaveText(["photo1.png", "photo10.png", "photo11.png", "photo12.png"]);
  await pane.locator('th[data-key="name"]').click();
  await expect(labels(pane)).toHaveText(["photo12.png", "photo11.png", "photo10.png", "photo1.png"]);

  await search(page, "bash");
  await expect(bar(page).locator(".search-status")).toHaveText("0 results");
  await bar(page).locator("input").press("Escape");
  await page.keyboard.press(keys.hidden);
  await search(page, "bash");
  await expect(labels(pane)).toHaveText([".bashrc"]);
});

test("search isn't offered inside archives", async ({ page }) => {
  await page.locator(".place", { hasText: "Downloads" }).click();
  await item(activePane(page), "photos.zip").dblclick();
  await page.keyboard.press(keys.search);
  await expect(bar(page)).toBeHidden();
  await expect(status(page)).toContainText("Search isn't available inside archives");
});
