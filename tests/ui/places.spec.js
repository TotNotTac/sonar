import { expect, test } from "@playwright/test";
import { activePane, expectPath, HOME, item, labels, message, mock, openApp, status } from "./helpers.js";

test.beforeEach(({ page }) => openApp(page));

const place = (page, name) => page.locator(".place", { hasText: name });

test("devices show how full they are", async ({ page }) => {
  const root = place(page, "Root");
  await expect(root.locator(".usage")).toHaveClass(/full/); // 92% used
  await expect(root).toHaveAttribute("title", "Root: 37 GiB free of 466 GiB");
  const width = await root.locator(".usage > div").evaluate((d) => d.style.width);
  expect(parseFloat(width)).toBeCloseTo(92, 1); // browsers normalise "92.0%" to "92%"
  await expect(place(page, "USB").locator(".usage")).not.toHaveClass(/full/);
  await expect(place(page, "Home").locator(".usage")).toHaveCount(0);
});

test("eject a USB stick, leaving it first if it's open", async ({ page }) => {
  await place(page, "USB").click();
  await expectPath(page, "/media/user/USB");
  await expect(labels(activePane(page))).toHaveText(["slides.pdf"]);
  await place(page, "USB").locator(".place-eject").click();
  await expectPath(page, HOME);
  await expect(status(page)).toHaveText("“USB” can be removed safely");
  await expect(place(page, "USB")).toHaveCount(0);
});

test("eject failures are reported", async ({ page }) => {
  await mock(page, () => window.__mock.busy.add("/media/user/USB"));
  await place(page, "USB").locator(".place-eject").click();
  await expect(message(page)).toContainText("target is busy");
  await expect(place(page, "USB")).toHaveCount(1);
});

test("an attached, unmounted drive mounts when clicked", async ({ page }) => {
  const backup = place(page, "Backup");
  await expect(backup).toHaveClass(/unmounted/);
  await expect(backup.locator(".place-eject")).toHaveCount(0);
  await backup.click();
  await expectPath(page, "/media/user/Backup");
  await expect(item(activePane(page), "backup-2026.tar")).toBeVisible();
  await expect(place(page, "Backup")).not.toHaveClass(/unmounted/);
  await expect(place(page, "Backup").locator(".place-eject")).toBeVisible();
});

test("drives plugged in later appear without a reload", async ({ page }) => {
  await mock(page, () => window.__mock.attach("Camera", "/dev/sdd1"));
  await expect(place(page, "Camera")).toHaveClass(/unmounted/);
  await place(page, "Camera").click({ button: "right" });
  await expect(page.locator(".menu .menu-label")).toHaveText(["Mount"]);
});

test("device context menu offers Eject", async ({ page }) => {
  await place(page, "USB").click({ button: "right" });
  await expect(page.locator(".menu .menu-label")).toHaveText(["Open", "Open in New Tab", "Open in Split View", "Eject"]);
  await page.locator(".menu-item", { hasText: "Eject" }).click();
  await expect(place(page, "USB")).toHaveCount(0);
});

test("OS icons and thumbnails are requested, with the built-in icons as fallback", async ({ page }) => {
  const html = await page.evaluate(() => {
    const pane = window.sonar.pane;
    pane.render();
    return pane.scroller.innerHTML;
  });
  expect(html).toContain('src="sonar://localhost/icon?key=folder&amp;size=18"');
  expect(html).toContain("sonar://localhost/icon?key=archive-x-generic%2Cunknown&amp;size=18");

  // The test browsers have no sonar: scheme, so every image fails and the SVGs stay.
  await expect(activePane(page).locator("img.os-icon")).toHaveCount(0);
  await expect(item(activePane(page), "readme.md").locator(".ficon > svg")).toBeVisible();

  await place(page, "Pictures").click();
  await page.keyboard.press(process.platform === "darwin" ? "Meta+1" : "Control+1");
  const thumbs = await page.evaluate(() => {
    const pane = window.sonar.pane;
    pane.render();
    return [...pane.scroller.querySelectorAll("img.thumb")].map((i) => decodeURIComponent(i.getAttribute("src")));
  });
  expect(thumbs).toContain("sonar://localhost/thumb?path=/home/user/Pictures/photo1.png&size=96");
  expect(thumbs.some((t) => t.includes("photo-copy.unknown"))).toBe(false);
});

test("loaded OS icons and thumbnails stay inside their icon box", async ({ page }) => {
  await place(page, "Pictures").click();
  await page.keyboard.press(process.platform === "darwin" ? "Meta+1" : "Control+1");
  // Stand in for the sonar: scheme with a real (large) image, as the app would load.
  await page.evaluate(() => {
    const c = Object.assign(document.createElement("canvas"), { width: 400, height: 300 });
    c.getContext("2d").fillRect(0, 0, 400, 300);
    const png = c.toDataURL();
    const pane = window.sonar.pane;
    pane.render();
    for (const img of pane.scroller.querySelectorAll("img.os-icon")) img.src = png;
  });
  const tiles = activePane(page).locator(".grid > [data-i]");
  await expect(activePane(page).locator(".ficon.has-os-icon")).toHaveCount(await tiles.count());
  for (const i of [0, 5]) {
    const box = await tiles.nth(i).locator(".ficon").boundingBox();
    const img = await tiles.nth(i).locator("img.os-icon").boundingBox();
    expect(img.width).toBeLessThanOrEqual(48.5);
    expect(img.height).toBeLessThanOrEqual(48.5);
    expect(img.x).toBeGreaterThanOrEqual(box.x - 0.5);
    expect(img.y).toBeGreaterThanOrEqual(box.y - 0.5);
  }
  // And in details view, at 18px.
  await page.keyboard.press(process.platform === "darwin" ? "Meta+2" : "Control+3");
  await page.evaluate(() => {
    const pane = window.sonar.pane;
    for (const img of pane.scroller.querySelectorAll("img.os-icon")) img.src = document.querySelector("canvas")?.toDataURL() ?? img.src;
  });
  const row = await activePane(page).locator("tbody tr").first().boundingBox();
  expect(row.height).toBeLessThan(30);
});
