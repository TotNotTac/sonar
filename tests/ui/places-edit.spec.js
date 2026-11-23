import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, item, openApp, status } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

const placesSection = (page) =>
  page.evaluate(() => window.sonar.sidebarPlaces.filter((p) => p.section === "places").map((p) => p.label));
const place = (page, name) => page.locator(".place", { hasText: new RegExp(`^${name}$`) });
const sidebarMenu = async (page) => {
  const box = await page.locator("#sidebar").boundingBox();
  await page.mouse.click(box.x + 60, box.y + box.height - 20, { button: "right" });
};

test("add a folder from its context menu; it persists", async ({ page }) => {
  await item(activePane(page), "Documents").dblclick();
  await item(activePane(page), "Projects").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Add to Places" }).click();
  await expect(status(page)).toHaveText("Added “Projects” to Places");
  expect((await placesSection(page)).at(-1)).toBe("Projects");
  await place(page, "Projects").click();
  await expectPath(page, `${HOME}/Documents/Projects`);

  await page.reload();
  await page.waitForFunction(() => window.sonar?.pane?.view);
  expect(await placesSection(page)).toContain("Projects");
});

test("drag folders onto the sidebar to add them", async ({ page }) => {
  const from = await item(activePane(page), "Videos").locator(".name-label").boundingBox();
  const side = await page.locator("#sidebar").boundingBox();
  await page.mouse.move(from.x + 5, from.y + 5);
  await page.mouse.down();
  await page.mouse.move(from.x + 20, from.y + 10, { steps: 3 });
  await page.mouse.move(side.x + 60, side.y + side.height - 30, { steps: 8 });
  await expect(page.locator("#sidebar.drop-target")).toHaveCount(1);
  await page.mouse.up();
  // Videos is already a system place, so pick a folder that isn't:
  await expect(status(page)).toHaveText("Already in Places");

  await item(activePane(page), "Documents").dblclick();
  const p = await item(activePane(page), "Projects").locator(".name-label").boundingBox();
  await page.mouse.move(p.x + 5, p.y + 5);
  await page.mouse.down();
  await page.mouse.move(p.x + 20, p.y + 10, { steps: 3 });
  await page.mouse.move(side.x + 60, side.y + side.height - 30, { steps: 8 });
  await page.mouse.up();
  await expect(place(page, "Projects")).toBeVisible();
  await expect(page.locator(".menu")).toHaveCount(0); // no Move/Copy question
});

test("rename and hide places; Show Hidden and Reset bring them back", async ({ page }) => {
  await place(page, "Music").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Rename…" }).click();
  await page.locator(".place-rename").fill("Tunes");
  await page.keyboard.press("Enter");
  await expect(place(page, "Tunes")).toBeVisible();

  await place(page, "Videos").click({ button: "right" });
  await page.locator(".menu-item").filter({ has: page.locator(".menu-label", { hasText: /^Hide$/ }) }).click();
  await expect(place(page, "Videos")).toHaveCount(0);

  await sidebarMenu(page);
  await page.locator(".menu-item", { hasText: "Show Hidden Places (1)" }).click();
  await expect(place(page, "Videos")).toBeVisible();
  await expect(place(page, "Tunes")).toBeVisible();

  await sidebarMenu(page);
  await page.locator(".menu-item", { hasText: "Reset Places" }).click();
  await expect(place(page, "Music")).toBeVisible();
});

test("remove a custom place", async ({ page }) => {
  await activePane(page).locator(".scroller").click({ button: "right", position: { x: 400, y: 400 } });
  await page.locator(".menu-item", { hasText: "Add to Places" }).click(); // the current folder: Home is already there
  await expect(status(page)).toHaveText("Already in Places");
  await item(activePane(page), "Documents").dblclick();
  await activePane(page).locator(".scroller").click({ button: "right", position: { x: 400, y: 400 } });
  await page.locator(".menu-item", { hasText: "Add to Places" }).click();
  // "Documents" exists as a system place too; adding the same path is a no-op.
  await expect(status(page)).toHaveText("Already in Places");
  await item(activePane(page), "Projects").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Add to Places" }).click();
  await place(page, "Projects").click({ button: "right" });
  await page.locator(".menu-item", { hasText: "Remove from Places" }).click();
  await expect(place(page, "Projects")).toHaveCount(0);
});

test("drag places to reorder them", async ({ page }) => {
  expect((await placesSection(page)).slice(0, 3)).toEqual(["Home", "Desktop", "Documents"]);
  const from = await place(page, "Documents").boundingBox();
  const to = await place(page, "Home").boundingBox();
  await page.mouse.move(from.x + 20, from.y + from.height / 2);
  await page.mouse.down();
  await page.mouse.move(from.x + 20, from.y - 10, { steps: 3 });
  await page.mouse.move(to.x + 20, to.y + 3, { steps: 6 });
  await expect(place(page, "Home")).toHaveClass(/drop-before/);
  await page.mouse.up();
  expect((await placesSection(page)).slice(0, 3)).toEqual(["Documents", "Home", "Desktop"]);
  await expectPath(page, HOME); // the drag didn't navigate

  await page.reload();
  await page.waitForFunction(() => window.sonar?.pane?.view);
  expect((await placesSection(page))[0]).toBe("Documents");
});
