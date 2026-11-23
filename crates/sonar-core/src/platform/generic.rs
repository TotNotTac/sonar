//! Fallback for platforms Sonar doesn't target yet (keeps the crate building).

use std::fs::Metadata;
use std::io;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};
use crate::places::{Place, PlaceSection};

pub struct HiddenRules;

impl HiddenRules {
    pub fn for_dir(_dir: &Path) -> Self {
        HiddenRules
    }

    pub fn is_hidden(&self, name: &str, _meta: &Metadata) -> bool {
        super::is_dotfile(name)
    }
}

pub fn open_path(path: &Path) -> Result<()> {
    Err(CoreError::Open { path: path.to_path_buf(), message: "not supported on this platform".into() })
}

pub(crate) fn rename_noreplace_raw(from: &Path, to: &Path) -> io::Result<()> {
    super::rename_noreplace_fallback(from, to)
}

pub(crate) fn clone_file(_src: &Path, _dst: &Path) -> io::Result<bool> {
    Ok(false)
}

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
    Err(CoreError::Other(format!("Trash is not supported on this platform ({})", path.display())))
}

pub fn restore(receipt: &TrashReceipt) -> Result<()> {
    Err(CoreError::Other(format!("Trash is not supported on this platform ({})", receipt.original.display())))
}

pub fn trash_dir() -> Option<PathBuf> {
    None
}

pub fn mounted_volumes() -> Vec<Place> {
    vec![Place::new("Root", "/", "root", PlaceSection::Devices)]
}
