// Fake Tauri backend for UI tests, injected before the app loads.
//
// Implements every command and event the frontend uses on top of a small
// in-memory filesystem, so tests exercise real UI flows (navigate, rename,
// copy with conflicts, undo, live refresh...) without touching disk.
// Tests reach it through `window.__mock`.
(() => {
  const HOME = "/home/user";
  const TRASH = `${HOME}/.local/share/Trash/files`;
  const now = Date.now();

  // ---------- Filesystem ----------

  const fs = new Map(); // path -> { kind: "dir" | "file", size, mtime }
  const parentOf = (p) => (p === "/" ? null : p.replace(/\/[^/]*$/, "") || "/");
  const nameOf = (p) => p.split("/").pop();
  const join = (dir, name) => (dir === "/" ? `/${name}` : `${dir}/${name}`);

  function mkdir(p, mtime = now) {
    if (fs.has(p)) return;
    const parent = parentOf(p);
    if (parent) mkdir(parent, mtime);
    fs.set(p, { kind: "dir", mtime });
  }
  function write(p, size, mtime = now) {
    mkdir(parentOf(p), mtime);
    fs.set(p, { kind: "file", size, mtime });
  }
  const children = (dir) => [...fs.keys()].filter((p) => p !== dir && parentOf(p) === dir);
  const descendants = (p) => [...fs.keys()].filter((q) => q === p || q.startsWith(`${p}/`));

  const day = 86400000;
  for (const d of ["Desktop", "Documents/Projects/sonar", "Downloads", "Music", "Pictures", "Videos", ".config/sonar"]) {
    mkdir(`${HOME}/${d}`, now - 3 * day);
  }
  mkdir(TRASH);
  write(`${HOME}/readme.md`, 2048, now - 2 * day);
  write(`${HOME}/archive.zip`, 5 * 1024 * 1024, now - 10 * day);
  write(`${HOME}/.bashrc`, 300, now - 40 * day);
  write(`${HOME}/Documents/report.pdf`, 120000, now - day);
  write(`${HOME}/Documents/notes.txt`, 900, now);
  write(`${HOME}/Documents/Projects/sonar/main.rs`, 4000, now);
  write(`${HOME}/Downloads/big.iso`, 50 * 1024 * 1024, now - 5 * day);
  for (let i = 1; i <= 12; i++) write(`${HOME}/Pictures/photo${i}.png`, 100000 + i, now - i * day);
  write(`${HOME}/Pictures/photo-copy.unknown`, 10);

  // ---------- Entries ----------

  const TYPES = { png: ["PNG image", "image"], pdf: ["PDF document", "document"], txt: ["TXT text", "text"],
    md: ["MD text", "text"], rs: ["RS source code", "code"], zip: ["ZIP archive", "archive"], iso: ["ISO disk image", "archive"] };

  function entry(p, countChildren) {
    const n = fs.get(p);
    const name = nameOf(p);
    const dir = n.kind === "dir";
    const ext = name.includes(".") ? name.split(".").pop().toLowerCase() : "";
    const [typeLabel, icon] = dir ? ["Folder", "folder"] : TYPES[ext] ?? [ext ? `${ext.toUpperCase()} file` : "File", "file"];
    return {
      name, path: p, kind: dir ? "directory" : "file", isSymlink: false, isHidden: name.startsWith("."),
      readonly: false, size: dir ? null : n.size, childCount: dir && countChildren ? children(p).length : null,
      modified: n.mtime, typeLabel, icon,
      typeId: dir ? "inode/directory" : `mock/${ext || "unknown"}`,
      iconKey: dir ? "folder" : `${icon}-x-generic,unknown`,
      thumbnail: ext === "png",
    };
  }

  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });
  function sortEntries(list, { key = "name", order = "ascending" } = {}) {
    list.sort((a, b) => {
      const fa = a.kind === "directory", fb = b.kind === "directory";
      if (fa !== fb) return fa ? -1 : 1; // folders always first
      const primary = {
        name: 0,
        size: (a.size ?? a.childCount ?? 0) - (b.size ?? b.childCount ?? 0),
        modified: a.modified - b.modified,
        type: a.typeLabel.localeCompare(b.typeLabel),
      }[key];
      const ord = primary || collator.compare(a.name, b.name);
      return order === "descending" ? -ord : ord;
    });
    return list;
  }

  function crumbs(p) {
    if (p === HOME || p.startsWith(`${HOME}/`)) {
      const out = [{ label: "Home", path: HOME, icon: "home" }];
      let acc = HOME;
      for (const part of p.slice(HOME.length).split("/").filter(Boolean)) {
        acc = join(acc, part);
        out.push({ label: part, path: acc, icon: null });
      }
      return out;
    }
    const out = [{ label: "/", path: "/", icon: "root" }];
    let acc = "/";
    for (const part of p.split("/").filter(Boolean)) {
      acc = join(acc, part);
      out.push({ label: part, path: acc, icon: null });
    }
    return out;
  }

  // ---------- Events ----------

  const listeners = {};
  const emit = (event, payload) => (listeners[event] ?? []).forEach((h) => h({ event, payload }));
  const watching = new Map(); // pane -> path
  const generations = new Map();
  function touched(dir) {
    // Like the debounced watcher: tell every pane showing `dir`.
    setTimeout(() => {
      for (const [pane, path] of watching) if (path === dir) emit("dir-changed", { pane, path });
    }, 30);
  }

  // ---------- Places and drives ----------

  const volumes = [
    { label: "USB", mountPoint: "/media/user/USB", device: "/dev/sdb1", mounted: true, removable: true, usage: { total: 16e9, free: 12e9 } },
    { label: "Backup", mountPoint: "/media/user/Backup", device: "/dev/sdc1", mounted: false, removable: true, usage: null },
  ];
  mkdir("/media/user/USB");
  write("/media/user/USB/slides.pdf", 3e6);

  function places() {
    const user = [["Home", HOME, "home"], ["Desktop", `${HOME}/Desktop`, "desktop"], ["Documents", `${HOME}/Documents`, "documents"],
      ["Downloads", `${HOME}/Downloads`, "downloads"], ["Music", `${HOME}/Music`, "music"], ["Pictures", `${HOME}/Pictures`, "pictures"],
      ["Videos", `${HOME}/Videos`, "videos"], ["Trash", TRASH, "trash"]];
    const base = { usage: null, ejectable: false, mounted: true, device: null };
    return [
      ...user.map(([label, path, icon]) => ({ ...base, label, path, icon, section: "places" })),
      { ...base, label: "Root", path: "/", icon: "root", section: "devices", usage: { total: 500e9, free: 40e9 } },
      ...volumes.map((v) => ({
        label: v.label, icon: "removable", section: "devices", mounted: v.mounted, ejectable: v.mounted,
        path: v.mounted ? v.mountPoint : v.device, device: v.device, usage: v.mounted ? v.usage ?? { total: 1e12, free: 3e11 } : null,
      })),
    ];
  }
  // Like the backend's poller noticing a change.
  const placesChanged = () => setTimeout(() => emit("places-changed", places()), 10);

  // ---------- Errors ----------

  const err = (code, message) => ({ code, message });
  const notFound = (p) => err("notFound", `The folder "${p}" does not exist`);

  // ---------- Undo ----------

  const undoStack = [];
  function move(from, to) {
    for (const p of descendants(from).sort()) {
      const node = fs.get(p);
      fs.delete(p);
      fs.set(to + p.slice(from.length), node);
    }
  }
  function copy(from, to) {
    for (const p of descendants(from).sort()) fs.set(to + p.slice(from.length), { ...fs.get(p) });
  }
  function keepBoth(dir, name, isDir) {
    const m = isDir ? null : name.match(/^(.+)(\.[^.]+)$/);
    const [stem, ext] = m ? [m[1], m[2]] : [name, ""];
    for (let i = 1; ; i++) {
      const p = join(dir, `${stem} (${i})${ext}`);
      if (!fs.has(p)) return p;
    }
  }

  // ---------- Jobs ----------

  let nextJob = 1;
  const jobs = new Map();

  async function runTransfer(id, { kind, sources, destDir }) {
    const job = jobs.get(id);
    const sizes = sources.map((s) => descendants(s).reduce((t, p) => t + (fs.get(p)?.size ?? 0), 0));
    const total = sizes.reduce((a, b) => a + b, 0);
    let done = 0;
    const created = [];
    const moved = [];
    const remembered = {};
    const progress = (current) => emit("job-progress", {
      id, totalBytes: total, doneBytes: done, totalItems: sources.length, doneItems: created.length + moved.length, current,
    });
    progress(null);
    await new Promise((r) => setTimeout(r, 20));
    for (const [i, src] of sources.entries()) {
      if (job.cancelled) break;
      const isDir = fs.get(src)?.kind === "dir";
      let dst = join(destDir, nameOf(src));
      if (parentOf(src) === destDir) {
        if (kind === "move") continue;
        dst = keepBoth(destDir, nameOf(src), isDir);
      }
      let replace = false;
      if (fs.has(dst)) {
        const conflictKind = isDir && fs.get(dst).kind === "dir" ? "folder" : !isDir && fs.get(dst).kind === "file" ? "file" : "mismatch";
        let answer = remembered[conflictKind];
        if (!answer) {
          const info = (p) => ({ path: p, name: nameOf(p), isDir: fs.get(p).kind === "dir", size: fs.get(p).size ?? null, modified: fs.get(p).mtime });
          emit("job-conflict", { id, kind: conflictKind, source: info(src), target: info(dst) });
          const [choice, all] = await new Promise((r) => { job.answer = r; });
          if (all && choice !== "cancel") remembered[conflictKind] = choice;
          answer = choice;
        }
        if (answer === "cancel") { job.cancelled = true; break; }
        if (answer === "skip") continue;
        if (answer === "keepBoth") dst = keepBoth(destDir, nameOf(src), isDir);
        if (answer === "replace") { replace = true; for (const p of descendants(dst)) if (!isDir) fs.delete(p); }
      }
      progress(nameOf(src));
      if (kind === "copy") copy(src, dst); else move(src, dst);
      done += sizes[i];
      if (!replace) (kind === "copy" ? created.push(dst) : moved.push([src, dst]));
    }
    if (created.length) undoStack.push({ label: "Copy", run: () => created.forEach((p) => move(p, join(TRASH, nameOf(p)))) });
    if (moved.length) undoStack.push({ label: "Move", run: () => moved.reverse().forEach(([a, b]) => move(b, a)) });
    jobs.delete(id);
    touched(destDir);
    sources.forEach((s) => touched(parentOf(s)));
    progress(null);
    emit("job-finished", { id, kind, cancelled: !!job.cancelled, completed: created.length + moved.length, errors: [] });
  }

  // ---------- Commands ----------

  const calls = [];
  const commands = {
    get_places: () => places(),
    mount_device: ({ device }) => {
      const v = volumes.find((d) => d.device === device);
      if (!v) throw err("other", `Could not mount ${device}: no such device`);
      v.mounted = true;
      mkdir(v.mountPoint);
      write(`${v.mountPoint}/backup-2026.tar`, 7e8);
      placesChanged();
      return v.mountPoint;
    },
    eject_volume: ({ path }) => {
      const v = volumes.find((d) => d.mounted && d.mountPoint === path);
      if (!v) throw err("other", `Could not eject "${path}": it is not mounted`);
      if (window.__mock.busy.has(path)) throw err("other", `Could not eject "${path}": target is busy`);
      descendants(path).forEach((p) => fs.delete(p));
      volumes.splice(volumes.indexOf(v), 1);
      placesChanged();
    },
    home_dir: () => HOME,
    open_dir: ({ pane, path, options = {} }) => {
      const node = fs.get(path);
      if (!node) throw notFound(path);
      if (node.kind !== "dir") throw err("notADirectory", `"${path}" is not a folder`);
      if (window.__mock.denied.has(path)) throw err("permissionDenied", `You do not have permission to open "${path}"`);
      const generation = (generations.get(pane) ?? 0) + 1;
      generations.set(pane, generation);
      watching.set(pane, path);
      const countNow = options.sort?.key === "size";
      const all = children(path).map((p) => entry(p, countNow));
      const shown = options.showHidden ? all : all.filter((e) => !e.isHidden);
      const folders = shown.filter((e) => e.kind === "directory");
      if (!countNow && folders.length) {
        // Item counts arrive later, like the real background job.
        setTimeout(() => emit("child-counts", { pane, generation, counts: folders.map((e) => [e.name, children(e.path).length]), done: true }), window.__mock.countDelay);
      }
      return {
        path, parent: parentOf(path), entries: sortEntries(shown, options.sort), hiddenCount: all.length - shown.length,
        skippedCount: 0, crumbs: crumbs(path), generation, countsComplete: countNow || !folders.length, watchError: null,
      };
    },
    close_pane: ({ pane }) => { watching.delete(pane); },
    resolve_path: ({ input, cwd }) => {
      let p = input.trim();
      if (p === "~" || p.startsWith("~/")) p = HOME + p.slice(1);
      else if (!p.startsWith("/")) p = join(cwd, p);
      return p.replace(/\/+$/, "") || "/";
    },
    open_path: ({ path }) => {
      if (path.endsWith(".unknown")) throw err("open", `Could not open "${path}": no application knows how to open it`);
    },
    rename_entry: ({ path, newName }) => {
      if (!newName.trim() || newName.includes("/")) throw err("invalidName", "The name cannot contain \"/\"");
      const to = join(parentOf(path), newName);
      if (fs.has(to)) throw err("alreadyExists", `"${newName}" already exists`);
      move(path, to);
      undoStack.push({ label: "Rename", run: () => move(to, path) });
      touched(parentOf(path));
      return to;
    },
    create_folder: ({ dir }) => {
      const path = fs.has(join(dir, "New Folder")) ? keepBoth(dir, "New Folder", true) : join(dir, "New Folder");
      mkdir(path);
      undoStack.push({ label: "Create Folder", run: () => fs.delete(path) });
      touched(dir);
      return path;
    },
    trash_entries: ({ paths }) => {
      const moved = paths.map((p) => {
        const to = fs.has(join(TRASH, nameOf(p))) ? keepBoth(TRASH, nameOf(p), fs.get(p).kind === "dir") : join(TRASH, nameOf(p));
        move(p, to);
        touched(parentOf(p));
        return [p, to];
      });
      undoStack.push({ label: "Move to Trash", run: () => moved.forEach(([a, b]) => move(b, a)) });
      return { done: paths.length, errors: [] };
    },
    delete_entries: ({ paths }) => {
      paths.forEach((p) => { descendants(p).forEach((q) => fs.delete(q)); touched(parentOf(p)); });
      return { done: paths.length, errors: [] };
    },
    start_transfer: ({ request }) => {
      const id = nextJob++;
      jobs.set(id, { cancelled: false, answer: null });
      setTimeout(() => runTransfer(id, request), 0);
      return id;
    },
    resolve_conflict: ({ id, choice, applyToAll }) => jobs.get(id)?.answer?.([choice, applyToAll]),
    cancel_job: ({ id }) => {
      const job = jobs.get(id);
      if (job) { job.cancelled = true; job.answer?.(["cancel", false]); }
    },
    // System clipboard stand-in. Tests set `__mock.clipboard` directly to
    // simulate files copied in Finder/Dolphin.
    clipboard_write_files: ({ clip }) => { window.__mock.clipboard = clip; },
    clipboard_read_files: () => {
      const c = window.__mock.clipboard;
      return c?.paths ? structuredClone(c) : null;
    },
    clipboard_write_text: ({ text }) => { window.__mock.clipboard = { text }; },
    clipboard_clear_cut: () => {
      if (window.__mock.clipboard?.mode === "cut") window.__mock.clipboard = null;
    },
    start_drag_out: ({ paths }) => { window.__mock.dragOuts.push(paths); },
    has_native_menu: () => window.__mock.nativeMenu,
    set_menu_state: ({ state }) => { window.__mock.menuState = state; },
    undo_label: () => undoStack.at(-1)?.label ?? null,
    undo: () => {
      const action = undoStack.pop();
      if (!action) return null;
      action.run();
      for (const [, path] of watching) touched(path);
      return action.label;
    },
  };

  window.__mock = {
    HOME, TRASH, fs, calls, emit, watching, undoStack, mkdir, write, touched,
    denied: new Set(),
    clipboard: null,
    busy: new Set(),
    nativeMenu: false,
    menuState: null,
    /** Simulate a click in the native menu bar. */
    menu: (id) => emit("menu", id),
    /** Simulate a USB stick being plugged in (the poller would report it). */
    attach(label, device) {
      volumes.push({ label, mountPoint: `/media/user/${label}`, device, mounted: false, removable: true, usage: null });
      placesChanged();
    },
    dragOuts: [],
    /** Simulate Tauri's native drag-drop events for files dragged in from another app. */
    dropFiles(paths, x, y, { leaveFirst = false } = {}) {
      const dpr = window.devicePixelRatio;
      const position = { x: x * dpr, y: y * dpr };
      emit("tauri://drag-enter", { paths, position });
      emit("tauri://drag-over", { position });
      if (leaveFirst) return emit("tauri://drag-leave", null);
      emit("tauri://drag-drop", { paths, position });
    },
    countDelay: 30,
    windowCloses: 0,
    exists: (p) => fs.has(p),
    remove: (p) => { descendants(p).forEach((q) => fs.delete(q)); touched(parentOf(p)); },
    callsTo: (cmd) => calls.filter(([c]) => c === cmd).map(([, a]) => a),
  };

  window.__TAURI__ = {
    core: {
      invoke: async (cmd, args = {}) => {
        calls.push([cmd, JSON.parse(JSON.stringify(args))]);
        const fn = commands[cmd];
        if (!fn) throw err("other", `mock: unknown command ${cmd}`);
        return fn(args);
      },
    },
    event: {
      listen: async (event, handler) => {
        (listeners[event] ??= []).push(handler);
        return () => { listeners[event] = listeners[event].filter((h) => h !== handler); };
      },
    },
    window: { getCurrentWindow: () => ({ close: async () => { window.__mock.windowCloses++; } }) },
  };
})();
