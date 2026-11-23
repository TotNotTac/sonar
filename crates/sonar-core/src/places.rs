use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaceSection {
    Places,
    Remote,
    Devices,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    pub label: String,
    pub path: PathBuf,
    /// Icon hint: home, desktop, documents, downloads, music, pictures,
    /// videos, trash, drive, root, removable, network.
    pub icon: &'static str,
    pub section: PlaceSection,
}

impl Place {
    pub fn new(label: impl Into<String>, path: impl Into<PathBuf>, icon: &'static str, section: PlaceSection) -> Self {
        Self { label: label.into(), path: path.into(), icon, section }
    }
}

/// All sidebar entries: standard user folders, trash, then mounted devices.
pub fn places() -> Vec<Place> {
    let mut out = Vec::new();
    let home = dirs::home_dir();

    if let Some(h) = &home {
        out.push(Place::new("Home", h, "home", PlaceSection::Places));
    }

    let user_dirs: [(Option<PathBuf>, &'static str); 6] = [
        (dirs::desktop_dir(), "desktop"),
        (dirs::document_dir(), "documents"),
        (dirs::download_dir(), "downloads"),
        (dirs::audio_dir(), "music"),
        (dirs::picture_dir(), "pictures"),
        (dirs::video_dir(), "videos"),
    ];
    for (dir, icon) in user_dirs {
        let Some(dir) = dir else { continue };
        // Unconfigured XDG dirs fall back to $HOME; don't list Home twice.
        if Some(&dir) == home.as_ref() || !dir.is_dir() {
            continue;
        }
        // Use the real folder name so localised XDG dirs and macOS "Movies"
        // show up the way the user sees them elsewhere.
        out.push(Place::new(folder_name(&dir), dir, icon, PlaceSection::Places));
    }

    if let Some(trash) = platform::trash_dir() {
        out.push(Place::new("Trash", trash, "trash", PlaceSection::Places));
    }

    out.extend(platform::mounted_volumes());
    out
}

fn folder_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}
