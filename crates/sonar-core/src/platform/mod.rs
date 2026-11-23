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
pub use imp::{mounted_volumes, open_path, restore, trash, trash_dir, HiddenRules, TrashReceipt};

use std::fs;
use std::io::{self, Read};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::{CoreError, Result};

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
