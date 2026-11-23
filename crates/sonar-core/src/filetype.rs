//! Lightweight, extension-based file type descriptions.
//!
//! This is deliberately simple; a later iteration can swap in shared-mime-info
//! on Linux and UTType on macOS via the platform module.

use crate::entry::EntryKind;

pub struct FileType {
    pub label: String,
    /// Built-in icon category, used when the OS icon is unavailable.
    pub icon: &'static str,
    /// MIME type (Linux) or UTI (macOS), if the OS knows it.
    pub type_id: Option<String>,
    /// Key for `platform::icon`, if the OS can provide an icon.
    pub icon_key: Option<String>,
}

/// The OS's description and icon where available, the built-in table otherwise.
pub fn describe_native(name: &str, path: &std::path::Path, kind: EntryKind) -> FileType {
    let mut ft = describe(name, kind);
    let native = crate::platform::native_type(name, path, kind);
    if let Some(label) = native.label {
        ft.label = label;
    }
    ft.type_id = native.type_id;
    ft.icon_key = native.icon_key;
    ft
}

const TABLE: &[(&[&str], &str, &str)] = &[
    (&["png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "heic", "avif", "ico"], "image", "image"),
    (&["svg"], "SVG image", "image"),
    (&["mp3", "flac", "ogg", "opus", "wav", "m4a", "aac"], "audio", "audio"),
    (&["mp4", "mkv", "webm", "mov", "avi", "m4v"], "video", "video"),
    (&["zip", "tar", "gz", "tgz", "xz", "bz2", "zst", "7z", "rar"], "archive", "archive"),
    (&["dmg", "iso", "img"], "disk image", "archive"),
    (&["pdf"], "PDF document", "document"),
    (&["doc", "docx", "odt", "rtf", "pages"], "document", "document"),
    (&["xls", "xlsx", "ods", "csv", "numbers"], "spreadsheet", "document"),
    (&["ppt", "pptx", "odp", "key"], "presentation", "document"),
    (&["txt", "md", "log", "ini", "conf", "cfg"], "text", "text"),
    (&["json", "toml", "yaml", "yml", "xml", "lock"], "data file", "text"),
    (
        &[
            "rs", "c", "h", "cpp", "hpp", "py", "js", "mjs", "ts", "tsx", "jsx", "go", "java", "kt", "swift", "rb",
            "php", "lua", "zig", "sh", "bash", "zsh", "fish", "html", "css", "scss", "sql",
        ],
        "source code",
        "code",
    ),
    (&["app"], "application", "app"),
    (&["desktop", "appimage", "exe", "pkg", "deb", "rpm"], "application package", "app"),
    (&["ttf", "otf", "woff", "woff2"], "font", "font"),
];

pub fn describe(name: &str, kind: EntryKind) -> FileType {
    match kind {
        EntryKind::Directory => {
            // macOS bundles are directories but users think of them as files.
            if extension(name).as_deref() == Some("app") {
                return FileType { label: "Application".into(), icon: "app", type_id: None, icon_key: None };
            }
            FileType { label: "Folder".into(), icon: "folder", type_id: None, icon_key: None }
        }
        EntryKind::BrokenSymlink => {
            FileType { label: "Broken link".into(), icon: "broken", type_id: None, icon_key: None }
        }
        EntryKind::Other => FileType { label: "Special file".into(), icon: "file", type_id: None, icon_key: None },
        EntryKind::File => {
            let Some(ext) = extension(name) else {
                return FileType { label: "File".into(), icon: "file", type_id: None, icon_key: None };
            };
            for (exts, kind_label, icon) in TABLE {
                if exts.contains(&ext.as_str()) {
                    let label = if kind_label.chars().next().is_some_and(|c| c.is_uppercase()) {
                        (*kind_label).to_string()
                    } else {
                        format!("{} {}", ext.to_uppercase(), kind_label)
                    };
                    return FileType { label, icon, type_id: None, icon_key: None };
                }
            }
            FileType { label: format!("{} file", ext.to_uppercase()), icon: "file", type_id: None, icon_key: None }
        }
    }
}

fn extension(name: &str) -> Option<String> {
    let trimmed = name.strip_prefix('.').unwrap_or(name);
    let (_, ext) = trimmed.rsplit_once('.')?;
    (!ext.is_empty()).then(|| ext.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(describe("a.PNG", EntryKind::File).label, "PNG image");
        assert_eq!(describe("doc.pdf", EntryKind::File).label, "PDF document");
        assert_eq!(describe(".bashrc", EntryKind::File).label, "File");
        assert_eq!(describe("x.weird", EntryKind::File).label, "WEIRD file");
        assert_eq!(describe("Safari.app", EntryKind::Directory).label, "Application");
    }
}
