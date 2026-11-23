use std::path::{Component, Path, PathBuf};

use crate::error::{CoreError, Result};

/// Turn what a user typed into the location bar into an absolute, lexically
/// normalised path. Supports `~`, `~/x`, and paths relative to `cwd`.
/// Does not touch the filesystem (symlinks are preserved).
pub fn resolve_user_path(input: &str, cwd: &Path) -> Result<PathBuf> {
    let input = input.trim();
    if input.is_empty() {
        return Err(CoreError::InvalidPath("empty path".into()));
    }
    let home = || dirs::home_dir().ok_or_else(|| CoreError::InvalidPath("no home directory".into()));

    let raw = if input == "~" {
        home()?
    } else if let Some(rest) = input.strip_prefix("~/") {
        home()?.join(rest)
    } else if let Some(rest) = input.strip_prefix("file://") {
        PathBuf::from(rest)
    } else {
        let p = PathBuf::from(input);
        if p.is_absolute() {
            p
        } else {
            cwd.join(p)
        }
    };
    Ok(normalize(&raw))
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                // Never pop past the root.
                if out.parent().is_some() {
                    out.pop();
                }
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves() {
        let cwd = Path::new("/a/b");
        assert_eq!(resolve_user_path("c", cwd).unwrap(), Path::new("/a/b/c"));
        assert_eq!(resolve_user_path("../c/./d/", cwd).unwrap(), Path::new("/a/c/d"));
        assert_eq!(resolve_user_path("/../..", cwd).unwrap(), Path::new("/"));
        assert_eq!(resolve_user_path("file:///tmp", cwd).unwrap(), Path::new("/tmp"));
        assert!(resolve_user_path("~", cwd).unwrap().is_absolute());
        assert!(resolve_user_path("  ", cwd).is_err());
    }
}
