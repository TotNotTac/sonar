# Notes for Claude

Sonar: a Dolphin-inspired file manager (Tauri 2 + Rust), for macOS and Arch Linux. See README.md for
the layout, features and shortcuts.

## Working agreements

- Commit only when asked; ask before pushing. Commit as `samtoxie <sam@toxopeus.it>` (already set in the
  repo-local git config). Never use any other email.
- Push through the Bitwarden SSH agent, not 1Password:
  `GIT_SSH_COMMAND="ssh -o IdentityAgent=$HOME/.bitwarden-ssh-agent.sock" git push origin main`
- CI (`.github/workflows/ci.yml`) runs on every push; `build.yml` builds macOS + Linux binaries on push.

## Architecture rules

- `crates/sonar-core`: all filesystem logic, no Tauri dependency, unit-tested. OS differences live behind
  `src/platform/` (macos.rs / linux.rs / generic.rs).
- `src-tauri`: thin `#[tauri::command]` wrappers, events, menu, native clipboard/drag.
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
(including passwords and nesting), search, Properties, Open With, editable Places, default file manager on macOS.

Next: an integrated terminal panel (Dolphin's F4). It was postponed and hasn't been started.
