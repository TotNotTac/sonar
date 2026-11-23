//! Debounced, non-recursive watching of a single folder.
//!
//! Uses FSEvents on macOS and inotify on Linux (via `notify`). Bursts of
//! events — a `git checkout`, an unpacking archive, a download writing
//! chunks — are coalesced into one callback per debounce window.

use std::path::{Path, PathBuf};
use std::time::Duration;

use notify_debouncer_mini::notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, DebouncedEventBatch, Debouncer, new_debouncer};

use crate::error::{CoreError, Result};

pub const DEFAULT_DEBOUNCE: Duration = Duration::from_millis(400);

/// Watches one folder until dropped.
pub struct DirWatcher {
    path: PathBuf,
    _debouncer: Debouncer<RecommendedWatcher>,
}

impl DirWatcher {
    /// Call `on_change` (from a background thread) whenever the direct
    /// contents of `path` change, or the folder itself is removed/renamed.
    pub fn new(path: &Path, debounce: Duration, on_change: impl Fn() + Send + 'static) -> Result<Self> {
        let err = |e: notify_debouncer_mini::notify::Error| CoreError::Watch {
            path: path.to_path_buf(),
            message: e.to_string(),
        };
        let mut debouncer = new_debouncer(debounce, move |res: DebounceEventResult| {
            // A rescan request or an error (e.g. inotify queue overflow) means
            // we may have missed something, so a refresh is right there too.
            let changed = match res {
                Ok(DebouncedEventBatch::Events(events)) => !events.is_empty(),
                _ => true,
            };
            if changed {
                on_change();
            }
        })
        .map_err(err)?;
        debouncer.watcher().watch(path, RecursiveMode::NonRecursive).map_err(err)?;
        Ok(Self { path: path.to_path_buf(), _debouncer: debouncer })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc;

    #[test]
    fn reports_changes_once_per_burst() {
        let dir = tempfile::tempdir().unwrap();
        // Canonicalise: on macOS the temp dir is behind the /var -> /private/var symlink.
        let root = dir.path().canonicalize().unwrap();
        let (tx, rx) = mpsc::channel();
        let _w = DirWatcher::new(&root, Duration::from_millis(250), move || tx.send(()).unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(200)); // let FSEvents settle

        for i in 0..20 {
            fs::write(root.join(format!("f{i}")), "x").unwrap();
        }
        rx.recv_timeout(Duration::from_secs(5)).expect("a change notification");
        // The burst should collapse into a few callbacks, not one per write.
        // The exact count depends on how long the writes take relative to the
        // debounce window (slow CI containers spread them over several), so
        // only assert that coalescing clearly happened.
        std::thread::sleep(Duration::from_millis(800));
        let total = 1 + rx.try_iter().count();
        assert!(total < 10, "{total} callbacks for 20 writes");
    }
}
