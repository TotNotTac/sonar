// Passwords for encrypted archives. The backend reports `passwordRequired`
// (with the archive's path); we ask, hand the password over for the session,
// and the caller retries.

import { basename, el, invoke, shown } from "./util.js";

const pending = new Map(); // archive path -> Promise<boolean>, so parallel requests ask once

/** Ask for the password of `path`. Resolves to the password, or null if cancelled. */
export function askPassword(path, wrong = false) {
  const dlg = el("dialog", "dialog password-dialog");
  dlg.innerHTML = `
    <form class="dialog-body">
      <h2 class="dialog-title"></h2>
      <p class="dialog-message"></p>
      <input class="text-input" type="password" autocomplete="off" spellcheck="false" />
      <div class="dialog-buttons">
        <button type="button" class="btn" value="cancel">Cancel</button>
        <button type="submit" class="btn primary" value="ok">OK</button>
      </div>
    </form>`;
  dlg.querySelector(".dialog-title").textContent = shown(`“${basename(path)}” is password protected`);
  const msg = dlg.querySelector(".dialog-message");
  msg.textContent = wrong ? "That password is not correct. Try again." : "Enter the password to open its contents.";
  msg.classList.toggle("error", wrong);
  const input = dlg.querySelector("input");
  return new Promise((resolve) => {
    const finish = (value) => {
      dlg.close();
      dlg.remove();
      resolve(value);
    };
    dlg.querySelector("form").addEventListener("submit", (ev) => {
      ev.preventDefault();
      if (input.value) finish(input.value);
    });
    dlg.querySelector(".btn[value=cancel]").addEventListener("click", () => finish(null));
    dlg.addEventListener("cancel", (ev) => { ev.preventDefault(); finish(null); });
    document.body.append(dlg);
    dlg.showModal();
    input.focus();
  });
}

/**
 * If `err` (an error, or a finished job's `needsPassword`) is about a missing
 * or wrong archive password, ask for it. Resolves to true when the caller
 * should retry, false otherwise (not a password error, or cancelled).
 */
export function obtainPassword(err) {
  const need = err?.code === "passwordRequired" ? err : err?.path && "wrong" in err ? err : null;
  if (!need) return Promise.resolve(false);
  if (!pending.has(need.path)) {
    const p = askPassword(need.path, need.wrong)
      .then(async (password) => {
        if (password == null) return false;
        await invoke("set_archive_password", { path: need.path, password });
        return true;
      })
      .finally(() => pending.delete(need.path));
    pending.set(need.path, p);
  }
  return pending.get(need.path);
}
