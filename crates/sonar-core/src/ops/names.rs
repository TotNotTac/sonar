use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

/// Reject names that can't be a single path component.
pub fn validate_name(name: &str) -> Result<()> {
    let bad = |m: &str| Err(CoreError::InvalidName(m.into()));
    if name.trim().is_empty() {
        return bad("The name cannot be empty");
    }
    if name == "." || name == ".." {
        return bad("\".\" and \"..\" are not valid names");
    }
    if name.contains('/') {
        return bad("The name cannot contain \"/\"");
    }
    if name.contains('\0') {
        return bad("The name contains an invalid character");
    }
    if name.len() > 255 {
        return bad("The name is too long");
    }
    Ok(())
}

/// `dir/name` if nothing is there, otherwise the next free "Keep both" name.
pub fn available_name(dir: &Path, name: &str, is_dir: bool) -> PathBuf {
    let p = dir.join(name);
    if exists(&p) {
        keep_both_name(dir, name, is_dir)
    } else {
        p
    }
}

/// First free name of the form `stem (n).ext` in `dir`, KIO-style.
/// `photo.jpg` -> `photo (1).jpg`; `photo (1).jpg` -> `photo (2).jpg`.
/// Folders and dot-files without a further extension keep the whole name as stem.
/// `name` may be [`crate::uipath`]-encoded, so names that aren't UTF-8 keep their bytes.
pub fn keep_both_name(dir: &Path, name: &str, is_dir: bool) -> PathBuf {
    let (stem, ext) = split_ext(name, is_dir);
    let (base, mut n) = match parse_counter(stem) {
        Some((base, n)) => (base, n + 1),
        None => (stem, 1),
    };
    loop {
        let candidate = match ext {
            Some(ext) => format!("{base} ({n}).{ext}"),
            None => format!("{base} ({n})"),
        };
        let p = dir.join(crate::uipath::decode(&candidate));
        if !exists(&p) {
            return p;
        }
        n += 1;
    }
}

fn exists(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok()
}

fn split_ext(name: &str, is_dir: bool) -> (&str, Option<&str>) {
    if is_dir {
        return (name, None);
    }
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (&name[..i], Some(&name[i + 1..])),
        _ => (name, None),
    }
}

/// `"name (3)"` -> `("name", 3)`.
fn parse_counter(stem: &str) -> Option<(&str, u32)> {
    let inner = stem.strip_suffix(')')?;
    let open = inner.rfind(" (")?;
    let n = inner[open + 2..].parse().ok()?;
    Some((&stem[..open], n))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation() {
        assert!(validate_name("ok.txt").is_ok());
        assert!(validate_name(".hidden").is_ok());
        for bad in ["", "  ", ".", "..", "a/b", "a\0b"] {
            assert!(validate_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn keep_both() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path();
        let name = |pb: PathBuf| pb.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(name(keep_both_name(p, "photo.jpg", false)), "photo (1).jpg");
        fs::write(p.join("photo (1).jpg"), "").unwrap();
        assert_eq!(name(keep_both_name(p, "photo.jpg", false)), "photo (2).jpg");
        assert_eq!(name(keep_both_name(p, "photo (1).jpg", false)), "photo (2).jpg");
        assert_eq!(name(keep_both_name(p, "my.folder", true)), "my.folder (1)");
        assert_eq!(name(keep_both_name(p, ".bashrc", false)), ".bashrc (1)");
        assert_eq!(name(available_name(p, "free", true)), "free");
    }

    #[cfg(unix)]
    #[test]
    fn keep_both_keeps_bytes_that_are_not_text() {
        use std::os::unix::ffi::OsStrExt;
        let d = tempfile::tempdir().unwrap();
        let raw = std::ffi::OsStr::from_bytes(b"caf\xe9.txt");
        let got = keep_both_name(d.path(), &crate::uipath::encode(raw), false);
        assert_eq!(got.file_name().unwrap().as_bytes(), b"caf\xe9 (1).txt");
    }
}
