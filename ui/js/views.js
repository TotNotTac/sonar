// View renderers. Both draw `pane.items` into the pane's scroller and expose
// the same small interface; the Pane owns selection, keyboard and data.
//
//   render()        redraw all items
//   item(i)         element for item i (carries data-i)
//   label(i)        element holding item i's name (for inline rename)
//   setCount(i, n)  update a folder's item count
//   columns()       items per row, for arrow-key movement

import { Fmt } from "./format.js";
import { Icons } from "./icons.js";
import { settings } from "./store.js";
import { assetUrl, esc } from "./util.js";

const COLUMNS = [
  { key: "name", label: "Name" },
  { key: "size", label: "Size", num: true },
  { key: "modified", label: "Modified" },
  { key: "type", label: "Type" },
];

function itemClasses(pane, e) {
  const c = [];
  if (e.isHidden) c.push("hidden-entry");
  if (pane.isCut(e)) c.push("cut");
  return c.length ? ` class="${c.join(" ")}"` : "";
}

/**
 * The built-in SVG, with the OS icon (or a thumbnail) layered on top. The
 * image is lazy-loaded; if it fails, the pane removes it and the SVG remains.
 */
function entryIcon(e, size, { thumbnail = false } = {}) {
  const badge = e.isSymlink ? `<span class="link-badge">${Icons.ui("link")}</span>` : "";
  let img = "";
  if (thumbnail && e.thumbnail) {
    img = `<img class="os-icon thumb" loading="lazy" decoding="async" alt="" src="${esc(assetUrl("thumb", { path: e.path, size: size * 2 }))}">`;
  } else if (e.iconKey) {
    img = `<img class="os-icon" loading="lazy" decoding="async" alt="" src="${esc(assetUrl("icon", { key: e.iconKey, size }))}">`;
  }
  return `${Icons.entry(e.icon)}${img}${badge}`;
}

const sizeText = (e) => (e.kind === "directory" ? Fmt.items(e.childCount) : Fmt.bytes(e.size));

export class DetailsView {
  constructor(pane, host) {
    this.pane = pane;
    this.root = document.createElement("table");
    this.root.className = "details";
    this.root.innerHTML = `
      <colgroup><col class="c-name" /><col class="c-size" /><col class="c-modified" /><col class="c-type" /></colgroup>
      <thead><tr>${COLUMNS.map((c) => `<th data-key="${c.key}"${c.num ? ' class="num"' : ""}>${c.label}</th>`).join("")}</tr></thead>
      <tbody></tbody>`;
    this.body = this.root.querySelector("tbody");
    this.root.querySelector("thead").addEventListener("click", (ev) => {
      const th = ev.target.closest("th");
      if (th) pane.toggleSort(th.dataset.key);
    });
    host.prepend(this.root);
  }

  render() {
    const pane = this.pane;
    this.body.innerHTML = pane.items.map((e, i) => `<tr data-i="${i}"${itemClasses(pane, e)}>
        <td class="name" title="${esc(e.name)}"><div class="name-cell"><span class="ficon">${entryIcon(e, 18)}</span><span class="name-label">${esc(e.name)}</span></div></td>
        <td class="num">${sizeText(e)}</td>
        <td>${Fmt.date(e.modified)}</td>
        ${pane.search
          ? `<td class="location" title="${esc(e.path)}">${esc(pane.relativeFolder(e.path))}</td>`
          : `<td title="${esc(e.typeLabel)}">${esc(e.typeLabel)}</td>`}
      </tr>`).join("");
    // In search results the 4th column shows where each result is.
    const typeHeader = this.root.querySelector('th[data-key="type"]');
    typeHeader.textContent = pane.search ? "Location" : "Type";
    const { key, order } = (pane.search ? pane.searchSort : pane.props.sort) ?? {};
    for (const th of this.root.querySelectorAll("th")) {
      if (th.dataset.key === key) th.setAttribute("aria-sort", order);
      else th.removeAttribute("aria-sort");
    }
  }

  item(i) { return this.body.children[i]; }
  label(i) { return this.item(i)?.querySelector(".name-label"); }
  columns() { return 1; }
  rowsPerPage(viewport) { return Math.max(1, Math.floor(viewport / (this.item(0)?.offsetHeight || 26)) - 1); }

  setCount(i, count) {
    const cell = this.item(i)?.children[1];
    if (cell) cell.textContent = Fmt.items(count);
  }

  dispose() { this.root.remove(); }
}

export class IconsView {
  constructor(pane, host) {
    this.pane = pane;
    this.root = document.createElement("div");
    this.root.className = "grid";
    host.prepend(this.root);
  }

  render() {
    const pane = this.pane;
    const { thumbnails: thumbnail } = settings();
    this.root.innerHTML = pane.items.map((e, i) => `<div data-i="${i}"${itemClasses(pane, e)} title="${esc(pane.search ? e.path : e.name)}">
        <span class="ficon big">${entryIcon(e, 48, { thumbnail })}</span>
        <span class="tile-label">${esc(e.name)}</span>
      </div>`).join("");
  }

  item(i) { return this.root.children[i]; }
  label(i) { return this.item(i)?.querySelector(".tile-label"); }
  setCount() {}

  columns() {
    const kids = this.root.children;
    if (!kids.length) return 1;
    const top = kids[0].offsetTop;
    let n = 0;
    while (n < kids.length && kids[n].offsetTop === top) n++;
    return Math.max(1, n);
  }

  rowsPerPage(viewport) {
    const h = this.item(0)?.offsetHeight || 100;
    return Math.max(1, Math.floor(viewport / h) - 1) * this.columns();
  }

  dispose() { this.root.remove(); }
}

export const VIEWS = { details: DetailsView, icons: IconsView };
