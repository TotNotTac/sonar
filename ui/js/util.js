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

export const basename = (p) => p.replace(/\/+$/, "").split("/").pop() || p;
export const parentOf = (p) => p.replace(/\/[^/]*\/?$/, "") || "/";

export const esc = (s) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

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
