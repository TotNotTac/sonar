//! Fallback for platforms without a native clipboard backend yet.

use sonar_core::clipboard::FileClip;

pub fn write_files(_clip: &FileClip) -> Result<(), String> {
    Err("not supported on this platform".into())
}

pub fn read_files() -> Option<FileClip> {
    None
}

pub fn write_text(_text: &str) -> Result<(), String> {
    Err("not supported on this platform".into())
}

pub fn clear_cut() {}
