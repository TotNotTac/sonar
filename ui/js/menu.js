// Context menus rendered in HTML: consistent on macOS and Linux, keyboard
// navigable, with one level of submenus.
//
// Item shape: { label, action?, shortcut?, checked?, disabled?, danger?, submenu? }
// or the string "-" for a separator.

import { Icons } from "./icons.js";

let current = null; // { root, close }

export const isMenuOpen = () => current != null;

export function closeMenu() {
  current?.close();
}

/** Open a menu at viewport coordinates. Replaces any open menu. */
export function showMenu(x, y, items) {
  closeMenu();
  const layer = document.createElement("div");
  layer.className = "menu-layer";
  document.body.append(layer);

  const menus = []; // stack: [root, submenu?]
  const close = () => {
    layer.remove();
    document.removeEventListener("keydown", onKey, true);
    window.removeEventListener("blur", close);
    window.removeEventListener("resize", close);
    current = null;
  };

  function build(list, level) {
    const m = document.createElement("div");
    m.className = "menu";
    m.setAttribute("role", "menu");
    for (const it of list.filter(Boolean)) {
      if (it === "-") {
        m.append(Object.assign(document.createElement("div"), { className: "menu-sep" }));
        continue;
      }
      const row = document.createElement("div");
      row.className = "menu-item";
      row.setAttribute("role", it.checked != null ? "menuitemcheckbox" : "menuitem");
      if (it.disabled) row.classList.add("disabled");
      if (it.danger) row.classList.add("danger");
      row.innerHTML = `<span class="menu-check">${it.checked ? Icons.ui("check") : ""}</span>
        <span class="menu-label"></span>
        <span class="menu-shortcut"></span>
        <span class="menu-arrow">${it.submenu ? Icons.ui("chevron") : ""}</span>`;
      row.querySelector(".menu-label").textContent = it.label;
      row.querySelector(".menu-shortcut").textContent = it.shortcut ?? "";
      row._item = it;
      row.addEventListener("mouseenter", () => highlight(level, row, true));
      row.addEventListener("click", (ev) => {
        ev.stopPropagation();
        activate(level, row);
      });
      m.append(row);
    }
    return m;
  }

  function place(m, px, py, anchor) {
    layer.append(m);
    const r = m.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let left = px;
    let top = py;
    if (anchor) {
      // Submenu: to the right of the parent row, or flip to the left.
      left = anchor.right - 2;
      top = anchor.top - 4;
      if (left + r.width > vw - 4) left = anchor.left - r.width + 2;
    } else if (left + r.width > vw - 4) {
      left = vw - r.width - 4;
    }
    if (top + r.height > vh - 4) top = Math.max(4, vh - r.height - 4);
    m.style.left = `${Math.max(4, left)}px`;
    m.style.top = `${Math.max(4, top)}px`;
  }

  const rows = (m) => [...m.querySelectorAll(".menu-item:not(.disabled)")];

  function highlight(level, row, openSub) {
    const m = menus[level];
    m.querySelectorAll(".menu-item.hot").forEach((r) => r.classList.remove("hot"));
    row?.classList.add("hot");
    // Close deeper menus, then open this row's submenu on hover.
    while (menus.length > level + 1) menus.pop().remove();
    if (openSub && row?._item.submenu && !row.classList.contains("disabled")) openSubmenu(level, row);
  }

  function openSubmenu(level, row) {
    const sub = build(row._item.submenu, level + 1);
    menus.push(sub);
    place(sub, 0, 0, row.getBoundingClientRect());
  }

  function activate(level, row) {
    const it = row._item;
    if (it.disabled) return;
    if (it.submenu) {
      highlight(level, row, true);
      const first = rows(menus[level + 1])[0];
      if (first) highlight(level + 1, first, false);
      return;
    }
    close();
    it.action?.();
  }

  function onKey(ev) {
    ev.stopPropagation(); // the menu owns the keyboard while open
    const level = menus.length - 1;
    const m = menus[level];
    const list = rows(m);
    const hot = m.querySelector(".menu-item.hot");
    const idx = list.indexOf(hot);
    // With nothing highlighted yet, Down starts at the top and Up at the bottom.
    const move = (d) => highlight(level, idx < 0 ? list.at(d > 0 ? 0 : -1) : list[(idx + d + list.length) % list.length], false);
    switch (ev.key) {
      case "Escape":
        ev.preventDefault();
        if (level > 0) menus.pop().remove();
        else close();
        break;
      case "ArrowDown": ev.preventDefault(); move(1); break;
      case "ArrowUp": ev.preventDefault(); move(-1); break;
      case "ArrowRight":
        ev.preventDefault();
        if (hot?._item.submenu) activate(level, hot);
        break;
      case "ArrowLeft":
        ev.preventDefault();
        if (level > 0) menus.pop().remove();
        break;
      case "Enter":
      case " ":
        ev.preventDefault();
        if (hot) activate(level, hot);
        break;
      default:
        ev.preventDefault();
    }
  }

  const root = build(items, 0);
  menus.push(root);
  place(root, x, y);
  layer.addEventListener("mousedown", (ev) => {
    if (ev.target === layer) close();
  });
  layer.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    if (ev.target === layer) close();
  });
  document.addEventListener("keydown", onKey, true);
  window.addEventListener("blur", close);
  window.addEventListener("resize", close);
  current = { root, close };
}
