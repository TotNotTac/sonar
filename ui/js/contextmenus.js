// Context menu contents. Each builder returns items for menu.js.

import { isMac, shortcut } from "./util.js";

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
  filter: isMac ? shortcut("mod+F") : "Ctrl+I",
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
  return [
    { label: "Open", shortcut: "Enter", action: () => pane.openSelection() },
    folder && { label: "Open in New Tab", action: () => app.newTab(folder.path, { activate: false }) },
    folder && { label: "Open in Split View", action: () => app.openInSplit(folder.path) },
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
    { label: "New Folder…", shortcut: keys.newFolder, action: () => ops.newFolder(pane) },
    { label: "Paste", shortcut: keys.paste, disabled: !canPaste, action: () => ops.paste(pane.path) },
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
    { label: "Filter…", shortcut: keys.filter, action: () => pane.showFilter() },
    { label: "Open in New Tab", shortcut: keys.newTab, action: () => app.newTab(pane.path) },
    { label: split ? "Close Split View" : "Split View", shortcut: "F3", action: () => app.toggleSplit() },
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
