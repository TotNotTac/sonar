import { expect, test } from "@playwright/test";
import { activePane, HOME, isMac, item, keys, mock, openApp, settle, tabLabels } from "./helpers.js";

const openSettings = async (page) => {
  await page.keyboard.press(isMac ? "Meta+Comma" : "Control+Shift+Comma");
  const dlg = page.locator("dialog.settings[open]");
  await expect(dlg).toBeVisible();
  return dlg;
};
const viewOf = (page) => page.evaluate(() => window.sonar.pane.props);
const seedSettings = (page, s) => page.addInitScript((v) => localStorage.setItem("sonar.settings", JSON.stringify(v)), s);

test.describe("with the app open", () => {
  test.beforeEach(({ page }) => openApp(page));

  test("the default view applies to folders without their own", async ({ page }) => {
    const pane = activePane(page);
    await item(pane, "Documents").dblclick();
    await settle(page);
    await page.keyboard.press(keys.icons); // Documents remembers icons
    await page.keyboard.press(keys.back);
    await settle(page);

    const dlg = await openSettings(page);
    await dlg.getByLabel("Default view").selectOption("icons");
    await dlg.getByLabel("Sort by").selectOption("size");
    await expect.poll(() => viewOf(page)).toEqual({ viewMode: "icons", sort: { key: "size", order: "ascending" } });
    await dlg.getByLabel("Default view").selectOption("details");
    await dlg.getByRole("button", { name: "Close" }).click();
    await expect(dlg).toBeHidden();
    await expect(page.locator(".pane.active table.details")).toBeVisible();

    await item(pane, "Documents").dblclick();
    await settle(page);
    expect((await viewOf(page)).viewMode).toBe("icons");
  });

  test("without per-folder views, a change applies everywhere", async ({ page }) => {
    const dlg = await openSettings(page);
    await dlg.getByLabel("Remember the view for each folder").uncheck();
    await page.keyboard.press("Escape");
    await expect(dlg).toBeHidden();

    await page.keyboard.press("F3"); // split: both panes show Home
    await page.keyboard.press(keys.icons);
    await expect(page.locator(".pane .grid")).toHaveCount(2);
    await item(activePane(page), "Music").dblclick();
    await settle(page);
    expect((await viewOf(page)).viewMode).toBe("icons");
  });

  test("hidden files can be toggled from Settings", async ({ page }) => {
    const dlg = await openSettings(page);
    await dlg.getByLabel("Show hidden files").check();
    await expect(item(activePane(page), ".config")).toBeVisible();
    await expect(page.locator("#btn-hidden")).toHaveAttribute("aria-pressed", "true");
  });

  test("confirmations follow the settings", async ({ page }) => {
    const dlg = await openSettings(page);
    await dlg.getByLabel("Moving items to the Trash").check();
    await dlg.getByLabel("Deleting items permanently").uncheck();
    await dlg.getByRole("button", { name: "Close" }).click();

    const pane = activePane(page);
    await item(pane, "Music").click();
    await page.keyboard.press(keys.trash);
    const ask = page.locator("#dialog[open]");
    await expect(ask.locator(".dialog-title")).toHaveText("Move “Music” to the Trash?");
    await ask.getByRole("button", { name: "Cancel" }).click();
    await expect(item(pane, "Music")).toBeVisible();

    await item(pane, "Videos").click();
    await page.keyboard.press(keys.deletePermanently);
    await expect(item(pane, "Videos")).toHaveCount(0);
    expect(await mock(page, () => window.__mock.callsTo("delete_entries").length)).toBe(1);
  });

  test("thumbnails can be turned off", async ({ page }) => {
    await item(activePane(page), "Pictures").dblclick();
    await settle(page);
    await page.keyboard.press(keys.icons);
    // The test browsers have no sonar: scheme, so failed images are removed: read them as rendered.
    const thumbs = () => page.evaluate(() => {
      window.sonar.pane.render();
      return window.sonar.pane.scroller.querySelectorAll("img.thumb").length;
    });
    expect(await thumbs()).toBeGreaterThan(0);
    const dlg = await openSettings(page);
    await dlg.getByLabel("Show image thumbnails").uncheck();
    expect(await thumbs()).toBe(0);
    await dlg.getByLabel("Show image thumbnails").check();
    expect(await thumbs()).toBeGreaterThan(0);
  });

  test("Restore Defaults resets everything", async ({ page }) => {
    const dlg = await openSettings(page);
    await dlg.getByLabel("Default view").selectOption("icons");
    await dlg.getByLabel("Show hidden files").check();
    await dlg.getByRole("button", { name: "Restore Defaults" }).click();
    await expect(dlg.getByLabel("Default view")).toHaveValue("details");
    await expect(dlg.getByLabel("Show hidden files")).not.toBeChecked();
    expect(await page.evaluate(() => localStorage.getItem("sonar.settings"))).toBe("{}");
  });
});

test("startup can ignore the saved session", async ({ page }) => {
  await seedSettings(page, { startup: "home" });
  await openApp(page, { session: { tabs: [{ paths: [`${HOME}/Music`] }, { paths: [`${HOME}/Videos`] }], active: 1 } });
  await expect(tabLabels(page)).toHaveText(["Home"]);
});

test("startup can open a chosen folder", async ({ page }) => {
  await seedSettings(page, { startup: "folder", startupFolder: `${HOME}/Downloads` });
  await openApp(page);
  await expect(tabLabels(page)).toHaveText(["Downloads"]);
});
