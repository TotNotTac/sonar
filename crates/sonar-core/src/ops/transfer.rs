//! Copy and move with progress, cancellation and conflict prompts.
//!
//! Design notes:
//! - Moves try an atomic rename first and only fall back to copy+delete
//!   across filesystems.
//! - Files are written to a hidden `.name.sonar-part` sibling and renamed into
//!   place, so a cancelled or failed copy never leaves a truncated file under
//!   the real name, and "Replace" swaps atomically.
//! - A failing item is recorded and skipped; the rest of the job continues.
//!   A moved folder's source is only removed if everything inside it moved.

use std::collections::HashMap;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::names::keep_both_name;
use crate::error::CoreError;
use crate::platform;

const CHUNK: usize = 1 << 20;
const EMIT_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransferKind {
    Copy,
    Move,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictKind {
    /// File (or link) over file: Replace is possible.
    File,
    /// Folder onto folder: "Replace" means merge into it.
    Folder,
    /// File onto folder or vice versa: only Skip / Keep both.
    Mismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ConflictChoice {
    Skip,
    /// Overwrite a file, or merge into a folder.
    Replace,
    KeepBoth,
    /// Stop the whole job.
    Cancel,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemInfo {
    pub path: PathBuf,
    pub name: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub modified: Option<i64>,
}

impl ItemInfo {
    fn new(path: &Path, meta: &Metadata) -> Self {
        Self {
            path: path.to_path_buf(),
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            is_dir: meta.is_dir(),
            size: meta.is_file().then_some(meta.len()),
            modified: meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub kind: ConflictKind,
    pub source: ItemInfo,
    pub target: ItemInfo,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub total_bytes: u64,
    pub done_bytes: u64,
    pub total_items: u64,
    pub done_items: u64,
    /// Name of the item being processed.
    pub current: Option<String>,
}

pub trait TransferObserver {
    /// Called at most every ~100 ms, plus once at the start and end.
    fn progress(&mut self, progress: &Progress);
    /// Decide a conflict. Return `true` as the second value to apply the
    /// choice to all further conflicts of the same kind in this job.
    fn conflict(&mut self, conflict: &Conflict) -> (ConflictChoice, bool);
}

#[derive(Debug, Default)]
pub struct TransferOutcome {
    /// Copy: top-level items newly created at the destination.
    pub created: Vec<PathBuf>,
    /// Move: top-level `(from, to)` pairs that moved completely.
    pub moved: Vec<(PathBuf, PathBuf)>,
    pub errors: Vec<CoreError>,
    pub cancelled: bool,
    pub progress: Progress,
}

/// Copy or move `sources` into `dest_dir`. Blocking; check `cancel` to stop.
pub fn run_transfer(
    kind: TransferKind,
    sources: &[PathBuf],
    dest_dir: &Path,
    cancel: &AtomicBool,
    observer: &mut dyn TransferObserver,
) -> TransferOutcome {
    let mut t = Transfer {
        kind,
        cancel,
        observer,
        last_emit: Instant::now(),
        remembered: HashMap::new(),
        out: TransferOutcome::default(),
    };

    match fs::metadata(dest_dir) {
        Ok(m) if m.is_dir() => {}
        Ok(_) => return t.abort(CoreError::NotADirectory(dest_dir.to_path_buf())),
        Err(e) => return t.abort(CoreError::from_io(dest_dir, e)),
    }

    for src in sources {
        let (bytes, items) = tree_size(src);
        t.out.progress.total_bytes += bytes;
        t.out.progress.total_items += items;
    }
    t.emit(true);

    for src in sources {
        if t.cancelled() {
            break;
        }
        let Some(name) = src.file_name() else {
            t.fail(CoreError::InvalidPath(src.display().to_string()));
            continue;
        };
        let src_is_dir = fs::symlink_metadata(src).map(|m| m.is_dir()).unwrap_or(false);
        if src_is_dir && dest_dir.starts_with(src) {
            let verb = if kind == TransferKind::Copy { "copy" } else { "move" };
            t.fail(CoreError::Other(format!("Cannot {verb} \"{}\" into itself", name.to_string_lossy())));
            t.skip(src);
            continue;
        }
        let same_dir = src.parent() == Some(dest_dir);
        let dst = match (same_dir, kind) {
            (true, TransferKind::Move) => {
                t.skip(src); // moving onto itself: nothing to do
                continue;
            }
            // Copying into the same folder makes a numbered duplicate.
            (true, TransferKind::Copy) => keep_both_name(dest_dir, &name.to_string_lossy(), src_is_dir),
            (false, _) => dest_dir.join(name),
        };
        t.item(src, dst, true);
    }

    t.out.cancelled = t.cancelled();
    t.out.progress.current = None;
    t.emit(true);
    t.out
}

struct Transfer<'a> {
    kind: TransferKind,
    cancel: &'a AtomicBool,
    observer: &'a mut dyn TransferObserver,
    last_emit: Instant,
    remembered: HashMap<ConflictKind, ConflictChoice>,
    out: TransferOutcome,
}

/// How an item at the destination is to be written.
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Fresh,
    Replace,
    Merge,
}

impl Transfer<'_> {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    fn abort(mut self, err: CoreError) -> TransferOutcome {
        self.out.errors.push(err);
        self.out
    }

    fn fail(&mut self, err: CoreError) {
        self.out.errors.push(err);
    }

    fn emit(&mut self, force: bool) {
        if force || self.last_emit.elapsed() >= EMIT_EVERY {
            self.observer.progress(&self.out.progress);
            self.last_emit = Instant::now();
        }
    }

    fn advance(&mut self, bytes: u64, items: u64) {
        self.out.progress.done_bytes += bytes;
        self.out.progress.done_items += items;
        self.emit(false);
    }

    /// Count a subtree as done without transferring it.
    fn skip(&mut self, src: &Path) {
        let (b, n) = tree_size(src);
        self.advance(b, n);
    }

    /// Transfer one item (recursively). Returns true if it fully completed.
    fn item(&mut self, src: &Path, mut dst: PathBuf, top_level: bool) -> bool {
        if self.cancelled() {
            return false;
        }
        let meta = match fs::symlink_metadata(src) {
            Ok(m) => m,
            Err(e) => {
                self.fail(CoreError::from_io(src, e));
                return false;
            }
        };
        self.out.progress.current = src.file_name().map(|n| n.to_string_lossy().into_owned());

        let mut mode = Mode::Fresh;
        if let Ok(dmeta) = fs::symlink_metadata(&dst) {
            let kind = match (meta.is_dir(), dmeta.is_dir()) {
                (true, true) => ConflictKind::Folder,
                (false, false) => ConflictKind::File,
                _ => ConflictKind::Mismatch,
            };
            let choice = self.decide(kind, src, &meta, &dst, &dmeta);
            match (choice, kind) {
                (ConflictChoice::Cancel, _) => {
                    self.cancel.store(true, Ordering::Relaxed);
                    return false;
                }
                (ConflictChoice::Skip, _) => {
                    self.skip(src);
                    return false;
                }
                (ConflictChoice::Replace, ConflictKind::Folder) => mode = Mode::Merge,
                (ConflictChoice::Replace, ConflictKind::File) => mode = Mode::Replace,
                // Replacing a folder with a file (or back) is too destructive
                // to do implicitly; keep both instead.
                (ConflictChoice::Replace, ConflictKind::Mismatch) | (ConflictChoice::KeepBoth, _) => {
                    let parent = dst.parent().unwrap_or(Path::new("/")).to_path_buf();
                    let name = dst.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    dst = keep_both_name(&parent, &name, meta.is_dir());
                }
            }
        }

        let ok = if meta.is_dir() { self.dir(src, &dst, &meta, mode) } else { self.leaf(src, &dst, &meta, mode) };

        // Only record what undo can cleanly reverse: new items, not overwrites or merges.
        if ok && top_level && mode == Mode::Fresh {
            match self.kind {
                TransferKind::Copy => self.out.created.push(dst),
                TransferKind::Move => self.out.moved.push((src.to_path_buf(), dst)),
            }
        }
        ok
    }

    fn decide(&mut self, kind: ConflictKind, src: &Path, sm: &Metadata, dst: &Path, dm: &Metadata) -> ConflictChoice {
        if let Some(&c) = self.remembered.get(&kind) {
            return c;
        }
        self.emit(true);
        let conflict = Conflict { kind, source: ItemInfo::new(src, sm), target: ItemInfo::new(dst, dm) };
        let (choice, apply_to_all) = self.observer.conflict(&conflict);
        if apply_to_all && choice != ConflictChoice::Cancel {
            self.remembered.insert(kind, choice);
        }
        self.last_emit = Instant::now(); // don't count time spent waiting on the user
        choice
    }

    fn dir(&mut self, src: &Path, dst: &Path, meta: &Metadata, mode: Mode) -> bool {
        if self.kind == TransferKind::Move && mode == Mode::Fresh {
            match platform::rename_noreplace(src, dst) {
                Ok(()) => {
                    let (b, n) = tree_size(dst);
                    self.advance(b, n);
                    return true;
                }
                Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {} // copy + delete below
                Err(e) => {
                    self.fail(CoreError::from_io(src, e));
                    return false;
                }
            }
        }

        if mode == Mode::Fresh {
            if let Err(e) = fs::create_dir(dst) {
                self.fail(CoreError::from_io(dst, e));
                self.skip(src);
                return false;
            }
        }
        self.advance(0, 1);

        let children: Vec<PathBuf> = match fs::read_dir(src) {
            Ok(rd) => rd.filter_map(|e| e.ok().map(|e| e.path())).collect(),
            Err(e) => {
                self.fail(CoreError::from_io(src, e));
                return false;
            }
        };
        let mut complete = true;
        for child in children {
            if self.cancelled() {
                return false;
            }
            let name = child.file_name().unwrap_or_default().to_owned();
            complete &= self.item(&child, dst.join(name), false);
        }

        // Apply the folder's own metadata last: a read-only source folder
        // would otherwise block writing its children.
        if mode == Mode::Fresh {
            let _ = fs::set_permissions(dst, meta.permissions());
            if let (Ok(f), Ok(t)) = (File::open(dst), meta.modified()) {
                let _ = f.set_modified(t);
            }
        }

        if self.kind == TransferKind::Move && complete {
            if let Err(e) = fs::remove_dir(src) {
                self.fail(CoreError::from_io(src, e));
                complete = false;
            }
        }
        complete
    }

    fn leaf(&mut self, src: &Path, dst: &Path, meta: &Metadata, mode: Mode) -> bool {
        if self.kind == TransferKind::Move {
            let res = if mode == Mode::Replace { fs::rename(src, dst) } else { platform::rename_noreplace(src, dst) };
            match res {
                Ok(()) => {
                    self.advance(meta.len(), 1);
                    return true;
                }
                Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {}
                Err(e) => {
                    self.fail(CoreError::from_io(src, e));
                    return false;
                }
            }
        }

        let ft = meta.file_type();
        let result = if ft.is_symlink() {
            copy_symlink(src, dst, mode).map(|_| true)
        } else if ft.is_file() {
            self.copy_file(src, dst, meta, mode)
        } else {
            Err(io::Error::new(io::ErrorKind::Unsupported, "special files (sockets, pipes, devices) are skipped"))
        };

        match result {
            Ok(true) => {}
            Ok(false) => return false, // cancelled mid-file
            Err(e) => {
                self.fail(CoreError::from_io(src, e));
                return false;
            }
        }
        if self.kind == TransferKind::Move {
            if let Err(e) = fs::remove_file(src) {
                self.fail(CoreError::from_io(src, e));
                return false;
            }
        }
        self.advance(0, 1);
        true
    }

    /// Returns `Ok(false)` if cancelled. Byte progress is reported as it goes.
    fn copy_file(&mut self, src: &Path, dst: &Path, meta: &Metadata, mode: Mode) -> io::Result<bool> {
        let part = part_path(dst);
        let _ = fs::remove_file(&part);

        let cleanup = |e: io::Error| {
            let _ = fs::remove_file(&part);
            e
        };

        if platform::clone_file(src, &part).unwrap_or(false) {
            self.advance(meta.len(), 0);
        } else {
            let mut input = File::open(src)?;
            let mut output = OpenOptions::new().write(true).create_new(true).open(&part)?;
            let mut buf = vec![0u8; CHUNK];
            loop {
                if self.cancelled() {
                    drop(output);
                    let _ = fs::remove_file(&part);
                    return Ok(false);
                }
                let n = input.read(&mut buf).map_err(cleanup)?;
                if n == 0 {
                    break;
                }
                output.write_all(&buf[..n]).map_err(cleanup)?;
                self.advance(n as u64, 0);
            }
            output.set_permissions(meta.permissions()).map_err(cleanup)?;
            if let Ok(t) = meta.modified() {
                let _ = output.set_modified(t);
            }
            output.sync_all().map_err(cleanup)?;
        }

        let res = if mode == Mode::Replace { fs::rename(&part, dst) } else { platform::rename_noreplace(&part, dst) };
        res.map_err(cleanup)?;
        Ok(true)
    }
}

fn copy_symlink(src: &Path, dst: &Path, mode: Mode) -> io::Result<()> {
    let target = fs::read_link(src)?;
    if mode == Mode::Replace {
        fs::remove_file(dst)?;
    }
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, dst);
    #[cfg(not(unix))]
    return Err(io::Error::new(io::ErrorKind::Unsupported, format!("cannot copy symlink to {}", target.display())));
}

fn part_path(dst: &Path) -> PathBuf {
    let name = dst.file_name().unwrap_or_default().to_string_lossy();
    dst.with_file_name(format!(".{name}.sonar-part"))
}

/// Total bytes of regular files and number of entries under `path`
/// (including itself). Symlinks are not followed. Unreadable parts are ignored.
fn tree_size(path: &Path) -> (u64, u64) {
    let (mut bytes, mut items) = (0, 0);
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        let Ok(meta) = fs::symlink_metadata(&p) else { continue };
        items += 1;
        if meta.is_dir() {
            if let Ok(rd) = fs::read_dir(&p) {
                stack.extend(rd.filter_map(|e| e.ok().map(|e| e.path())));
            }
        } else if meta.is_file() {
            bytes += meta.len();
        }
    }
    (bytes, items)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Answers every conflict with a fixed choice and records what happened.
    struct Scripted {
        answer: (ConflictChoice, bool),
        conflicts: Vec<Conflict>,
        last: Progress,
    }

    impl Scripted {
        fn new(choice: ConflictChoice, all: bool) -> Self {
            Self { answer: (choice, all), conflicts: vec![], last: Progress::default() }
        }
    }

    impl TransferObserver for Scripted {
        fn progress(&mut self, p: &Progress) {
            self.last = p.clone();
        }
        fn conflict(&mut self, c: &Conflict) -> (ConflictChoice, bool) {
            self.conflicts.push(c.clone());
            self.answer
        }
    }

    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let (src, dst) = (d.path().join("src"), d.path().join("dst"));
        fs::create_dir_all(src.join("folder/nested")).unwrap();
        fs::write(src.join("a.txt"), "aaaa").unwrap();
        fs::write(src.join("folder/b.txt"), "bb").unwrap();
        fs::write(src.join("folder/nested/c.txt"), "c").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("a.txt", src.join("link")).unwrap();
        fs::create_dir(&dst).unwrap();
        (d, src, dst)
    }

    fn run(kind: TransferKind, srcs: &[PathBuf], dst: &Path, obs: &mut Scripted) -> TransferOutcome {
        run_transfer(kind, srcs, dst, &AtomicBool::new(false), obs)
    }

    #[test]
    fn copy_tree_with_progress() {
        let (_d, src, dst) = setup();
        let mut obs = Scripted::new(ConflictChoice::Skip, false);
        let srcs = [src.join("a.txt"), src.join("folder"), src.join("link")];
        let out = run(TransferKind::Copy, &srcs, &dst, &mut obs);
        assert!(out.errors.is_empty(), "{:?}", out.errors);
        assert_eq!(fs::read_to_string(dst.join("folder/nested/c.txt")).unwrap(), "c");
        assert_eq!(fs::read_link(dst.join("link")).unwrap(), Path::new("a.txt"));
        assert_eq!(out.created.len(), 3);
        assert_eq!(obs.last.done_bytes, 7);
        assert_eq!(obs.last.done_items, obs.last.total_items);
        assert!(src.join("folder/b.txt").exists(), "copy leaves source intact");
        let leftovers: Vec<_> = fs::read_dir(&dst)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".sonar-part"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn move_tree() {
        let (_d, src, dst) = setup();
        let mut obs = Scripted::new(ConflictChoice::Skip, false);
        let out = run(TransferKind::Move, &[src.join("folder")], &dst, &mut obs);
        assert!(out.errors.is_empty());
        assert!(!src.join("folder").exists());
        assert!(dst.join("folder/nested/c.txt").exists());
        assert_eq!(out.moved, [(src.join("folder"), dst.join("folder"))]);
    }

    #[test]
    fn copy_into_same_folder_duplicates() {
        let (_d, src, _dst) = setup();
        let mut obs = Scripted::new(ConflictChoice::Skip, false);
        let out = run(TransferKind::Copy, &[src.join("a.txt")], &src, &mut obs);
        assert_eq!(out.created, [src.join("a (1).txt")]);
        assert!(obs.conflicts.is_empty());
    }

    #[test]
    fn refuses_folder_into_itself() {
        let (_d, src, _dst) = setup();
        let mut obs = Scripted::new(ConflictChoice::Skip, false);
        let out = run(TransferKind::Copy, &[src.join("folder")], &src.join("folder/nested"), &mut obs);
        assert_eq!(out.errors.len(), 1);
        assert!(!src.join("folder/nested/folder").exists());
    }

    #[test]
    fn conflicts() {
        let (_d, src, dst) = setup();
        fs::write(dst.join("a.txt"), "old").unwrap();

        // Skip keeps the existing file.
        let mut obs = Scripted::new(ConflictChoice::Skip, false);
        run(TransferKind::Copy, &[src.join("a.txt")], &dst, &mut obs);
        assert_eq!(obs.conflicts.len(), 1);
        assert_eq!(obs.conflicts[0].kind, ConflictKind::File);
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "old");

        // Keep both.
        let mut obs = Scripted::new(ConflictChoice::KeepBoth, false);
        let out = run(TransferKind::Copy, &[src.join("a.txt")], &dst, &mut obs);
        assert_eq!(out.created, [dst.join("a (1).txt")]);

        // Replace overwrites but is not recorded for undo.
        let mut obs = Scripted::new(ConflictChoice::Replace, false);
        let out = run(TransferKind::Copy, &[src.join("a.txt")], &dst, &mut obs);
        assert_eq!(fs::read_to_string(dst.join("a.txt")).unwrap(), "aaaa");
        assert!(out.created.is_empty());
    }

    #[test]
    fn merge_folders_and_apply_to_all() {
        let (_d, src, dst) = setup();
        fs::create_dir_all(dst.join("folder/nested")).unwrap();
        fs::write(dst.join("folder/b.txt"), "old").unwrap();
        fs::write(dst.join("folder/nested/c.txt"), "old").unwrap();
        fs::write(dst.join("folder/keep.txt"), "mine").unwrap();

        // "Replace" + apply to all: the first Folder conflict merges, and File
        // conflicts are asked once and then remembered.
        let mut obs = Scripted::new(ConflictChoice::Replace, true);
        let out = run(TransferKind::Copy, &[src.join("folder")], &dst, &mut obs);
        assert!(out.errors.is_empty());
        let kinds: Vec<_> = obs.conflicts.iter().map(|c| c.kind).collect();
        assert_eq!(kinds, [ConflictKind::Folder, ConflictKind::File]);
        assert_eq!(fs::read_to_string(dst.join("folder/b.txt")).unwrap(), "bb");
        assert_eq!(fs::read_to_string(dst.join("folder/nested/c.txt")).unwrap(), "c");
        assert_eq!(fs::read_to_string(dst.join("folder/keep.txt")).unwrap(), "mine");
    }

    #[test]
    fn cancel_via_conflict() {
        let (_d, src, dst) = setup();
        fs::write(dst.join("a.txt"), "old").unwrap();
        let mut obs = Scripted::new(ConflictChoice::Cancel, false);
        let out = run(TransferKind::Copy, &[src.join("a.txt"), src.join("folder")], &dst, &mut obs);
        assert!(out.cancelled);
        assert!(!dst.join("folder").exists(), "nothing after the cancel point runs");
    }

    #[test]
    fn partial_move_keeps_source_folder() {
        let (_d, src, dst) = setup();
        fs::create_dir_all(dst.join("folder")).unwrap();
        fs::write(dst.join("folder/b.txt"), "old").unwrap();
        // Merge the folder, but skip the conflicting file.
        struct MergeThenSkip;
        impl TransferObserver for MergeThenSkip {
            fn progress(&mut self, _: &Progress) {}
            fn conflict(&mut self, c: &Conflict) -> (ConflictChoice, bool) {
                (if c.kind == ConflictKind::Folder { ConflictChoice::Replace } else { ConflictChoice::Skip }, false)
            }
        }
        let out =
            run_transfer(TransferKind::Move, &[src.join("folder")], &dst, &AtomicBool::new(false), &mut MergeThenSkip);
        assert!(out.errors.is_empty());
        assert!(src.join("folder/b.txt").exists(), "skipped file stays behind");
        assert!(!src.join("folder/nested").exists(), "the rest moved");
        assert!(dst.join("folder/nested/c.txt").exists());
    }
}
