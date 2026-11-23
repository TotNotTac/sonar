// View renderers. Both draw `pane.items` into the pane's scroller and expose
// the same small interface; the Pane owns selection, keyboard and data.
//
// Only the items in and near the viewport are in the DOM (a folder can hold
// 100k files). The rendered ones are a contiguous range, grown and trimmed at
// its ends while scrolling, so existing elements (an open rename field) stay.
//
//   render()           redraw from pane.items (after the items changed)
//   item(i)            element for item i if it is rendered (carries data-i)
//   label(i)           element holding item i's name (for inline rename)
//   reveal(i, block)   scroll item i into view ("nearest" or "center") and render it
//   rendered()         [index, element] pairs currently in the DOM
//   indicesIn(rect)    items overlapping a rectangle in scroll-content coordinates
//   setCount(i, n)     update a folder's item count
//   columns()          items per row, for arrow-key movement
//   rowsPerPage()      items per screenful, for Page Up/Down

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

/** Items rendered beyond each edge of the viewport, so scrolling stays smooth. */
const OVERSCAN_PX = 600;

function itemClasses(pane, e, i, extra = []) {
  const c = [...extra];
  if (e.isHidden) c.push("hidden-entry");
  if (pane.isCut(e)) c.push("cut");
  if (pane.selected.has(i)) c.push("selected");
  if (pane.focus === i) c.push("focused");
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

/**
 * The windowing shared by both views. Subclasses provide the geometry
 * (`measure()` -> { top, rowHeight, cols, left?, colWidth? }), the markup for
 * a run of items (`html(from, to)`), where new elements go
 * (`insertNodes(nodes, atStart)`), and the space standing in for the items
 * that aren't rendered (`setSpacers(before, after)`, in px).
 */
class VirtualView {
  constructor(pane) {
    this.pane = pane;
    this.start = 0;
    this.nodes = []; // elements for items start .. start + nodes.length
    this.geo = null;
    this.frame = 0;
    this.onScroll = () => {
      if (!this.frame) this.frame = requestAnimationFrame(() => { this.frame = 0; this.update(); });
    };
    pane.scroller.addEventListener("scroll", this.onScroll, { passive: true });
    this.resize = new ResizeObserver(() => {
      const cols = this.geo?.cols;
      this.geo = null;
      // A different column count moves every tile: draw again.
      if (cols != null && this.geometry().cols !== cols) this.render();
      else this.update();
    });
    this.resize.observe(pane.scroller);
  }

  geometry() {
    this.geo ??= this.measure();
    return this.geo;
  }

  rows() { return Math.ceil(this.pane.items.length / this.geometry().cols); }

  /** The items to render: the viewport plus a margin, stretched to keep a row being renamed. */
  wanted() {
    const n = this.pane.items.length;
    const { top, rowHeight, cols } = this.geometry();
    const s = this.pane.scroller;
    const first = Math.floor((s.scrollTop - top - OVERSCAN_PX) / rowHeight);
    const last = Math.ceil((s.scrollTop + s.clientHeight - top + OVERSCAN_PX) / rowHeight);
    let from = Math.min(n, Math.max(0, first * cols));
    let to = Math.min(n, Math.max(0, last * cols));
    const pinned = this.pane.renaming?.index;
    if (pinned != null && pinned < n) {
      from = Math.min(from, pinned);
      to = Math.max(to, pinned + 1);
    }
    return [from, Math.max(from, to)];
  }

  clear() {
    for (const node of this.nodes) node.remove();
    this.nodes = [];
  }

  /** Throw away the rendered items and draw the wanted range from scratch. */
  render() {
    // Work out the range first: once the old rows are gone the content is
    // briefly shorter, and reading the scroll position then would clamp it.
    const s = this.pane.scroller;
    const scrollTop = s.scrollTop;
    this.geo = null;
    const [from, to] = this.wanted();
    this.clear();
    this.start = from;
    this.insert(from, to, false);
    this.geo = null; // row height is measured from a rendered item
    this.spacers();
    if (s.scrollTop !== scrollTop) s.scrollTop = scrollTop;
  }

  /** After scrolling or resizing: add and drop items at the ends of the rendered range. */
  update() {
    const [from, to] = this.wanted();
    let end = this.start + this.nodes.length;
    if (from >= end || to <= this.start) {
      // No overlap (a jump): start over.
      this.clear();
      this.start = from;
      end = from;
    }
    if (from < this.start) this.insert(from, this.start, true);
    if (to > end) this.insert(end, to, false);
    while (this.start < from && this.nodes.length) { this.nodes.shift().remove(); this.start++; }
    while (this.start + this.nodes.length > to && this.nodes.length) this.nodes.pop().remove();
    this.spacers();
  }

  insert(from, to, atStart) {
    if (to <= from) return;
    const tmp = this.container();
    tmp.innerHTML = this.html(from, to);
    const fresh = [...tmp.children];
    this.insertNodes(fresh, atStart);
    if (atStart) {
      this.nodes.unshift(...fresh);
      this.start = from;
    } else {
      this.nodes.push(...fresh);
    }
  }

  spacers() {
    const { rowHeight, cols } = this.geometry();
    const firstRow = Math.floor(this.start / cols);
    const endRow = Math.ceil((this.start + this.nodes.length) / cols);
    this.setSpacers(firstRow * rowHeight, Math.max(0, this.rows() - Math.max(endRow, firstRow)) * rowHeight);
  }

  item(i) { return i >= this.start ? this.nodes[i - this.start] ?? null : null; }

  rendered() { return this.nodes.map((node, k) => [this.start + k, node]); }

  /** Scroll so item `i` is visible, and make sure it is rendered. */
  reveal(i, block = "nearest") {
    if (i < 0 || i >= this.pane.items.length) return;
    const { top, rowHeight, cols } = this.geometry();
    const s = this.pane.scroller;
    const y = top + Math.floor(i / cols) * rowHeight;
    const header = this.stickyHeight();
    if (block === "center") s.scrollTop = y - header - (s.clientHeight - header - rowHeight) / 2;
    else if (y - header < s.scrollTop) s.scrollTop = y - header;
    else if (y + rowHeight > s.scrollTop + s.clientHeight) s.scrollTop = y + rowHeight - s.clientHeight;
    this.update(); // now, not on the next frame: callers want item(i)
  }

  /** Items whose cells overlap `r` (scroll-content coordinates). */
  indicesIn(r) {
    const { top, rowHeight, cols, left = 0, colWidth = Infinity } = this.geometry();
    if (r.bottom <= top || (cols > 1 && r.right <= left)) return [];
    const n = this.pane.items.length;
    const firstRow = Math.max(0, Math.floor((r.top - top) / rowHeight));
    const lastRow = Math.min(this.rows() - 1, Math.floor((r.bottom - top) / rowHeight));
    const firstCol = cols === 1 ? 0 : Math.max(0, Math.floor((r.left - left) / colWidth));
    const lastCol = cols === 1 ? 0 : Math.min(cols - 1, Math.floor((r.right - left) / colWidth));
    const out = [];
    for (let row = firstRow; row <= lastRow; row++) {
      for (let col = firstCol; col <= lastCol; col++) {
        const i = row * cols + col;
        if (i < n) out.push(i);
      }
    }
    return out;
  }

  columns() { return this.geometry().cols; }

  rowsPerPage() {
    const { rowHeight, cols } = this.geometry();
    const view = this.pane.scroller.clientHeight - this.stickyHeight();
    return Math.max(1, Math.floor(view / rowHeight) - 1) * cols;
  }

  stickyHeight() { return 0; }

  setCount(i, count) {
    const node = this.item(i);
    if (node) this.countCell(node, count);
  }

  countCell() {}

  dispose() {
    cancelAnimationFrame(this.frame);
    this.pane.scroller.removeEventListener("scroll", this.onScroll);
    this.resize.disconnect();
    this.root.remove();
  }
}

export class DetailsView extends VirtualView {
  constructor(pane, host) {
    super(pane);
    this.root = document.createElement("table");
    this.root.className = "details";
    this.root.innerHTML = `
      <colgroup><col class="c-name" /><col class="c-size" /><col class="c-modified" /><col class="c-type" /></colgroup>
      <thead><tr>${COLUMNS.map((c) => `<th data-key="${c.key}"${c.num ? ' class="num"' : ""}>${c.label}</th>`).join("")}</tr></thead>
      <tbody><tr class="spacer"></tr><tr class="spacer"></tr></tbody>`;
    this.body = this.root.querySelector("tbody");
    [this.above, this.below] = this.body.children;
    this.root.querySelector("thead").addEventListener("click", (ev) => {
      const th = ev.target.closest("th");
      if (th) pane.toggleSort(th.dataset.key);
    });
    host.prepend(this.root);
  }

  measure() {
    return { top: this.root.offsetTop + this.stickyHeight(), rowHeight: this.nodes[0]?.offsetHeight || 26, cols: 1 };
  }

  stickyHeight() { return this.root.tHead.offsetHeight || 26; }

  container() { return document.createElement("tbody"); }

  insertNodes(nodes, atStart) {
    if (atStart) this.above.after(...nodes);
    else this.below.before(...nodes);
  }

  setSpacers(before, after) {
    this.above.style.height = `${before}px`;
    this.below.style.height = `${after}px`;
    this.above.hidden = !before;
    this.below.hidden = !after;
  }

  html(from, to) {
    const pane = this.pane;
    let out = "";
    for (let i = from; i < to; i++) {
      const e = pane.items[i];
      out += `<tr data-i="${i}"${itemClasses(pane, e, i, i % 2 ? ["alt"] : [])}>
        <td class="name" title="${esc(e.name)}"><div class="name-cell"><span class="ficon">${entryIcon(e, 18)}</span><span class="name-label">${esc(e.name)}</span></div></td>
        <td class="num">${sizeText(e)}</td>
        <td>${Fmt.date(e.modified)}</td>
        ${pane.search
          ? `<td class="location" title="${esc(e.path)}">${esc(pane.relativeFolder(e.path))}</td>`
          : `<td title="${esc(e.typeLabel)}">${esc(e.typeLabel)}</td>`}
      </tr>`;
    }
    return out;
  }

  render() {
    super.render();
    const pane = this.pane;
    // In search results the 4th column shows where each result is.
    const typeHeader = this.root.querySelector('th[data-key="type"]');
    typeHeader.textContent = pane.search ? "Location" : "Type";
    const { key, order } = (pane.search ? pane.searchSort : pane.props.sort) ?? {};
    for (const th of this.root.querySelectorAll("th")) {
      if (th.dataset.key === key) th.setAttribute("aria-sort", order);
      else th.removeAttribute("aria-sort");
    }
  }

  label(i) { return this.item(i)?.querySelector(".name-label"); }

  countCell(node, count) { node.children[1].textContent = Fmt.items(count); }
}

export class IconsView extends VirtualView {
  constructor(pane, host) {
    super(pane);
    this.root = document.createElement("div");
    this.root.className = "grid";
    host.prepend(this.root);
  }

  /**
   * Columns as CSS `repeat(auto-fill, minmax(var(--tile-min), 1fr))` lays them
   * out. Tiles all have the same height (--tile-height), so rows are regular.
   */
  measure() {
    const cs = getComputedStyle(this.root);
    const pad = parseFloat(cs.getPropertyValue("--grid-pad")) || 8;
    const gap = parseFloat(cs.columnGap) || 0;
    const min = parseFloat(cs.getPropertyValue("--tile-min")) || 104;
    const width = this.root.clientWidth - 2 * pad;
    const cols = Math.max(1, Math.floor((width + gap) / (min + gap)));
    return {
      top: this.root.offsetTop + pad,
      rowHeight: (parseFloat(cs.getPropertyValue("--tile-height")) || 96) + (parseFloat(cs.rowGap) || 0),
      cols,
      left: pad,
      colWidth: (width + gap) / cols,
    };
  }

  container() { return document.createElement("div"); }

  insertNodes(nodes, atStart) {
    if (atStart) this.root.prepend(...nodes);
    else this.root.append(...nodes);
  }

  setSpacers(before, after) {
    this.root.style.paddingTop = `calc(var(--grid-pad) + ${before}px)`;
    this.root.style.paddingBottom = `calc(var(--grid-pad) + ${after}px)`;
  }

  html(from, to) {
    const pane = this.pane;
    const { thumbnails: thumbnail } = settings();
    let out = "";
    for (let i = from; i < to; i++) {
      const e = pane.items[i];
      out += `<div data-i="${i}"${itemClasses(pane, e, i)} title="${esc(pane.search ? e.path : e.name)}">
        <span class="ficon big">${entryIcon(e, 48, { thumbnail })}</span>
        <span class="tile-label">${esc(e.name)}</span>
      </div>`;
    }
    return out;
  }

  label(i) { return this.item(i)?.querySelector(".tile-label"); }
}

export const VIEWS = { details: DetailsView, icons: IconsView };
