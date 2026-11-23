//! Item details for the Properties dialog, and recursive size totals.

use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::entry::EntryKind;
use crate::error::{CoreError, Result};
use crate::filetype;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Properties {
    pub name: String,
    pub path: PathBuf,
    /// "file", "folder", "symlink" or "other".
    pub kind: &'static str,
    pub type_label: String,
    /// Bytes, for files (folders: see [`total_size`]).
    pub size: Option<u64>,
    /// Space actually used on disk (allocated blocks), for files.
    pub size_on_disk: Option<u64>,
    pub created: Option<i64>,
    pub modified: Option<i64>,
    pub accessed: Option<i64>,
    /// "rwxr-xr-x" and "755".
    pub permissions: Option<String>,
    pub mode: Option<String>,
    pub owner: Option<String>,
    pub group: Option<String>,
    pub link_target: Option<PathBuf>,
}

fn ms(t: std::io::Result<SystemTime>) -> Option<i64> {
    t.ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64)
}

pub fn properties(path: &Path) -> Result<Properties> {
    let link = fs::symlink_metadata(path).map_err(|e| CoreError::from_io(path, e))?;
    let is_link = link.file_type().is_symlink();
    // Describe what a symlink points at (like Finder/Dolphin), but keep its own target.
    let meta = if is_link { fs::metadata(path).unwrap_or_else(|_| link.clone()) } else { link.clone() };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    let entry_kind = if meta.is_dir() {
        EntryKind::Directory
    } else if meta.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    let kind = match entry_kind {
        _ if is_link && fs::metadata(path).is_err() => "symlink",
        EntryKind::Directory => "folder",
        EntryKind::File => "file",
        _ => "other",
    };
    let (permissions, mode, owner, group) = unix_details(&meta);
    Ok(Properties {
        type_label: filetype::describe_native(&name, path, entry_kind).label,
        size: meta.is_file().then_some(meta.len()),
        size_on_disk: meta.is_file().then(|| disk_usage(&meta)),
        created: ms(meta.created()),
        modified: ms(meta.modified()),
        accessed: ms(meta.accessed()),
        permissions,
        mode,
        owner,
        group,
        link_target: if is_link { fs::read_link(path).ok() } else { None },
        name,
        path: path.to_path_buf(),
        kind,
    })
}

#[cfg(unix)]
fn disk_usage(meta: &Metadata) -> u64 {
    use std::os::unix::fs::MetadataExt;
    meta.blocks() * 512 // st_blocks is always in 512-byte units
}

#[cfg(not(unix))]
fn disk_usage(meta: &Metadata) -> u64 {
    meta.len()
}

#[cfg(unix)]
fn unix_details(meta: &Metadata) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    use std::os::unix::fs::MetadataExt;
    let mode = meta.mode() & 0o7777;
    (Some(rwx(mode)), Some(format!("{:o}", mode & 0o777)), user_name(meta.uid()), group_name(meta.gid()))
}

#[cfg(not(unix))]
fn unix_details(_meta: &Metadata) -> (Option<String>, Option<String>, Option<String>, Option<String>) {
    (None, None, None, None)
}

/// `0o755` -> "rwxr-xr-x" (setuid/setgid/sticky shown as s/S, t/T like ls).
pub fn rwx(mode: u32) -> String {
    let mut s = String::with_capacity(9);
    for (shift, special, special_char) in [(6, 0o4000, 's'), (3, 0o2000, 's'), (0, 0o1000, 't')] {
        let bits = (mode >> shift) & 0o7;
        s.push(if bits & 4 != 0 { 'r' } else { '-' });
        s.push(if bits & 2 != 0 { 'w' } else { '-' });
        let x = bits & 1 != 0;
        s.push(match (mode & special != 0, x) {
            (true, true) => special_char,
            (true, false) => special_char.to_ascii_uppercase(),
            (false, true) => 'x',
            (false, false) => '-',
        });
    }
    s
}

#[cfg(unix)]
fn user_name(uid: u32) -> Option<String> {
    let mut buf = vec![0i8; 4096];
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: buffers are valid for the call; getpwuid_r is the thread-safe variant.
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr().cast(), buf.len(), &mut out) };
    if rc != 0 || out.is_null() {
        return Some(uid.to_string());
    }
    // SAFETY: on success pw_name points into `buf`, NUL-terminated.
    Some(unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }.to_string_lossy().into_owned())
}

#[cfg(unix)]
fn group_name(gid: u32) -> Option<String> {
    let mut buf = vec![0i8; 4096];
    let mut grp: libc::group = unsafe { std::mem::zeroed() };
    let mut out: *mut libc::group = std::ptr::null_mut();
    // SAFETY: as above, with getgrgid_r.
    let rc = unsafe { libc::getgrgid_r(gid, &mut grp, buf.as_mut_ptr().cast(), buf.len(), &mut out) };
    if rc != 0 || out.is_null() {
        return Some(gid.to_string());
    }
    // SAFETY: on success gr_name points into `buf`, NUL-terminated.
    Some(unsafe { std::ffi::CStr::from_ptr(grp.gr_name) }.to_string_lossy().into_owned())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SizeTotals {
    pub bytes: u64,
    pub bytes_on_disk: u64,
    pub files: u64,
    pub folders: u64,
}

/// Total size of `paths`, recursively (symlinks counted, not followed; hard
/// links counted once). `progress` gets running totals every ~100 ms.
/// Returns `None` if cancelled.
pub fn total_size(paths: &[PathBuf], cancel: &AtomicBool, progress: &mut dyn FnMut(SizeTotals)) -> Option<SizeTotals> {
    let mut t = SizeTotals::default();
    let mut seen_inodes = std::collections::HashSet::new();
    let mut stack: Vec<PathBuf> = paths.to_vec();
    let mut last = Instant::now();
    while let Some(p) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let Ok(meta) = fs::symlink_metadata(&p) else { continue };
        if meta.is_dir() {
            t.folders += 1;
            if let Ok(rd) = fs::read_dir(&p) {
                stack.extend(rd.flatten().map(|e| e.path()));
            }
        } else {
            if !first_link(&meta, &mut seen_inodes) {
                continue;
            }
            t.files += 1;
            t.bytes += meta.len();
            t.bytes_on_disk += disk_usage(&meta);
        }
        if last.elapsed() >= Duration::from_millis(100) {
            progress(t);
            last = Instant::now();
        }
    }
    // The selected folders themselves aren't "contents".
    t.folders -= paths.iter().filter(|p| fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())).count() as u64;
    progress(t);
    Some(t)
}

#[cfg(unix)]
fn first_link(meta: &Metadata, seen: &mut std::collections::HashSet<(u64, u64)>) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.nlink() <= 1 || seen.insert((meta.dev(), meta.ino()))
}

#[cfg(not(unix))]
fn first_link(_meta: &Metadata, _seen: &mut std::collections::HashSet<(u64, u64)>) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_strings() {
        assert_eq!(rwx(0o755), "rwxr-xr-x");
        assert_eq!(rwx(0o640), "rw-r-----");
        assert_eq!(rwx(0o4755), "rwsr-xr-x");
        assert_eq!(rwx(0o1777), "rwxrwxrwt");
        assert_eq!(rwx(0o2644), "rw-r-Sr--");
    }

    #[test]
    fn file_and_folder_details() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("notes.txt");
        fs::write(&f, "hello").unwrap();
        let p = properties(&f).unwrap();
        assert_eq!((p.kind, p.size, p.name.as_str()), ("file", Some(5), "notes.txt"));
        assert!(p.modified.is_some() && p.permissions.is_some() && p.owner.is_some());
        assert!(p.size_on_disk.unwrap() >= 5 || p.size_on_disk == Some(0));
        let dp = properties(d.path()).unwrap();
        assert_eq!((dp.kind, dp.size), ("folder", None));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&f, d.path().join("link")).unwrap();
            let l = properties(&d.path().join("link")).unwrap();
            assert_eq!(l.link_target.as_deref(), Some(f.as_path()));
            assert_eq!(l.kind, "file", "described as what it points to");
            std::os::unix::fs::symlink(d.path().join("gone"), d.path().join("broken")).unwrap();
            assert_eq!(properties(&d.path().join("broken")).unwrap().kind, "symlink");
        }
        assert!(matches!(properties(&d.path().join("nope")), Err(CoreError::NotFound(_))));
    }

    #[test]
    fn totals() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("a/b")).unwrap();
        fs::write(d.path().join("a/one"), "12345").unwrap();
        fs::write(d.path().join("a/b/two"), "123").unwrap();
        fs::write(d.path().join("three"), "1").unwrap();
        #[cfg(unix)]
        fs::hard_link(d.path().join("a/one"), d.path().join("a/one-again")).unwrap();
        let mut updates = 0;
        let t =
            total_size(&[d.path().join("a"), d.path().join("three")], &AtomicBool::new(false), &mut |_| updates += 1)
                .unwrap();
        assert_eq!((t.bytes, t.files, t.folders), (9, 3, 1), "hard link counted once; 'a' itself isn't content");
        assert!(updates >= 1);
        assert_eq!(total_size(&[d.path().to_path_buf()], &AtomicBool::new(true), &mut |_| {}), None);
    }
}
