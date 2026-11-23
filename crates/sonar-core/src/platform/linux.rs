use std::collections::HashSet;
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};

use super::cstring;
use crate::error::{CoreError, Result};
use crate::places::{Place, PlaceSection};

/// Linux hides dot-files plus any names listed in the folder's `.hidden`
/// file — the convention honoured by Dolphin, Nautilus and friends.
pub struct HiddenRules {
    listed: HashSet<String>,
}

impl HiddenRules {
    pub fn for_dir(dir: &Path) -> Self {
        let listed = fs::read_to_string(dir.join(".hidden"))
            .map(|s| s.lines().map(str::trim).filter(|l| !l.is_empty()).map(String::from).collect())
            .unwrap_or_default();
        HiddenRules { listed }
    }

    pub fn is_hidden(&self, name: &str, _meta: &Metadata) -> bool {
        super::is_dotfile(name) || self.listed.contains(name)
    }
}

/// `xdg-open` defers to the desktop (kde-open, gio, ...) and honours
/// `mimeapps.list`, so it matches what Dolphin would open.
pub fn open_path(path: &Path) -> Result<()> {
    super::run_launcher("xdg-open", path)
}

/// `renameat2(RENAME_NOREPLACE)`, falling back to check-then-rename on
/// filesystems that don't support the flag (some FUSE/network mounts).
pub(crate) fn rename_noreplace_raw(from: &Path, to: &Path) -> io::Result<()> {
    let (f, t) = (cstring(from)?, cstring(to)?);
    // SAFETY: both are valid NUL-terminated paths; AT_FDCWD makes them relative to the cwd.
    let rc = unsafe { libc::renameat2(libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), libc::RENAME_NOREPLACE) };
    if rc == 0 {
        return Ok(());
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EINVAL) | Some(libc::ENOSYS) => super::rename_noreplace_fallback(from, to),
        _ => Err(err),
    }
}

/// Reflinks (btrfs/XFS FICLONE) would go here; for now always byte-copy.
pub(crate) fn clone_file(_src: &Path, _dst: &Path) -> io::Result<bool> {
    Ok(false)
}

/// The FreeDesktop trash records the original path in a `.trashinfo` file,
/// which is how undo finds the item again.
#[derive(Debug, Clone)]
pub struct TrashReceipt {
    original: PathBuf,
}

impl TrashReceipt {
    pub fn original(&self) -> &Path {
        &self.original
    }
}

pub fn trash(path: &Path) -> Result<TrashReceipt> {
    fs::symlink_metadata(path).map_err(|e| CoreError::from_io(path, e))?;
    // The trash records the canonical parent; match that for restore.
    let original = match (path.parent().and_then(|p| p.canonicalize().ok()), path.file_name()) {
        (Some(parent), Some(name)) => parent.join(name),
        _ => path.to_path_buf(),
    };
    trash::delete(path)
        .map_err(|e| CoreError::Other(format!("Could not move \"{}\" to the Trash: {e}", path.display())))?;
    Ok(TrashReceipt { original })
}

pub fn restore(receipt: &TrashReceipt) -> Result<()> {
    let fail = |msg: String| CoreError::Other(format!("Could not restore \"{}\": {msg}", receipt.original.display()));
    let items = trash::os_limited::list().map_err(|e| fail(e.to_string()))?;
    // Newest matching entry: undo is LIFO, so that's the one we trashed.
    let item = items
        .into_iter()
        .filter(|i| i.original_path() == receipt.original)
        .max_by_key(|i| i.time_deleted)
        .ok_or_else(|| fail("it is no longer in the Trash".into()))?;
    trash::os_limited::restore_all([item]).map_err(|e| match e {
        trash::Error::RestoreCollision { .. } => CoreError::AlreadyExists(receipt.original.clone()),
        other => fail(other.to_string()),
    })
}

/// FreeDesktop trash spec: `$XDG_DATA_HOME/Trash/files`.
pub fn trash_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("Trash").join("files"))
}

const NETWORK_FS: &[&str] =
    &["nfs", "nfs4", "cifs", "smb3", "smbfs", "fuse.sshfs", "sshfs", "davfs", "fuse.rclone", "9p"];

/// Root plus user-relevant mounts from `/proc/self/mounts`: udisks2
/// automounts (`/run/media/$USER`, `/media`), `/mnt/*`, and network shares.
pub fn mounted_volumes() -> Vec<Place> {
    volumes_from_mounts(&fs::read_to_string("/proc/self/mounts").unwrap_or_default())
}

/// Parse a `/proc/mounts`-format table into sidebar entries.
fn volumes_from_mounts(mounts: &str) -> Vec<Place> {
    let mut out = vec![Place::new("Root", "/", "root", PlaceSection::Devices)];
    let mut seen = HashSet::new();
    for line in mounts.lines() {
        let mut f = line.split_whitespace();
        let (Some(_dev), Some(mnt), Some(fstype)) = (f.next(), f.next(), f.next()) else { continue };
        let mnt = unescape_mount(mnt);
        let path = Path::new(&mnt);

        let is_network = NETWORK_FS.contains(&fstype);
        let is_removable = mnt.starts_with("/run/media/") || mnt.starts_with("/media/");
        let is_user_mount = mnt.starts_with("/mnt/");
        if !(is_network || is_removable || is_user_mount) || !seen.insert(mnt.clone()) {
            continue;
        }

        let label = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| mnt.clone());
        let (section, icon) = if is_network {
            (PlaceSection::Remote, "network")
        } else if is_removable {
            (PlaceSection::Devices, "removable")
        } else {
            (PlaceSection::Devices, "drive")
        };
        out.push(Place::new(label, path, icon, section));
    }
    out
}

/// `/proc/mounts` escapes space, tab, newline and backslash as `\ooo` octal.
fn unescape_mount(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 4 <= b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            let v = u32::from(b[i + 1] - b'0') * 64 + u32::from(b[i + 2] - b'0') * 8 + u32::from(b[i + 3] - b'0');
            out.push(v as u8);
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unescapes() {
        assert_eq!(unescape_mount(r"/run/media/me/My\040Disk"), "/run/media/me/My Disk");
        assert_eq!(unescape_mount(r"/plain"), "/plain");
        assert_eq!(unescape_mount(r"/trailing\04"), r"/trailing\04");
    }

    #[test]
    fn mount_table() {
        let table = "\
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
proc /proc proc rw 0 0
tmpfs /run tmpfs rw 0 0
/dev/sda1 /run/media/me/My\\040Stick vfat rw 0 0
/dev/sda1 /run/media/me/My\\040Stick vfat rw 0 0
/dev/sdb1 /mnt/backup btrfs rw 0 0
nas:/export /home/me/nas nfs4 rw 0 0
me@host:/srv /home/me/srv fuse.sshfs rw 0 0
/dev/nvme0n1p3 /home ext4 rw 0 0
";
        let v = volumes_from_mounts(table);
        let got: Vec<_> = v.iter().map(|p| (p.label.as_str(), p.icon, p.section)).collect();
        assert_eq!(
            got,
            [
                ("Root", "root", PlaceSection::Devices),
                ("My Stick", "removable", PlaceSection::Devices), // unescaped, listed once
                ("backup", "drive", PlaceSection::Devices),
                ("nas", "network", PlaceSection::Remote),
                ("srv", "network", PlaceSection::Remote),
            ]
        );
    }

    #[test]
    fn dot_hidden_file() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join(".hidden"), "snap\n\n  go \n").unwrap();
        let meta = fs::metadata(d.path()).unwrap();
        let r = HiddenRules::for_dir(d.path());
        assert!(r.is_hidden("snap", &meta));
        assert!(r.is_hidden("go", &meta));
        assert!(r.is_hidden(".x", &meta));
        assert!(!r.is_hidden("Documents", &meta));
    }
}
