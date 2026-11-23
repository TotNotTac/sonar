// "Open With": choose an app for the selection, optionally as the default
// for that file type.

import { Icons } from "./icons.js";
import { assetUrl, el, errMsg, invoke, plural, shown } from "./util.js";

export async function openWith(app, pane, appId, { makeDefault = false } = {}) {
  const items = pane.selectedEntries();
  if (!items.length) return;
  try {
    if (makeDefault) await invoke("set_default_app", { path: items[0].path, app: appId });
    await invoke("open_with", { paths: items.map((e) => e.path), app: appId });
  } catch (err) {
    pane.showMessage(errMsg(err));
  }
}

const appIcon = (a, size) =>
  a.iconKey ? `<img class="app-icon" alt="" src="${assetUrl("icon", { key: a.iconKey, size })}">` : Icons.entry("app");

/** The full chooser: every candidate app, and "always use" for the type. */
export async function chooseApp(app, pane, apps = null) {
  const items = pane.selectedEntries();
  if (!items.length) return;
  apps ??= await invoke("apps_for", { path: items[0].path }).catch(() => []);
  const dlg = el("dialog", "dialog choose-app");
  const what = items.length === 1 ? `“${items[0].name}”` : plural(items.length, "item");
  dlg.innerHTML = `
    <form class="dialog-body" method="dialog">
      <h2 class="dialog-title"></h2>
      <div class="app-list" role="listbox"></div>
      <label class="dialog-check"><input type="checkbox" /> <span></span></label>
      <div class="dialog-buttons">
        <button type="button" class="btn" value="cancel">Cancel</button>
        <button type="button" class="btn primary" value="open">Open</button>
      </div>
    </form>`;
  dlg.querySelector(".dialog-title").textContent = shown(`Open ${what} with`);
  dlg.querySelector(".dialog-check span").textContent = `Always use this app for ${items[0].typeLabel} files`;
  const list = dlg.querySelector(".app-list");
  let chosen = apps.find((a) => a.isDefault)?.id ?? apps[0]?.id;
  if (!apps.length) list.textContent = "No applications found for this type of file.";
  for (const a of apps) {
    const row = el("div", `app-row${a.id === chosen ? " chosen" : ""}`, `${appIcon(a, 24)}<span></span>`);
    row.setAttribute("role", "option");
    row.dataset.id = a.id;
    row.querySelector("span").textContent = a.isDefault ? `${a.name} (default)` : a.name;
    row.addEventListener("click", () => {
      chosen = a.id;
      list.querySelectorAll(".app-row").forEach((r) => r.classList.toggle("chosen", r.dataset.id === chosen));
    });
    row.addEventListener("dblclick", () => finish("open"));
    list.append(row);
  }
  for (const img of list.querySelectorAll("img")) img.addEventListener("error", () => img.remove());
  const finish = (value) => {
    const always = dlg.querySelector(".dialog-check input").checked;
    dlg.close();
    dlg.remove();
    pane.scroller.focus();
    if (value === "open" && chosen) openWith(app, pane, chosen, { makeDefault: always });
  };
  dlg.querySelectorAll(".dialog-buttons .btn").forEach((b) => b.addEventListener("click", () => finish(b.value)));
  dlg.addEventListener("cancel", (ev) => { ev.preventDefault(); finish("cancel"); });
  document.body.append(dlg);
  dlg.showModal();
  dlg.querySelector(".btn.primary").focus();
}
