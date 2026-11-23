// One folder view. A tab shows one Pane, or two side by side in split view.

import { Fmt } from "./format.js";
import { Icons } from "./icons.js";
import { saveViewProps, viewPropsFor } from "./store.js";
import { childNameOnPath, el, errMsg, invoke, isMac, isMod, plural } from "./util.js";
import { VIEWS } from "./views.js";

let nextId = 1;

export class Pane {
  constructor(app) {
    this.app = app;
    this.id = `pane-${nextId++}`;
    this.path = null;
    this.view = null;        // last DirView from the backend
    this.items = [];         // entries currently shown (after the filter)
    this.byName = new Map(); // name -> index into items
    this.history = [];
    this.historyIndex = -1;
    this.selected = new Set(); // indices into items
    this.anchor = -1;
    this.focus = -1;
    this.props = viewPropsFor("");
    this.filter = "";
    this.renaming = null;
    this.requestId = 0;
    this.refreshing = false;
    this.refreshQueued = false;
    this.closed = false;
    this.typeAhead = "";
    this.typeAheadTimer = 0;
    this.buildDom();
  }

  // ---------- DOM ----------

  buildDom() {
    this.root = el("section", "pane");
    this.root.dataset.pane = this.id;
    this.header = el("div", "pane-header");
    this.message = el("div", "message", `<span class="message-text"></span><button class="message-close" title="Dismiss">×</button>`);
    this.message.hidden = true;
    this.message.setAttribute("role", "alert");
    this.message.querySelector(".message-close").addEventListener("click", () => this.hideMessage());
    this.scroller = el("div", "scroller");
    this.scroller.tabIndex = 0;
    this.empty = el("div", "empty");
    this.empty.hidden = true;
    this.scroller.append(this.empty);
    this.filterBar = el("div", "filterbar",
      `${Icons.ui("filter")}<input type="text" placeholder="Filter…" spellcheck="false" autocomplete="off" />
       <span class="filter-count"></span><button class="tool filter-close" title="Close filter (Esc)">${Icons.ui("close")}</button>`);
    this.filterBar.hidden = true;
    this.filterInput = this.filterBar.querySelector("input");
    this.searchBar = el("div", "searchbar",
      `${Icons.ui("search")}<input type="text" spellcheck="false" autocomplete="off" />
       <span class="search-status"></span><button class="tool search-close" title="Close search (Esc)">${Icons.ui("close")}</button>`);
    this.searchBar.hidden = true;
    this.searchInput = this.searchBar.querySelector("input");
    this.root.append(this.header, this.searchBar, this.message, this.scroller, this.filterBar);

    this.searchInput.addEventListener("input", () => {
      clearTimeout(this.searchTimer);
      this.searchTimer = setTimeout(() => this.runSearch(this.searchInput.value), 200);
    });
    this.searchInput.addEventListener("keydown", (ev) => {
      ev.stopPropagation();
      if (ev.key === "Escape") { ev.preventDefault(); this.hideSearch(); }
      if (ev.key === "Enter" || ev.key === "ArrowDown") {
        ev.preventDefault();
        if (this.items.length && this.focus < 0) this.setFocus(0);
        this.scroller.focus();
      }
    });
    this.searchBar.querySelector(".search-close").addEventListener("click", () => this.hideSearch());

    // Any interaction makes this the active pane.
    this.root.addEventListener("mousedown", () => this.app.activate(this), true);
    this.root.addEventListener("focusin", () => this.app.activate(this));
    this.header.addEventListener("click", () => this.scroller.focus());

    this.scroller.addEventListener("mousedown", (ev) => this.onMouseDown(ev));
    // load/error don't bubble, so listen in the capture phase. A loaded OS
    // icon hides the built-in SVG beneath it; a failed one is dropped.
    this.scroller.addEventListener("load", (ev) => {
      if (ev.target.classList?.contains("os-icon")) ev.target.parentElement.classList.add("has-os-icon");
    }, true);
    this.scroller.addEventListener("error", (ev) => {
      if (ev.target.classList?.contains("os-icon")) ev.target.remove();
    }, true);
    window.addEventListener("mouseup", () => this.onMouseUp());
    this.scroller.addEventListener("dblclick", (ev) => {
      const i = this.indexAt(ev.target);
      if (i != null) this.openEntry(i);
    });
    this.scroller.addEventListener("auxclick", (ev) => {
      // Middle-click a folder: open it in a new background tab (Dolphin/browser style).
      const i = this.indexAt(ev.target);
      if (ev.button === 1 && i != null && this.isFolder(this.items[i])) {
        ev.preventDefault();
        this.app.newTab(this.items[i].path, { activate: false });
      }
    });
    this.scroller.addEventListener("contextmenu", (ev) => {
      ev.preventDefault();
      const i = this.indexAt(ev.target);
      if (i != null && !this.selected.has(i)) this.setFocus(i);
      if (i == null) this.clearSelection();
      this.app.contextMenu(ev.clientX, ev.clientY, i == null ? { kind: "background", pane: this } : { kind: "items", pane: this });
    });

    this.filterInput.addEventListener("input", () => this.setFilter(this.filterInput.value));
    this.filterInput.addEventListener("keydown", (ev) => {
      ev.stopPropagation();
      if (ev.key === "Escape") { ev.preventDefault(); this.hideFilter(); }
      if (ev.key === "Enter" || ev.key === "ArrowDown") {
        ev.preventDefault();
        if (this.items.length && this.focus < 0) this.setFocus(0);
        this.scroller.focus();
      }
    });
    this.filterBar.querySelector(".filter-close").addEventListener("click", () => this.hideFilter());
  }

  mountView() {
    this.viewImpl?.dispose();
    const View = VIEWS[this.props.viewMode] ?? VIEWS.details;
    this.viewImpl = new View(this, this.scroller);
    this.scroller.dataset.view = this.props.viewMode;
  }

  indexAt(target) {
    if (target.closest(".rename-input")) return null;
    const node = target.closest("[data-i]");
    return node ? Number(node.dataset.i) : null;
  }

  /**
   * Press on an item's icon or name (any part of a tile in icons view): select
   * it, and maybe start dragging it. Press anywhere else (empty space, or a
   * row outside its name): select the row, and maybe draw a rubber band.
   */
  onMouseDown(ev) {
    if (ev.button !== 0 || ev.target.closest(".rename-input") || ev.target.closest("thead")) return;
    const i = this.indexAt(ev.target);
    const mod = isMod(ev);
    const onHandle = i != null && (this.props.viewMode === "icons" || ev.target.closest(".ficon, .name-label"));
    if (onHandle) {
      // Keep an existing multi-selection so it can be dragged as a whole;
      // a plain click (no drag) collapses it on release.
      if (this.selected.has(i) && !mod && !ev.shiftKey) {
        this.pendingCollapse = i;
        this.focus = i;
        this.renderSelection();
      } else {
        this.setFocus(i, { extend: ev.shiftKey, toggle: mod });
      }
      this.app.dnd.press(ev, this);
      return;
    }
    if (mod || ev.shiftKey) {
      // With modifiers the band combines with the selection as it was before
      // this press, so the click itself only applies if no band gets drawn.
      this.startRubberBand(ev, () => { if (i != null) this.setFocus(i, { extend: ev.shiftKey, toggle: mod }); });
      return;
    }
    if (i != null) this.setFocus(i);
    else this.clearSelection();
    this.startRubberBand(ev);
  }

  onMouseUp() {
    if (this.pendingCollapse == null) return;
    const i = this.pendingCollapse;
    this.pendingCollapse = null;
    this.setFocus(i);
  }

  /** Drag a selection rectangle; Ctrl/⌘ toggles against, Shift adds to, the existing selection. */
  startRubberBand(ev, onClick = null) {
    const scroller = this.scroller;
    const box = scroller.getBoundingClientRect();
    const toContent = (x, y) => ({ x: x - box.left + scroller.scrollLeft, y: y - box.top + scroller.scrollTop });
    const origin = toContent(ev.clientX, ev.clientY);
    const toggle = isMod(ev);
    const base = toggle || ev.shiftKey ? new Set(this.selected) : new Set();
    let pointer = { x: ev.clientX, y: ev.clientY };
    let band = null;
    let rects = null;

    const update = () => {
      const p = toContent(pointer.x, pointer.y);
      if (!band) {
        if (Math.hypot(p.x - origin.x, p.y - origin.y) < 5) return;
        band = el("div", "rubberband");
        scroller.append(band);
        // Item positions in scroll-content coordinates, measured once.
        rects = this.items.map((_, i) => {
          const r = this.viewImpl.item(i).getBoundingClientRect();
          const a = toContent(r.left, r.top);
          return { left: a.x, top: a.y, right: a.x + r.width, bottom: a.y + r.height };
        });
      }
      const r = {
        left: Math.min(origin.x, p.x), top: Math.min(origin.y, p.y),
        right: Math.max(origin.x, p.x), bottom: Math.max(origin.y, p.y),
      };
      Object.assign(band.style, {
        left: `${r.left}px`, top: `${r.top}px`, width: `${r.right - r.left}px`, height: `${r.bottom - r.top}px`,
      });
      const selected = new Set(base);
      let last = -1;
      rects.forEach((q, i) => {
        if (q.left < r.right && q.right > r.left && q.top < r.bottom && q.bottom > r.top) {
          if (toggle && base.has(i)) selected.delete(i);
          else selected.add(i);
          last = i;
        }
      });
      this.selected = selected;
      if (last >= 0) this.focus = last;
      this.renderSelection();
    };
    // Keep scrolling while the pointer is held past the top or bottom edge.
    const timer = setInterval(() => {
      if (!band) return;
      if (pointer.y < box.top + 16) scroller.scrollTop -= 16;
      else if (pointer.y > box.bottom - 16) scroller.scrollTop += 16;
      else return;
      update();
    }, 40);
    const move = (e) => {
      pointer = { x: e.clientX, y: e.clientY };
      update();
    };
    const up = () => {
      clearInterval(timer);
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      if (band) {
        band.remove();
        this.anchor = this.focus;
      } else {
        onClick?.();
      }
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  }

  // ---------- Navigation ----------

  /**
   * Load `path`. `mode`: "push" adds a history entry, "replace" doesn't.
   * `select` names an entry to highlight; `preserveScroll` keeps the scroll
   * position (live refresh); `onError(err)` replaces the message bar.
   */
  async navigate(path, { mode = "push", select = null, preserveScroll = false, onError = null } = {}) {
    if (this.search) {
      // Refreshing the searched folder (after a rename, trash, ...) re-runs the search;
      // going anywhere else ends it.
      if (path === this.search.root && mode === "replace") {
        await this.runSearch(this.search.query, { keepSelection: true });
        return true;
      }
      this.endSearch();
      this.searchBar.hidden = true;
      this.searchInput.value = "";
    }
    const id = ++this.requestId;
    const previous = this.path;
    const props = path === previous ? this.props : viewPropsFor(path);
    let view;
    try {
      view = await invoke("open_dir", {
        pane: this.id,
        path,
        options: { showHidden: this.app.showHidden, sort: props.sort },
      });
    } catch (err) {
      if (id !== this.requestId || this.closed) return false;
      if (onError) onError(err);
      else this.showMessage(errMsg(err));
      return false;
    }
    if (id !== this.requestId || this.closed) return false; // a newer navigation won

    this.hideMessage();
    if (mode === "push" && view.path !== previous) {
      this.history.splice(this.historyIndex + 1);
      this.history.push(view.path);
      this.historyIndex = this.history.length - 1;
    }

    const changedFolder = view.path !== previous;
    // When going up (or back to an ancestor), highlight the folder we came from.
    const selectName = select ?? childNameOnPath(view.path, previous);
    const keep = !changedFolder && select == null ? this.selectedNames() : null;
    const scrollTop = this.scroller.scrollTop;

    if (changedFolder && this.filter) {
      this.filter = "";
      this.filterInput.value = "";
    }
    const remount = !this.viewImpl || props.viewMode !== this.props.viewMode;
    this.path = view.path;
    this.view = view;
    this.props = props;
    if (remount) this.mountView();
    this.computeItems();
    this.render();
    this.restoreSelection(keep, selectName, preserveScroll);
    if (preserveScroll) this.scroller.scrollTop = scrollTop;
    this.app.paneChanged(this);
    return true;
  }

  goBack() {
    if (this.historyIndex <= 0) return;
    this.historyIndex--;
    this.navigate(this.history[this.historyIndex], { mode: "replace" });
  }

  goForward() {
    if (this.historyIndex >= this.history.length - 1) return;
    this.historyIndex++;
    this.navigate(this.history[this.historyIndex], { mode: "replace" });
  }

  goUp() {
    if (this.view?.parent) this.navigate(this.view.parent);
  }

  reload(opts = {}) {
    if (this.path) return this.navigate(this.path, { mode: "replace", preserveScroll: true, ...opts });
  }

  canGoBack() { return this.historyIndex > 0; }
  canGoForward() { return this.historyIndex < this.history.length - 1; }

  isFolder(e) {
    // macOS .app bundles are directories on disk but open like files.
    return e.kind === "directory" && e.icon !== "app";
  }

  async openEntry(i) {
    const e = this.items[i];
    if (!e) return;
    // Archives open like folders (read-only); "Open With" is in the context menu.
    if (this.isFolder(e) || e.archive) return this.navigate(e.path);
    if (e.kind === "brokenSymlink") return this.showMessage(`"${e.name}" is a broken link`);
    this.app.flashStatus(`Opening "${e.name}"…`);
    try {
      await invoke("open_path", { path: e.path });
    } catch (err) {
      this.showMessage(errMsg(err));
    }
  }

  /** Enter: open the folder under the cursor, or every selected file. */
  openSelection() {
    const sel = [...this.selected].sort((a, b) => a - b);
    if (sel.length <= 1) return this.openEntry(sel[0] ?? this.focus);
    const files = sel.filter((i) => !this.isFolder(this.items[i]));
    if (files.length) files.forEach((i) => this.openEntry(i));
    else this.openEntry(this.focus);
  }

  // ---------- Live updates (events routed here by the app) ----------

  onCounts({ generation, counts }) {
    if (!this.view || generation !== this.view.generation) return; // stale job
    const all = new Map(this.view.entries.map((e) => [e.name, e]));
    for (const [name, count] of counts) {
      const e = all.get(name);
      if (!e) continue;
      e.childCount = count;
      const i = this.byName.get(name);
      if (i != null) this.viewImpl.setCount(i, count);
    }
    if (this.selected.size) this.app.paneChanged(this, { statusOnly: true });
  }

  onDirChanged({ path }) {
    if (path === this.path && !this.search) this.liveRefresh();
  }

  // ---------- Search ----------

  showSearch() {
    if (this.view?.archive) {
      this.app.flashStatus("Search isn't available inside archives");
      return;
    }
    if (!this.search) this.searchRoot = this.path;
    this.searchBar.hidden = false;
    this.searchInput.placeholder = `Search in “${this.app.folderLabel(this.searchRoot)}”…`;
    this.searchInput.focus();
    this.searchInput.select();
  }

  hideSearch() {
    clearTimeout(this.searchTimer);
    const searching = !!this.search;
    this.endSearch();
    this.searchBar.hidden = true;
    this.searchInput.value = "";
    if (searching) this.navigate(this.searchRoot, { mode: "replace" });
    this.scroller.focus();
  }

  endSearch() {
    if (this.search) invoke("cancel_search", { pane: this.id }).catch(() => {});
    this.search = null;
    this.searchSort = null;
  }

  async runSearch(query, { keepSelection = false } = {}) {
    query = query.trim();
    if (!query) {
      if (this.search) {
        this.endSearch();
        await this.navigate(this.searchRoot, { mode: "replace" });
      }
      return;
    }
    if (!this.search) this.folderView = this.view;
    const keep = keepSelection ? this.selectedPaths() : null;
    const generation = await invoke("start_search", {
      pane: this.id, root: this.searchRoot, query, showHidden: this.app.showHidden,
    });
    this.search = { query, root: this.searchRoot, generation, hits: [], done: false, scanned: 0, truncated: false };
    this.showSearchResults(keep);
    this.app.paneChanged(this);
  }

  onSearchResults({ generation, hits, foldersScanned, done, summary }) {
    const s = this.search;
    if (!s || generation !== s.generation) return; // an older search
    s.hits.push(...hits);
    s.scanned = foldersScanned;
    if (done) {
      s.done = true;
      s.truncated = !!summary?.truncated;
    }
    this.showSearchResults(this.selectedPaths(), true);
    this.app.paneChanged(this, { statusOnly: true });
  }

  showSearchResults(keepPaths, preserveScroll = false) {
    const s = this.search;
    const base = this.folderView ?? this.view;
    const scrollTop = this.scroller.scrollTop;
    this.view = { ...base, path: s.root, entries: this.sortHits(s.hits), hiddenCount: 0, archive: null, searching: true };
    this.computeItems();
    this.render();
    this.selected = new Set();
    this.focus = this.anchor = -1;
    if (keepPaths?.size) {
      this.items.forEach((e, i) => { if (keepPaths.has(e.path)) this.selected.add(i); });
      this.focus = this.anchor = [...this.selected][0] ?? -1;
    }
    this.renderSelection();
    if (preserveScroll) this.scroller.scrollTop = scrollTop;
    const n = s.hits.length;
    this.searchBar.querySelector(".search-status").textContent =
      !s.done ? `Searching… ${n}` : s.truncated ? `First ${n} results` : plural(n, "result");
  }

  /** Best matches first, unless a column header was clicked. */
  sortHits(hits) {
    const sorted = [...hits];
    const sort = this.searchSort;
    if (!sort) return sorted.sort((a, b) => b.score - a.score || a.path.length - b.path.length || a.name.localeCompare(b.name));
    const key = {
      name: (e) => e.name.toLowerCase(),
      size: (e) => e.size ?? -1,
      modified: (e) => e.modified ?? 0,
      type: (e) => this.relativeFolder(e.path),
    }[sort.key];
    const dir = sort.order === "descending" ? -1 : 1;
    return sorted.sort((a, b) => {
      const x = key(a), y = key(b);
      return (x < y ? -1 : x > y ? 1 : a.name.localeCompare(b.name, undefined, { numeric: true })) * dir;
    });
  }

  /** Folder of a search result, relative to where the search started. */
  relativeFolder(path) {
    const root = this.search?.root ?? this.path;
    const dir = path.slice(0, path.lastIndexOf("/")) || "/";
    if (dir === root) return "";
    return dir.startsWith(`${root}/`) ? dir.slice(root.length + 1) : dir;
  }

  selectedPaths() {
    return new Set(this.selectedEntries().map((e) => e.path));
  }

  async liveRefresh() {
    // Re-rendering would destroy an open rename field; catch up afterwards.
    if (this.refreshing || this.renaming) { this.refreshQueued = true; return; }
    this.refreshing = true;
    try {
      await this.reload({ onError: (err) => this.onFolderGone(err) });
    } finally {
      this.refreshing = false;
      this.resumeRefresh();
    }
  }

  resumeRefresh() {
    if (this.refreshQueued && !this.renaming) {
      this.refreshQueued = false;
      this.liveRefresh();
    }
  }

  // The folder was deleted or renamed away: walk up to the nearest one that
  // still exists, like Dolphin does.
  async onFolderGone(err) {
    if (err?.code !== "notFound") return this.showMessage(errMsg(err));
    const gone = this.path;
    let p = this.view?.parent;
    while (p) {
      if (await this.navigate(p, { onError: () => {} })) {
        this.showMessage(`"${gone}" no longer exists`);
        return;
      }
      p = p === "/" ? null : p.replace(/\/[^/]*$/, "") || "/";
    }
  }

  // ---------- Items, filter, view properties ----------

  computeItems() {
    const f = this.filter.toLowerCase();
    const entries = this.view?.entries ?? [];
    this.items = f ? entries.filter((e) => e.name.toLowerCase().includes(f)) : entries;
    this.byName = new Map(this.items.map((e, i) => [e.name, i]));
  }

  render() {
    this.viewImpl.render();
    const total = this.view?.entries.length ?? 0;
    this.empty.hidden = this.items.length > 0;
    this.empty.textContent = this.filter && total
      ? "No items match the filter"
      : this.view?.hiddenCount ? "Folder contains only hidden items" : "Folder is empty";
    this.filterBar.querySelector(".filter-count").textContent = this.filter ? `${this.items.length} of ${total}` : "";
  }

  setFilter(text) {
    const names = this.selectedNames();
    this.filter = text;
    this.computeItems();
    this.render();
    this.restoreSelection(names, null, true);
    this.app.paneChanged(this, { statusOnly: true });
  }

  showFilter() {
    this.filterBar.hidden = false;
    this.filterInput.focus();
    this.filterInput.select();
  }

  hideFilter() {
    this.filterBar.hidden = true;
    if (this.filter) {
      this.filterInput.value = "";
      this.setFilter("");
    }
    this.scroller.focus();
  }

  setViewMode(mode) {
    if (mode === this.props.viewMode || !this.view) return;
    const names = this.selectedNames();
    this.props = { ...this.props, viewMode: mode };
    saveViewProps(this.path, this.props);
    this.mountView();
    this.render();
    this.restoreSelection(names, null, false);
    this.app.paneChanged(this);
  }

  setSort(sort) {
    if (this.search) {
      this.searchSort = sort; // results only; the folder's own sort is unchanged
      this.showSearchResults(this.selectedPaths(), true);
      return;
    }
    this.props = { ...this.props, sort };
    saveViewProps(this.path, this.props);
    this.reload();
  }

  /** Header click: same column flips the order, a new column starts at its natural order. */
  toggleSort(key) {
    const s = (this.search ? this.searchSort : this.props.sort) ?? { key: null, order: "ascending" };
    this.setSort(s.key === key
      ? { key, order: s.order === "ascending" ? "descending" : "ascending" }
      : { key, order: key === "modified" ? "descending" : "ascending" });
  }

  isCut(e) {
    const clip = this.app.clipboard;
    return clip?.mode === "cut" && clip.paths.has(e.path);
  }

  markCut() {
    this.items.forEach((e, i) => this.viewImpl.item(i)?.classList.toggle("cut", this.isCut(e)));
  }

  // ---------- Selection ----------

  selectedEntries() {
    return [...this.selected].sort((a, b) => a - b).map((i) => this.items[i]).filter(Boolean);
  }

  selectedNames() {
    return new Set(this.selectedEntries().map((e) => e.name));
  }

  renderSelection() {
    for (const n of this.scroller.querySelectorAll(".selected, .focused")) n.classList.remove("selected", "focused");
    for (const i of this.selected) this.viewImpl.item(i)?.classList.add("selected");
    this.viewImpl.item(this.focus)?.classList.add("focused");
    this.app.paneChanged(this, { statusOnly: true });
  }

  clearSelection() {
    if (!this.selected.size) return;
    this.selected.clear();
    this.renderSelection();
  }

  selectAll() {
    this.selected = new Set(this.items.map((_, i) => i));
    this.renderSelection();
  }

  setFocus(i, { extend = false, toggle = false } = {}) {
    const n = this.items.length;
    if (!n) return;
    i = Math.max(0, Math.min(n - 1, i));
    if (extend && this.anchor >= 0) {
      this.selected.clear();
      const [a, b] = [Math.min(this.anchor, i), Math.max(this.anchor, i)];
      for (let k = a; k <= b; k++) this.selected.add(k);
    } else if (toggle) {
      if (this.selected.has(i)) this.selected.delete(i);
      else this.selected.add(i);
      this.anchor = i;
    } else {
      this.selected = new Set([i]);
      this.anchor = i;
    }
    this.focus = i;
    this.renderSelection();
    this.viewImpl.item(i)?.scrollIntoView({ block: "nearest" });
  }

  restoreSelection(names, single, keepScroll) {
    this.selected = new Set();
    this.focus = this.anchor = -1;
    if (names?.size) {
      this.items.forEach((e, i) => { if (names.has(e.name)) this.selected.add(i); });
      this.focus = this.anchor = [...this.selected][0] ?? -1;
    } else if (single) {
      const i = this.byName.get(single);
      if (i != null) { this.selected.add(i); this.focus = this.anchor = i; }
    }
    this.renderSelection();
    if (keepScroll) return;
    if (this.focus >= 0) this.viewImpl.item(this.focus)?.scrollIntoView({ block: "center" });
    else this.scroller.scrollTop = 0;
  }

  // ---------- Keyboard (view navigation; app handles global shortcuts) ----------

  keydown(ev) {
    const n = this.items.length;
    const cur = this.focus;
    const cols = this.viewImpl.columns();
    const extend = ev.shiftKey;
    const run = (fn) => { ev.preventDefault(); fn(); return true; };
    // Arrow movement: in the grid, moving past an edge does nothing.
    const move = (delta) => {
      if (cur < 0) return this.setFocus(0);
      const t = cur + delta;
      if ((t < 0 || t >= n) && cols > 1 && Math.abs(delta) === cols) return;
      this.setFocus(t, { extend });
    };
    const page = () => this.viewImpl.rowsPerPage(this.scroller.clientHeight);

    switch (ev.key) {
      case "ArrowDown":
        if (isMac && ev.metaKey) return run(() => this.openSelection());
        return run(() => move(cols));
      case "ArrowUp": return run(() => move(-cols));
      case "ArrowRight": return cols > 1 ? run(() => move(1)) : false;
      case "ArrowLeft": return cols > 1 ? run(() => move(-1)) : false;
      case "Home": return run(() => this.setFocus(0, { extend }));
      case "End": return run(() => this.setFocus(n - 1, { extend }));
      case "PageDown": return run(() => this.setFocus(Math.max(cur, 0) + page(), { extend }));
      case "PageUp": return run(() => this.setFocus(Math.max(cur, 0) - page(), { extend }));
      case "Enter": return run(() => this.openSelection());
      case "Backspace": return ev.metaKey || ev.ctrlKey || ev.altKey ? false : run(() => this.goUp());
      case "Escape":
        if (!this.filterBar.hidden) return run(() => this.hideFilter());
        return run(() => this.clearSelection());
    }
    if (isMod(ev) && ev.key.toLowerCase() === "a") return run(() => this.selectAll());

    // Type-ahead: jump to the first entry starting with what was typed.
    if (ev.key.length === 1 && !isMod(ev) && !ev.altKey && n) {
      clearTimeout(this.typeAheadTimer);
      this.typeAhead += ev.key.toLowerCase();
      this.typeAheadTimer = setTimeout(() => { this.typeAhead = ""; }, 800);
      const i = this.items.findIndex((e) => e.name.toLowerCase().startsWith(this.typeAhead));
      if (i >= 0) return run(() => this.setFocus(i));
    }
    return false;
  }

  // ---------- Messages and status ----------

  showMessage(text) {
    this.message.querySelector(".message-text").textContent = text;
    this.message.hidden = false;
  }

  hideMessage() { this.message.hidden = true; }

  statusText() {
    const v = this.view;
    if (!v) return { left: "", right: "" };
    let left;
    if (this.selected.size === 1) {
      const e = this.items[[...this.selected][0]];
      const detail = e.kind === "directory" ? Fmt.items(e.childCount) : Fmt.bytes(e.size);
      left = `"${e.name}" selected${detail ? ` (${detail})` : ""}`;
    } else if (this.selected.size > 1) {
      left = `${this.selected.size} items selected — ${summarize(this.selectedEntries())}`;
    } else if (this.search) {
      const s = this.search;
      left = `${s.done ? "" : "Searching… "}${plural(this.items.length, "result")} for “${s.query}”`;
      if (s.truncated) left += " (stopped at 10,000; refine the search)";
    } else {
      left = summarize(this.items);
      if (this.filter) left = `${left} — filtered from ${plural(v.entries.length, "item")}`;
    }
    const right = [];
    if (this.search) right.push(`in “${this.app.folderLabel(this.search.root)}”`);
    if (!this.app.showHidden && v.hiddenCount) right.push(`${v.hiddenCount} hidden`);
    if (v.watchError) right.push("Live updates off");
    if (v.archive) right.push("Read-only archive");
    return { left, right: right.join(" · "), rightTitle: v.watchError ?? "" };
  }

  title() {
    return this.view?.crumbs.at(-1)?.label ?? this.path ?? "";
  }

  close() {
    this.endSearch();
    this.closed = true;
    this.viewImpl?.dispose();
    this.root.remove();
    invoke("close_pane", { pane: this.id }).catch(() => {});
  }
}

function summarize(entries) {
  let dirs = 0, files = 0, size = 0;
  for (const e of entries) {
    if (e.kind === "directory") dirs++;
    else { files++; size += e.size ?? 0; }
  }
  const parts = [];
  if (dirs) parts.push(plural(dirs, "folder"));
  if (files) parts.push(plural(files, "file"));
  let s = parts.join(", ") || "No items";
  if (files) s += ` (${Fmt.bytes(size)})`;
  return s;
}
