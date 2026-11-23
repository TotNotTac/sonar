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

// ---------- Per-folder view properties ----------
//
// Like Dolphin's "remember display style for each folder": view mode and sort
// order are stored per folder. Hidden-file visibility stays global (it is
// usually a "this session" preference, and macOS treats it as global too).

const DEFAULT_PROPS = { viewMode: "details", sort: { key: "name", order: "ascending" } };
const MAX_FOLDERS = 1000;

export function viewPropsFor(path) {
  const saved = store.get("viewProps", {})[path];
  return {
    viewMode: saved?.viewMode ?? DEFAULT_PROPS.viewMode,
    sort: { ...DEFAULT_PROPS.sort, ...saved?.sort },
  };
}

export function saveViewProps(path, props) {
  const all = store.get("viewProps", {});
  delete all[path]; // re-insert so key order doubles as recency
  const isDefault = props.viewMode === DEFAULT_PROPS.viewMode
    && props.sort.key === DEFAULT_PROPS.sort.key
    && props.sort.order === DEFAULT_PROPS.sort.order;
  if (!isDefault) all[path] = { viewMode: props.viewMode, sort: props.sort };
  const keys = Object.keys(all);
  for (const k of keys.slice(0, Math.max(0, keys.length - MAX_FOLDERS))) delete all[k];
  store.set("viewProps", all);
}
