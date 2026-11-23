use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use super::transfer::{Conflict, ConflictChoice, Progress, TransferKind, TransferObserver, run_transfer};
use crate::error::{CoreError, Result};
use crate::platform::{self, TrashReceipt};

/// A completed operation that can be reversed.
#[derive(Debug, Clone)]
pub enum UndoAction {
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    NewFolder(PathBuf),
    Trash(Vec<TrashReceipt>),
    /// Items created by a copy.
    Copy(Vec<PathBuf>),
    /// Items created by another job (extracting, compressing), with its name.
    Created(&'static str, Vec<PathBuf>),
    /// `(from, to)` pairs of a move.
    Move(Vec<(PathBuf, PathBuf)>),
}

impl UndoAction {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Rename { .. } => "Rename",
            Self::NewFolder(_) => "Create Folder",
            Self::Trash(_) => "Move to Trash",
            Self::Copy(_) => "Copy",
            Self::Created(label, _) => label,
            Self::Move(_) => "Move",
        }
    }

    /// Reverse the operation. Tries every item and returns the first error.
    /// Undoing a copy moves the copies to the trash (like Dolphin), so even
    /// undo never destroys data outright.
    pub fn undo(self) -> Result<()> {
        let mut first_err = None;
        let mut note = |r: Result<()>| {
            if let Err(e) = r {
                first_err.get_or_insert(e);
            }
        };
        match self {
            Self::Rename { from, to } => note(rename_back(&to, &from)),
            Self::NewFolder(path) => note(match fs::remove_dir(&path) {
                Ok(()) => Ok(()),
                // Something was put in it since: keep it recoverable.
                Err(e) if e.kind() == io::ErrorKind::DirectoryNotEmpty => platform::trash(&path).map(drop),
                Err(e) => Err(CoreError::from_io(&path, e)),
            }),
            Self::Trash(receipts) => receipts.iter().rev().for_each(|r| note(platform::restore(r))),
            Self::Copy(created) | Self::Created(_, created) => {
                created.iter().for_each(|p| note(platform::trash(p).map(drop)))
            }
            Self::Move(pairs) => pairs.iter().rev().for_each(|(from, to)| note(move_back(to, from))),
        }
        first_err.map_or(Ok(()), Err)
    }
}

fn rename_back(current: &Path, original: &Path) -> Result<()> {
    platform::rename_noreplace(current, original).map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => CoreError::AlreadyExists(original.to_path_buf()),
        _ => CoreError::from_io(current, e),
    })
}

/// Undo one moved item: rename back, or copy+delete across filesystems.
fn move_back(current: &Path, original: &Path) -> Result<()> {
    match platform::rename_noreplace(current, original) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Err(CoreError::AlreadyExists(original.into())),
        Err(e) => return Err(CoreError::from_io(current, e)),
    }
    if fs::symlink_metadata(original).is_ok() {
        return Err(CoreError::AlreadyExists(original.into()));
    }
    let parent = original.parent().ok_or_else(|| CoreError::InvalidPath(original.display().to_string()))?;

    struct RefuseConflicts;
    impl TransferObserver for RefuseConflicts {
        fn progress(&mut self, _: &Progress) {}
        fn conflict(&mut self, _: &Conflict) -> (ConflictChoice, bool) {
            (ConflictChoice::Cancel, false)
        }
    }
    let out = run_transfer(
        TransferKind::Move,
        &[current.to_path_buf()],
        parent,
        &AtomicBool::new(false),
        &mut RefuseConflicts,
    );
    if let Some(e) = out.errors.into_iter().next() {
        return Err(e);
    }
    // If the move had renamed the item ("x (1)"), restore the original name.
    match out.moved.first() {
        Some((_, landed)) if landed != original => rename_back(landed, original),
        Some(_) => Ok(()),
        None => Err(CoreError::Other(format!("Could not move \"{}\" back", current.display()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_rename_and_new_folder() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        fs::write(&a, "").unwrap();
        let b = crate::ops::rename(&a, "b").unwrap();
        UndoAction::Rename { from: a.clone(), to: b.clone() }.undo().unwrap();
        assert!(a.exists() && !b.exists());

        let f = crate::ops::new_folder(d.path()).unwrap();
        UndoAction::NewFolder(f.clone()).undo().unwrap();
        assert!(!f.exists());
    }

    #[test]
    fn undo_move() {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("src"), d.path().join("dst"));
        fs::create_dir_all(src.join("x")).unwrap();
        fs::create_dir(&dst).unwrap();
        let out = run_transfer(TransferKind::Move, &[src.join("x")], &dst, &AtomicBool::new(false), &mut NoConflicts);
        UndoAction::Move(out.moved).undo().unwrap();
        assert!(src.join("x").is_dir() && !dst.join("x").exists());
    }

    struct NoConflicts;
    impl TransferObserver for NoConflicts {
        fn progress(&mut self, _: &Progress) {}
        fn conflict(&mut self, _: &Conflict) -> (ConflictChoice, bool) {
            unreachable!()
        }
    }
}
