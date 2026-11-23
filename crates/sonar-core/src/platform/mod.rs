//! OS-specific behaviour behind one small, uniform API.
//!
//! Each backend exposes:
//! - [`HiddenRules`]: decides whether an entry is hidden in a given folder.
//! - [`trash_dir`]: the user's trash location, if any.
//! - [`mounted_volumes`]: sidebar entries for root, disks and network mounts.
//! - [`open_path`]: open a file or app with the user's default handler.
//! - [`trash`] / [`restore`]: move to the system trash and back (for undo).
//! - `rename_noreplace_raw`: atomic rename that fails if the target exists.
//! - `clone_file`: copy-on-write clone where the filesystem supports it.
//! - [`native_type`] / [`icon`]: the OS's own file type names and icons
//!   (UTType + NSWorkspace on macOS, shared-mime-info + icon theme on Linux).
//! - [`volume_usage`]: total and free space of a mounted filesystem.
//! - [`mount_device`] / [`eject_volume`] / [`unmounted_volumes`]: removable media.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as imp;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod generic;
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
use generic as imp;

pub(crate) use imp::clone_file;
pub use imp::{
    eject_volume, icon, mount_device, mounted_volumes, native_type, open_path, restore, trash, trash_dir,
    unmounted_volumes, HiddenRules, TrashReceipt,
};

use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::error::{CoreError, Result};

/// What the OS knows about a file's type. Any field may be missing, in
/// which case the built-in extension table is used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeType {
    /// Human-readable, localised where the OS provides it ("PNG image").
    pub label: Option<String>,
    /// MIME type (Linux) or Uniform Type Identifier (macOS).
    pub type_id: Option<String>,
    /// Opaque key for [`icon`].
    pub icon_key: Option<String>,
}

/// An icon image ready to serve to the UI.
pub struct Icon {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub total: u64,
    pub free: u64,
}

/// Size and free space (available to unprivileged users) of the filesystem
/// holding `path`.
#[cfg(unix)]
pub fn volume_usage(path: &Path) -> Option<Usage> {
    let c = cstring(path).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` a properly sized out-param.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast, clippy::useless_conversion)] // field types differ per platform
    let (blocks, avail, frsize) = (st.f_blocks as u64, st.f_bavail as u64, st.f_frsize as u64);
    (blocks > 0).then_some(Usage { total: blocks * frsize, free: avail * frsize })
}

#[cfg(not(unix))]
pub fn volume_usage(_path: &Path) -> Option<Usage> {
    None
}

/// Paths of the user's standard folders, for platform-specific folder icons.
pub(crate) fn special_folder(path: &Path) -> Option<&'static str> {
    let is = |d: Option<std::path::PathBuf>| d.as_deref() == Some(path);
    if is(dirs::home_dir()) {
        Some("home")
    } else if is(dirs::desktop_dir()) {
        Some("desktop")
    } else if is(dirs::document_dir()) {
        Some("documents")
    } else if is(dirs::download_dir()) {
        Some("downloads")
    } else if is(dirs::audio_dir()) {
        Some("music")
    } else if is(dirs::picture_dir()) {
        Some("pictures")
    } else if is(dirs::video_dir()) {
        Some("videos")
    } else if is(dirs::public_dir()) {
        Some("public")
    } else if is(trash_dir()) {
        Some("trash")
    } else {
        None
    }
}

/// Run a helper tool (`udisksctl`, `diskutil`, ...) and return its stdout,
/// or a readable error built from its stderr.
#[allow(dead_code)] // unused on the generic backend
pub(crate) fn run_tool(program: &str, args: &[&str]) -> std::result::Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let msg = err.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or("").to_string();
    Err(if msg.is_empty() { format!("{program} failed ({})", out.status) } else { msg })
}

/// Dot-files are hidden everywhere we support.
pub(crate) fn is_dotfile(name: &str) -> bool {
    name.starts_with('.')
}

/// Rename `from` to `to`, failing with `AlreadyExists` instead of replacing
/// an existing `to`. Case-only renames ("readme" -> "README") on
/// case-insensitive filesystems are allowed: there the "existing" target is
/// the source itself.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    match imp::rename_noreplace_raw(from, to) {
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && same_file(from, to) => fs::rename(from, to),
        other => other,
    }
}

#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::symlink_metadata(a), fs::symlink_metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

#[cfg(not(unix))]
fn same_file(_: &Path, _: &Path) -> bool {
    false
}

#[cfg(unix)]
pub(crate) fn cstring(p: &Path) -> io::Result<std::ffi::CString> {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::CString::new(p.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "path contains a NUL byte"))
}

/// Fallback for filesystems without an atomic no-replace rename. Racy, but
/// only in the window between the check and the rename.
#[allow(dead_code)]
pub(crate) fn rename_noreplace_fallback(from: &Path, to: &Path) -> io::Result<()> {
    if fs::symlink_metadata(to).is_ok() {
        return Err(io::Error::from(io::ErrorKind::AlreadyExists));
    }
    fs::rename(from, to)
}

/// Run a launcher such as `open` or `xdg-open` on `path`.
///
/// Launchers normally hand off to the target app and exit at once, which is
/// when they report failures like "no application for this type". Some
/// (xdg-open's generic fallback) instead keep running as long as the app does.
/// So: wait briefly for an exit status, and if the launcher is still running
/// after that, treat it as success and reap it in the background.
#[allow(dead_code)] // unused on the generic backend
pub(crate) fn run_launcher(program: &str, path: &Path) -> Result<()> {
    const GRACE: Duration = Duration::from_millis(1500);

    let open_err = |message: String| CoreError::Open { path: path.to_path_buf(), message };
    if !path.exists() {
        return Err(CoreError::NotFound(path.to_path_buf()));
    }
    let mut child = Command::new(program)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| open_err(format!("could not run {program}: {e}")))?;

    let deadline = Instant::now() + GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                let mut stderr = String::new();
                if let Some(mut s) = child.stderr.take() {
                    let _ = s.read_to_string(&mut stderr);
                }
                let stderr = stderr.trim();
                return Err(open_err(if stderr.is_empty() {
                    format!("{program} failed ({status})")
                } else {
                    stderr.lines().last().unwrap_or(stderr).to_string()
                }));
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            Ok(None) => {
                // Drain stderr so the child can never block on a full pipe.
                std::thread::spawn(move || child.wait_with_output());
                return Ok(());
            }
            Err(e) => return Err(open_err(e.to_string())),
        }
    }
}
