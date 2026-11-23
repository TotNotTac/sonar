use std::fs::{self, Metadata};
use std::path::PathBuf;
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::filetype;
use crate::platform::HiddenRules;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Directory,
    File,
    /// A symlink whose target could not be resolved.
    BrokenSymlink,
    /// Sockets, FIFOs, device nodes, ...
    Other,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// Encoded for the UI (see [`crate::uipath`]), so it joins onto the encoded folder path.
    pub name: String,
    #[serde(with = "crate::uipath::path")]
    pub path: PathBuf,
    pub kind: EntryKind,
    pub is_symlink: bool,
    pub is_hidden: bool,
    pub readonly: bool,
    /// Size in bytes, for files only.
    pub size: Option<u64>,
    /// Number of children, for directories only. `None` if unreadable or
    /// not counted yet (see [`crate::child_count`]).
    pub child_count: Option<u64>,
    /// Last modification time as milliseconds since the Unix epoch.
    pub modified: Option<i64>,
    /// Human readable type, e.g. "Folder" or "PNG image".
    pub type_label: String,
    /// Built-in icon category for the UI, e.g. "folder", "image", "text".
    pub icon: &'static str,
    /// MIME type (Linux) or UTI (macOS), when the OS knows it.
    pub type_id: Option<String>,
    /// Key to fetch the OS icon through the app's icon protocol.
    pub icon_key: Option<String>,
    /// Whether a thumbnail can be generated for this file.
    pub thumbnail: bool,
    /// An archive Sonar can browse into and extract.
    pub archive: bool,
}

impl Entry {
    /// Build an entry from a `read_dir` result. Symlinks are followed for
    /// kind/size/mtime, matching how Dolphin presents them.
    pub(crate) fn from_dir_entry(de: &fs::DirEntry, hidden: &HiddenRules, count_children: bool) -> Option<Self> {
        let path = de.path();
        let name = crate::uipath::encode(&de.file_name());
        let link_meta = de.metadata().ok()?; // does not follow symlinks
        let is_symlink = link_meta.file_type().is_symlink();
        let target_meta = if is_symlink { fs::metadata(&path).ok() } else { Some(link_meta.clone()) };

        let is_hidden = hidden.is_hidden(&name, &link_meta);
        let meta: &Metadata = target_meta.as_ref().unwrap_or(&link_meta);

        let kind = match &target_meta {
            None => EntryKind::BrokenSymlink,
            Some(m) if m.is_dir() => EntryKind::Directory,
            Some(m) if m.is_file() => EntryKind::File,
            Some(_) => EntryKind::Other,
        };

        let (size, child_count) = match kind {
            EntryKind::File => (Some(meta.len()), None),
            EntryKind::Directory => (None, if count_children { crate::listing::child_count(&path) } else { None }),
            _ => (None, None),
        };

        let modified =
            meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64);

        let ft = filetype::describe_native(&name, &path, kind);
        let thumbnail = kind == EntryKind::File && crate::thumbnail::supported(&name);
        let archive = kind == EntryKind::File && crate::archive::is_archive(&name);

        Some(Entry {
            name,
            path,
            kind,
            is_symlink,
            is_hidden,
            readonly: meta.permissions().readonly(),
            size,
            child_count,
            modified,
            type_label: ft.label,
            icon: ft.icon,
            type_id: ft.type_id,
            icon_key: ft.icon_key,
            thumbnail,
            archive,
        })
    }

    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Directory
    }

    /// A directory the user navigates into. macOS `.app` bundles are
    /// directories on disk but behave like files, so they sort with files.
    pub fn is_folder(&self) -> bool {
        self.is_dir() && self.icon != "app"
    }
}
