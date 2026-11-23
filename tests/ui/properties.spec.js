import { expect, test } from "@playwright/test";
import { activePane, HOME, item, keys, mock, openApp } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

const dialog = (page) => page.locator("dialog.props[open]");
/** The dialog's rows as { label: value }. */
async function rows(page) {
  const dts = await dialog(page).locator("dt").allTextContents();
  const dds = await dialog(page).locator("dd").allTextContents();
  return Object.fromEntries(dts.map((t, i) => [t, dds[i]]));
}

test("a file's details", async ({ page }) => {
  await item(activePane(page), "readme.md").click();
  await page.keyboard.press(keys.properties);
  await expect(dialog(page).locator(".dialog-title")).toHaveText("readme.md");
  const r = await rows(page);
  expect(r).toMatchObject({
    Type: "MD text", Location: HOME, Size: "2.0 KiB (2,048 bytes)", "Size on disk": "4.0 KiB (4,096 bytes)",
    Permissions: "rw-r--r-- (644)", Owner: "user, group staff",
  });
  expect(r.Modified).toBeTruthy();
  await page.keyboard.press("Escape");
  await expect(dialog(page)).toHaveCount(0);
});

test("a folder's size and contents are counted live", async ({ page }) => {
  await mock(page, () => { window.__mock.searchDelay = 300; });
  await item(activePane(page), "Documents").click();
  await page.keyboard.press(keys.properties);
  await expect(dialog(page).locator("dd").nth(2)).toHaveText(/…$/); // still counting
  await expect.poll(async () => (await rows(page)).Size).toBe("122 KiB (124,900 bytes)");
  expect((await rows(page)).Contains).toBe("3 files, 2 folders");
});

test("several items: combined totals", async ({ page }) => {
  const pane = activePane(page);
  await item(pane, "archive.zip").click();
  await item(pane, "readme.md").click({ modifiers: ["Shift"] });
  await page.keyboard.press(keys.properties);
  await expect(dialog(page).locator(".dialog-title")).toHaveText("2 items");
  await expect.poll(async () => (await rows(page)).Contains).toBe("2 files, 0 folders");
  const r = await rows(page);
  expect(r.Type).toBe("Various");
  expect(r.Location).toBe(HOME);
});

test("nothing selected: the current folder; closing stops the count", async ({ page }) => {
  await mock(page, () => { window.__mock.searchDelay = 2000; });
  await activePane(page).locator(".scroller").click({ position: { x: 400, y: 400 } });
  await activePane(page).locator(".scroller").click({ button: "right", position: { x: 400, y: 400 } });
  await page.locator(".menu-item", { hasText: "Properties" }).click();
  await expect(dialog(page).locator(".dialog-title")).toHaveText("user");
  await dialog(page).getByRole("button", { name: "Close" }).click();
  const [started, cancelled] = await mock(page, () => [window.__mock.sizeJobs, window.__mock.cancelledSizes]);
  expect(cancelled).toEqual(started);
});

test("items inside an archive", async ({ page }) => {
  await page.locator(".place", { hasText: "Downloads" }).click();
  await item(activePane(page), "photos.zip").dblclick();
  await item(activePane(page), "photos").dblclick();
  await item(activePane(page), "readme.txt").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Properties" }).click();
  const r = await rows(page);
  expect(r).toMatchObject({ Size: "12 B", "In archive": "photos.zip", Location: `${HOME}/Downloads/photos.zip/photos` });
});
