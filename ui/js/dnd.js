// Drag and drop.
//
// - Inside the window: pointer events (no HTML5 drag API, which Tauri's
//   native file-drop handling interferes with). Targets: folders in any pane,
//   a pane's empty space (= its folder), sidebar places, breadcrumbs, and
//   tabs (hovering a tab switches to it).
// - Leaving the window hands the drag to the OS (`start_drag_out`), so files
//   can be dropped on Finder, Dolphin, mail, ...
// - Files dropped from other apps arrive as Tauri's native drag-drop events.
//
// Dropping without a modifier asks Move / Copy, like Dolphin; Shift moves,
// Ctrl (⌥ on macOS) copies.

import { Icons } from "./icons.js";
import { showMenu } from "./menu.js";
import { el, errMsg, invoke, isMac, listen, parentOf, plural } from "./util.js";

const THRESHOLD = 5;
const TAB_HOVER_MS = 600;
const EDGE = 28;

export class DragDrop {
  constructor(app) {
    this.app = app;
    this.pending = null;   // pointer pressed on an item, not yet dragging
    this.drag = null;      // { pane, paths }
    this.target = null;    // { path, el }
    this.external = null;  // paths of a drag coming from another app
    this.tabTimer = 0;
    window.addEventListener("pointermove", (ev) => this.onMove(ev));
    window.addEventListener("pointerup", (ev) => this.onUp(ev));
    window.addEventListener("keydown", (ev) => {
      if (this.drag && ev.key === "Escape") {
        ev.preventDefault();
        ev.stopPropagation();
        this.end();
      }
    }, true);
    window.addEventListener("blur", () => this.end());
    this.listenExternal();
  }

  get active() { return !!this.drag; }

  /** A pane calls this on mousedown over an item: maybe the start of a drag. */
  press(ev, pane) {
    this.pending = { x: ev.clientX, y: ev.clientY, pane };
  }

  onMove(ev) {
    if (this.pending && !this.drag) {
      if (Math.hypot(ev.clientX - this.pending.x, ev.clientY - this.pending.y) < THRESHOLD) return;
      this.start(this.pending.pane, ev);
    }
    if (!this.drag) return;
    const { clientX: x, clientY: y } = ev;
    if (x < 0 || y < 0 || x >= window.innerWidth || y >= window.innerHeight) return this.dragOut();
    this.ghost.style.transform = `translate(${x + 14}px, ${y + 10}px)`;
    this.hover(this.targetAt(x, y, this.drag.paths));
    this.autoScroll(x, y);
  }

  start(pane, ev) {
    this.pending = null;
    const items = pane.selectedEntries();
    if (!items.length) return;
    pane.pendingCollapse = null; // keep the multi-selection being dragged
    this.drag = { pane, paths: items.map((e) => e.path), folders: !pane.view?.archive && items.every((e) => pane.isFolder(e)) };
    const label = items.length === 1 ? items[0].name : plural(items.length, "item");
    this.ghost = el("div", "drag-ghost", `${Icons.entry(items.length === 1 ? items[0].icon : "file")}<span></span>`);
    this.ghost.querySelector("span").textContent = label;
    this.ghost.style.transform = `translate(${ev.clientX + 14}px, ${ev.clientY + 10}px)`;
    document.body.append(this.ghost);
    document.body.classList.add("dragging-items");
  }

  onUp(ev) {
    this.pending = null;
    if (!this.drag) return;
    const target = this.target;
    const paths = this.drag.paths;
    this.end();
    if (target?.addPlace) return this.app.addPlaces(paths);
    if (target) this.drop(paths, target.path, ev, ev.clientX, ev.clientY);
  }

  end() {
    this.pending = null;
    this.drag = null;
    this.hover(null);
    this.ghost?.remove();
    this.ghost = null;
    document.body.classList.remove("dragging-items");
  }

  /** The pointer left the window: continue as a native drag other apps understand. */
  dragOut() {
    const paths = this.drag.paths;
    const archive = this.drag.pane.view?.archive;
    this.end();
    if (archive) {
      this.app.flashStatus("Items inside an archive can't be dragged to other apps — extract them first");
      return;
    }
    invoke("start_drag_out", { paths }).catch((err) => this.app.pane.showMessage(errMsg(err)));
  }

  /** Drop `paths` into `dest`: Shift moves, Ctrl/⌥ copies, otherwise ask. */
  drop(paths, dest, mods, x, y) {
    const ops = this.app.ops;
    const move = () => ops.startJob("move", paths, dest);
    const copy = () => ops.startJob("copy", paths, dest);
    if (mods.shiftKey) return move();
    if (isMac ? mods.altKey : mods.ctrlKey) return copy();
    showMenu(x, y, [
      { label: "Move Here", shortcut: "Shift", action: move },
      { label: "Copy Here", shortcut: isMac ? "⌥" : "Ctrl", action: copy },
      "-",
      { label: "Cancel", shortcut: "Esc", action: () => {} },
    ]);
  }

  /** What a drop at (x, y) would land on, or null if nothing useful. */
  targetAt(x, y, paths) {
    const node = document.elementFromPoint(x, y);
    if (!node) return null;
    let path = null;
    let target = null;
    let tab = null;
    const tabEl = node.closest(".tab");
    const place = node.closest(".place:not(.unmounted)");
    const crumb = node.closest(".crumb");
    const paneEl = node.closest(".pane");
    // Folders dropped on empty sidebar space become new Places.
    if (!place && !crumb && node.closest(".sidebar")) {
      return this.drag?.folders ? { addPlace: true, path: "(add place)", el: node.closest(".sidebar") } : null;
    }
    if (tabEl) {
      tab = this.app.tabs.find((t) => String(t.id) === tabEl.dataset.tab);
      const p = tab?.panes[tab.active];
      path = p?.path;
      target = tabEl;
    } else if (place || crumb) {
      target = place ?? crumb;
      path = target.dataset.path;
    } else if (paneEl) {
      const pane = this.app.findPane(paneEl.dataset.pane);
      if (pane?.view?.archive) return tab ? { tab } : null; // read-only
      const itemEl = node.closest("[data-i]");
      const entry = itemEl && pane?.items[Number(itemEl.dataset.i)];
      if (entry && pane.isFolder(entry) && !paths.includes(entry.path)) {
        path = entry.path;
        target = itemEl;
      } else if (pane?.path) {
        path = pane.path;
        target = pane.scroller;
      }
    }
    if (!path) return tab ? { tab } : null;
    // Pointless or impossible: dropping into the folder they're already in,
    // or a folder into itself / one of its subfolders.
    const intoSameFolder = paths.every((p) => parentOf(p) === path);
    const intoItself = paths.some((p) => path === p || path.startsWith(`${p}/`));
    if (intoSameFolder || intoItself) return tab ? { tab } : null;
    return { path, el: target, tab };
  }

  hover(target) {
    if (this.target?.el !== target?.el) {
      this.target?.el?.classList.remove("drop-target");
      target?.el?.classList.add("drop-target");
    }
    // Hovering a tab for a moment switches to it, so the drag can continue there.
    const tab = target?.tab ?? null;
    if (tab !== this.target?.tab) {
      clearTimeout(this.tabTimer);
      if (tab && tab !== this.app.activeTab) {
        this.tabTimer = setTimeout(() => this.app.activateTab(tab), TAB_HOVER_MS);
      }
    }
    this.target = target?.path ? target : target?.tab ? { tab: target.tab } : null;
  }

  autoScroll(x, y) {
    const scroller = document.elementFromPoint(x, y)?.closest(".scroller");
    if (!scroller) return;
    const r = scroller.getBoundingClientRect();
    if (y < r.top + EDGE) scroller.scrollTop -= 12;
    else if (y > r.bottom - EDGE) scroller.scrollTop += 12;
  }

  // ---------- Drops from other apps (and our own drag-out coming back) ----------

  listenExternal() {
    const css = (pos) => ({ x: pos.x / window.devicePixelRatio, y: pos.y / window.devicePixelRatio });
    const over = (pos) => {
      if (!this.external || !pos) return;
      const { x, y } = css(pos);
      this.hover(this.targetAt(x, y, this.external));
    };
    listen("tauri://drag-enter", ({ payload }) => {
      this.external = payload.paths ?? [];
      over(payload.position);
    });
    listen("tauri://drag-over", ({ payload }) => over(payload.position));
    listen("tauri://drag-leave", () => {
      this.external = null;
      this.hover(null);
    });
    listen("tauri://drag-drop", ({ payload }) => {
      const paths = payload.paths?.length ? payload.paths : this.external ?? [];
      this.external = null;
      this.hover(null);
      if (!paths.length || !payload.position) return;
      const { x, y } = css(payload.position);
      const target = this.targetAt(x, y, paths);
      if (target?.path) this.drop(paths, target.path, {}, x, y);
    });
  }
}
