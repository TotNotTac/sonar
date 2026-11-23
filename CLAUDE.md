# Notes for Claude

Sonar: a Dolphin-inspired file manager (Tauri 2 + Rust), for macOS and Arch Linux. See README.md for
the layout, features and shortcuts.

## Working agreements

- Commit only when asked; ask before pushing. Commit as `samtoxie <sam@toxopeus.it>` (already set in the
  repo-local git config). Never use any other email.
- Push through the Bitwarden SSH agent, not 1Password:
  `GIT_SSH_COMMAND="ssh -o IdentityAgent=$HOME/.bitwarden-ssh-agent.sock" git push origin main`
- CI (`.github/workflows/ci.yml`) runs on every push; `build.yml` builds macOS + Linux binaries on push.

## Releasing

1. Tag the commit (`git tag -a vX.Y.Z -m "Sonar X.Y.Z" && git push origin vX.Y.Z`). `build.yml` attaches the
   AppImage, `.deb`, `.rpm` and `.dmg` to a draft release. macOS runners are sometimes unavailable; then rerun
   the failed job (`gh run rerun <id> --failed`).
2. Publish the release. `aur.yml` then updates `sonar` and `sonar-bin` on the AUR (version, checksums), tests
   `sonar-bin`, and commits `packaging/aur/` back to main. `sonar-git` needs nothing. Rerun it from the
   Actions tab (tag as input) if it fails. It needs the secret `AUR_SSH_KEY` (a key used only for this, whose
   public half is in the AUR account).

## Architecture rules

- `crates/sonar-core`: all filesystem logic, no Tauri dependency, unit-tested. OS differences live behind
  `src/platform/` (macos.rs / linux.rs / generic.rs).
- `src-tauri`: thin `#[tauri::command]` wrappers, events, menu, native clipboard/drag. Also the XDG
  Desktop Portal file chooser (`portal.rs`, Linux only): `sonar --portal` serves
  `org.freedesktop.impl.portal.FileChooser` over D-Bus; requests open picker windows (`picker.rs`,
  `ui/js/picker.js`). The `.portal`/`.service` files live in `src-tauri/linux/` and are installed by the
  `.deb`/`.rpm` (`bundle.linux.*.files`) and the AUR packages — not by the AppImage (can't register
  system-wide). Users opt in via `~/.config/xdg-desktop-portal/portals.conf`.
- `ui/`: vanilla ES modules, no bundler. Backend errors are `{ code, message }` (see `error.rs`).
- Archives go through the `bsdtar` CLI. Always pass `--passphrase` when reading, or bsdtar can hang
  waiting for a password.
- File operations never overwrite silently. They write to temporary paths and rename into place.

## Testing (run all before committing)

- `cargo test --workspace` and `cargo clippy --workspace --all-targets -- -D warnings`, after `cargo fmt --all`.
- `npx playwright test`: UI tests (Chromium + WebKit) run against `tests/ui/mock-backend.js`. When adding a
  backend command, add it to the mock too.
- Linux: run the core tests in Docker:
  `docker run --rm --platform linux/amd64 -v "$PWD":/src:ro archlinux:latest bash -c "sed -i 's/^\[options\]/[options]\nDisableSandbox/' /etc/pacman.conf && pacman -Sy --noconfirm --needed rust gcc shared-mime-info && cp -r /src /w && cd /w && cargo test -p sonar-core"`
  It takes a while under emulation.
- The macOS clipboard round-trip test uses the real pasteboard and is occasionally flaky; rerun before
  investigating.

## Roadmap

Done: browsing, file operations + undo, drag and drop, clipboard, tabs/split, thumbnails, volumes, archives
(including passwords and nesting), search, Properties, Open With, editable Places, default file manager on macOS,
and (Linux) the XDG portal file chooser for other apps' Open/Save dialogs.

Next: an integrated terminal panel (Dolphin's F4). It was postponed and hasn't been started.
