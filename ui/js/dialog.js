// Modal dialogs (conflicts, confirmations). Dialogs are queued so concurrent
// jobs can never stack them.

import { shown } from "./util.js";

let chain = Promise.resolve();

/**
 * Resolves to `{ value, checked }`. Escape resolves to the button marked
 * `cancel`.
 * @param {{title: string, message?: string, details?: [string, string][],
 *          buttons: {label: string, value: string, primary?: boolean, danger?: boolean, cancel?: boolean}[],
 *          checkbox?: string}} opts
 */
export function ask(opts) {
  const p = chain.then(() => show(opts));
  chain = p.catch(() => {});
  return p;
}

function show({ title, message = "", details = [], buttons, checkbox }) {
  const dlg = document.getElementById("dialog");
  return new Promise((resolve) => {
    const cancelValue = buttons.find((b) => b.cancel)?.value ?? "cancel";
    dlg.innerHTML = `
      <form method="dialog" class="dialog-body">
        <h2 class="dialog-title"></h2>
        <p class="dialog-message"></p>
        <dl class="dialog-details"></dl>
        <label class="dialog-check" hidden><input type="checkbox" /> <span></span></label>
        <div class="dialog-buttons"></div>
      </form>`;
    dlg.querySelector(".dialog-title").textContent = shown(title);
    const msg = dlg.querySelector(".dialog-message");
    msg.textContent = shown(message);
    msg.hidden = !message;
    const dl = dlg.querySelector(".dialog-details");
    for (const [k, v] of details) {
      const dt = document.createElement("dt");
      const dd = document.createElement("dd");
      dt.textContent = k;
      dd.textContent = shown(v);
      dl.append(dt, dd);
    }
    dl.hidden = !details.length;
    const check = dlg.querySelector(".dialog-check");
    if (checkbox) {
      check.hidden = false;
      check.querySelector("span").textContent = checkbox;
    }
    const row = dlg.querySelector(".dialog-buttons");
    let primary = null;
    for (const b of buttons) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.textContent = b.label;
      btn.className = ["btn", b.primary && "primary", b.danger && "danger"].filter(Boolean).join(" ");
      btn.addEventListener("click", () => finish(b.value));
      row.append(btn);
      if (b.primary) primary = btn;
    }
    const finish = (value) => {
      dlg.removeEventListener("cancel", onCancel);
      const checked = check.querySelector("input").checked;
      dlg.close();
      resolve({ value, checked });
    };
    const onCancel = (ev) => {
      ev.preventDefault();
      finish(cancelValue);
    };
    dlg.addEventListener("cancel", onCancel);
    dlg.showModal();
    (primary ?? row.lastElementChild)?.focus();
  });
}
