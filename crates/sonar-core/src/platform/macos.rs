use std::collections::HashMap;
use std::ffi::{CStr, CString, OsStr};
use std::fs::{self, Metadata};
use std::io;
use std::os::macos::fs::MetadataExt;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::{Mutex, OnceLock};

use objc2::AnyThread;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSBitmapImageRepPropertyKey, NSWorkspace};
use objc2_foundation::{NSBundle, NSDictionary, NSFileManager, NSPoint, NSRect, NSSize, NSString, NSURL};
use objc2_uniform_type_identifiers::UTType;

use super::{AppInfo, Icon, NativeType, cstring};
use crate::entry::EntryKind;
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
            let (section, icon, ejectable) = match is_local(&path) {
                // Finder offers "eject" (unmount) for servers, too.
                Some(false) => (PlaceSection::Remote, "network", true),
                _ => (PlaceSection::Devices, "drive", is_ejectable(&path)),
            };
            others.push(Place::new(name, path, icon, section).ejectable(ejectable));
        }
    }
    others.sort_by(|a, b| crate::sort::natural_cmp(&a.label, &b.label));

    let mut out = vec![root];
    out.extend(others);
    out
}

/// External disks, USB sticks and disk images can be ejected; secondary
/// internal partitions cannot. Asks `diskutil` once per mounted volume.
fn is_ejectable(path: &Path) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, u64), bool>>> = OnceLock::new();
    let dev = fs::metadata(path).map(|m| m.st_dev()).unwrap_or(0);
    let key = (path.to_path_buf(), dev);
    let cache = CACHE.get_or_init(Default::default);
    if let Some(&v) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
        return v;
    }
    let v = super::run_tool("diskutil", &["info", "-plist", &path.to_string_lossy()])
        .ok()
        .and_then(|xml| plist::from_bytes::<plist::Dictionary>(xml.as_bytes()).ok())
        .is_some_and(|d| diskutil_says_ejectable(&d));
    cache.lock().unwrap_or_else(|p| p.into_inner()).insert(key, v);
    v
}

fn diskutil_says_ejectable(info: &plist::Dictionary) -> bool {
    let flag = |k: &str| info.get(k).and_then(plist::Value::as_boolean).unwrap_or(false);
    flag("Ejectable")
        || flag("Removable")
        || flag("RemovableMedia")
        || !info.get("Internal").and_then(plist::Value::as_boolean).unwrap_or(true)
}

/// macOS mounts attached volumes automatically, so there is nothing to list.
/// Mount points a search shouldn't wander into. Network volumes live under
/// /Volumes, which a search in the home folder doesn't reach.
pub(crate) fn search_skip_mounts() -> Vec<PathBuf> {
    Vec::new()
}

pub fn unmounted_volumes() -> Vec<Place> {
    Vec::new()
}

pub fn mount_device(device: &str) -> Result<PathBuf> {
    Err(CoreError::Other(format!("Mounting {device} manually is not supported on macOS")))
}

/// Eject (or, for servers and non-ejectable volumes, unmount) the volume at `path`.
pub fn eject_volume(path: &Path) -> Result<()> {
    let p = path.to_string_lossy();
    super::run_tool("diskutil", &["eject", &p])
        .or_else(|_| super::run_tool("diskutil", &["unmount", &p]))
        .map(drop)
        .map_err(|e| CoreError::Other(format!("Could not eject \"{}\": {e}", path.display())))
}

// ---------- Open With ----------

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    // CFStringRef arguments; NSString is toll-free bridged.
    fn LSSetDefaultRoleHandlerForContentType(
        content_type: *const NSString,
        role: u32,
        bundle_id: *const NSString,
    ) -> i32;
}
const LS_ROLES_ALL: u32 = 0xFFFF_FFFF;

fn file_url(path: &Path) -> Option<Retained<NSURL>> {
    let c = cstring(path).ok()?;
    let ptr = NonNull::new(c.as_ptr() as *mut _)?;
    // SAFETY: `c` is a valid NUL-terminated path for the duration of the call.
    Some(unsafe { NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(ptr, path.is_dir(), None) })
}

/// Apps Launch Services offers for this file (default first).
pub fn apps_for(path: &Path) -> Vec<AppInfo> {
    autoreleasepool(|_| {
        let Some(url) = file_url(path) else { return Vec::new() };
        let ws = NSWorkspace::sharedWorkspace();
        let default = ws.URLForApplicationToOpenURL(&url).and_then(|u| u.path()).map(|p| p.to_string());
        let fm = NSFileManager::defaultManager();
        let apps = ws
            .URLsForApplicationsToOpenURL(&url)
            .iter()
            .filter_map(|u| u.path().map(|p| p.to_string()))
            .filter(|p| !p.starts_with("/Volumes/") && !p.contains("/.Trash/")) // mounted installers etc.
            .map(|p| {
                let name = fm.displayNameAtPath(&NSString::from_str(&p)).to_string();
                AppInfo {
                    name: name.strip_suffix(".app").unwrap_or(&name).to_string(),
                    icon_key: Some(format!("path:{p}")),
                    is_default: default.as_deref() == Some(p.as_str()),
                    id: p,
                }
            })
            .collect();
        super::tidy_apps(apps)
    })
}

pub fn open_with(paths: &[PathBuf], app: &str) -> Result<()> {
    let mut args = vec!["-a".to_string(), app.to_string()];
    args.extend(paths.iter().map(|p| p.to_string_lossy().into_owned()));
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    super::run_tool("/usr/bin/open", &args)
        .map(drop)
        .map_err(|e| CoreError::Other(format!("Could not open with {app}: {e}")))
}

/// Make `app` the default for files of `path`'s type.
pub fn set_default_app(path: &Path, app: &str) -> Result<()> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let kind = if path.is_dir() { EntryKind::Directory } else { EntryKind::File };
    let uti = native_type(&name, path, kind).type_id.ok_or_else(|| CoreError::Other("Unknown file type".into()))?;
    let bundle_id = autoreleasepool(|_| {
        let url = NSURL::fileURLWithPath(&NSString::from_str(app));
        NSBundle::bundleWithURL(&url).and_then(|b| b.bundleIdentifier()).map(|s| s.to_string())
    })
    .ok_or_else(|| CoreError::Other(format!("{app} is not an application")))?;
    let (uti_ns, id_ns) = (NSString::from_str(&uti), NSString::from_str(&bundle_id));
    // SAFETY: both strings are valid for the call (toll-free bridged CFStrings).
    let status = unsafe { LSSetDefaultRoleHandlerForContentType(&*uti_ns, LS_ROLES_ALL, &*id_ns) };
    if status == 0 {
        Ok(())
    } else {
        Err(CoreError::Other(format!("macOS refused to change the default app for {uti} (error {status})")))
    }
}

// ---------- File types and icons ----------

/// Type description and identifier from UTType, cached per extension.
pub fn native_type(name: &str, path: &Path, kind: EntryKind) -> NativeType {
    let ext = name
        .rsplit_once('.')
        .map(|(stem, e)| (stem, e.to_ascii_lowercase()))
        .filter(|(s, e)| !s.is_empty() && !e.is_empty())
        .map(|(_, e)| e);
    match kind {
        EntryKind::Directory if ext.as_deref() == Some("app") => NativeType {
            label: None,
            type_id: Some("com.apple.application-bundle".into()),
            icon_key: Some(format!("path:{}", path.display())),
        },
        EntryKind::Directory => NativeType {
            label: None,
            type_id: Some("public.folder".into()),
            // Desktop, Documents, ... have their own Finder icons.
            icon_key: Some(if super::special_folder(path).is_some() {
                format!("path:{}", path.display())
            } else {
                "uti:public.folder".into()
            }),
        },
        EntryKind::File => {
            let Some(ext) = ext else {
                return NativeType {
                    label: None,
                    type_id: Some("public.data".into()),
                    icon_key: Some("uti:public.data".into()),
                };
            };
            let (label, id) = uttype_for_extension(&ext);
            NativeType { label, icon_key: Some(format!("uti:{id}")), type_id: Some(id) }
        }
        EntryKind::BrokenSymlink | EntryKind::Other => NativeType::default(),
    }
}

/// (description, identifier)
type UtiInfo = (Option<String>, String);

fn uttype_for_extension(ext: &str) -> UtiInfo {
    static CACHE: OnceLock<Mutex<HashMap<String, UtiInfo>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().unwrap_or_else(|p| p.into_inner()).get(ext) {
        return hit.clone();
    }
    let found = autoreleasepool(|_| {
        UTType::typeWithFilenameExtension(&NSString::from_str(ext)).map(|t| {
            let id = t.identifier().to_string();
            // Dynamic types ("dyn.ah62...") have no useful description.
            let label = (!id.starts_with("dyn.")).then(|| t.localizedDescription().map(|d| d.to_string())).flatten();
            (label, id)
        })
    });
    let value = found.unwrap_or((None, "public.data".into()));
    cache.lock().unwrap_or_else(|p| p.into_inner()).insert(ext.to_string(), value.clone());
    value
}

/// Render the icon for `key` (`uti:<identifier>` or `path:<file>`) as a PNG
/// of `size` points. AppKit picks the pixel density of the main screen, so
/// on a Retina display the PNG has twice as many pixels.
pub fn icon(key: &str, size: u32) -> Option<Icon> {
    let px = f64::from(size.clamp(8, 512));
    autoreleasepool(|_| {
        let ws = NSWorkspace::sharedWorkspace();
        let image = match key.split_once(':') {
            Some(("uti", id)) => {
                let content_type: Retained<UTType> = UTType::typeWithIdentifier(&NSString::from_str(id))?;
                ws.iconForContentType(&content_type)
            }
            Some(("path", p)) => ws.iconForFile(&NSString::from_str(p)),
            _ => return None,
        };
        image.setSize(NSSize::new(px, px));
        let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(px, px));
        // SAFETY: `rect` is a valid, writable NSRect for the duration of the call.
        let cg = unsafe { image.CGImageForProposedRect_context_hints(&mut rect, None, None) }?;
        let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), &cg);
        let props = NSDictionary::<NSBitmapImageRepPropertyKey, AnyObject>::new();
        // SAFETY: an empty properties dictionary is valid for PNG encoding.
        let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) }?;
        Some(Icon { bytes: data.to_vec(), content_type: "image/png" })
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_from_uttype() {
        let t = native_type("photo.PNG", Path::new("/tmp/photo.PNG"), EntryKind::File);
        assert_eq!(t.type_id.as_deref(), Some("public.png"));
        assert_eq!(t.icon_key.as_deref(), Some("uti:public.png"));
        assert!(t.label.is_some_and(|l| l.to_lowercase().contains("png")));
        let unknown = native_type("x.zzqqunknown", Path::new("/tmp/x.zzqqunknown"), EntryKind::File);
        assert_eq!(unknown.label, None, "dynamic types fall back to Sonar's own labels");
        let app = native_type("Safari.app", Path::new("/Applications/Safari.app"), EntryKind::Directory);
        assert_eq!(app.icon_key.as_deref(), Some("path:/Applications/Safari.app"));
    }

    #[test]
    fn icons_render_as_png() {
        for key in ["uti:public.png", "uti:public.folder", "path:/Applications"] {
            let icon = icon(key, 32).unwrap_or_else(|| panic!("no icon for {key}"));
            assert_eq!(&icon.bytes[..8], b"\x89PNG\r\n\x1a\n", "{key}");
            let img = image::load_from_memory(&icon.bytes).unwrap();
            assert!([32, 64].contains(&img.width()) && img.width() == img.height(), "{key}: {}px", img.width());
        }
        assert!(icon("uti:", 32).is_none() || icon("bogus", 32).is_none());
    }

    #[test]
    fn ejectable_from_diskutil_plist() {
        let parse = |xml: &str| diskutil_says_ejectable(&plist::from_bytes(xml.as_bytes()).unwrap());
        let doc = |body: &str| {
            format!(r#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>{body}</dict></plist>"#)
        };
        assert!(parse(&doc("<key>Ejectable</key><true/><key>Internal</key><false/>")));
        assert!(parse(&doc("<key>Ejectable</key><false/><key>Internal</key><false/>")), "external disk");
        assert!(!parse(&doc("<key>Ejectable</key><false/><key>Removable</key><false/><key>Internal</key><true/>")));
    }
}

#[cfg(test)]
mod open_with_tests {
    use super::*;

    #[test]
    fn lists_apps_for_a_text_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("notes.txt");
        fs::write(&f, "x").unwrap();
        let apps = apps_for(&f);
        assert!(apps.iter().any(|a| a.name == "TextEdit"), "{:?}", apps.iter().map(|a| &a.name).collect::<Vec<_>>());
        assert_eq!(apps.iter().filter(|a| a.is_default).count(), 1, "exactly one default, listed first");
        assert!(apps[0].is_default);
        assert!(
            apps.iter()
                .all(|a| a.id.ends_with(".app") && a.icon_key.as_deref().is_some_and(|k| k.starts_with("path:")))
        );
    }
}
