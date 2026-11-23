use std::cmp::Ordering;
use std::iter::Peekable;
use std::str::Chars;

use serde::{Deserialize, Serialize};

use crate::entry::Entry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortKey {
    #[default]
    Name,
    Size,
    Modified,
    Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortOrder {
    #[default]
    Ascending,
    Descending,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SortSpec {
    pub key: SortKey,
    pub order: SortOrder,
}

/// Folders always come first, regardless of key or direction; the sort
/// order only applies within the folder group and within the file group.
pub fn sort_entries(entries: &mut [Entry], spec: SortSpec) {
    entries.sort_by(|a, b| {
        let group = b.is_folder().cmp(&a.is_folder());
        if group != Ordering::Equal {
            return group;
        }
        let primary = match spec.key {
            SortKey::Name => Ordering::Equal,
            // Folders sort by item count in the size column, like Dolphin.
            SortKey::Size => a.size.or(a.child_count).cmp(&b.size.or(b.child_count)),
            SortKey::Modified => a.modified.cmp(&b.modified),
            SortKey::Type => a.type_label.cmp(&b.type_label),
        };
        let ord = primary.then_with(|| natural_cmp(&a.name, &b.name));
        match spec.order {
            SortOrder::Ascending => ord,
            SortOrder::Descending => ord.reverse(),
        }
    });
}

/// Case-insensitive "natural" comparison: `file2` < `file10`.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut ai, mut bi) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (ai.peek().copied(), bi.peek().copied()) {
            (None, None) => return a.cmp(b), // stable tie-break on exact bytes
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (na, nb) = (take_digits(&mut ai), take_digits(&mut bi));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb)).then_with(|| na.len().cmp(&nb.len()));
                if ord != Ordering::Equal {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord != Ordering::Equal {
                    return ord;
                }
                ai.next();
                bi.next();
            }
        }
    }
}

fn take_digits(it: &mut Peekable<Chars>) -> String {
    let mut s = String::new();
    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
        s.push(c);
        it.next();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::EntryKind;
    use std::cmp::Ordering::*;

    fn entry(name: &str, kind: EntryKind, size: u64) -> Entry {
        let icon = if name.ends_with(".app") { "app" } else { "file" };
        Entry {
            name: name.into(),
            path: name.into(),
            kind,
            is_symlink: false,
            is_hidden: false,
            readonly: false,
            size: (kind == EntryKind::File).then_some(size),
            child_count: (kind == EntryKind::Directory).then_some(size),
            modified: Some(size as i64),
            type_label: String::new(),
            icon,
            type_id: None,
            icon_key: None,
            thumbnail: false,
            archive: false,
        }
    }

    #[test]
    fn folders_first_in_every_order() {
        use EntryKind::*;
        let mut v = vec![
            entry("a.txt", File, 5),
            entry("zeta", Directory, 1),
            entry("Safari.app", Directory, 9),
            entry("b.txt", File, 50),
            entry("alpha", Directory, 3),
        ];
        for key in [SortKey::Name, SortKey::Size, SortKey::Modified, SortKey::Type] {
            for order in [SortOrder::Ascending, SortOrder::Descending] {
                sort_entries(&mut v, SortSpec { key, order });
                let folders: Vec<_> = v.iter().map(|e| e.is_folder()).collect();
                assert_eq!(folders, [true, true, false, false, false], "{key:?} {order:?}");
            }
        }
        sort_entries(&mut v, SortSpec { key: SortKey::Name, order: SortOrder::Descending });
        let names: Vec<_> = v.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["zeta", "alpha", "Safari.app", "b.txt", "a.txt"]);
    }

    #[test]
    fn natural() {
        assert_eq!(natural_cmp("file2", "file10"), Less);
        assert_eq!(natural_cmp("File", "file"), Less); // tie-break only
        assert_eq!(natural_cmp("apple", "Banana"), Less);
        assert_eq!(natural_cmp("a01", "a1"), Greater);
        assert_eq!(natural_cmp("x", "x1"), Less);
    }
}
