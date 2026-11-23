use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::names::{available_name, validate_name};
use crate::error::{CoreError, Result};
use crate::platform::{self, TrashReceipt};

/// Rename in place. Never overwrites: fails with `AlreadyExists` if the name
/// is taken (a case-only change of the same file is fine).
pub fn rename(path: &Path, new_name: &str) -> Result<PathBuf> {
    validate_name(new_name)?;
    let parent = path.parent().ok_or_else(|| CoreError::InvalidPath(path.display().to_string()))?;
    let target = parent.join(new_name);
    if target == path {
        return Ok(target);
    }
    platform::rename_noreplace(path, &target).map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => CoreError::AlreadyExists(target.clone()),
        _ => CoreError::from_io(path, e),
    })?;
    Ok(target)
}

/// Create `parent/name`. Fails if anything with that name exists.
pub fn create_folder(parent: &Path, name: &str) -> Result<PathBuf> {
    validate_name(name)?;
    let path = parent.join(name);
    fs::create_dir(&path).map_err(|e| CoreError::from_io(&path, e))?;
    Ok(path)
}

/// Create "New Folder" (or "New Folder (n)") in `parent`.
pub fn new_folder(parent: &Path) -> Result<PathBuf> {
    // Retry in case another process grabs the name between check and create.
    for _ in 0..5 {
        let path = available_name(parent, "New Folder", true);
        match fs::create_dir(&path) {
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            r => return r.map(|_| path.clone()).map_err(|e| CoreError::from_io(&path, e)),
        }
    }
    Err(CoreError::Other("Could not find a free name for the new folder".into()))
}

/// Move one item to the system trash.
pub fn trash(path: &Path) -> Result<TrashReceipt> {
    platform::trash(path)
}

/// Delete without using the trash. Folders are removed recursively; symlinks
/// are removed themselves, never their targets.
pub fn delete_permanently(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path).map_err(|e| CoreError::from_io(path, e))?;
    let res = if meta.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
    res.map_err(|e| CoreError::from_io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_never_overwrites() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a"), d.path().join("b"));
        fs::write(&a, "A").unwrap();
        fs::write(&b, "B").unwrap();
        assert!(matches!(rename(&a, "b"), Err(CoreError::AlreadyExists(_))));
        assert_eq!(fs::read_to_string(&b).unwrap(), "B");
        assert!(matches!(rename(&a, "x/y"), Err(CoreError::InvalidName(_))));
        let c = rename(&a, "c").unwrap();
        assert_eq!(fs::read_to_string(c).unwrap(), "A");
    }

    #[test]
    fn rename_case_only() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("readme");
        fs::write(&a, "x").unwrap();
        let r = rename(&a, "README").unwrap();
        let names: Vec<_> = fs::read_dir(d.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, ["README"]);
        assert!(r.ends_with("README"));
    }

    #[test]
    fn folders() {
        let d = tempfile::tempdir().unwrap();
        assert!(new_folder(d.path()).unwrap().ends_with("New Folder"));
        assert!(new_folder(d.path()).unwrap().ends_with("New Folder (1)"));
        assert!(matches!(create_folder(d.path(), "New Folder"), Err(CoreError::AlreadyExists(_))));
        let f = create_folder(d.path(), "x").unwrap();
        fs::write(f.join("inner"), "").unwrap();
        delete_permanently(&f).unwrap();
        assert!(!f.exists());
    }
}

/// Uses the real system trash, so it's opt-in: `cargo test -- --ignored`.
#[cfg(test)]
mod trash_tests {
    use super::*;
    use crate::ops::UndoAction;

    #[test]
    #[ignore = "touches the user's real Trash"]
    fn trash_and_restore_roundtrip() {
        let d = tempfile::tempdir_in(dirs::home_dir().unwrap()).unwrap();
        let f = d.path().join("sonar-trash-test.txt");
        fs::write(&f, "hello").unwrap();
        let receipt = trash(&f).unwrap();
        assert!(!f.exists());
        UndoAction::Trash(vec![receipt]).undo().unwrap();
        assert_eq!(fs::read_to_string(&f).unwrap(), "hello");
    }
}
