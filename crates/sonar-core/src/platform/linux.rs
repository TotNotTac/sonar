use std::collections::{HashMap, HashSet};
use std::fs::{self, Metadata};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use xdg_mime::SharedMimeInfo;

use super::{AppInfo, Icon, NativeType, cstring};
use crate::entry::EntryKind;
use crate::error::{CoreError, Result};
use crate::mimeglob::GlobIndex;
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

/// Kernel and other virtual filesystems: nothing a file search should walk.
const VIRTUAL_FS: &[&str] = &[
    "proc",
    "sysfs",
    "devtmpfs",
    "devpts",
    "cgroup",
    "cgroup2",
    "securityfs",
    "debugfs",
    "tracefs",
    "pstore",
    "bpf",
    "mqueue",
    "hugetlbfs",
    "configfs",
    "fusectl",
    "binfmt_misc",
    "autofs",
    "efivarfs",
    "rpc_pipefs",
    "nsfs",
];

/// Mount points a search shouldn't wander into: network shares (slow, and
/// they hang while the server is gone) and virtual filesystems.
pub(crate) fn search_skip_mounts() -> Vec<PathBuf> {
    skip_mounts_in(&fs::read_to_string("/proc/self/mounts").unwrap_or_default())
}

fn skip_mounts_in(mounts: &str) -> Vec<PathBuf> {
    mounts
        .lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            let (_, mnt, fstype) = (f.next()?, f.next()?, f.next()?);
            (NETWORK_FS.contains(&fstype) || VIRTUAL_FS.contains(&fstype)).then(|| unescape_mount(mnt).into())
        })
        .collect()
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

/// File name -> MIME type, built once from the shared-mime-info globs.
fn glob_index() -> &'static GlobIndex {
    static INDEX: OnceLock<GlobIndex> = OnceLock::new();
    INDEX.get_or_init(|| GlobIndex::load(&data_dirs()))
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
        EntryKind::File => match glob_index().lookup(name) {
            Some(mime) => mime_type_info(mime),
            None => NativeType {
                label: None,
                type_id: Some("application/octet-stream".into()),
                icon_key: Some("application-octet-stream,unknown".into()),
            },
        },
        EntryKind::BrokenSymlink | EntryKind::Other => NativeType::default(),
    }
}

/// Description and icon names for a MIME type, looked up once per type.
fn mime_type_info(mime: &str) -> NativeType {
    static CACHE: OnceLock<Mutex<HashMap<String, NativeType>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(mime) {
        return hit.clone();
    }
    let icons = match mime.parse() {
        Ok(parsed) => {
            let db = mime_db().lock().unwrap_or_else(|p| p.into_inner());
            let mut icons = db.lookup_icon_names(&parsed);
            icons.extend(db.lookup_generic_icon_name(&parsed));
            icons
        }
        Err(_) => Vec::new(),
    };
    let mut icons: Vec<String> = icons.into_iter().chain(["unknown".to_string()]).collect();
    icons.dedup();
    let info =
        NativeType { label: mime_comment(mime), icon_key: Some(icons.join(",")), type_id: Some(mime.to_string()) };
    cache.lock().unwrap_or_else(|p| p.into_inner()).insert(mime.to_string(), info.clone());
    info
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

// ---------- Open With (desktop files + xdg-mime) ----------

fn mime_of(path: &Path) -> String {
    if path.is_dir() {
        return "inode/directory".into();
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    native_type(&name, path, EntryKind::File).type_id.unwrap_or_else(|| "application/octet-stream".into())
}

/// The type, its canonical name if it's an alias (text/x-python ->
/// text/x-python3), and all its ancestors (-> text/plain), most specific
/// first. Every text/* type is also text/plain, per the shared-mime-info spec.
fn mime_family(mime: &str) -> Vec<String> {
    let db = mime_db().lock().unwrap_or_else(|p| p.into_inner());
    let mut out: Vec<String> = Vec::new();
    let mut queue = std::collections::VecDeque::from([mime.to_string()]);
    while let Some(m) = queue.pop_front() {
        if out.contains(&m) || out.len() > 32 {
            continue;
        }
        out.push(m.clone());
        let Ok(parsed) = m.parse() else { continue };
        if let Some(canonical) = db.unalias_mime_type(&parsed) {
            queue.push_back(canonical.essence_str().to_string());
        }
        for parent in db.get_parents(&parsed).unwrap_or_default() {
            queue.push_back(parent.essence_str().to_string());
        }
    }
    if mime.starts_with("text/") && !out.iter().any(|m| m == "text/plain") {
        out.push("text/plain".into());
    }
    out
}

/// All application desktop files, by id (`org.kde.okular.desktop`); user
/// entries override system ones, per the spec.
fn desktop_entries(data_dirs: &[PathBuf]) -> Vec<(String, PathBuf, crate::desktop::DesktopEntry)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for base in data_dirs.iter().map(|d| d.join("applications")) {
        let mut stack = vec![base.clone()];
        while let Some(dir) = stack.pop() {
            for de in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let p = de.path();
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if p.extension().and_then(|e| e.to_str()) != Some("desktop") {
                    continue;
                }
                // Subfolders become part of the id: kde/foo.desktop -> kde-foo.desktop.
                let id = p.strip_prefix(&base).unwrap_or(&p).to_string_lossy().replace('/', "-");
                if !seen.insert(id.clone()) {
                    continue;
                }
                if let Some(entry) = fs::read_to_string(&p).ok().and_then(|t| crate::desktop::parse(&t)) {
                    out.push((id, p, entry));
                }
            }
        }
    }
    out
}

pub fn apps_for(path: &Path) -> Vec<AppInfo> {
    let mime = mime_of(path);
    let default = super::run_tool("xdg-mime", &["query", "default", &mime]).ok().map(|s| s.trim().to_string());
    apps_in(&mime, &data_dirs(), default.as_deref())
}

fn apps_in(mime: &str, data_dirs: &[PathBuf], default: Option<&str>) -> Vec<AppInfo> {
    let family = mime_family(mime);
    let apps = desktop_entries(data_dirs)
        .into_iter()
        .filter(|(_, _, e)| !e.hidden && !e.terminal && e.mime_types.iter().any(|m| family.contains(m)))
        .map(|(id, _, e)| AppInfo { is_default: default == Some(id.as_str()), name: e.name, icon_key: e.icon, id })
        .collect();
    super::tidy_apps(apps)
}

pub fn open_with(paths: &[PathBuf], app: &str) -> Result<()> {
    let fail = |m: String| CoreError::Other(format!("Could not open with {app}: {m}"));
    let (_, file, entry) = desktop_entries(&data_dirs())
        .into_iter()
        .find(|(id, _, _)| id == app)
        .ok_or_else(|| fail("application not found".into()))?;
    for cmd in crate::desktop::commands(&entry, &file, paths) {
        let Some((program, args)) = cmd.split_first() else { continue };
        let child = super::host_command(program)
            .args(args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| fail(super::spawn_error(&program.to_string_lossy(), &e)))?;
        std::thread::spawn(move || {
            let mut child = child;
            child.wait() // reap; the app keeps running
        });
    }
    Ok(())
}

pub fn set_default_app(path: &Path, app: &str) -> Result<()> {
    let mime = mime_of(path);
    super::run_tool("xdg-mime", &["default", app, &mime])
        .map(drop)
        .map_err(|e| CoreError::Other(format!("Could not make {app} the default for {mime}: {e}")))
}

/// The user's icon theme: KDE's setting, then GTK's, then sensible defaults,
/// whichever is installed first.
fn icon_theme() -> &'static str {
    static THEME: OnceLock<String> = OnceLock::new();
    THEME.get_or_init(|| {
        let kde = dirs::config_dir()
            .and_then(|c| fs::read_to_string(c.join("kdeglobals")).ok())
            .and_then(|ini| ini_value(&ini, "Icons", "Theme"));
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        let fallback = if desktop.contains("KDE") { ["breeze", "Adwaita"] } else { ["Adwaita", "breeze"] };
        let bases = icon_base_dirs();
        kde.into_iter()
            .chain(freedesktop_icons::default_theme_gtk())
            .chain(fallback.map(String::from))
            .find_map(|name| installed_theme(&name, &bases))
            .unwrap_or_else(|| "hicolor".into())
    })
}

/// Where icon themes live (Icon Theme Specification, "Directory Layout").
fn icon_base_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = dirs::home_dir().map(|h| h.join(".icons")).into_iter().collect();
    out.extend(data_dirs().into_iter().map(|d| d.join("icons")));
    out
}

/// The directory name of the installed theme `name` refers to. Settings don't
/// always use the directory's case (GTK may say "Breeze" for `breeze/`), and
/// sometimes use the display name from `index.theme`.
fn installed_theme(name: &str, bases: &[PathBuf]) -> Option<String> {
    let themes: Vec<(String, PathBuf)> = bases
        .iter()
        .filter_map(|b| fs::read_dir(b).ok())
        .flatten()
        .flatten()
        .filter(|d| d.path().join("index.theme").is_file())
        .map(|d| (d.file_name().to_string_lossy().into_owned(), d.path()))
        .collect();
    let exact = themes.iter().find(|(dir, _)| dir == name);
    let folded = || themes.iter().find(|(dir, _)| dir.eq_ignore_ascii_case(name));
    let by_label = || {
        themes.iter().find(|(_, path)| {
            let index = fs::read_to_string(path.join("index.theme")).unwrap_or_default();
            ini_value(&index, "Icon Theme", "Name").is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
    };
    exact.or_else(folded).or_else(by_label).map(|(dir, _)| dir.clone())
}

fn ini_value(ini: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in ini.lines().map(str::trim) {
        if line.starts_with('[') {
            in_section = line == format!("[{section}]");
        } else if in_section
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().to_string()).filter(|v| !v.is_empty());
        }
    }
    None
}

/// Look up the first available name in `key` (comma-separated, most specific
/// first) in the icon theme, at `size` logical pixels (2x for HiDPI).
pub fn icon(key: &str, size: u32) -> Option<Icon> {
    // Desktop files may name an icon by absolute path.
    if key.starts_with('/') {
        let content_type = match Path::new(key).extension()?.to_str()? {
            "svg" => "image/svg+xml",
            "png" => "image/png",
            _ => return None,
        };
        return Some(Icon { bytes: fs::read(key).ok()?, content_type });
    }
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
    fn icon_theme_names_resolve_to_installed_themes() {
        let d = tempfile::tempdir().unwrap();
        let theme = |dir: &str, label: &str| {
            fs::create_dir_all(d.path().join(dir)).unwrap();
            fs::write(d.path().join(dir).join("index.theme"), format!("[Icon Theme]\nName={label}\n")).unwrap();
        };
        theme("breeze", "Breeze");
        theme("breeze-dark", "Breeze Dark");
        theme("Papirus", "Papirus");
        fs::create_dir_all(d.path().join("cursors-only")).unwrap(); // no index.theme: not a theme
        let bases = [d.path().to_path_buf(), d.path().join("missing")];
        assert_eq!(installed_theme("breeze", &bases).as_deref(), Some("breeze"));
        assert_eq!(installed_theme("Breeze", &bases).as_deref(), Some("breeze"), "case from GTK settings");
        assert_eq!(installed_theme("Breeze Dark", &bases).as_deref(), Some("breeze-dark"), "display name");
        assert_eq!(installed_theme("papirus", &bases).as_deref(), Some("Papirus"));
        assert_eq!(installed_theme("cursors-only", &bases), None);
        assert_eq!(installed_theme("Nope", &bases), None);
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
        assert_eq!(
            skip_mounts_in(table),
            [PathBuf::from("/proc"), PathBuf::from("/home/me/nas"), PathBuf::from("/home/me/srv")]
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
    fn apps_from_desktop_files() {
        if !Path::new("/usr/share/mime/mime.cache").exists() {
            return; // parent types come from shared-mime-info
        }
        let d = tempfile::tempdir().unwrap();
        let apps = d.path().join("applications");
        fs::create_dir_all(apps.join("kde")).unwrap();
        let entry = |name: &str, mime: &str, extra: &str| {
            format!("[Desktop Entry]\nType=Application\nName={name}\nExec={name} %F\nMimeType={mime};\n{extra}")
        };
        fs::write(apps.join("kate.desktop"), entry("Kate", "text/plain", "Icon=kate")).unwrap();
        fs::write(apps.join("kde/kwrite.desktop"), entry("KWrite", "text/plain", "")).unwrap();
        fs::write(apps.join("pyide.desktop"), entry("PyIDE", "text/x-python", "")).unwrap();
        fs::write(apps.join("hidden.desktop"), entry("Hidden", "text/plain", "NoDisplay=true")).unwrap();
        fs::write(apps.join("vim.desktop"), entry("Vim", "text/plain", "Terminal=true")).unwrap();
        fs::write(apps.join("gimp.desktop"), entry("GIMP", "image/png", "")).unwrap();

        // A Python file: its own type plus text/plain editors, default first.
        let got = apps_in("text/x-python", &[d.path().to_path_buf()], Some("kde-kwrite.desktop"));
        let names: Vec<_> = got.iter().map(|a| (a.name.as_str(), a.id.as_str(), a.is_default)).collect();
        assert_eq!(
            names,
            [
                ("KWrite", "kde-kwrite.desktop", true),
                ("Kate", "kate.desktop", false),
                ("PyIDE", "pyide.desktop", false)
            ]
        );
        assert_eq!(got[1].icon_key.as_deref(), Some("kate"));
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
