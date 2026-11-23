use std::ffi::{CStr, CString, OsStr};
use std::fs::{self, Metadata};
use std::io;
use std::os::macos::fs::MetadataExt;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

use objc2::rc::{autoreleasepool, Retained};
use objc2_foundation::{NSFileManager, NSURL};

use super::cstring;
use crate::error::{CoreError, Result};
use crate::places::{Place, PlaceSection};

/// `UF_HIDDEN` from <sys/stat.h>: set by `chflags hidden`, used by Finder.
const UF_HIDDEN: u32 = 0x0000_8000;

/// macOS hides dot-files and anything carrying the `UF_HIDDEN` flag
/// (e.g. `~/Library`, `/usr`, `/bin` at the root).
pub struct HiddenRules;

impl HiddenRules {
    pub fn for_dir(_dir: &Path) -> Self {
        HiddenRules
    }

    pub fn is_hidden(&self, name: &str, meta: &Metadata) -> bool {
        super::is_dotfile(name) || meta.st_flags() & UF_HIDDEN != 0
    }
}

/// LaunchServices via `open`: default app for documents, launches `.app` bundles.
pub fn open_path(path: &Path) -> Result<()> {
    super::run_launcher("/usr/bin/open", path)
}

/// `renamex_np(RENAME_EXCL)`: atomic, fails with EEXIST if `to` exists.
pub(crate) fn rename_noreplace_raw(from: &Path, to: &Path) -> io::Result<()> {
    let (f, t) = (cstring(from)?, cstring(to)?);
    // SAFETY: both are valid NUL-terminated paths.
    if unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// APFS copy-on-write clone: instant, and copies metadata too. Returns
/// `Ok(false)` when the volume can't clone (HFS+, exFAT, cross-volume), so the
/// caller falls back to a byte copy.
pub(crate) fn clone_file(src: &Path, dst: &Path) -> io::Result<bool> {
    let (s, d) = (cstring(src)?, cstring(dst)?);
    // SAFETY: both are valid NUL-terminated paths; flags 0 is always valid.
    if unsafe { libc::clonefile(s.as_ptr(), d.as_ptr(), 0) } == 0 {
        return Ok(true);
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::ENOTSUP) | Some(libc::EXDEV) => Ok(false),
        _ => Err(err),
    }
}

/// Where an item went in the Trash, so undo can put it back.
#[derive(Debug, Clone)]
pub struct TrashReceipt {
    original: PathBuf,
    in_trash: Option<PathBuf>,
}

impl TrashReceipt {
    pub fn original(&self) -> &Path {
        &self.original
    }
}

/// `-[NSFileManager trashItemAtURL:resultingItemURL:error:]`. Unlike asking
/// Finder via AppleScript this needs no Automation permission, and it tells
/// us where the item landed.
pub fn trash(path: &Path) -> Result<TrashReceipt> {
    let meta = fs::symlink_metadata(path).map_err(|e| CoreError::from_io(path, e))?;
    let c = cstring(path).map_err(|e| CoreError::from_io(path, e))?;
    autoreleasepool(|_| {
        // SAFETY: `c` is a valid NUL-terminated path that outlives the call.
        let url = unsafe {
            NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
                NonNull::new(c.as_ptr() as *mut _).expect("CString is non-null"),
                meta.is_dir(),
                None,
            )
        };
        let mut resulting: Option<Retained<NSURL>> = None;
        NSFileManager::defaultManager().trashItemAtURL_resultingItemURL_error(&url, Some(&mut resulting)).map_err(
            |e| {
                CoreError::Other(format!(
                    "Could not move \"{}\" to the Trash: {}",
                    path.display(),
                    e.localizedDescription()
                ))
            },
        )?;
        let in_trash = resulting.map(|u| {
            // SAFETY: the returned buffer is valid for the lifetime of the pool; we copy it out.
            let bytes = unsafe { CStr::from_ptr(u.fileSystemRepresentation().as_ptr()) }.to_bytes();
            PathBuf::from(OsStr::from_bytes(bytes))
        });
        Ok(TrashReceipt { original: path.to_path_buf(), in_trash })
    })
}

pub fn restore(receipt: &TrashReceipt) -> Result<()> {
    let original = &receipt.original;
    let Some(in_trash) = &receipt.in_trash else {
        return Err(CoreError::Other(format!("Don't know where \"{}\" went in the Trash", original.display())));
    };
    if let Some(parent) = original.parent() {
        fs::create_dir_all(parent).map_err(|e| CoreError::from_io(parent, e))?;
    }
    super::rename_noreplace(in_trash, original).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => CoreError::Other(format!("\"{}\" is no longer in the Trash", original.display())),
        _ => CoreError::from_io(original, e),
    })
}

pub fn trash_dir() -> Option<PathBuf> {
    // Listing it requires Full Disk Access; the UI reports that gracefully.
    dirs::home_dir().map(|h| h.join(".Trash"))
}

/// Volumes come from `/Volumes`. The boot volume appears there as a symlink
/// to `/`, which gives us its user-visible name ("Macintosh HD").
pub fn mounted_volumes() -> Vec<Place> {
    let mut root = Place::new("Macintosh HD", "/", "root", PlaceSection::Devices);
    let mut others = Vec::new();

    if let Ok(rd) = fs::read_dir("/Volumes") {
        for de in rd.flatten() {
            let name = de.file_name().to_string_lossy().into_owned();
            if super::is_dotfile(&name) {
                continue;
            }
            let path = de.path();
            if fs::read_link(&path).is_ok_and(|t| t == Path::new("/")) {
                root.label = name;
                continue;
            }
            if !path.is_dir() {
                continue;
            }
            let (section, icon) = match is_local(&path) {
                Some(false) => (PlaceSection::Remote, "network"),
                _ => (PlaceSection::Devices, "drive"),
            };
            others.push(Place::new(name, path, icon, section));
        }
    }
    others.sort_by(|a, b| crate::sort::natural_cmp(&a.label, &b.label));

    let mut out = vec![root];
    out.extend(others);
    out
}

/// `Some(true)` for local filesystems, `Some(false)` for network mounts.
fn is_local(path: &Path) -> Option<bool> {
    let c = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` is a properly sized out-param.
    if unsafe { libc::statfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_flags & libc::MNT_LOCAL as u32 != 0)
}
