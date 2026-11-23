// Window-level controller: tabs, split view, toolbar, location bar, places
// sidebar, status bar, keyboard routing and context menus.

import { appMenu, backgroundMenu, itemsMenu, locationMenu, shortcuts, tabMenu } from "./contextmenus.js";
import { Icons } from "./icons.js";
import { isMenuOpen, showMenu } from "./menu.js";
import { DragDrop } from "./dnd.js";
import { Ops } from "./ops.js";
import { Pane } from "./pane.js";
import { store } from "./store.js";
import { Fmt } from "./format.js";
import { basename, el, errMsg, invoke, isMac, isMod, listen, shortcut } from "./util.js";

const $ = (id) => document.getElementById(id);
let nextTabId = 1;

export class App {
  constructor() {
    this.tabs = [];
    this.activeTab = null;
    this.showHidden = store.get("showHidden", false);
    this.places = [];
    this.clipboard = null; // { mode: "copy" | "cut", paths: Set<string> }
    this.closedTabs = [];  // for "Reopen Closed Tab"
    this.tabDrag = null;
    this.home = null;
    this.flashTimer = 0;
    this.els = {
      back: $("btn-back"), forward: $("btn-forward"), up: $("btn-up"),
      viewIcons: $("btn-view-icons"), viewDetails: $("btn-view-details"),
      split: $("btn-split"), hidden: $("btn-hidden"), refresh: $("btn-refresh"),
      location: $("location"), crumbs: $("crumbs"), input: $("location-input"),
      tabbar: $("tabbar"), panes: $("panes"), sidebar: $("sidebar"), splitter: $("splitter"),
      statusLeft: $("status-left"), statusRight: $("status-right"),
    };
    this.ops = new Ops(this);
    this.dnd = new DragDrop(this);
    this.actions = this.buildActions();
    this.nativeMenu = false; // set in start(): macOS has a real menu bar
    this.lastKeyHandled = -Infinity;
    document.body.classList.add(isMac ? "platform-macos" : "platform-linux");
    this.initAppMenu();
    this.initClipboardEvents();
    // Another app may have changed the clipboard while we were in the background.
    window.addEventListener("focus", () => this.ops.syncClipboard());
    this.initToolbar();
    this.initLocation();
    this.initSidebar();
    this.initKeyboard();
    this.initTabBar();
    listen("child-counts", ({ payload }) => this.findPane(payload.pane)?.onCounts(payload));
    listen("dir-changed", ({ payload }) => this.findPane(payload.pane)?.onDirChanged(payload));
    listen("search-results", ({ payload }) => this.findPane(payload.pane)?.onSearchResults(payload));
  }

  async start() {
    try {
      await this.loadPlaces();
    } catch (err) {
      console.error(err);
    }
    this.home = (await invoke("home_dir")) ?? "/";
    this.nativeMenu = await invoke("has_native_menu").catch(() => false);
    document.body.classList.toggle("native-menu", this.nativeMenu);
    // Folders the OS asked us to show (Sonar as the default file manager):
    // listen first, then collect what arrived before we were ready.
    listen("open-paths", ({ payload }) => this.openRequested(payload));
    const requested = await invoke("take_open_requests").catch(() => []);
    const restored = this.restoreSession();
    if (requested.length) this.openRequested(requested);
    else if (!restored) this.newTab(this.home);
  }

  /** Open each requested folder in a new tab (selecting a requested file). */
  openRequested(requests) {
    let last = null;
    for (const { folder, select } of requests) last = this.addTab([folder], { select, index: this.tabs.length });
    if (last) this.activateTab(last);
  }

  // ---------- Actions (shared by the menu bar, the ☰ menu and shortcuts) ----------

  buildActions() {
    const go = (icon) => () => {
      const p = this.places.find((x) => x.icon === icon);
      if (p) this.pane.navigate(p.path);
    };
    return {
      "new-tab": () => this.newTab(this.pane?.path ?? this.home),
      "new-folder": () => this.ops.newFolder(),
      "open-location": () => this.editLocation(),
      "close-tab": () => this.closeCurrent(),
      "close-window": () => this.closeWindow(),
      "reopen-tab": () => this.reopenClosedTab(),
      "next-tab": () => this.cycleTab(1),
      "prev-tab": () => this.cycleTab(-1),
      undo: () => this.ops.undo(),
      "paste-move": () => this.ops.paste(this.pane.path, { move: true }),
      "select-all": () => this.pane.selectAll(),
      "copy-location": () => this.ops.copyLocation(),
      rename: () => this.ops.beginRename(),
      trash: () => this.ops.removeSelected(),
      delete: () => this.ops.removeSelected({ permanent: true }),
      "view-icons": () => this.setViewMode("icons"),
      "view-details": () => this.setViewMode("details"),
      "show-hidden": () => this.toggleHidden(),
      split: () => this.toggleSplit(),
      filter: () => this.pane.showFilter(),
      search: () => this.pane.showSearch(),
      reload: () => this.pane.reload(),
      back: () => this.pane.goBack(),
      forward: () => this.pane.goForward(),
      up: () => this.pane.goUp(),
      "go-home": go("home"),
      "go-desktop": go("desktop"),
      "go-documents": go("documents"),
      "go-downloads": go("downloads"),
      "go-trash": go("trash"),
    };
  }

  /**
   * Run an action from a menu. While a text field has focus, Undo and Select
   * All apply to the text and file actions are ignored, so e.g. ⌘⌫ in the
   * rename field never trashes files.
   */
  runAction(id) {
    const field = document.activeElement?.closest?.("input, textarea");
    if (field) {
      if (id === "undo") return document.execCommand("undo");
      if (id === "select-all") return field.select();
      const harmless = /^(new-tab|close-window|reopen-tab|next-tab|prev-tab|view-|show-hidden|split|reload|go-)/;
      if (!harmless.test(id)) return;
    }
    if (!this.pane) return;
    this.actions[id]?.();
  }

  initAppMenu() {
    // Native menu bar (macOS). A shortcut handled by the page doesn't reach
    // the menu, but guard against both firing for one key press anyway: the
    // echo of a key press arrives within milliseconds; people can't click a
    // menu that fast.
    listen("menu", ({ payload }) => {
      if (performance.now() - this.lastKeyHandled < 120) return;
      this.runAction(payload);
    });
    // ☰ menu (Linux, where Dolphin also hides the menu bar by default).
    const btn = document.getElementById("btn-menu");
    btn.innerHTML = Icons.ui("menu");
    btn.title = "Menu";
    btn.addEventListener("click", () => {
      const r = btn.getBoundingClientRect();
      showMenu(r.right - 220, r.bottom + 4, appMenu(this));
    });
  }

  /** Mirror state in the native menu's check marks (macOS). */
  syncMenu() {
    if (!this.nativeMenu || this.menuSyncQueued) return;
    this.menuSyncQueued = true;
    requestAnimationFrame(() => {
      this.menuSyncQueued = false;
      const p = this.pane;
      if (!p) return;
      invoke("set_menu_state", {
        state: {
          viewMode: p.props.viewMode, showHidden: this.showHidden, split: this.activeTab.panes.length === 2,
          canGoBack: p.canGoBack(), canGoForward: p.canGoForward(), canGoUp: !!p.view?.parent,
        },
      }).catch(() => {});
    });
  }

  /**
   * Edit ▸ Cut/Copy/Paste in the macOS menu are the system's own items (so
   * they work in text fields). With the file view focused they arrive as DOM
   * clipboard events; "before*" handlers tell WebKit to enable the items.
   */
  initClipboardEvents() {
    const inField = () => !!document.activeElement?.closest?.("input, textarea");
    for (const type of ["beforecopy", "beforecut", "beforepaste"]) {
      document.addEventListener(type, (ev) => { if (!inField() && this.pane) ev.preventDefault(); });
    }
    document.addEventListener("copy", (ev) => { if (!inField()) { ev.preventDefault(); this.ops.setClipboard("copy"); } });
    document.addEventListener("cut", (ev) => { if (!inField()) { ev.preventDefault(); this.ops.setClipboard("cut"); } });
    document.addEventListener("paste", (ev) => { if (!inField()) { ev.preventDefault(); this.ops.paste(); } });
  }

  // ---------- Panes and tabs ----------

  /** The pane keyboard and toolbar actions apply to. */
  get pane() {
    const t = this.activeTab;
    return t?.panes[t.active];
  }

  forEachPane(fn) {
    for (const t of this.tabs) t.panes.forEach(fn);
  }

  findPane(id) {
    for (const t of this.tabs) for (const p of t.panes) if (p.id === id) return p;
    return null;
  }

  tabOf(pane) {
    return this.tabs.find((t) => t.panes.includes(pane));
  }

  createPane(tab, path, select) {
    const pane = new Pane(this);
    pane.initialPath = path; // until the first listing arrives (used by session save)
    tab.panes.push(pane);
    tab.el.append(pane.root);
    pane.navigate(path, { select }).then((ok) => {
      if (!ok && !pane.path && path !== this.home) pane.navigate(this.home);
    });
    return pane;
  }

  /** Create a tab showing `paths` (one, or two for a split) at `index`. Does not activate it. */
  addTab(paths, { active = 0, index, select } = {}) {
    const tab = { id: nextTabId++, panes: [], active: 0, el: el("div", "tab-panes") };
    tab.el.hidden = true;
    const at = index ?? (this.activeTab ? this.tabs.indexOf(this.activeTab) + 1 : this.tabs.length);
    this.tabs.splice(Math.min(at, this.tabs.length), 0, tab);
    this.els.panes.append(tab.el);
    for (const p of paths.slice(0, 2)) this.createPane(tab, p, select);
    tab.active = Math.min(active, tab.panes.length - 1);
    tab.el.classList.toggle("split", tab.panes.length === 2);
    return tab;
  }

  newTab(path, { activate = true } = {}) {
    const tab = this.addTab([path]);
    if (activate) this.activateTab(tab);
    else {
      this.renderTabs();
      this.flashStatus(`Opened “${this.folderLabel(path)}” in a new tab`);
    }
    return tab;
  }

  activateTab(tab) {
    this.activeTab = tab;
    for (const t of this.tabs) t.el.hidden = t !== tab;
    this.renderTabs();
    this.updateChrome();
    this.pane?.scroller.focus();
  }

  /** Close a tab. Closing the last one closes the window, like Dolphin. */
  closeTab(tab = this.activeTab) {
    if (this.tabs.length <= 1) return this.closeWindow();
    const idx = this.tabs.indexOf(tab);
    this.closedTabs.push({ paths: tab.panes.map((p) => p.path ?? p.initialPath), active: tab.active, index: idx });
    if (this.closedTabs.length > 20) this.closedTabs.shift();
    tab.panes.forEach((p) => p.close());
    tab.el.remove();
    this.tabs.splice(idx, 1);
    if (tab === this.activeTab) this.activateTab(this.tabs[Math.min(idx, this.tabs.length - 1)]);
    else this.renderTabs();
  }

  closeOtherTabs(keep) {
    for (const t of [...this.tabs]) if (t !== keep) this.closeTab(t);
    this.activateTab(keep);
  }

  closeTabsToRight(tab) {
    for (const t of this.tabs.slice(this.tabs.indexOf(tab) + 1)) this.closeTab(t);
  }

  reopenClosedTab() {
    const last = this.closedTabs.pop();
    if (!last) return;
    this.activateTab(this.addTab(last.paths, { active: last.active, index: last.index }));
  }

  /** Ctrl/⌘+W: on the last tab, close its split first, then the window. */
  closeCurrent() {
    if (this.tabs.length === 1 && this.activeTab.panes.length === 2) return this.toggleSplit();
    this.closeTab();
  }

  closeWindow() {
    this.saveSession();
    window.__TAURI__.window.getCurrentWindow().close().catch((err) => this.pane?.showMessage(errMsg(err)));
  }

  cycleTab(delta) {
    if (this.tabs.length < 2) return;
    const i = this.tabs.indexOf(this.activeTab);
    this.activateTab(this.tabs[(i + delta + this.tabs.length) % this.tabs.length]);
  }

  moveTab(tab, toIndex) {
    const from = this.tabs.indexOf(tab);
    this.tabs.splice(from, 1);
    this.tabs.splice(Math.max(0, Math.min(toIndex, this.tabs.length)), 0, tab);
    this.renderTabs();
  }

  renderTabs() {
    if (this.tabDrag?.started) return; // don't rebuild under the pointer; redone on drop
    const bar = this.els.tabbar;
    bar.replaceChildren(...this.tabs.map((tab) => {
      const node = el("div", `tab${tab === this.activeTab ? " active" : ""}`);
      node.setAttribute("role", "tab");
      node.setAttribute("aria-selected", String(tab === this.activeTab));
      node.dataset.tab = tab.id;
      node.title = tab.panes.map((p) => p.path ?? p.initialPath).join("\n");
      const label = el("span", "tab-label");
      label.textContent = tab.panes.map((p) => p.title() || this.folderLabel(p.initialPath)).join(" | ");
      const close = el("button", "tab-close", Icons.ui("close"));
      close.title = this.tabs.length > 1 ? `Close Tab (${shortcuts.closeTab})` : "Close Window";
      close.addEventListener("pointerdown", (ev) => ev.stopPropagation());
      close.addEventListener("click", (ev) => { ev.stopPropagation(); this.closeTab(tab); });
      node.append(label, close);
      node.addEventListener("pointerdown", (ev) => this.onTabPointerDown(ev, tab));
      node.addEventListener("auxclick", (ev) => { if (ev.button === 1) this.closeTab(tab); });
      node.addEventListener("contextmenu", (ev) => {
        ev.preventDefault();
        showMenu(ev.clientX, ev.clientY, tabMenu(this, tab));
      });
      return node;
    }));
    const add = el("button", "tab-new tool", Icons.ui("plus"));
    add.title = `New Tab (${shortcuts.newTab})`;
    add.addEventListener("click", () => this.newTab(this.pane?.path ?? this.home));
    bar.append(add);
    bar.querySelector(".tab.active")?.scrollIntoView({ block: "nearest", inline: "nearest" });
    this.saveSession();
  }

  initTabBar() {
    const bar = this.els.tabbar;
    // Double-click empty space in the tab bar: new tab (browser style).
    bar.addEventListener("dblclick", (ev) => {
      if (ev.target === bar) this.newTab(this.pane?.path ?? this.home);
    });
    window.addEventListener("pointermove", (ev) => this.onTabPointerMove(ev));
    window.addEventListener("pointerup", () => this.onTabPointerUp());
    window.addEventListener("pointercancel", () => this.onTabPointerUp());
  }

  // Drag to reorder: activate on press, start dragging after a few pixels,
  // and drop where the pointer passes the middle of a neighbouring tab.
  onTabPointerDown(ev, tab) {
    if (ev.button !== 0) return;
    if (tab !== this.activeTab) this.activateTab(tab);
    this.tabDrag = { tab, startX: ev.clientX, started: false, index: this.tabs.indexOf(tab) };
  }

  onTabPointerMove(ev) {
    const d = this.tabDrag;
    if (!d) return;
    const dx = ev.clientX - d.startX;
    if (!d.started) {
      if (Math.abs(dx) < 5 || this.tabs.length < 2) return;
      d.started = true;
      this.els.tabbar.classList.add("dragging");
    }
    const nodes = [...this.els.tabbar.querySelectorAll(".tab")];
    const node = nodes.find((n) => n.dataset.tab === String(d.tab.id));
    node.classList.add("drag");
    node.style.transform = `translateX(${dx}px)`;
    // Target index = number of other tabs whose centre is left of the pointer.
    d.index = nodes.filter((n) => n !== node).filter((n) => {
      const r = n.getBoundingClientRect();
      return r.left + r.width / 2 < ev.clientX;
    }).length;
  }

  onTabPointerUp() {
    const d = this.tabDrag;
    this.tabDrag = null;
    if (!d?.started) return;
    this.els.tabbar.classList.remove("dragging");
    this.moveTab(d.tab, d.index);
  }

  // ---------- Session ----------

  saveSession() {
    if (!this.tabs.length) return;
    store.set("session", {
      tabs: this.tabs.map((t) => ({ paths: t.panes.map((p) => p.path ?? p.initialPath), active: t.active })),
      active: Math.max(0, this.tabs.indexOf(this.activeTab)),
    });
  }

  /** Reopen the tabs (and splits) from last time. Missing folders fall back to Home. */
  restoreSession() {
    const s = store.get("session", null);
    const saved = (s?.tabs ?? []).filter((t) => Array.isArray(t.paths) && t.paths.length && t.paths.every((p) => typeof p === "string"));
    if (!saved.length) return false;
    for (const t of saved) this.addTab(t.paths, { active: t.active ?? 0, index: this.tabs.length });
    this.activateTab(this.tabs[Math.min(Math.max(0, s.active ?? 0), this.tabs.length - 1)]);
    return true;
  }

  // ---------- Split view ----------

  toggleSplit(tab = this.activeTab) {
    if (tab.panes.length === 2) {
      // Close the inactive side; keep the one being worked in.
      const drop = tab.panes[1 - tab.active];
      drop.close();
      tab.panes = [tab.panes[tab.active]];
      tab.active = 0;
    } else {
      const from = tab.panes[0];
      this.createPane(tab, from.path ?? this.home);
      tab.active = 1;
    }
    tab.el.classList.toggle("split", tab.panes.length === 2);
    this.renderTabs();
    this.updateChrome();
    this.pane.scroller.focus();
  }

  /** Show `path` in the other side of the split, creating the split if needed. */
  openInSplit(path) {
    const tab = this.activeTab;
    if (tab.panes.length === 1) {
      this.createPane(tab, path);
      tab.active = 1;
      tab.el.classList.add("split");
    } else {
      tab.active = 1 - tab.active;
      this.pane.navigate(path);
    }
    this.renderTabs();
    this.updateChrome();
    this.pane.scroller.focus();
  }

  activate(pane) {
    const tab = this.tabOf(pane);
    if (!tab || tab !== this.activeTab) return;
    const idx = tab.panes.indexOf(pane);
    if (tab.active === idx) return;
    tab.active = idx;
    this.updateChrome();
    this.saveSession();
  }

  /** Called by a pane after it navigates or its selection changes. */
  paneChanged(pane, { statusOnly = false } = {}) {
    if (!statusOnly) {
      pane.header.innerHTML = `${Icons.ui("folder")}<span></span>`;
      pane.header.querySelector("span").textContent = pane.title();
      pane.header.title = pane.path ?? "";
      this.renderTabs();
    }
    if (pane !== this.pane) return;
    if (statusOnly) this.updateStatus();
    else this.updateChrome();
  }

  // ---------- Chrome ----------

  updateChrome() {
    const pane = this.pane;
    if (!pane) return;
    const tab = this.activeTab;
    tab.panes.forEach((p, i) => p.root.classList.toggle("active", i === tab.active));
    const e = this.els;
    e.back.disabled = !pane.canGoBack();
    e.forward.disabled = !pane.canGoForward();
    e.up.disabled = !pane.view?.parent;
    e.viewIcons.setAttribute("aria-pressed", String(pane.props.viewMode === "icons"));
    e.viewDetails.setAttribute("aria-pressed", String(pane.props.viewMode === "details"));
    e.split.setAttribute("aria-pressed", String(tab.panes.length === 2));
    e.hidden.setAttribute("aria-pressed", String(this.showHidden));
    this.renderCrumbs();
    for (const b of e.sidebar.querySelectorAll(".place")) b.classList.toggle("active", b.dataset.path === pane.path);
    this.updateStatus();
    this.syncMenu();
    document.title = pane.path ? `${pane.title()} — Sonar` : "Sonar";
  }

  updateStatus() {
    if (this.flashTimer) return; // a transient message is showing
    const s = this.pane?.statusText() ?? { left: "", right: "" };
    this.els.statusLeft.textContent = s.left;
    this.els.statusRight.textContent = s.right;
    this.els.statusRight.title = s.rightTitle ?? "";
  }

  flashStatus(text) {
    clearTimeout(this.flashTimer);
    this.els.statusLeft.textContent = text;
    this.flashTimer = setTimeout(() => {
      this.flashTimer = 0;
      this.updateStatus();
    }, 2500);
  }

  folderLabel(path) {
    if (path === this.home) return "Home";
    return path === "/" ? "/" : basename(path);
  }

  toggleHidden() {
    this.showHidden = !this.showHidden;
    store.set("showHidden", this.showHidden);
    this.forEachPane((p) => p.reload());
    this.updateChrome();
  }

  setViewMode(mode) {
    this.pane?.setViewMode(mode);
  }

  async contextMenu(x, y, ctx) {
    // Look up what Undo would do and whether there is anything to paste first.
    const [undoLabel, clip] = await Promise.all([this.ops.undoLabel(), this.ops.readClipboard()]);
    const info = { undoLabel, canPaste: !!clip?.paths?.length };
    showMenu(x, y, ctx.kind === "items" ? itemsMenu(this, ctx.pane, info) : backgroundMenu(this, ctx.pane, info));
  }

  // ---------- Toolbar ----------

  initToolbar() {
    const e = this.els;
    const set = (btn, icon, title) => { btn.innerHTML = Icons.ui(icon); btn.title = title; };
    set(e.back, "back", `Back (${isMac ? "⌘[" : "Alt+Left"})`);
    set(e.forward, "forward", `Forward (${isMac ? "⌘]" : "Alt+Right"})`);
    set(e.up, "up", `Up (${isMac ? "⌘↑" : "Alt+Up"})`);
    set(e.viewIcons, "viewIcons", `Icons (${shortcuts.icons})`);
    set(e.viewDetails, "viewDetails", `Details (${shortcuts.details})`);
    set(e.split, "split", "Split View (F3)");
    set(e.hidden, "eye", `Show Hidden Files (${shortcuts.hidden})`);
    set(e.refresh, "refresh", `Reload (${isMac ? "⌘R" : "F5"})`);
    e.back.addEventListener("click", () => this.pane.goBack());
    e.forward.addEventListener("click", () => this.pane.goForward());
    e.up.addEventListener("click", () => this.pane.goUp());
    e.viewIcons.addEventListener("click", () => this.setViewMode("icons"));
    e.viewDetails.addEventListener("click", () => this.setViewMode("details"));
    e.split.addEventListener("click", () => this.toggleSplit());
    e.hidden.addEventListener("click", () => this.toggleHidden());
    e.refresh.addEventListener("click", () => this.pane.reload());
    // Mouse back/forward buttons.
    window.addEventListener("mouseup", (ev) => {
      if (ev.button === 3) { ev.preventDefault(); this.pane.goBack(); }
      if (ev.button === 4) { ev.preventDefault(); this.pane.goForward(); }
    });
  }

  // ---------- Location bar ----------

  initLocation() {
    const { crumbs, location, input } = this.els;
    const crumbPath = (ev) => ev.target.closest(".crumb")?.dataset.path;
    crumbs.addEventListener("click", (ev) => {
      const p = crumbPath(ev);
      if (p) this.pane.navigate(p);
    });
    crumbs.addEventListener("auxclick", (ev) => {
      const p = crumbPath(ev);
      if (p && ev.button === 1) this.newTab(p, { activate: false });
    });
    crumbs.addEventListener("contextmenu", (ev) => {
      const p = crumbPath(ev);
      ev.preventDefault();
      if (p) showMenu(ev.clientX, ev.clientY, locationMenu(this, p));
    });
    // Clicking empty space in the bar switches to text editing, like Dolphin.
    location.addEventListener("mousedown", (ev) => {
      if (ev.button !== 0 || ev.target.closest(".crumb") || ev.target === input) return;
      ev.preventDefault();
      this.editLocation();
    });
    input.addEventListener("keydown", async (ev) => {
      ev.stopPropagation();
      if (ev.key === "Escape") {
        ev.preventDefault();
        this.endEditLocation();
        this.pane.scroller.focus();
      } else if (ev.key === "Enter") {
        ev.preventDefault();
        const pane = this.pane;
        try {
          const target = await invoke("resolve_path", { input: input.value, cwd: pane.path ?? "/" });
          if (await pane.navigate(target)) {
            this.endEditLocation();
            pane.scroller.focus();
          }
        } catch (err) {
          pane.showMessage(errMsg(err));
        }
      }
    });
    input.addEventListener("blur", () => this.endEditLocation());
  }

  renderCrumbs() {
    const list = this.pane?.view?.crumbs ?? [];
    const frag = document.createDocumentFragment();
    list.forEach((c, i) => {
      if (i > 0) frag.append(el("li", "crumb-sep", Icons.ui("chevron")));
      const li = el("li");
      const btn = el("button", "crumb");
      btn.dataset.path = c.path;
      btn.title = c.path;
      if (c.icon) btn.innerHTML = Icons.ui(c.icon === "root" ? "drive" : c.icon);
      if (c.label !== "/") btn.append(c.label);
      if (i === list.length - 1) btn.setAttribute("aria-current", "page");
      li.append(btn);
      frag.append(li);
    });
    this.els.crumbs.replaceChildren(frag);
    this.els.crumbs.scrollLeft = this.els.crumbs.scrollWidth; // keep the deepest part visible
  }

  editLocation() {
    const { crumbs, input } = this.els;
    crumbs.hidden = true;
    input.hidden = false;
    input.value = this.pane?.path ?? "";
    input.focus();
    input.select();
  }

  endEditLocation() {
    const { crumbs, input } = this.els;
    input.hidden = true;
    crumbs.hidden = false;
    crumbs.scrollLeft = crumbs.scrollWidth;
  }

  // ---------- Places sidebar ----------

  async loadPlaces() {
    this.renderPlaces(await invoke("get_places"));
  }

  /** Sidebar: places, network shares, and devices with usage bars and eject buttons. */
  renderPlaces(places) {
    this.places = places;
    const titles = { places: "Places", remote: "Remote", devices: "Devices" };
    const frag = document.createDocumentFragment();
    for (const section of ["places", "remote", "devices"]) {
      const items = places.filter((p) => p.section === section);
      if (!items.length) continue;
      frag.append(Object.assign(el("div", "side-heading"), { textContent: titles[section] }));
      for (const p of items) {
        const row = el("div", `place${p.mounted ? "" : " unmounted"}`, Icons.ui(p.icon));
        row.setAttribute("role", "button");
        row.dataset.path = p.path;
        const text = el("div", "place-text");
        text.append(Object.assign(el("span", "place-label"), { textContent: p.label }));
        if (p.usage?.total) {
          const used = 1 - p.usage.free / p.usage.total;
          const bar = el("div", `usage${used > 0.9 ? " full" : ""}`, `<div style="width:${(used * 100).toFixed(1)}%"></div>`);
          text.append(bar);
          row.title = `${p.label}: ${Fmt.bytes(p.usage.free)} free of ${Fmt.bytes(p.usage.total)}`;
        } else {
          row.title = p.mounted ? p.path : `${p.label} (${p.device}) — click to mount`;
        }
        row.append(text);
        if (p.ejectable) {
          const eject = el("button", "place-eject", Icons.ui("eject"));
          eject.title = p.section === "remote" ? "Disconnect" : "Eject";
          row.append(eject);
        }
        frag.append(row);
      }
    }
    this.els.sidebar.replaceChildren(frag);
    for (const b of this.els.sidebar.querySelectorAll(".place")) b.classList.toggle("active", b.dataset.path === this.pane?.path);
  }

  placeAt(node) {
    const row = node.closest(".place");
    return row && this.places.find((p) => p.path === row.dataset.path);
  }

  /** Attached but unmounted drive: mount it, then open it. */
  async mountPlace(place) {
    this.flashStatus(`Mounting “${place.label}”…`);
    try {
      const path = await invoke("mount_device", { device: place.device });
      await this.loadPlaces();
      await this.pane.navigate(path);
    } catch (err) {
      this.pane.showMessage(errMsg(err));
    }
  }

  async ejectPlace(place) {
    // Leave the volume first so nothing in Sonar keeps it busy.
    const inside = (p) => p.path === place.path || p.path?.startsWith(`${place.path}/`);
    const moves = [];
    this.forEachPane((p) => { if (inside(p)) moves.push(p.navigate(this.home)); });
    await Promise.all(moves);
    this.flashStatus(`Ejecting “${place.label}”…`);
    try {
      await invoke("eject_volume", { path: place.path });
      this.flashStatus(place.section === "remote" ? `Disconnected “${place.label}”` : `“${place.label}” can be removed safely`);
      await this.loadPlaces();
    } catch (err) {
      this.pane.showMessage(errMsg(err));
    }
  }

  initSidebar() {
    const { sidebar, splitter } = this.els;
    sidebar.addEventListener("click", (ev) => {
      const place = this.placeAt(ev.target);
      if (!place) return;
      if (ev.target.closest(".place-eject")) this.ejectPlace(place);
      else if (!place.mounted) this.mountPlace(place);
      else this.pane.navigate(place.path);
    });
    sidebar.addEventListener("auxclick", (ev) => {
      const place = this.placeAt(ev.target);
      if (place?.mounted && ev.button === 1) this.newTab(place.path, { activate: false });
    });
    sidebar.addEventListener("contextmenu", (ev) => {
      ev.preventDefault();
      const place = this.placeAt(ev.target);
      if (!place) return;
      const items = place.mounted ? locationMenu(this, place.path) : [{ label: "Mount", action: () => this.mountPlace(place) }];
      if (place.ejectable) items.push("-", { label: place.section === "remote" ? "Disconnect" : "Eject", action: () => this.ejectPlace(place) });
      showMenu(ev.clientX, ev.clientY, items);
    });
    // Drives attached, mounted or removed elsewhere (backend polls).
    listen("places-changed", ({ payload }) => this.renderPlaces(payload));

    // Resizable sidebar.
    const root = document.documentElement;
    root.style.setProperty("--sidebar-w", `${store.get("sidebarWidth", 200)}px`);
    splitter.addEventListener("pointerdown", (ev) => {
      splitter.setPointerCapture(ev.pointerId);
      splitter.classList.add("dragging");
      const move = (e) => {
        const w = Math.min(Math.max(e.clientX, 120), window.innerWidth * 0.5);
        root.style.setProperty("--sidebar-w", `${w}px`);
      };
      const up = (e) => {
        splitter.classList.remove("dragging");
        splitter.removeEventListener("pointermove", move);
        splitter.removeEventListener("pointerup", up);
        store.set("sidebarWidth", Math.round(parseFloat(getComputedStyle(root).getPropertyValue("--sidebar-w"))));
        splitter.releasePointerCapture(e.pointerId);
      };
      splitter.addEventListener("pointermove", move);
      splitter.addEventListener("pointerup", up);
    });
  }

  // ---------- Keyboard ----------

  initKeyboard() {
    window.addEventListener("keydown", (ev) => {
      if (isMenuOpen()) return; // the menu owns the keyboard
      // Text fields and dialogs handle their own keys.
      if (ev.target.closest?.("input, textarea") || document.querySelector("dialog[open]")) return;
      if (!this.pane) return;
      const handled = this.globalKey(ev) || this.ops.keydown(ev) || this.pane.keydown(ev);
      if (handled) this.lastKeyHandled = performance.now();
    });
  }

  /** Window-wide shortcuts. Returns true if handled. */
  globalKey(ev) {
    const pane = this.pane;
    const mod = isMod(ev);
    const k = ev.key;
    const lower = k.toLowerCase();
    const run = (fn) => { ev.preventDefault(); fn(); return true; };

    // History and navigation.
    if ((ev.altKey && k === "ArrowLeft") || (isMac && ev.metaKey && !ev.shiftKey && k === "[")) return run(() => pane.goBack());
    if ((ev.altKey && k === "ArrowRight") || (isMac && ev.metaKey && !ev.shiftKey && k === "]")) return run(() => pane.goForward());
    if ((ev.altKey && k === "ArrowUp") || (isMac && ev.metaKey && k === "ArrowUp")) return run(() => pane.goUp());
    if (ev.altKey && k === "Home") return run(() => pane.navigate(this.home));
    if (k === "F5" || (mod && lower === "r")) return run(() => pane.reload());
    if (k === "F6" || (mod && lower === "l")) return run(() => this.editLocation());

    // Tabs.
    if (mod && !ev.shiftKey && lower === "t") return run(() => this.newTab(pane.path ?? this.home));
    if (mod && ev.shiftKey && lower === "t") return run(() => this.reopenClosedTab());
    if (mod && !ev.shiftKey && lower === "w") return run(() => this.closeCurrent());
    if (ev.ctrlKey && k === "Tab") return run(() => this.cycleTab(ev.shiftKey ? -1 : 1));
    if (isMac && ev.metaKey && ev.shiftKey && ev.code === "BracketRight") return run(() => this.cycleTab(1));
    if (isMac && ev.metaKey && ev.shiftKey && ev.code === "BracketLeft") return run(() => this.cycleTab(-1));

    // Split view, view modes, filter, hidden files.
    if (k === "F3") return run(() => this.toggleSplit());
    if (mod && !ev.shiftKey && ev.code === "Digit1") return run(() => this.setViewMode("icons"));
    if (mod && !ev.shiftKey && (ev.code === "Digit2" || ev.code === "Digit3")) return run(() => this.setViewMode("details"));
    // Search: ⌘F / Ctrl+F (Finder, Dolphin). Filter: Ctrl+I (Dolphin) / ⌥⌘F (⌘I is Get Info on macOS).
    if (mod && !ev.altKey && !ev.shiftKey && lower === "f") return run(() => pane.showSearch());
    if ((!isMac && ev.ctrlKey && lower === "i") || (isMac && ev.metaKey && ev.altKey && ev.code === "KeyF")) return run(() => pane.showFilter());
    // Match the physical key for "." : with Shift held, ev.key is ">" on most layouts.
    if ((ev.ctrlKey && lower === "h") || (isMac && ev.metaKey && ev.shiftKey && ev.code === "Period") || (ev.altKey && ev.code === "Period"))
      return run(() => this.toggleHidden());
    return false;
  }
}
