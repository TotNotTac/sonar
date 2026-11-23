use std::collections::{HashMap, HashSet};
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use xdg_mime::SharedMimeInfo;

use super::{cstring, Icon, NativeType};
use crate::entry::EntryKind;
use crate::error::{CoreError, Result};
use crate::places::{Place, PlaceSection};
use crate::volumes;

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
        let (Some(dev), Some(mnt), Some(fstype)) = (f.next(), f.next(), f.next()) else { continue };
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
        let mut place = Place::new(label, path, icon, section).ejectable(is_network || is_removable);
        place.device = Some(unescape_mount(dev));
        out.push(place);
    }
    out
}

// ---------- Removable media (udisks2 via its CLI) ----------

/// Attached but unmounted devices (USB sticks, other partitions), from `lsblk`.
pub fn unmounted_volumes() -> Vec<Place> {
    let Ok(json) = super::run_tool("lsblk", volumes::LSBLK_ARGS) else { return Vec::new() };
    volumes::parse_lsblk(&json)
        .iter()
        .filter(|d| volumes::mountable(d))
        .map(|d| {
            let icon = if d.hotplug || d.removable { "removable" } else { "drive" };
            let mut p = Place::new(volumes::device_label(d), &d.path, icon, PlaceSection::Devices);
            p.mounted = false;
            p.device = Some(d.path.clone());
            p
        })
        .collect()
}

/// Mount a block device with udisks (as the user, via polkit). Returns the mount point.
pub fn mount_device(device: &str) -> Result<PathBuf> {
    let out = super::run_tool("udisksctl", &["mount", "--block-device", device])
        .map_err(|e| CoreError::Other(format!("Could not mount {device}: {e}")))?;
    volumes::parse_udisksctl_mount(&out)
        .ok_or_else(|| CoreError::Other(format!("Mounted {device}, but could not tell where: {}", out.trim())))
}

/// Unmount the volume at `path`; for USB drives also power the drive off so
/// it is safe to unplug, like Dolphin's eject.
pub fn eject_volume(path: &Path) -> Result<()> {
    let fail = |e: String| CoreError::Other(format!("Could not eject \"{}\": {e}", path.display()));
    let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    let (dev, fstype) = mounts
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((unescape_mount(f.next()?), unescape_mount(f.next()?), f.next()?.to_string()))
        })
        .find(|(_, mnt, _)| Path::new(mnt) == path)
        .map(|(d, _, t)| (d, t))
        .ok_or_else(|| fail("it is not mounted".into()))?;

    let p = path.to_string_lossy();
    if fstype.starts_with("fuse") {
        return super::run_tool("fusermount3", &["-u", &p])
            .or_else(|_| super::run_tool("fusermount", &["-u", &p]))
            .map(drop)
            .map_err(fail);
    }
    if !dev.starts_with("/dev/") {
        return super::run_tool("umount", &[&p]).map(drop).map_err(fail); // NFS/SMB from fstab
    }
    super::run_tool("udisksctl", &["unmount", "--block-device", &dev]).map_err(fail)?;
    // Power off the whole drive if it is hot-pluggable; failures here are harmless.
    if let Ok(json) = super::run_tool("lsblk", volumes::LSBLK_ARGS) {
        let devs = volumes::parse_lsblk(&json);
        if let Some(d) = devs.iter().find(|d| d.path == dev).filter(|d| d.hotplug || d.removable) {
            let disk = d.parent.clone().unwrap_or_else(|| dev.clone());
            let _ = super::run_tool("udisksctl", &["power-off", "--block-device", &disk]);
        }
    }
    Ok(())
}

// ---------- File types and icons (shared-mime-info, icon themes) ----------

fn mime_db() -> &'static Mutex<SharedMimeInfo> {
    static DB: OnceLock<Mutex<SharedMimeInfo>> = OnceLock::new();
    DB.get_or_init(|| Mutex::new(SharedMimeInfo::new()))
}

/// MIME type, description and icon names for a file, by name (like Dolphin's
/// fast path; no content sniffing).
pub fn native_type(name: &str, path: &Path, kind: EntryKind) -> NativeType {
    match kind {
        EntryKind::Directory => NativeType {
            label: None,
            type_id: Some("inode/directory".into()),
            icon_key: Some(folder_icon(path).into()),
        },
        EntryKind::File => {
            let db = mime_db().lock().unwrap_or_else(|p| p.into_inner());
            let Some(mime) = db.get_mime_types_from_file_name(name).into_iter().next() else {
                return NativeType {
                    label: None,
                    type_id: Some("application/octet-stream".into()),
                    icon_key: Some("application-octet-stream,unknown".into()),
                };
            };
            let id = mime.essence_str().to_string();
            let mut icons = db.lookup_icon_names(&mime);
            icons.extend(db.lookup_generic_icon_name(&mime));
            icons.push("unknown".into());
            icons.dedup();
            NativeType { label: mime_comment(&id), icon_key: Some(icons.join(",")), type_id: Some(id) }
        }
        EntryKind::BrokenSymlink | EntryKind::Other => NativeType::default(),
    }
}

fn folder_icon(path: &Path) -> &'static str {
    match super::special_folder(path) {
        Some("home") => "user-home,folder",
        Some("desktop") => "user-desktop,folder",
        Some("documents") => "folder-documents,folder",
        Some("downloads") => "folder-download,folder",
        Some("music") => "folder-music,folder",
        Some("pictures") => "folder-pictures,folder",
        Some("videos") => "folder-videos,folder",
        Some("public") => "folder-public,folder",
        Some("trash") => "user-trash,folder",
        _ => "folder",
    }
}

/// The human-readable description from `mime/<type>.xml` in the XDG data dirs.
fn mime_comment(mime: &str) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(mime) {
        return hit.clone();
    }
    let found = data_dirs()
        .iter()
        .find_map(|d| fs::read_to_string(d.join("mime").join(format!("{mime}.xml"))).ok())
        .and_then(|xml| default_comment(&xml));
    cache.lock().unwrap_or_else(|p| p.into_inner()).insert(mime.to_string(), found.clone());
    found
}

fn data_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = dirs::data_dir().into_iter().collect();
    let system = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(system.split(':').filter(|s| !s.is_empty()).map(PathBuf::from));
    dirs
}

/// First untranslated `<comment>` in a shared-mime-info XML file.
fn default_comment(xml: &str) -> Option<String> {
    let start = xml.find("<comment>")? + "<comment>".len();
    let end = start + xml[start..].find("</comment>")?;
    let text = xml[start..end]
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&");
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// The user's icon theme: KDE's setting, then GTK's, then sensible defaults.
fn icon_theme() -> &'static str {
    static THEME: OnceLock<String> = OnceLock::new();
    THEME.get_or_init(|| {
        let kde = dirs::config_dir()
            .and_then(|c| fs::read_to_string(c.join("kdeglobals")).ok())
            .and_then(|ini| ini_value(&ini, "Icons", "Theme"));
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        kde.or_else(freedesktop_icons::default_theme_gtk).unwrap_or_else(|| {
            if desktop.contains("KDE") {
                "breeze".into()
            } else {
                "Adwaita".into()
            }
        })
    })
}

fn ini_value(ini: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in ini.lines().map(str::trim) {
        if line.starts_with('[') {
            in_section = line == format!("[{section}]");
        } else if in_section {
            if let Some((k, v)) = line.split_once('=') {
                if k.trim() == key {
                    return Some(v.trim().to_string()).filter(|v| !v.is_empty());
                }
            }
        }
    }
    None
}

/// Look up the first available name in `key` (comma-separated, most specific
/// first) in the icon theme, at `size` logical pixels (2x for HiDPI).
pub fn icon(key: &str, size: u32) -> Option<Icon> {
    let px = u16::try_from(size.clamp(8, 256) * 2).ok()?;
    let theme = icon_theme();
    key.split(',').filter(|n| !n.is_empty()).find_map(|name| {
        let path = freedesktop_icons::lookup(name).with_size(px).with_theme(theme).with_cache().find()?;
        let content_type = match path.extension()?.to_str()? {
            "svg" => "image/svg+xml",
            "png" => "image/png",
            _ => return None,
        };
        Some(Icon { bytes: fs::read(&path).ok()?, content_type })
    })
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

    /// Uses the real shared-mime-info database and icon theme when installed
    /// (CI installs them); skipped on machines without them.
    #[test]
    fn system_mime_database_and_icons() {
        if !Path::new("/usr/share/mime/mime.cache").exists() {
            eprintln!("shared-mime-info not installed; skipping");
            return;
        }
        let t = native_type("Photo.PNG", Path::new("/tmp/Photo.PNG"), EntryKind::File);
        assert_eq!(t.type_id.as_deref(), Some("image/png"));
        assert_eq!(t.label.as_deref(), Some("PNG image"));
        assert!(t.icon_key.as_deref().unwrap().starts_with("image-png,"), "{:?}", t.icon_key);
        let rs = native_type("main.rs", Path::new("/tmp/main.rs"), EntryKind::File);
        assert_eq!(rs.type_id.as_deref(), Some("text/rust"));
        let none = native_type("README", Path::new("/tmp/README"), EntryKind::File);
        assert!(none.type_id.is_some());

        if Path::new("/usr/share/icons/Adwaita").exists() {
            let icon = icon("no-such-icon,folder", 16).expect("folder icon from the theme");
            assert!(icon.content_type.starts_with("image/"));
            assert!(!icon.bytes.is_empty());
        }
    }

    #[test]
    fn ini_and_mime_xml() {
        let ini = "[General]\nTheme=nope\n[Icons]\nTheme=Papirus-Dark\n";
        assert_eq!(ini_value(ini, "Icons", "Theme").as_deref(), Some("Papirus-Dark"));
        assert_eq!(ini_value(ini, "Icons", "Size"), None);
        let xml = r#"<mime-type type="image/png"><comment>PNG image</comment><comment xml:lang="nl">PNG-afbeelding</comment></mime-type>"#;
        assert_eq!(default_comment(xml).as_deref(), Some("PNG image"));
        assert_eq!(default_comment("<comment>R&amp;D notes</comment>").as_deref(), Some("R&D notes"));
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
