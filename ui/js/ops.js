// File operations: rename, new folder, trash/delete, clipboard, copy/move
// jobs with progress and conflict prompts, and undo. Acts on the active pane
// unless told otherwise.

import { ask } from "./dialog.js";
import { Fmt } from "./format.js";
import { basename, el, errMsg, invoke, isMac, isMod, listen, modLabel, parentOf, plural } from "./util.js";

export class Ops {
  constructor(app) {
    this.app = app;
    this.jobsEl = document.getElementById("jobs");
    this.jobs = new Map();
    listen("job-progress", ({ payload }) => this.renderProgress(this.jobFor(payload.id), payload));
    listen("job-finished", ({ payload }) => this.finishJob(this.jobFor(payload.id), payload));
    listen("job-conflict", ({ payload }) => this.onConflict(payload));
  }

  get pane() { return this.app.pane; }

  describe(items) {
    return items.length === 1 ? `“${items[0].name}”` : plural(items.length, "item");
  }

  // ---------- Inline rename ----------

  beginRename(pane = this.pane, i = pane.selected.has(pane.focus) ? pane.focus : [...pane.selected][0]) {
    if (pane.renaming || i == null || i < 0) return;
    const e = pane.items[i];
    const label = pane.viewImpl.label(i);
    if (!e || !label) return;
    pane.setFocus(i);

    const input = el("input", "rename-input");
    input.value = e.name;
    input.spellcheck = false;
    input.autocomplete = "off";
    label.replaceWith(input);
    pane.renaming = { path: e.path };
    input.focus();
    // Select the name without its extension, like Dolphin.
    const dot = pane.isFolder(e) ? -1 : e.name.lastIndexOf(".");
    input.setSelectionRange(0, dot > 0 ? dot : e.name.length);

    let busy = false;
    const end = () => {
      pane.renaming = null;
      pane.scroller.focus();
      pane.resumeRefresh();
    };
    const cancel = () => {
      if (input.isConnected) input.replaceWith(label);
      end();
    };
    const commit = async (fromBlur) => {
      if (busy || !pane.renaming) return;
      const newName = input.value;
      if (newName === e.name || !newName.trim()) return cancel();
      busy = true;
      try {
        const to = await invoke("rename_entry", { path: e.path, newName });
        pane.renaming = null;
        await pane.navigate(pane.path, { mode: "replace", select: basename(to), preserveScroll: true });
        end();
      } catch (err) {
        pane.showMessage(errMsg(err));
        busy = false;
        // Keep editing so the name can be fixed, unless the user clicked away.
        if (fromBlur) cancel();
        else input.focus();
      }
    };
    input.addEventListener("keydown", (ev) => {
      ev.stopPropagation();
      if (ev.key === "Enter") { ev.preventDefault(); commit(false); }
      if (ev.key === "Escape") { ev.preventDefault(); cancel(); }
    });
    input.addEventListener("blur", () => commit(true));
    input.addEventListener("mousedown", (ev) => ev.stopPropagation());
  }

  // ---------- New folder ----------

  async newFolder(pane = this.pane) {
    if (!pane.path) return;
    try {
      const path = await invoke("create_folder", { dir: pane.path });
      const name = basename(path);
      await pane.navigate(pane.path, { mode: "replace", select: name, preserveScroll: true });
      const i = pane.byName.get(name);
      if (i != null) this.beginRename(pane, i);
    } catch (err) {
      pane.showMessage(errMsg(err));
    }
  }

  // ---------- Trash / delete ----------

  inTrash(pane) {
    const t = this.app.places.find((p) => p.icon === "trash")?.path;
    return !!t && (pane.path === t || pane.path?.startsWith(`${t}/`));
  }

  /** Name of the entry to select once `removed` disappear (keeps the cursor in place). */
  nameAfterRemoval(pane, removed) {
    const gone = new Set(removed.map((e) => e.name));
    const items = pane.items;
    const last = Math.max(...pane.selected);
    for (let i = last + 1; i < items.length; i++) if (!gone.has(items[i].name)) return items[i].name;
    for (let i = last - 1; i >= 0; i--) if (!gone.has(items[i].name)) return items[i].name;
    return null;
  }

  async removeSelected({ permanent = false } = {}, pane = this.pane) {
    const items = pane.selectedEntries();
    if (!items.length) return;
    // Items already in the trash can only be deleted for real.
    permanent ||= this.inTrash(pane);
    const what = this.describe(items);

    if (permanent) {
      const { value } = await ask({
        title: `Permanently delete ${what}?`,
        message: "This cannot be undone.",
        buttons: [
          { label: "Cancel", value: "cancel", cancel: true },
          { label: "Delete", value: "delete", primary: true, danger: true },
        ],
      });
      if (value !== "delete") return;
    }

    const select = this.nameAfterRemoval(pane, items);
    try {
      const report = await invoke(permanent ? "delete_entries" : "trash_entries", { paths: items.map((e) => e.path) });
      await pane.navigate(pane.path, { mode: "replace", select, preserveScroll: true });
      if (report.errors.length) pane.showMessage(summarizeErrors(report.errors));
      else if (permanent) this.app.flashStatus(`Deleted ${what}`);
      else this.app.flashStatus(`Moved ${what} to the Trash — ${modLabel}Z to undo`);
    } catch (err) {
      pane.showMessage(errMsg(err));
    }
  }

  // ---------- System clipboard ----------
  //
  // Files go on the OS clipboard, so they can be pasted into Finder/Dolphin
  // and files copied there can be pasted here. `app.clipboard` mirrors it
  // only to dim cut items.

  setClipboard(mode, pane = this.pane) {
    const items = pane.selectedEntries();
    if (!items.length) return;
    const paths = items.map((e) => e.path);
    this.setCutMarks(mode, paths);
    const what = this.describe(items);
    invoke("clipboard_write_files", { clip: { mode, paths } })
      .then(() => this.app.flashStatus(mode === "cut" ? `Cut ${what} — paste to move` : `Copied ${what} — paste to duplicate`))
      .catch((err) => pane.showMessage(errMsg(err)));
  }

  setCutMarks(mode, paths) {
    this.app.clipboard = paths?.length ? { mode, paths: new Set(paths) } : null;
    this.app.forEachPane((p) => p.markCut());
  }

  /** Files on the system clipboard (from Sonar or any other app), or null. */
  readClipboard() {
    return invoke("clipboard_read_files").catch(() => null);
  }

  /** Re-read the clipboard (e.g. when the window regains focus) to refresh cut marks. */
  async syncClipboard() {
    const clip = await this.readClipboard();
    this.setCutMarks(clip?.mode, clip?.paths);
  }

  /** Paste into `destDir`. `move` forces a move (⌘⌥V on macOS, like Finder). */
  async paste(destDir = this.pane.path, { move = false } = {}) {
    if (!destDir) return;
    const clip = await this.readClipboard();
    if (!clip?.paths?.length) {
      this.app.flashStatus("The clipboard has no files to paste");
      return;
    }
    const kind = move || clip.mode === "cut" ? "move" : "copy";
    this.startJob(kind, clip.paths, destDir, { fromCut: kind === "move" });
  }

  /** Put the selected items' paths (or the folder's) on the clipboard as text. */
  copyLocation(pane = this.pane) {
    const items = pane.selectedEntries();
    const paths = items.length ? items.map((e) => e.path) : [pane.path];
    invoke("clipboard_write_text", { text: paths.join("\n") })
      .then(() => this.app.flashStatus(paths.length === 1 ? `Copied “${paths[0]}”` : `Copied ${paths.length} locations`))
      .catch((err) => pane.showMessage(errMsg(err)));
  }

  // ---------- Jobs ----------

  jobFor(id) {
    if (!this.jobs.has(id)) this.jobs.set(id, { id });
    return this.jobs.get(id);
  }

  async startJob(kind, sources, destDir, meta = {}) {
    const id = await invoke("start_transfer", { request: { kind, sources, destDir } });
    const node = el("div", "job", `
      <div class="job-text"><div class="job-title"></div><div class="job-detail">Preparing…</div></div>
      <div class="job-bar"><div class="job-fill"></div></div>
      <button class="tool job-cancel" title="Cancel">×</button>`);
    const verb = kind === "move" ? "Moving" : "Copying";
    const what = sources.length === 1 ? `“${basename(sources[0])}”` : plural(sources.length, "item");
    node.querySelector(".job-title").textContent = `${verb} ${what} to “${this.app.folderLabel(destDir)}”`;
    node.querySelector(".job-cancel").addEventListener("click", () => invoke("cancel_job", { id }));
    this.jobsEl.append(node);
    this.jobsEl.hidden = false;
    // Events can in theory beat the invoke reply; replay anything that arrived early.
    const job = Object.assign(this.jobFor(id), { el: node, kind, what, destDir, ...meta, speed: 0, last: null });
    if (job.pendingProgress) this.renderProgress(job, job.pendingProgress);
    if (job.pendingFinish) this.finishJob(job, job.pendingFinish);
  }

  renderProgress(job, p) {
    if (!job.el) { job.pendingProgress = p; return; }
    const now = performance.now();
    if (job.last) {
      const dt = (now - job.last.t) / 1000;
      if (dt > 0) {
        const inst = (p.doneBytes - job.last.bytes) / dt;
        job.speed = job.speed ? job.speed * 0.7 + inst * 0.3 : inst;
      }
    }
    job.last = { t: now, bytes: p.doneBytes };
    const frac = p.totalBytes ? p.doneBytes / p.totalBytes : p.totalItems ? p.doneItems / p.totalItems : 0;
    job.el.querySelector(".job-fill").style.width = `${Math.min(100, frac * 100).toFixed(1)}%`;
    const parts = [];
    if (p.totalBytes) parts.push(`${Fmt.bytes(p.doneBytes)} of ${Fmt.bytes(p.totalBytes)}`);
    parts.push(`${p.doneItems} of ${plural(p.totalItems, "item")}`);
    if (job.speed > 1024) parts.push(`${Fmt.bytes(Math.round(job.speed))}/s`);
    if (p.current) parts.push(p.current);
    job.el.querySelector(".job-detail").textContent = parts.join(" · ");
  }

  finishJob(job, result) {
    if (!job.el) { job.pendingFinish = result; return; }
    job.el.remove();
    this.jobs.delete(job.id);
    this.jobsEl.hidden = this.jobs.size === 0;

    if (job.fromCut && !result.cancelled && !result.errors.length) {
      // A completed cut-paste empties the clipboard: the sources are gone.
      this.setCutMarks(null, null);
      invoke("clipboard_clear_cut").catch(() => {});
    }
    if (result.errors.length) {
      this.pane.showMessage(summarizeErrors(result.errors));
    } else if (result.cancelled) {
      this.app.flashStatus(`${result.kind === "move" ? "Move" : "Copy"} cancelled`);
    } else if (result.completed) {
      const done = result.kind === "move" ? "Moved" : "Copied";
      this.app.flashStatus(`${done} ${job.what} to “${this.app.folderLabel(job.destDir)}” — ${modLabel}Z to undo`);
    }
  }

  async onConflict(c) {
    const { source, target } = c;
    const where = this.app.folderLabel(parentOf(target.path));
    const describe = (it) => [it.isDir ? "Folder" : Fmt.bytes(it.size), Fmt.date(it.modified)].filter(Boolean).join(" · ");
    const title = {
      file: `“${target.name}” already exists in “${where}”`,
      folder: `A folder named “${target.name}” already exists in “${where}”`,
      mismatch: `An item named “${target.name}” already exists in “${where}”`,
    }[c.kind];
    const buttons = [
      { label: "Cancel", value: "cancel", cancel: true },
      { label: "Skip", value: "skip" },
      { label: "Keep Both", value: "keepBoth", primary: c.kind === "mismatch" },
    ];
    if (c.kind === "file") buttons.push({ label: "Replace", value: "replace", primary: true });
    if (c.kind === "folder") buttons.push({ label: "Merge", value: "replace", primary: true });
    const { value, checked } = await ask({
      title,
      message: c.kind === "folder" ? "Merging adds the incoming items to the existing folder." : "",
      details: [["Existing", describe(target)], ["Incoming", describe(source)]],
      checkbox: "Apply to all conflicts",
      buttons,
    });
    invoke("resolve_conflict", { id: c.id, choice: value, applyToAll: checked });
  }

  // ---------- Undo ----------

  undoLabel() { return invoke("undo_label").catch(() => null); }

  async undo() {
    try {
      const label = await invoke("undo");
      this.app.flashStatus(label ? `Undone: ${label}` : "Nothing to undo");
    } catch (err) {
      this.pane.showMessage(`Undo failed: ${errMsg(err)}`);
    }
  }

  // ---------- Keyboard ----------

  /** Returns true if handled. Called by the app before pane navigation keys. */
  keydown(ev) {
    const mod = isMod(ev);
    const k = ev.key;
    const lower = k.toLowerCase();
    const run = (fn) => { ev.preventDefault(); fn(); return true; };

    if (k === "F2") return run(() => this.beginRename());
    if (mod && ev.shiftKey && lower === "n") return run(() => this.newFolder());

    if ((k === "Delete" && ev.shiftKey) || (isMac && ev.metaKey && ev.altKey && k === "Backspace"))
      return run(() => this.removeSelected({ permanent: true }));
    if ((k === "Delete" && !ev.shiftKey) || (isMac && ev.metaKey && k === "Backspace"))
      return run(() => this.removeSelected());

    // ev.code, not ev.key: with ⌥ held, macOS reports e.g. "√" for V.
    if (isMac && ev.metaKey && ev.altKey && ev.code === "KeyV") return run(() => this.paste(this.pane.path, { move: true }));
    if (mod && ev.altKey && ev.code === "KeyC") return run(() => this.copyLocation());
    if (mod && !ev.shiftKey && !ev.altKey) {
      if (lower === "c") return run(() => this.setClipboard("copy"));
      if (lower === "x") return run(() => this.setClipboard("cut"));
      if (lower === "v") return run(() => this.paste());
      if (lower === "z") return run(() => this.undo());
    }
    return false;
  }
}

function summarizeErrors(errors) {
  return errors.length === 1 ? errors[0] : `${errors[0]} (and ${errors.length - 1} more)`;
}
