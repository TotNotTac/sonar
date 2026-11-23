// Configure Sonar (Ctrl+Shift+, / ⌘,): startup, view defaults, thumbnails and
// confirmations. Changes apply as they are made.

import { resetSettings, saveSettings, settings } from "./store.js";
import { el } from "./util.js";

const SORT_KEYS = { name: "Name", size: "Size", modified: "Modified", type: "Type" };

export function showSettings(app) {
  if (document.querySelector("dialog.settings")) return;
  const dlg = el("dialog", "dialog settings");
  dlg.innerHTML = `
    <form class="dialog-body" method="dialog">
      <h2 class="dialog-title">Configure Sonar</h2>

      <fieldset class="settings-group">
        <legend>Startup</legend>
        <label class="dialog-check"><input type="radio" name="startup" value="session"> Restore the previous session</label>
        <label class="dialog-check"><input type="radio" name="startup" value="home"> Open the Home folder</label>
        <label class="dialog-check"><input type="radio" name="startup" value="folder"> Open this folder:</label>
        <div class="settings-folder">
          <input class="text-input" name="startupFolder" spellcheck="false" autocomplete="off" aria-label="Startup folder">
          <button type="button" class="btn" data-act="current">Use Current</button>
        </div>
      </fieldset>

      <fieldset class="settings-group">
        <legend>View</legend>
        <div class="settings-row">
          <label for="set-view">Default view</label>
          <select id="set-view" class="text-input" name="viewMode">
            <option value="details">Details</option>
            <option value="icons">Icons</option>
          </select>
        </div>
        <div class="settings-row">
          <label for="set-sort">Sort by</label>
          <span class="settings-pair">
            <select id="set-sort" class="text-input" name="sortKey">
              ${Object.entries(SORT_KEYS).map(([k, label]) => `<option value="${k}">${label}</option>`).join("")}
            </select>
            <select class="text-input" name="sortOrder" aria-label="Sort order">
              <option value="ascending">Ascending</option>
              <option value="descending">Descending</option>
            </select>
          </span>
        </div>
        <label class="dialog-check"><input type="checkbox" name="rememberViewPerFolder"> Remember the view for each folder</label>
        <label class="dialog-check"><input type="checkbox" name="showHidden"> Show hidden files</label>
        <label class="dialog-check"><input type="checkbox" name="thumbnails"> Show image thumbnails</label>
      </fieldset>

      <fieldset class="settings-group">
        <legend>Ask for confirmation when</legend>
        <label class="dialog-check"><input type="checkbox" name="confirmTrash"> Moving items to the Trash</label>
        <label class="dialog-check"><input type="checkbox" name="confirmDelete"> Deleting items permanently</label>
      </fieldset>

      <div class="dialog-buttons">
        <button type="button" class="btn settings-reset" data-act="reset">Restore Defaults</button>
        <button type="button" class="btn primary" data-act="close">Close</button>
      </div>
    </form>`;
  const form = dlg.querySelector("form");
  const field = (name) => form.elements[name];

  const fill = () => {
    const s = settings();
    for (const r of form.querySelectorAll('[name="startup"]')) r.checked = r.value === s.startup;
    field("startupFolder").value = s.startupFolder;
    field("startupFolder").disabled = s.startup !== "folder";
    dlg.querySelector('[data-act="current"]').disabled = s.startup !== "folder";
    field("viewMode").value = s.viewMode;
    field("sortKey").value = s.sort.key;
    field("sortOrder").value = s.sort.order;
    for (const k of ["rememberViewPerFolder", "thumbnails", "confirmTrash", "confirmDelete"]) field(k).checked = s[k];
    field("showHidden").checked = app.showHidden;
  };

  form.addEventListener("change", (ev) => {
    const t = ev.target;
    const s = settings();
    switch (t.name) {
      case "startup":
        saveSettings({ startup: t.value });
        if (t.value === "folder" && !s.startupFolder) saveSettings({ startupFolder: app.pane?.path ?? app.home ?? "" });
        break;
      case "startupFolder":
        saveSettings({ startupFolder: t.value.trim() });
        break;
      case "viewMode":
        saveSettings({ viewMode: t.value });
        app.viewSettingsChanged();
        break;
      case "sortKey":
      case "sortOrder":
        saveSettings({ sort: { key: field("sortKey").value, order: field("sortOrder").value } });
        app.viewSettingsChanged();
        break;
      case "rememberViewPerFolder":
        saveSettings({ rememberViewPerFolder: t.checked });
        app.viewSettingsChanged();
        break;
      case "showHidden":
        if (t.checked !== app.showHidden) app.toggleHidden();
        break;
      case "thumbnails":
        saveSettings({ thumbnails: t.checked });
        app.forEachPane((p) => p.render());
        break;
      case "confirmTrash":
      case "confirmDelete":
        saveSettings({ [t.name]: t.checked });
        break;
    }
    fill();
  });

  const close = () => {
    // A typed folder is saved on change, which fires on blur; Escape skips that.
    if (settings().startup === "folder") saveSettings({ startupFolder: field("startupFolder").value.trim() });
    dlg.close();
    dlg.remove();
    app.pane?.scroller.focus();
  };
  dlg.addEventListener("click", (ev) => {
    const act = ev.target.closest("[data-act]")?.dataset.act;
    if (act === "close") close();
    if (act === "current") {
      field("startupFolder").value = app.pane?.path ?? "";
      saveSettings({ startupFolder: field("startupFolder").value });
    }
    if (act === "reset") {
      resetSettings();
      if (app.showHidden) app.toggleHidden();
      app.viewSettingsChanged();
      app.forEachPane((p) => p.render());
      fill();
    }
  });
  dlg.addEventListener("cancel", (ev) => { ev.preventDefault(); close(); });

  fill();
  document.body.append(dlg);
  dlg.showModal();
  dlg.querySelector('[data-act="close"]').focus();
}
