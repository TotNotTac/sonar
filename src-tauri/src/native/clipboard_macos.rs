//! macOS clipboard via NSPasteboard, the same way Finder does it: one
//! pasteboard item per file carrying a `public.file-url`.
//!
//! macOS has no "cut" for files on the pasteboard. Sonar remembers the
//! pasteboard change count of its own cut; if the pasteboard is unchanged at
//! paste time, the paste is a move. (⌘⌥V forces a move, as in Finder.)

use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardTypeString, NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSString};
use sonar_core::clipboard::{path_to_uri, uri_to_path, ClipMode, FileClip};

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
    let items: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> = clip
        .paths
        .iter()
        .map(|p| {
            let item = NSPasteboardItem::new();
            // SAFETY: reading an immutable AppKit constant.
            item.setString_forType(&NSString::from_str(&path_to_uri(p)), unsafe { NSPasteboardTypeFileURL });
            ProtocolObject::from_retained(item)
        })
        .collect();
    pb.clearContents();
    if pb.writeObjects(&NSArray::from_retained_slice(&items)) {
        Ok(())
    } else {
        Err("the pasteboard refused the files".into())
    }
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
