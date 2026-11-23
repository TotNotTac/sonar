// Context menu contents. Each builder returns items for menu.js.

import { invoke, isMac, shortcut } from "./util.js";

const keys = {
  newFolder: shortcut("mod+shift+N"),
  cut: shortcut("mod+X"),
  copy: shortcut("mod+C"),
  paste: shortcut("mod+V"),
  undo: shortcut("mod+Z"),
  trash: isMac ? shortcut("mod+Backspace") : "Delete",
  deletePermanently: isMac ? shortcut("mod+alt+Backspace") : "Shift+Delete",
  copyLocation: shortcut("mod+alt+C"),
  pasteMove: shortcut("mod+alt+V"),
  hidden: isMac ? shortcut("mod+shift+.") : "Ctrl+H",
  filter: isMac ? shortcut("mod+alt+F") : "Ctrl+I",
  search: shortcut("mod+F"),
  properties: isMac ? shortcut("mod+I") : "Alt+Enter",
  icons: shortcut("mod+1"),
  details: isMac ? shortcut("mod+2") : "Ctrl+3",
  newTab: shortcut("mod+T"),
  closeTab: shortcut("mod+W"),
  reopenTab: shortcut("mod+shift+T"),
};
export { keys as shortcuts };

/** `info`: `{ undoLabel, canPaste }`, looked up before the menu opens. */
export function itemsMenu(app, pane, info = {}) {
  const { ops } = app;
  const sel = pane.selectedEntries();
  if (!sel.length) return backgroundMenu(app, pane, info);
  const one = sel.length === 1 ? sel[0] : null;
  const folder = one && pane.isFolder(one) ? one : null;
  const inTrash = ops.inTrash(pane);
  const archive = pane.view?.archive;
  if (archive) {
    // Inside an archive: read-only, so only opening and copying out.
    const outside = archive.replace(/\/[^/]*$/, "") || "/";
    return [
      { label: "Open", shortcut: "Enter", action: () => pane.openSelection() },
      "-",
      { label: `Extract to “${app.folderLabel(outside)}”`, action: () => ops.extractSelectionOut(pane) },
      { label: "Copy", shortcut: keys.copy, action: () => ops.setClipboard("copy", pane) },
      { label: "Copy Location", shortcut: keys.copyLocation, action: () => ops.copyLocation(pane) },
      "-",
      { label: "Properties", shortcut: keys.properties, action: () => app.runAction("properties") },
    ];
  }
  const archives = sel.every((e) => e.archive);
  const other = app.activeTab.panes.length === 2 ? app.activeTab.panes.find((p) => p !== pane) : null;
  const compress = {
    label: "Compress",
    submenu: [["zip", "as ZIP"], ["sevenZip", "as 7-Zip"], ["tarGz", "as tar.gz"], ["tarXz", "as tar.xz"], ["tarBz2", "as tar.bz2"]]
      .map(([format, label]) => ({ label, action: () => ops.compress(format, pane) })),
  };
  return [
    { label: "Open", shortcut: "Enter", action: () => pane.openSelection() },
    pane.search && one && {
      label: "Open Containing Folder",
      action: () => pane.navigate(one.path.slice(0, one.path.lastIndexOf("/")) || "/", { select: one.name }),
    },
    folder && { label: "Open in New Tab", action: () => app.newTab(folder.path, { activate: false }) },
    folder && { label: "Open in Split View", action: () => app.openInSplit(folder.path) },
    archives && { label: "Open With Default App", action: () => sel.forEach((e) => invoke("open_path", { path: e.path }).catch(() => {})) },
    "-",
    archives && { label: "Extract Here", action: () => ops.extract(pane) },
    archives && other?.path && !other.view?.archive && { label: `Extract to “${app.folderLabel(other.path)}”`, action: () => ops.extract(pane, other.path) },
    compress,
    "-",
    { label: "Cut", shortcut: keys.cut, action: () => ops.setClipboard("cut", pane) },
    { label: "Copy", shortcut: keys.copy, action: () => ops.setClipboard("copy", pane) },
    folder && { label: "Paste Into Folder", disabled: !info.canPaste, action: () => ops.paste(folder.path) },
    { label: "Copy Location", shortcut: keys.copyLocation, action: () => ops.copyLocation(pane) },
    "-",
    { label: "Rename…", shortcut: "F2", disabled: !one, action: () => ops.beginRename(pane) },
    inTrash
      ? { label: "Delete Permanently…", shortcut: keys.trash, danger: true, action: () => ops.removeSelected({}, pane) }
      : { label: "Move to Trash", shortcut: keys.trash, action: () => ops.removeSelected({}, pane) },
    !inTrash && {
      label: "Delete Permanently…",
      shortcut: keys.deletePermanently,
      danger: true,
      action: () => ops.removeSelected({ permanent: true }, pane),
    },
    "-",
    { label: "Properties", shortcut: keys.properties, action: () => app.runAction("properties") },
  ];
}

export function backgroundMenu(app, pane, { undoLabel = null, canPaste = false } = {}) {
  const { ops } = app;
  const { viewMode, sort } = pane.props;
  const sortBy = (key) => ({
    label: { name: "Name", size: "Size", modified: "Modified", type: "Type" }[key],
    checked: sort.key === key,
    action: () => pane.setSort({ key, order: sort.order }),
  });
  const order = (o, label) => ({ label, checked: sort.order === o, action: () => pane.setSort({ key: sort.key, order: o }) });
  const split = app.activeTab.panes.length === 2;
  return [
    { label: "New Folder…", shortcut: keys.newFolder, disabled: !!pane.view?.archive, action: () => ops.newFolder(pane) },
    { label: "Paste", shortcut: keys.paste, disabled: !canPaste || !!pane.view?.archive, action: () => ops.paste(pane.path) },
    isMac && canPaste && { label: "Move Here", shortcut: keys.pasteMove, action: () => ops.paste(pane.path, { move: true }) },
    { label: undoLabel ? `Undo ${undoLabel}` : "Undo", shortcut: keys.undo, disabled: !undoLabel, action: () => ops.undo() },
    "-",
    {
      label: "View Mode",
      submenu: [
        { label: "Icons", shortcut: keys.icons, checked: viewMode === "icons", action: () => pane.setViewMode("icons") },
        { label: "Details", shortcut: keys.details, checked: viewMode === "details", action: () => pane.setViewMode("details") },
      ],
    },
    {
      label: "Sort By",
      submenu: [sortBy("name"), sortBy("size"), sortBy("modified"), sortBy("type"), "-",
        order("ascending", "Ascending"), order("descending", "Descending")],
    },
    { label: "Show Hidden Files", shortcut: keys.hidden, checked: app.showHidden, action: () => app.toggleHidden() },
    "-",
    { label: "Copy Location", shortcut: keys.copyLocation, action: () => ops.copyLocation(pane) },
    { label: "Search…", shortcut: keys.search, action: () => pane.showSearch() },
    { label: "Filter…", shortcut: keys.filter, action: () => pane.showFilter() },
    { label: "Open in New Tab", shortcut: keys.newTab, action: () => app.newTab(pane.path) },
    { label: split ? "Close Split View" : "Split View", shortcut: "F3", action: () => app.toggleSplit() },
    "-",
    { label: "Properties", shortcut: keys.properties, action: () => app.runAction("properties") },
  ];
}

/** The ☰ application menu (Linux; macOS uses the native menu bar). */
export function appMenu(app) {
  const pane = app.pane;
  const run = (id) => () => app.runAction(id);
  const split = app.activeTab.panes.length === 2;
  return [
    { label: "New Tab", shortcut: keys.newTab, action: run("new-tab") },
    { label: "New Folder…", shortcut: keys.newFolder, action: run("new-folder") },
    { label: "Reopen Closed Tab", shortcut: keys.reopenTab, disabled: !app.closedTabs.length, action: run("reopen-tab") },
    "-",
    {
      label: "View Mode",
      submenu: [
        { label: "Icons", shortcut: keys.icons, checked: pane.props.viewMode === "icons", action: run("view-icons") },
        { label: "Details", shortcut: keys.details, checked: pane.props.viewMode === "details", action: run("view-details") },
      ],
    },
    { label: "Show Hidden Files", shortcut: keys.hidden, checked: app.showHidden, action: run("show-hidden") },
    { label: "Split View", shortcut: "F3", checked: split, action: run("split") },
    { label: "Search…", shortcut: keys.search, action: run("search") },
    { label: "Filter…", shortcut: keys.filter, action: run("filter") },
    "-",
    {
      label: "Go",
      submenu: [
        { label: "Back", disabled: !pane.canGoBack(), action: run("back") },
        { label: "Forward", disabled: !pane.canGoForward(), action: run("forward") },
        { label: "Up", disabled: !pane.view?.parent, action: run("up") },
        "-",
        { label: "Home", action: run("go-home") },
        { label: "Desktop", action: run("go-desktop") },
        { label: "Documents", action: run("go-documents") },
        { label: "Downloads", action: run("go-downloads") },
        { label: "Trash", action: run("go-trash") },
      ],
    },
    { label: "Copy Location", shortcut: keys.copyLocation, action: run("copy-location") },
    "-",
    { label: "Close Window", action: run("close-window") },
  ];
}

/** For places in the sidebar and breadcrumb segments. */
export function locationMenu(app, path) {
  return [
    { label: "Open", action: () => app.pane.navigate(path) },
    { label: "Open in New Tab", action: () => app.newTab(path, { activate: false }) },
    { label: "Open in Split View", action: () => app.openInSplit(path) },
  ];
}

export function tabMenu(app, tab) {
  const several = app.tabs.length > 1;
  const isLast = app.tabs.indexOf(tab) === app.tabs.length - 1;
  const paths = tab.panes.map((p) => p.path ?? p.initialPath);
  return [
    { label: "New Tab", shortcut: keys.newTab, action: () => app.newTab(paths[tab.active]) },
    { label: "Duplicate Tab", action: () => app.activateTab(app.addTab(paths, { active: tab.active, index: app.tabs.indexOf(tab) + 1 })) },
    { label: "Reopen Closed Tab", shortcut: keys.reopenTab, disabled: !app.closedTabs.length, action: () => app.reopenClosedTab() },
    "-",
    { label: "Close Tab", shortcut: keys.closeTab, disabled: !several, action: () => app.closeTab(tab) },
    { label: "Close Other Tabs", disabled: !several, action: () => app.closeOtherTabs(tab) },
    { label: "Close Tabs to the Right", disabled: isLast, action: () => app.closeTabsToRight(tab) },
  ];
}
