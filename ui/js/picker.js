// Picker mode: this window is a file chooser for another app ("Open" /
// "Save" dialogs of Firefox and others, through the XDG Desktop Portal; see
// src-tauri/src/portal.rs). It is the normal UI with a bar at the bottom:
// the file name (Save), the file type filter, the app's extra options, and
// Cancel / Open. Double-clicking a file chooses it.

import { ask } from "./dialog.js";
import { store } from "./store.js";
import { el, errMsg, invoke, shown } from "./util.js";

/** The request id when this window is a picker (`index.html?picker=<id>`). */
export const pickerId = new URLSearchParams(location.search).get("picker");

/** `*.png`-style globs, matched without regard to case like most pickers. */
function globToRegExp(glob) {
  const re = glob.replace(/[.+^${}()|\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\?/g, ".");
  return new RegExp(`^${re}$`, "i");
}

function mimeMatches(pattern, type) {
  if (!type) return false;
  if (pattern === type) return true;
  return pattern.endsWith("/*") && type.startsWith(pattern.slice(0, -1));
}

export class Picker {
  constructor(app, id) {
    this.app = app;
    this.id = Number(id);
    this.request = null;
    this.filter = null; // index into request.filters
    this.lastSelected = null;
  }

  /** Load the request and open its folder. False if it's gone (answered or withdrawn). */
  async start() {
    this.request = await invoke("picker_request", { id: this.id }).catch(() => null);
    if (!this.request) {
      window.__TAURI__.window.getCurrentWindow().close().catch(() => {});
      return false;
    }
    const r = this.request;
    this.filter = r.currentFilter ?? (r.filters.length ? 0 : null);
    document.body.classList.add("picker-mode");
    document.title = r.title || (r.save ? "Save File" : "Open File");
    this.buildBar();
    const folder = r.folder ?? store.get("pickerFolder", null) ?? this.app.home;
    this.app.newTab(folder);
    return true;
  }

  get pane() { return this.app.pane; }

  // ---------- The bar ----------

  buildBar() {
    const r = this.request;
    const bar = el("footer", "picker-bar");
    bar.innerHTML = `
      <label class="picker-name" ${r.save ? "" : "hidden"}><span>Name</span>
        <input class="text-input" spellcheck="false" autocomplete="off" aria-label="File name"></label>
      <select class="text-input picker-filter" aria-label="File type" ${r.filters.length > 1 ? "" : "hidden"}></select>
      <span class="picker-choices"></span>
      <span class="picker-spacer"></span>
      <button type="button" class="btn" data-act="cancel">Cancel</button>
      <button type="button" class="btn primary" data-act="accept"></button>`;
    this.nameInput = bar.querySelector(".picker-name input");
    this.nameInput.value = shown(r.name ?? "");
    this.acceptButton = bar.querySelector('[data-act="accept"]');
    this.acceptButton.textContent = r.acceptLabel || (r.save ? "Save" : r.directory ? "Select Folder" : "Open");

    const select = bar.querySelector(".picker-filter");
    r.filters.forEach((f, i) => select.append(new Option(f.name, String(i), false, i === this.filter)));
    select.addEventListener("change", () => {
      this.filter = Number(select.value);
      this.app.forEachPane((p) => p.setFilter(p.filter)); // re-filter, keeping the selection
    });

    this.choiceValues = new Map(r.choices.map((c) => [c.id, c.selected]));
    const choices = bar.querySelector(".picker-choices");
    for (const c of r.choices) {
      if (!c.options.length) {
        const label = el("label", "dialog-check");
        const box = el("input");
        box.type = "checkbox";
        box.checked = c.selected === "true";
        box.addEventListener("change", () => this.choiceValues.set(c.id, String(box.checked)));
        label.append(box, document.createTextNode(` ${c.label}`));
        choices.append(label);
      } else {
        const label = el("label", "picker-choice");
        label.append(document.createTextNode(c.label));
        const sel = el("select", "text-input");
        for (const [value, text] of c.options) sel.append(new Option(text, value, false, value === c.selected));
        sel.addEventListener("change", () => this.choiceValues.set(c.id, sel.value));
        label.append(sel);
        choices.append(label);
      }
    }

    bar.addEventListener("click", (ev) => {
      const act = ev.target.closest("[data-act]")?.dataset.act;
      if (act === "cancel") this.cancel();
      if (act === "accept") this.accept();
    });
    this.nameInput.addEventListener("keydown", (ev) => {
      ev.stopPropagation();
      if (ev.key === "Enter") { ev.preventDefault(); this.accept(); }
      if (ev.key === "Escape") { ev.preventDefault(); this.cancel(); }
    });
    document.querySelector(".statusbar").before(bar);
    this.bar = bar;
    if (r.save) {
      // Select the name without its extension, ready to type over.
      const name = this.nameInput.value;
      const dot = name.lastIndexOf(".");
      requestAnimationFrame(() => {
        this.nameInput.focus();
        this.nameInput.setSelectionRange(0, dot > 0 ? dot : name.length);
      });
    }
  }

  // ---------- Which items show ----------

  /** Folders always show (to navigate); files only if they match the filter. */
  shows(pane, e) {
    if (pane.isFolder(e)) return true;
    if (this.request?.directory) return false;
    const f = this.request?.filters[this.filter];
    if (!f) return true;
    return f.patterns.some(([kind, pattern]) => (kind === 0 ? globToRegExp(pattern).test(e.name) : mimeMatches(pattern, e.typeId)));
  }

  /** Selection or folder changed: keep the bar in step. */
  update() {
    if (!this.request || !this.pane) return;
    const sel = this.pane.selectedEntries();
    const file = sel.length === 1 && !this.pane.isFolder(sel[0]) ? sel[0] : null;
    // Save: clicking a file takes its name (to replace it).
    if (this.request.save && file && file.path !== this.lastSelected) this.nameInput.value = shown(file.name);
    this.lastSelected = file?.path ?? null;
  }

  // ---------- Answering ----------

  /** Double-click on a file. */
  choose(entry) {
    if (this.request.save) this.nameInput.value = shown(entry.name);
    return this.accept();
  }

  async accept() {
    const pane = this.pane;
    if (!pane?.path) return;
    if (pane.view?.archive) {
      return this.app.flashStatus("Items inside an archive can't be chosen; extract them first");
    }
    const r = this.request;
    const sel = pane.selectedEntries();
    if (r.save) return this.save(pane);
    if (r.directory) {
      const folders = sel.filter((e) => pane.isFolder(e));
      const paths = folders.length ? folders.map((e) => e.path) : [pane.path];
      return this.respond(r.multiple ? paths : paths.slice(0, 1));
    }
    const files = sel.filter((e) => !pane.isFolder(e));
    if (files.length) return this.respond((r.multiple ? files : files.slice(0, 1)).map((e) => e.path));
    // Only a folder selected: Open goes into it.
    if (sel.length === 1) return pane.navigate(sel[0].path);
    this.app.flashStatus(r.multiple ? "Select one or more files" : "Select a file");
  }

  async save(pane) {
    const name = this.nameInput.value.trim();
    if (!name) return this.nameInput.focus();
    if (name.includes("/")) {
      pane.showMessage("The name cannot contain “/”");
      return this.nameInput.focus();
    }
    // Compared as shown, so a name with undecodable bytes is found (and its real path used).
    const existing = pane.search ? null : pane.view?.entries.find((e) => shown(e.name) === name);
    if (existing && pane.isFolder(existing)) return pane.navigate(existing.path); // like other pickers
    if (existing) {
      const { value } = await ask({
        title: `Replace “${name}”?`,
        message: "A file with this name already exists in this folder.",
        buttons: [
          { label: "Cancel", value: "cancel", cancel: true },
          { label: "Replace", value: "replace", primary: true, danger: true },
        ],
      });
      if (value !== "replace") return;
      return this.respond([existing.path]);
    }
    const folder = pane.search?.root ?? pane.path;
    return this.respond([folder === "/" ? `/${name}` : `${folder}/${name}`]);
  }

  async respond(paths) {
    store.set("pickerFolder", this.pane.search?.root ?? this.pane.path);
    await this.send({ ok: true, paths, choices: [...this.choiceValues], filter: this.filter });
  }

  cancel() {
    return this.send({ ok: false, paths: [], choices: [], filter: null });
  }

  async send(answer) {
    try {
      await invoke("picker_respond", { id: this.id, answer });
    } catch (err) {
      this.pane?.showMessage(errMsg(err));
    }
  }
}
