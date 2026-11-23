//! Data formats for file lists on the clipboard and in drag and drop.
//!
//! Pure functions only; the native clipboard code (NSPasteboard, GTK) lives in
//! the app crate because it must run on the UI thread.
//!
//! Formats:
//! - `text/uri-list` (RFC 2483): one `file://` URI per line, CRLF-separated.
//!   Read and written by every Linux file manager and by macOS.
//! - `x-special/gnome-copied-files`: `copy` or `cut`, then one URI per line.
//!   Nautilus, Nemo, Thunar, and Dolphin (for interop) use it.
//! - `application/x-kde-cutselection`: `1` when the URIs on the clipboard
//!   were cut rather than copied (Dolphin).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipMode {
    Copy,
    Cut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileClip {
    pub mode: ClipMode,
    pub paths: Vec<PathBuf>,
}

/// `/a b/ü.txt` -> `file:///a%20b/%C3%BC.txt`. Bytes outside the RFC 3986
/// unreserved set (and `/`) are percent-encoded, so any Unix path round-trips.
pub fn path_to_uri(path: &Path) -> String {
    let mut out = String::from("file://");
    for &b in path_bytes(path).iter() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Inverse of [`path_to_uri`]. Accepts `file:///p` and `file://localhost/p`;
/// returns `None` for other schemes or remote hosts.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.trim().strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None; // file://host/... names another machine
    }
    let raw = rest.as_bytes();
    let mut bytes = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' && i + 2 < raw.len() {
            let hex = std::str::from_utf8(&raw[i + 1..i + 3]).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            bytes.push(raw[i]);
            i += 1;
        }
    }
    Some(path_from_bytes(bytes))
}

/// RFC 2483 `text/uri-list`.
pub fn to_uri_list(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| format!("{}\r\n", path_to_uri(p))).collect()
}

/// Parse `text/uri-list`, skipping comments and non-file URIs.
pub fn parse_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(uri_to_path).collect()
}

/// `x-special/gnome-copied-files`.
pub fn to_gnome_copied_files(clip: &FileClip) -> String {
    let verb = match clip.mode {
        ClipMode::Copy => "copy",
        ClipMode::Cut => "cut",
    };
    let uris: Vec<String> = clip.paths.iter().map(|p| path_to_uri(p)).collect();
    format!("{verb}\n{}", uris.join("\n"))
}

pub fn parse_gnome_copied_files(text: &str) -> Option<FileClip> {
    let mut lines = text.lines();
    let mode = match lines.next()?.trim() {
        "copy" => ClipMode::Copy,
        "cut" => ClipMode::Cut,
        _ => return None,
    };
    let paths: Vec<PathBuf> = lines.filter_map(uri_to_path).collect();
    (!paths.is_empty()).then_some(FileClip { mode, paths })
}

/// `application/x-kde-cutselection` payload.
pub fn kde_cut_selection(mode: ClipMode) -> &'static str {
    match mode {
        ClipMode::Copy => "0",
        ClipMode::Cut => "1",
    }
}

/// Plain-text fallback: one path per line, for pasting into a terminal or editor.
pub fn to_plain_text(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>().join("\n")
}

#[cfg(unix)]
fn path_bytes(p: &Path) -> std::borrow::Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    p.as_os_str().as_bytes().into()
}

#[cfg(not(unix))]
fn path_bytes(p: &Path) -> std::borrow::Cow<'_, [u8]> {
    p.to_string_lossy().into_owned().into_bytes().into()
}

#[cfg(unix)]
fn path_from_bytes(b: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(b).into()
}

#[cfg(not(unix))]
fn path_from_bytes(b: Vec<u8>) -> PathBuf {
    String::from_utf8_lossy(&b).into_owned().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_round_trip() {
        for p in ["/a b/ü.txt", "/x/100%.txt", "/plain/path-1_2.~", "/#hash?q"] {
            let uri = path_to_uri(Path::new(p));
            assert!(!uri[7..].contains(' ') && !uri[7..].contains('#') && !uri[7..].contains('?'), "{uri}");
            assert_eq!(uri_to_path(&uri).unwrap(), Path::new(p));
        }
        assert_eq!(path_to_uri(Path::new("/a b")), "file:///a%20b");
        assert_eq!(uri_to_path("file://localhost/tmp/x").unwrap(), Path::new("/tmp/x"));
        assert_eq!(uri_to_path("file://otherhost/tmp/x"), None);
        assert_eq!(uri_to_path("https://example.com/x"), None);
        assert_eq!(uri_to_path("file:///bad%zz"), None);
        assert_eq!(uri_to_path("file:///trailing%2"), Some(PathBuf::from("/trailing%2")));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_paths_round_trip() {
        use std::os::unix::ffi::OsStrExt;
        let p = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/\xff\xfe"));
        assert_eq!(uri_to_path(&path_to_uri(p)).unwrap(), p);
    }

    #[test]
    fn uri_list() {
        let paths = vec![PathBuf::from("/a"), PathBuf::from("/b c")];
        let text = to_uri_list(&paths);
        assert_eq!(text, "file:///a\r\nfile:///b%20c\r\n");
        assert_eq!(parse_uri_list(&format!("# comment\r\n{text}https://x.org\r\n\r\n")), paths);
    }

    #[test]
    fn gnome_format() {
        let clip = FileClip { mode: ClipMode::Cut, paths: vec!["/a".into(), "/b c".into()] };
        let text = to_gnome_copied_files(&clip);
        assert_eq!(text, "cut\nfile:///a\nfile:///b%20c");
        assert_eq!(parse_gnome_copied_files(&text), Some(clip));
        assert_eq!(parse_gnome_copied_files("copy\n"), None);
        assert_eq!(parse_gnome_copied_files("hello\nfile:///a"), None);
    }
}
