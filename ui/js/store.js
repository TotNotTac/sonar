// Persistent UI settings (localStorage, per user/webview profile).

export const store = {
  get(key, fallback) {
    try {
      const v = localStorage.getItem(`sonar.${key}`);
      return v == null ? fallback : JSON.parse(v);
    } catch {
      return fallback;
    }
  },
  set(key, value) {
    try {
      localStorage.setItem(`sonar.${key}`, JSON.stringify(value));
    } catch {
      /* storage full or unavailable: settings just won't persist */
    }
  },
};

// ---------- Sidebar customisations ----------
//
// Layered over the system places, so new drives and user folders still show up:
// custom entries, hidden ones, renamed ones, and the order of the Places section.

export function placePrefs() {
  const p = store.get("places", {});
  return { custom: p.custom ?? [], hidden: p.hidden ?? [], renamed: p.renamed ?? {}, order: p.order ?? [] };
}

export function savePlacePrefs(prefs) {
  store.set("places", prefs);
}

// ---------- Settings ----------
//
// The Configure Sonar dialog. Hidden-file visibility keeps its own key: it is
// also toggled from the toolbar and the shortcut.

export const SETTING_DEFAULTS = Object.freeze({
  startup: "session", // "session" | "home" | "folder"
  startupFolder: "",
  viewMode: "details",
  sort: Object.freeze({ key: "name", order: "ascending" }),
  rememberViewPerFolder: true,
  thumbnails: true,
  confirmTrash: false,
  confirmDelete: true,
});

let cached = null;

export function settings() {
  cached ??= { ...SETTING_DEFAULTS, ...store.get("settings", {}) };
  return cached;
}

/** Merge `changes` into the settings. Only values that differ from the defaults are stored. */
export function saveSettings(changes) {
  cached = { ...settings(), ...changes };
  const stored = {};
  for (const [k, v] of Object.entries(cached)) {
    if (JSON.stringify(v) !== JSON.stringify(SETTING_DEFAULTS[k])) stored[k] = v;
  }
  store.set("settings", stored);
}

export function resetSettings() {
  cached = null;
  store.set("settings", {});
}

// ---------- Per-folder view properties ----------
//
// Like Dolphin's "remember display style for each folder": view mode and sort
// order are stored per folder, falling back to the defaults from Settings.
// With that option off, every folder shares the defaults and changing the
// view changes them. Hidden-file visibility stays global (it is usually a
// "this session" preference, and macOS treats it as global too).

const MAX_FOLDERS = 1000;

const defaultProps = () => {
  const s = settings();
  return { viewMode: s.viewMode, sort: { ...s.sort } };
};

export function viewPropsFor(path) {
  const defaults = defaultProps();
  if (!settings().rememberViewPerFolder) return defaults;
  const saved = store.get("viewProps", {})[path];
  return {
    viewMode: saved?.viewMode ?? defaults.viewMode,
    sort: { ...defaults.sort, ...saved?.sort },
  };
}

export function saveViewProps(path, props) {
  if (!settings().rememberViewPerFolder) {
    saveSettings({ viewMode: props.viewMode, sort: { ...props.sort } });
    return;
  }
  const defaults = defaultProps();
  const all = store.get("viewProps", {});
  delete all[path]; // re-insert so key order doubles as recency
  const isDefault = props.viewMode === defaults.viewMode
    && props.sort.key === defaults.sort.key
    && props.sort.order === defaults.sort.order;
  if (!isDefault) all[path] = { viewMode: props.viewMode, sort: props.sort };
  const keys = Object.keys(all);
  for (const k of keys.slice(0, Math.max(0, keys.length - MAX_FOLDERS))) delete all[k];
  store.set("viewProps", all);
}
