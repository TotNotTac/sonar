//! Native menu bar (macOS). Items carry action ids that the frontend runs
//! through the same registry as keyboard shortcuts and Linux's ☰ menu.
//!
//! Cut/Copy/Paste/Redo are the system's predefined items so they keep
//! working in text fields; for files, the frontend handles the resulting
//! DOM clipboard events. Undo and Select All are custom items: the frontend
//! applies them to a focused text field or to the files, as appropriate.
//!
//! Only macOS installs the menu bar (Linux uses the in-window ☰ menu, like
//! Dolphin), but the definitions are compiled and tested everywhere.
#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use serde::Deserialize;
use tauri::menu::{AboutMetadata, CheckMenuItemBuilder, Menu, MenuItemBuilder, MenuItemKind, SubmenuBuilder};
use tauri::{AppHandle, Emitter, Runtime};

pub const EVENT_MENU: &str = "menu";

/// (id, label, accelerator) for plain items; ids are the frontend's action ids.
type Item = (&'static str, &'static str, Option<&'static str>);

const FILE: &[Item] = &[
    ("new-tab", "New Tab", Some("CmdOrCtrl+T")),
    ("new-folder", "New Folder", Some("CmdOrCtrl+Shift+N")),
    ("", "", None),
    ("open-location", "Go to Location…", Some("CmdOrCtrl+L")),
    ("", "", None),
    ("close-tab", "Close Tab", Some("CmdOrCtrl+W")),
    ("close-window", "Close Window", Some("CmdOrCtrl+Shift+W")),
];
const EDIT_FILES: &[Item] = &[
    ("paste-move", "Move Items Here", Some("CmdOrCtrl+Alt+V")),
    ("select-all", "Select All", Some("CmdOrCtrl+A")),
    ("", "", None),
    ("copy-location", "Copy Location", Some("CmdOrCtrl+Alt+C")),
    ("rename", "Rename", Some("F2")),
    ("", "", None),
    ("trash", "Move to Trash", Some("CmdOrCtrl+Backspace")),
    ("delete", "Delete Immediately…", Some("CmdOrCtrl+Alt+Backspace")),
];
const VIEW_CHECKS: &[Item] = &[
    ("view-icons", "as Icons", Some("CmdOrCtrl+1")),
    ("view-details", "as Details", Some("CmdOrCtrl+2")),
    ("show-hidden", "Show Hidden Files", Some("CmdOrCtrl+Shift+Period")),
    ("split", "Split View", Some("F3")),
];
const VIEW_ITEMS: &[Item] = &[
    ("search", "Find…", Some("CmdOrCtrl+F")),
    ("filter", "Filter…", Some("CmdOrCtrl+Alt+F")),
    ("reload", "Reload", Some("CmdOrCtrl+R")),
];
const GO: &[Item] = &[
    ("back", "Back", Some("CmdOrCtrl+BracketLeft")),
    ("forward", "Forward", Some("CmdOrCtrl+BracketRight")),
    ("up", "Enclosing Folder", Some("CmdOrCtrl+ArrowUp")),
    ("", "", None),
    ("go-home", "Home", Some("CmdOrCtrl+Shift+H")),
    ("go-desktop", "Desktop", Some("CmdOrCtrl+Shift+D")),
    ("go-documents", "Documents", Some("CmdOrCtrl+Shift+O")),
    ("go-downloads", "Downloads", Some("CmdOrCtrl+Alt+L")),
    ("go-trash", "Trash", None),
    ("", "", None),
    ("reopen-tab", "Reopen Closed Tab", Some("CmdOrCtrl+Shift+T")),
];
const WINDOW: &[Item] =
    &[("next-tab", "Show Next Tab", Some("Ctrl+Tab")), ("prev-tab", "Show Previous Tab", Some("Ctrl+Shift+Tab"))];

type Sub<'m, R> = SubmenuBuilder<'m, R, AppHandle<R>>;

fn items<'m, R: Runtime>(app: &'m AppHandle<R>, mut sub: Sub<'m, R>, list: &[Item]) -> tauri::Result<Sub<'m, R>> {
    for &(id, text, accel) in list {
        if id.is_empty() {
            sub = sub.separator();
            continue;
        }
        let mut b = MenuItemBuilder::with_id(id, text);
        if let Some(a) = accel {
            b = b.accelerator(a);
        }
        sub = sub.item(&b.build(app)?);
    }
    Ok(sub)
}

fn checks<'m, R: Runtime>(app: &'m AppHandle<R>, mut sub: Sub<'m, R>, list: &[Item]) -> tauri::Result<Sub<'m, R>> {
    for &(id, text, accel) in list {
        let mut b = CheckMenuItemBuilder::with_id(id, text);
        if let Some(a) = accel {
            b = b.accelerator(a);
        }
        sub = sub.item(&b.build(app)?);
    }
    Ok(sub)
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let about = AboutMetadata {
        name: Some("Sonar".into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        comments: Some("A file manager inspired by KDE Dolphin".into()),
        ..Default::default()
    };
    let app_menu = SubmenuBuilder::new(app, "Sonar")
        .about(Some(about))
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;
    let file = items(app, SubmenuBuilder::new(app, "File"), FILE)?.build()?;
    let undo = MenuItemBuilder::with_id("undo", "Undo").accelerator("CmdOrCtrl+Z").build(app)?;
    let edit = SubmenuBuilder::new(app, "Edit").item(&undo).redo().separator().cut().copy().paste();
    let edit = items(app, edit, EDIT_FILES)?.build()?;
    let view = checks(app, SubmenuBuilder::new(app, "View"), VIEW_CHECKS)?.separator();
    let view = items(app, view, VIEW_ITEMS)?.separator().fullscreen().build()?;
    let go = items(app, SubmenuBuilder::new(app, "Go"), GO)?.build()?;
    let window = SubmenuBuilder::new(app, "Window").minimize().maximize().separator();
    let window = items(app, window, WINDOW)?.separator().bring_all_to_front().build()?;
    Menu::with_items(app, &[&app_menu, &file, &edit, &view, &go, &window])
}

/// Install the menu bar and forward clicks to the frontend as `menu` events.
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    match build(app).and_then(|m| app.set_menu(m)) {
        Ok(_) => app.on_menu_event(|app, event| {
            let _ = app.emit(EVENT_MENU, event.id().as_ref());
        }),
        // A broken menu must never stop the app from starting.
        Err(e) => eprintln!("sonar: could not build the menu bar: {e}"),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MenuState {
    view_mode: String,
    show_hidden: bool,
    split: bool,
    can_go_back: bool,
    can_go_forward: bool,
    can_go_up: bool,
}

#[tauri::command]
pub fn has_native_menu() -> bool {
    cfg!(target_os = "macos")
}

/// Mirror the active pane's state in check marks and enabled items.
#[tauri::command]
pub fn set_menu_state<R: Runtime>(app: AppHandle<R>, state: MenuState) {
    let Some(menu) = app.menu() else { return };
    let find = |id: &str| find_item(&menu.items().unwrap_or_default(), id);
    let check = |id: &str, on: bool| {
        if let Some(MenuItemKind::Check(c)) = find(id) {
            let _ = c.set_checked(on);
        }
    };
    let enable = |id: &str, on: bool| {
        if let Some(MenuItemKind::MenuItem(m)) = find(id) {
            let _ = m.set_enabled(on);
        }
    };
    check("view-icons", state.view_mode == "icons");
    check("view-details", state.view_mode == "details");
    check("show-hidden", state.show_hidden);
    check("split", state.split);
    enable("back", state.can_go_back);
    enable("forward", state.can_go_forward);
    enable("up", state.can_go_up);
}

fn find_item<R: Runtime>(items: &[MenuItemKind<R>], id: &str) -> Option<MenuItemKind<R>> {
    items.iter().find_map(|item| match item {
        MenuItemKind::Submenu(s) => find_item(&s.items().unwrap_or_default(), id),
        other if other.id().as_ref() == id => Some(other.clone()),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerators_parse() {
        let all = [FILE, EDIT_FILES, VIEW_CHECKS, VIEW_ITEMS, GO, WINDOW].concat();
        for (id, _, accel) in all {
            if let Some(a) = accel {
                a.parse::<muda::accelerator::Accelerator>().unwrap_or_else(|e| panic!("{id}: {a}: {e}"));
            }
        }
        "CmdOrCtrl+Z".parse::<muda::accelerator::Accelerator>().unwrap();
    }

    #[test]
    fn ids_are_unique() {
        let all = [FILE, EDIT_FILES, VIEW_CHECKS, VIEW_ITEMS, GO, WINDOW].concat();
        let ids: Vec<_> = all.iter().map(|i| i.0).filter(|i| !i.is_empty()).collect();
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len());
    }
}
