# Sonar

A file manager inspired by KDE Dolphin, built with Tauri 2 and Rust. Targets macOS and Arch Linux.

**Status:** browsing, live updates, opening files, and file operations (rename, new folder, trash, delete, copy/cut/paste with progress and conflict handling, undo). Clipboard is internal to Sonar for now.

## Layout

```
crates/sonar-core/      UI-agnostic filesystem model (no Tauri dependency)
  src/listing.rs        read a directory into sorted Entry values
  src/entry.rs          one row in the details view
  src/sort.rs           sort keys, folders first, natural ("file2" < "file10") ordering
  src/breadcrumb.rs     path to clickable segments (paths under $HOME start with "Home")
  src/places.rs         sidebar: standard user dirs from the `dirs` crate, trash, volumes
  src/paths.rs          resolve typed locations (~, relative, file://)
  src/filetype.rs       extension-based type labels and icon hints
  src/watch.rs          debounced folder watcher (FSEvents / inotify via `notify`)
  src/ops/              file operations
    basic.rs            rename (never overwrites), new folder, trash, permanent delete
    transfer.rs         copy/move engine: progress, cancel, conflicts, atomic .sonar-part writes
    undo.rs             reversible record of each operation
    names.rs            name validation, "name (1).ext" generation
  src/platform/         OS-specific behaviour behind one API
    macos.rs            UF_HIDDEN flag, /Volumes, `open`, NSFileManager trash, renamex_np, APFS clonefile
    linux.rs            `.hidden` files, /proc/self/mounts, `xdg-open`, FreeDesktop trash (`trash` crate), renameat2
    generic.rs          fallback so the crate still builds on other platforms
src-tauri/              Tauri shell: thin #[tauri::command] wrappers over sonar-core, plus per-view
                        state (current watcher, background item-count job)
ui/                     static frontend, native ES modules (no bundler)
  js/app.js             window: tabs, split view, toolbar, location bar, sidebar, keyboard routing
  js/pane.js            one folder view: navigation, history, selection, filter, live updates
  js/views.js           details and icons renderers
  js/ops.js             file operations, jobs panel, conflict dialog, undo
  js/menu.js            HTML context menus (+ contextmenus.js for their contents)
  js/store.js           settings, per-folder view properties
```

## Prerequisites

- Rust (stable) and Node.js
- **macOS:** Xcode Command Line Tools
- **Arch Linux:** `sudo pacman -S --needed webkit2gtk-4.1 base-devel curl wget file openssl appmenu-gtk-module libappindicator-gtk3 librsvg`

## Running

```sh
npm install
npm run dev      # tauri dev
npm run build    # release bundle (.app/.dmg on macOS; .deb/.rpm/AppImage on Linux)
```

## Tests

```sh
cargo test --workspace                   # core: listing, sorting, ops engine, undo, watcher, platform
cargo test -p sonar-core -- --ignored    # also round-trips a file through the real Trash
npx playwright install chromium webkit   # once
npm run test:ui                          # UI suite in Chromium and WebKit
```

The UI tests (`tests/ui/`) run the real frontend against `mock-backend.js`, an in-memory filesystem that
implements every Tauri command and event, so they cover full flows (navigation, tabs, split view, rename,
copy with conflicts, undo, live refresh) without touching disk. WebKit is the engine Tauri uses on both
macOS and Linux. CI (`.github/workflows/ci.yml`) runs everything on Ubuntu and macOS, and the core crate on
Arch Linux.

## Keyboard

| Action | Linux | macOS |
|---|---|---|
| Back / Forward | Alt+← / Alt+→ | ⌘[ / ⌘] |
| Up | Alt+↑, Backspace | ⌘↑, Backspace |
| Open folder / file | Enter, double-click | Enter, ⌘↓, double-click |
| Edit location | Ctrl+L, F6, click empty space in the bar | ⌘L, F6 |
| Show hidden files | Ctrl+H, Alt+. | ⌘⇧., Ctrl+H |
| Reload | F5, Ctrl+R | ⌘R |
| Select | click, Shift/Ctrl+click, Shift+arrows, Ctrl+A | ⌘ instead of Ctrl |

### File operations

| Action | Linux | macOS |
|---|---|---|
| Rename | F2 | F2 |
| New folder | Ctrl+Shift+N | ⌘⇧N |
| Move to Trash | Delete | ⌘⌫, Delete |
| Delete permanently (asks first) | Shift+Delete | ⌘⌥⌫ |
| Copy / Cut / Paste | Ctrl+C / X / V | ⌘C / X / V |
| Undo | Ctrl+Z | ⌘Z |

Deleting inside the Trash always deletes permanently. Copies and moves run in the background with a progress
panel; name clashes ask Skip / Keep Both / Replace (or Merge for folders), optionally for all remaining conflicts.

### Views, tabs and split

| Action | Linux | macOS |
|---|---|---|
| Icons / Details view | Ctrl+1 / Ctrl+3 | ⌘1 / ⌘2 |
| Filter current folder | Ctrl+I | ⌘F |
| Split view on/off | F3 | F3 |
| New tab / Close tab | Ctrl+T / Ctrl+W | ⌘T / ⌘W |
| Reopen closed tab | Ctrl+Shift+T | ⌘⇧T |
| Next / previous tab | Ctrl+Tab / Ctrl+Shift+Tab | ⌃Tab, ⌘⇧] / ⌘⇧[ |

Tabs (and their split views) are restored on the next launch. Drag tabs to reorder them, double-click empty
tab-bar space for a new tab, and middle-click a tab to close it. Closing the last tab closes the window.
Middle-click a folder, place or breadcrumb to open it in a background tab. Right-click items, empty space,
places, breadcrumbs or tabs for context menus. View mode and sort order are remembered per folder; hidden-file
visibility is global.

Type a few letters to jump to a matching entry. Mouse back/forward buttons also work.
