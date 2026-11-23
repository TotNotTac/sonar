//! macOS clipboard via NSPasteboard, offering what Finder offers:
//!
//! - one item per file with a `public.file-url` (modern apps, Finder),
//! - the legacy `NSFilenamesPboardType` list (older AppKit apps such as
//!   Terminal.app),
//! - plain text with the full paths (Electron apps and terminals that only
//!   read text; Claude Code turns a pasted image path into an attachment).
//!
//! Sonar used to write only the file URLs, so pasting into text-only apps
//! produced nothing.
//!
//! macOS has no "cut" for files on the pasteboard. Sonar remembers the
//! pasteboard change count of its own cut; if the pasteboard is unchanged at
//! paste time, the paste is a move. (⌘⌥V forces a move, as in Finder.)

use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2_app_kit::{
    NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardTypeString, NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSString};
use sonar_core::clipboard::{ClipMode, FileClip, path_to_uri, to_plain_text, uri_to_path};

/// The UTI AppKit maps the legacy `NSFilenamesPboardType` to (a list of
/// paths). Older AppKit apps, including Terminal.app, read this.
const LEGACY_FILENAMES_UTI: &str = "dyn.ah62d4rv4gu8y6y4grf0gn5xbrzw1gydcr7u1e3cytf2gn";

/// Change count right after our last cut, if the last write was a cut.
static OUR_CUT: Mutex<Option<isize>> = Mutex::new(None);

fn our_cut() -> std::sync::MutexGuard<'static, Option<isize>> {
    OUR_CUT.lock().unwrap_or_else(|p| p.into_inner())
}

pub fn write_files(clip: &FileClip) -> Result<(), String> {
    let pb = NSPasteboard::generalPasteboard();
    write_files_to(&pb, clip)?;
    *our_cut() = (clip.mode == ClipMode::Cut).then(|| pb.changeCount());
    Ok(())
}

pub fn read_files() -> Option<FileClip> {
    let pb = NSPasteboard::generalPasteboard();
    let mut clip = read_files_from(&pb)?;
    if *our_cut() == Some(pb.changeCount()) {
        clip.mode = ClipMode::Cut;
    }
    Some(clip)
}

pub fn write_text(text: &str) -> Result<(), String> {
    let pb = NSPasteboard::generalPasteboard();
    pb.clearContents();
    *our_cut() = None;
    // SAFETY: reading an immutable AppKit constant.
    let ok = pb.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
    ok.then_some(()).ok_or_else(|| "could not write text".into())
}

pub fn clear_cut() {
    let pb = NSPasteboard::generalPasteboard();
    let mut cut = our_cut();
    if *cut == Some(pb.changeCount()) {
        pb.clearContents();
    }
    *cut = None;
}

fn write_files_to(pb: &NSPasteboard, clip: &FileClip) -> Result<(), String> {
    if clip.paths.is_empty() {
        return Err("nothing to copy".into());
    }
    let items: Vec<Retained<NSPasteboardItem>> = clip
        .paths
        .iter()
        .map(|p| {
            let item = NSPasteboardItem::new();
            // SAFETY: reading an immutable AppKit constant.
            item.setString_forType(&NSString::from_str(&path_to_uri(p)), unsafe { NSPasteboardTypeFileURL });
            item
        })
        .collect();
    // Plain text (all paths) and the legacy file list ride on the first item,
    // where Finder puts them. The list is set under its UTI directly: going
    // through the legacy pasteboard API instead makes AppKit regenerate the
    // item's file URL from it (decomposing non-ASCII names).
    let names: Vec<Retained<NSString>> = clip.paths.iter().map(|p| NSString::from_str(&p.to_string_lossy())).collect();
    let list = NSArray::from_retained_slice(&names);
    let plist: &AnyObject = &list;
    // SAFETY: reading an immutable AppKit constant; an array of path strings
    // is the documented property list for the file-list type.
    unsafe {
        items[0].setString_forType(&NSString::from_str(&to_plain_text(&clip.paths)), NSPasteboardTypeString);
        items[0].setPropertyList_forType(plist, &NSString::from_str(LEGACY_FILENAMES_UTI));
    }

    let writers: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
        items.into_iter().map(ProtocolObject::from_retained).collect();
    pb.clearContents();
    if !pb.writeObjects(&NSArray::from_retained_slice(&writers)) {
        return Err("the pasteboard refused the files".into());
    }
    Ok(())
}

fn read_files_from(pb: &NSPasteboard) -> Option<FileClip> {
    let items = pb.pasteboardItems()?;
    let paths: Vec<_> = items
        .iter()
        // SAFETY: reading an immutable AppKit constant.
        .filter_map(|item| item.stringForType(unsafe { NSPasteboardTypeFileURL }))
        .filter_map(|uri| uri_to_path(&uri.to_string()))
        .collect();
    (!paths.is_empty()).then_some(FileClip { mode: ClipMode::Copy, paths })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything Finder offers is there: file URLs, legacy file list, text.
    #[test]
    fn offers_the_formats_other_apps_read() {
        let pb = NSPasteboard::pasteboardWithUniqueName();
        let paths = vec!["/tmp/a b.png".into(), "/tmp/c.txt".into()];
        write_files_to(&pb, &FileClip { mode: ClipMode::Copy, paths }).unwrap();
        let items = pb.pasteboardItems().unwrap();
        assert_eq!(items.len(), 2);
        // SAFETY (all three): reading immutable AppKit constants.
        let text = items.iter().next().unwrap().stringForType(unsafe { NSPasteboardTypeString }).unwrap();
        assert_eq!(text.to_string(), "/tmp/a b.png\n/tmp/c.txt");
        // Read back through the legacy name: proves the UTI mapping is right.
        #[allow(deprecated)]
        let legacy = pb.propertyListForType(unsafe { objc2_app_kit::NSFilenamesPboardType }).expect("legacy file list");
        let legacy = legacy.downcast::<NSArray>().expect("an array");
        let names: Vec<String> = legacy.iter().map(|s| s.downcast::<NSString>().unwrap().to_string()).collect();
        assert_eq!(names, ["/tmp/a b.png", "/tmp/c.txt"]);
        let urls = pb.types().unwrap().iter().filter(|t| t.to_string() == "public.file-url").count();
        assert_eq!(urls, 1, "pasteboard-level types list each type once");
    }

    /// Uses a private, uniquely named pasteboard so the user's clipboard is untouched.
    #[test]
    fn files_round_trip_through_a_pasteboard() {
        let pb = NSPasteboard::pasteboardWithUniqueName();
        let clip = FileClip { mode: ClipMode::Copy, paths: vec!["/tmp/a b.txt".into(), "/Users/x/ü".into()] };
        write_files_to(&pb, &clip).unwrap();
        assert_eq!(read_files_from(&pb), Some(clip));
        pb.clearContents();
        assert_eq!(read_files_from(&pb), None);
    }
}

/// Checks interoperability the way other apps read file clipboards
/// (AppleScript, used by e.g. terminal tools). Uses the real clipboard, so
/// it's opt-in: `cargo test -p sonar -- --ignored`.
#[cfg(test)]
mod system_tests {
    use super::*;

    #[test]
    #[ignore = "overwrites the user's clipboard"]
    fn other_apps_can_read_copied_files() {
        let dir = std::env::temp_dir().join("sonar clipboard test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("shot ü.png");
        std::fs::write(&file, b"x").unwrap();
        write_files(&FileClip { mode: ClipMode::Copy, paths: vec![file.clone()] }).unwrap();

        let out = std::process::Command::new("osascript")
            .args(["-e", "POSIX path of (the clipboard as «class furl»)"])
            .output()
            .unwrap();
        let got = String::from_utf8_lossy(&out.stdout).trim().to_string();
        assert_eq!(got, file.to_string_lossy(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
        // What text-only apps (terminals, Electron) see:
        let text = std::process::Command::new("pbpaste").output().unwrap();
        assert_eq!(String::from_utf8_lossy(&text.stdout), file.to_string_lossy());
        assert_eq!(read_files().unwrap().paths, vec![file]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
