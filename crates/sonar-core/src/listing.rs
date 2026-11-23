use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::entry::Entry;
use crate::error::{CoreError, Result};
use crate::platform::HiddenRules;
use crate::sort::{sort_entries, SortSpec};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ListOptions {
    pub show_hidden: bool,
    pub sort: SortSpec,
    /// Count each folder's children while listing. This costs one extra
    /// `read_dir` per folder, so callers normally leave it off and fill in
    /// counts afterwards with [`child_count`]. Required to sort folders by size.
    pub count_children: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    #[serde(with = "crate::uipath::path")]
    pub path: PathBuf,
    #[serde(with = "crate::uipath::opt")]
    pub parent: Option<PathBuf>,
    pub entries: Vec<Entry>,
    /// Entries filtered out because they are hidden.
    pub hidden_count: usize,
    /// Number of entries that could not be stat'ed (e.g. vanished mid-read).
    pub skipped_count: usize,
}

pub fn list_dir(path: &Path, opts: &ListOptions) -> Result<Listing> {
    if !path.is_absolute() {
        return Err(CoreError::InvalidPath(format!("{} is not an absolute path", path.display())));
    }
    let meta = fs::metadata(path).map_err(|e| CoreError::from_io(path, e))?;
    if !meta.is_dir() {
        return Err(CoreError::NotADirectory(path.to_path_buf()));
    }

    let read = fs::read_dir(path).map_err(|e| CoreError::from_io(path, e))?;
    let rules = HiddenRules::for_dir(path);

    let mut entries = Vec::new();
    let (mut hidden_count, mut skipped_count) = (0, 0);
    for de in read {
        let Some(entry) = de.ok().and_then(|de| Entry::from_dir_entry(&de, &rules, opts.count_children)) else {
            skipped_count += 1;
            continue;
        };
        if entry.is_hidden && !opts.show_hidden {
            hidden_count += 1;
            continue;
        }
        entries.push(entry);
    }
    sort_entries(&mut entries, opts.sort);

    Ok(Listing {
        path: path.to_path_buf(),
        parent: path.parent().map(Path::to_path_buf),
        entries,
        hidden_count,
        skipped_count,
    })
}

/// Number of entries in a folder (hidden ones included), or `None` if it
/// can't be read.
pub fn child_count(path: &Path) -> Option<u64> {
    fs::read_dir(path).ok().map(|rd| rd.count() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntryKind;

    #[test]
    fn lists_sorted_and_filters_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        fs::create_dir(p.join("zeta")).unwrap();
        fs::write(p.join("b10.txt"), "hello").unwrap();
        fs::write(p.join("b2.txt"), "").unwrap();
        fs::write(p.join(".secret"), "").unwrap();

        let l = list_dir(p, &ListOptions { count_children: true, ..Default::default() }).unwrap();
        let names: Vec<_> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["zeta", "b2.txt", "b10.txt"]);
        assert_eq!(l.hidden_count, 1);
        assert_eq!(l.entries[0].kind, EntryKind::Directory);
        assert_eq!(l.entries[0].child_count, Some(0));
        assert_eq!(l.entries[2].size, Some(5));

        let l = list_dir(p, &ListOptions { show_hidden: true, ..Default::default() }).unwrap();
        assert_eq!(l.entries.len(), 4);
        assert_eq!(l.entries[0].child_count, None, "not counted unless asked");
        assert_eq!(child_count(&p.join("zeta")), Some(0));
        assert_eq!(child_count(&p.join("missing")), None);
    }

    #[test]
    fn errors() {
        assert!(matches!(list_dir(Path::new("relative"), &Default::default()), Err(CoreError::InvalidPath(_))));
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope");
        assert!(matches!(list_dir(&missing, &Default::default()), Err(CoreError::NotFound(_))));
        let file = dir.path().join("f");
        fs::write(&file, "").unwrap();
        assert!(matches!(list_dir(&file, &Default::default()), Err(CoreError::NotADirectory(_))));
    }
}
