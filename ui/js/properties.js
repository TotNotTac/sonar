// Properties dialog (⌘I / Alt+Enter): details of the selection, or of the
// current folder when nothing is selected. Folder sizes count up live.

import { Fmt } from "./format.js";
import { Icons } from "./icons.js";
import { assetUrl, basename, el, errMsg, invoke, listen, parentOf, plural, shown } from "./util.js";

const handlers = new Map(); // size job token -> update function
listen("size-progress", ({ payload }) => handlers.get(payload.token)?.(payload));

const fullDate = new Intl.DateTimeFormat(undefined, { dateStyle: "long", timeStyle: "short" });
const when = (ms) => (ms == null ? null : fullDate.format(new Date(ms)));
const withBytes = (n) => (n == null ? null : n < 1024 ? Fmt.bytes(n) : `${Fmt.bytes(n)} (${n.toLocaleString()} bytes)`);

export async function showProperties(app, pane) {
  const selected = pane.selectedEntries();
  const archive = pane.view?.archive;
  let props;
  try {
    if (archive) {
      // Items inside an archive only exist in its listing.
      const list = selected.length ? selected : [];
      props = list.map((e) => ({ name: e.name, path: e.path, kind: e.kind === "directory" ? "folder" : "file",
        typeLabel: e.typeLabel, size: e.size, modified: e.modified }));
      if (!props.length) props = [{ name: basename(archive), path: archive, kind: "file", typeLabel: "Archive" }];
    } else {
      props = await invoke("get_properties", { paths: selected.length ? selected.map((e) => e.path) : [pane.path] });
    }
  } catch (err) {
    pane.showMessage(errMsg(err));
    return;
  }

  const single = props.length === 1 ? props[0] : null;
  const entry = selected.length === 1 ? selected[0] : null;
  const dlg = el("dialog", "dialog props");
  dlg.innerHTML = `
    <div class="dialog-body">
      <div class="props-head"><span class="ficon big"></span><h2 class="dialog-title"></h2></div>
      <dl class="props-grid"></dl>
      <div class="dialog-buttons"><button type="button" class="btn primary">Close</button></div>
    </div>`;
  const icon = dlg.querySelector(".ficon");
  icon.innerHTML = Icons.entry(single?.kind === "folder" ? "folder" : entry?.icon ?? "file");
  if (entry?.iconKey) {
    const img = el("img", "os-icon");
    img.src = assetUrl("icon", { key: entry.iconKey, size: 48 });
    img.addEventListener("load", () => icon.classList.add("has-os-icon"));
    img.addEventListener("error", () => img.remove());
    icon.append(img);
  }
  dlg.querySelector(".dialog-title").textContent = single ? shown(single.name) : plural(props.length, "item");

  const grid = dlg.querySelector(".props-grid");
  const row = (label, value, cls = "") => {
    if (value == null || value === "") return null;
    const dt = el("dt");
    const dd = el("dd", cls);
    dt.textContent = label;
    dd.textContent = shown(value);
    grid.append(dt, dd);
    return dd;
  };

  const types = [...new Set(props.map((p) => p.typeLabel))];
  row("Type", types.length === 1 ? types[0] : "Various");
  const parents = [...new Set(props.map((p) => parentOf(p.path)))];
  row("Location", parents.length === 1 ? parents[0] : "Various folders", "selectable");
  if (single?.linkTarget) row("Link to", single.linkTarget, "selectable");

  // Size: known for single files; counted in the background for folders and selections.
  const needsCount = !archive && (props.length > 1 || props.some((p) => p.kind === "folder"));
  const sizeCell = row("Size", needsCount ? "Calculating…" : withBytes(single?.size));
  const diskCell = needsCount ? row("Size on disk", "…") : row("Size on disk", withBytes(single?.sizeOnDisk));
  const containsCell = needsCount ? row("Contains", "…") : null;
  let token = null;
  if (needsCount) {
    token = `size-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    handlers.set(token, ({ totals: t, done }) => {
      sizeCell.textContent = `${withBytes(t.bytes)}${done ? "" : " …"}`;
      diskCell.textContent = withBytes(t.bytesOnDisk);
      containsCell.textContent = `${plural(t.files, "file")}, ${plural(t.folders, "folder")}`;
    });
    invoke("start_size", { token, paths: props.map((p) => p.path) }).catch(() => {});
  }

  if (single) {
    row("Created", when(single.created));
    row("Modified", when(single.modified));
    row("Accessed", when(single.accessed));
    if (single.permissions) row("Permissions", `${single.permissions} (${single.mode})`, "mono");
    if (single.owner) row("Owner", `${single.owner}${single.group ? `, group ${single.group}` : ""}`);
  }
  if (archive) row("In archive", basename(archive));

  const close = () => {
    if (token) {
      invoke("cancel_size", { token }).catch(() => {});
      handlers.delete(token);
    }
    dlg.close();
    dlg.remove();
    pane.scroller.focus();
  };
  dlg.querySelector(".btn").addEventListener("click", close);
  dlg.addEventListener("cancel", (ev) => { ev.preventDefault(); close(); });
  document.body.append(dlg);
  dlg.showModal();
  dlg.querySelector(".btn").focus();
}
