// Small shared helpers and the Tauri bridge.

const T = window.__TAURI__;
export const invoke = T.core.invoke;
export const listen = T.event.listen;

export const isMac = /Mac/.test(navigator.platform || navigator.userAgent);
/** The platform's primary modifier: ⌘ on macOS, Ctrl elsewhere. */
export const isMod = (ev) => (isMac ? ev.metaKey : ev.ctrlKey);
export const modLabel = isMac ? "⌘" : "Ctrl+";

/** Shortcut text for menus/tooltips, e.g. shortcut("mod+shift+N") -> "⌘⇧N" / "Ctrl+Shift+N". */
export function shortcut(spec) {
  const parts = spec.split("+");
  const key = parts.pop();
  if (isMac) {
    const sym = { mod: "⌘", shift: "⇧", alt: "⌥", ctrl: "⌃" };
    const keys = { Backspace: "⌫", Delete: "⌦", Up: "↑", Down: "↓", Left: "←", Right: "→" };
    return parts.map((p) => sym[p]).join("") + (keys[key] ?? key);
  }
  const names = { mod: "Ctrl", shift: "Shift", alt: "Alt", ctrl: "Ctrl" };
  return [...parts.map((p) => names[p]), key].join("+");
}

/** URL for the app's `sonar:` scheme (OS icons, thumbnails). */
// Windows webviews expose custom schemes as http://<scheme>.localhost instead.
const ASSET_BASE = /^Win/.test(navigator.platform) ? "http://sonar.localhost" : "sonar://localhost";
export function assetUrl(kind, params) {
  return `${ASSET_BASE}/${kind}?${new URLSearchParams(params).toString().replace(/\+/g, "%20")}`;
}

export const basename = (p) => p.replace(/\/+$/, "").split("/").pop() || p;
export const parentOf = (p) => p.replace(/\/[^/]*\/?$/, "") || "/";

// Names and paths from the backend carry bytes that aren't valid UTF-8 as
// characters U+10FF00–U+10FFFF (see sonar-core's uipath.rs). They stay that
// way wherever paths are used, and show as U+FFFD wherever text is displayed.
const ENCODED_BYTE = /[\u{10FF00}-\u{10FFFF}]/gu;
export const shown = (s) => (s == null ? s : String(s).replace(ENCODED_BYTE, "\uFFFD"));

export const esc = (s) =>
  shown(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

export const plural = (n, one, many = `${one}s`) => (n === 1 ? `1 ${one}` : `${n} ${many}`);
export const errMsg = (err) => err?.message ?? String(err);

/** First path component of `descendant` below `ancestor`, or null. */
export function childNameOnPath(ancestor, descendant) {
  if (!ancestor || !descendant || ancestor === descendant) return null;
  const prefix = ancestor.endsWith("/") ? ancestor : `${ancestor}/`;
  if (!descendant.startsWith(prefix)) return null;
  return descendant.slice(prefix.length).split("/")[0] || null;
}

export function el(tag, className, html) {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (html != null) e.innerHTML = html;
  return e;
}
