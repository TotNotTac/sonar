//! Linux clipboard via GTK (works on X11 and Wayland). Sonar offers the
//! formats Dolphin, Nautilus and friends read, and reads them back:
//!
//! - `x-special/gnome-copied-files` (copy/cut + URIs)
//! - `text/uri-list` + `application/x-kde-cutselection`
//! - plain text (paths), for terminals and editors

use std::sync::atomic::{AtomicU64, Ordering};

use gtk::{Clipboard, TargetEntry, TargetFlags, gdk};
use sonar_core::clipboard::{
    ClipMode, FileClip, kde_cut_selection, parse_gnome_copied_files, parse_uri_list, to_gnome_copied_files,
    to_plain_text, to_uri_list,
};

const GNOME: &str = "x-special/gnome-copied-files";
const URI_LIST: &str = "text/uri-list";
const KDE_CUT: &str = "application/x-kde-cutselection";

/// Generation of the clipboard contents we currently own (0 = not ours).
/// GTK drops our data closure when another app takes the clipboard; a guard
/// inside the closure resets this.
static OWNED: AtomicU64 = AtomicU64::new(0);
static NEXT: AtomicU64 = AtomicU64::new(1);

struct Ownership(u64);
impl Drop for Ownership {
    fn drop(&mut self) {
        let _ = OWNED.compare_exchange(self.0, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

fn clipboard() -> Clipboard {
    Clipboard::get(&gdk::SELECTION_CLIPBOARD)
}

pub fn write_files(clip: &FileClip) -> Result<(), String> {
    let payloads = [
        to_gnome_copied_files(clip),
        to_uri_list(&clip.paths),
        kde_cut_selection(clip.mode).to_string(),
        to_plain_text(&clip.paths),
    ];
    let targets = [
        TargetEntry::new(GNOME, TargetFlags::empty(), 0),
        TargetEntry::new(URI_LIST, TargetFlags::empty(), 1),
        TargetEntry::new(KDE_CUT, TargetFlags::empty(), 2),
        TargetEntry::new("UTF8_STRING", TargetFlags::empty(), 3),
        TargetEntry::new("text/plain;charset=utf-8", TargetFlags::empty(), 3),
        TargetEntry::new("text/plain", TargetFlags::empty(), 3),
    ];
    let generation = NEXT.fetch_add(1, Ordering::SeqCst);
    let guard = Ownership(generation);
    let ok = clipboard().set_with_data(&targets, move |_, selection, info| {
        let _keep_alive = &guard;
        if let Some(data) = payloads.get(info as usize) {
            selection.set(&selection.target(), 8, data.as_bytes());
        }
    });
    if !ok {
        return Err("another application holds the clipboard".into());
    }
    OWNED.store(generation, Ordering::SeqCst);
    Ok(())
}

pub fn read_files() -> Option<FileClip> {
    let cb = clipboard();
    let read = |target: &str| {
        cb.wait_for_contents(&gdk::Atom::intern(target))
            .map(|d| String::from_utf8_lossy(&d.data()).into_owned())
            .filter(|s| !s.is_empty())
    };
    if let Some(clip) = read(GNOME).and_then(|t| parse_gnome_copied_files(&t)) {
        return Some(clip);
    }
    let paths = parse_uri_list(&read(URI_LIST)?);
    if paths.is_empty() {
        return None;
    }
    let cut = read(KDE_CUT).is_some_and(|t| t.trim_start().starts_with('1'));
    Some(FileClip { mode: if cut { ClipMode::Cut } else { ClipMode::Copy }, paths })
}

pub fn write_text(text: &str) -> Result<(), String> {
    clipboard().set_text(text);
    OWNED.store(0, Ordering::SeqCst);
    Ok(())
}

pub fn clear_cut() {
    if OWNED.load(Ordering::SeqCst) != 0 {
        clipboard().clear();
    }
}
