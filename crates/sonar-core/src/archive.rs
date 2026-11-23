//! Archives (zip, 7z, rar, tar.gz/xz/bz2/zst, iso, ...) via `bsdtar`
//! (libarchive). It ships with macOS and is always installed on Arch
//! (pacman depends on libarchive), and it reads RAR, which has no free Rust
//! implementation.
//!
//! - [`list`]: contents, for browsing an archive like a folder.
//! - [`extract`]: all of it ("Extract Here", with Ark-style subfolder
//!   detection) or selected entries (copying out of a browsed archive).
//! - [`compress`]: files and folders into zip, 7z or tar.gz/xz/bz2.
//!
//! Everything is written to a hidden temporary path first and moved into
//! place without overwriting, so cancelling or failing never leaves partial
//! results or clobbers existing files.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::{CoreError, Result};
use crate::ops::available_name;

const BSDTAR: &str = "bsdtar";

/// Multi-part extensions first, so "x.tar.gz" isn't mistaken for "x.gz".
const EXTENSIONS: &[&str] = &[
    ".tar.gz", ".tar.xz", ".tar.bz2", ".tar.zst", ".tar.lz", ".tgz", ".txz", ".tbz", ".tbz2", ".tzst", ".tar", ".zip",
    ".jar", ".7z", ".rar", ".iso", ".cpio", ".xar", ".cab",
];

/// Whether `name` looks like an archive Sonar can open.
pub fn is_archive(name: &str) -> bool {
    archive_suffix(name).is_some()
}

fn archive_suffix(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase();
    EXTENSIONS.iter().copied().find(|ext| lower.len() > ext.len() && lower.ends_with(ext))
}

/// "photos.tar.gz" -> "photos": the folder name for "Extract Here".
pub fn stem(name: &str) -> &str {
    match archive_suffix(name) {
        Some(ext) => &name[..name.len() - ext.len()],
        None => name,
    }
}

/// Formats Sonar can create.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Format {
    Zip,
    SevenZip,
    TarGz,
    TarXz,
    TarBz2,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Zip => "zip",
            Format::SevenZip => "7z",
            Format::TarGz => "tar.gz",
            Format::TarXz => "tar.xz",
            Format::TarBz2 => "tar.bz2",
        }
    }

    fn bsdtar_args(self) -> &'static [&'static str] {
        match self {
            Format::Zip => &["--format", "zip"],
            Format::SevenZip => &["--format", "7zip"],
            // Default tar format is "restricted pax": plain ustar unless long
            // or non-ASCII names need pax headers.
            Format::TarGz => &["-z"],
            Format::TarXz => &["-J"],
            Format::TarBz2 => &["-j"],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveEntry {
    /// Path inside the archive, without leading "./" or trailing "/".
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    /// Milliseconds since the Unix epoch (minute precision; local time read as UTC).
    pub modified: Option<i64>,
}

fn bsdtar() -> Command {
    let mut cmd = Command::new(BSDTAR);
    // Without a UTF-8 locale bsdtar escapes non-ASCII names ("\314\210").
    cmd.env("LC_ALL", "C.UTF-8").stdin(Stdio::null());
    cmd
}

fn tool_error(archive: &Path, verb: &str, stderr: &str) -> CoreError {
    let detail = stderr
        .lines()
        .map(|l| l.trim().trim_start_matches("bsdtar: ").to_string())
        .rfind(|l| !l.is_empty() && !l.starts_with("x ") && !l.starts_with("a "))
        .unwrap_or_else(|| "the archive could not be read".into());
    CoreError::Other(format!("Could not {verb} \"{}\": {detail}", file_name(archive)))
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

fn spawn_error(e: std::io::Error) -> CoreError {
    if e.kind() == std::io::ErrorKind::NotFound {
        CoreError::Other(
            "Archive support needs bsdtar (libarchive). On Debian/Ubuntu: apt install libarchive-tools".into(),
        )
    } else {
        CoreError::Other(format!("Could not run bsdtar: {e}"))
    }
}

// ---------- Listing ----------

/// Everything in the archive, with the directory entries archives often
/// leave out filled in.
pub fn list(archive: &Path) -> Result<Vec<ArchiveEntry>> {
    let out = bsdtar().arg("-tvf").arg(archive).output().map_err(spawn_error)?;
    if !out.status.success() {
        return Err(tool_error(archive, "open", &String::from_utf8_lossy(&out.stderr)));
    }
    Ok(complete_dirs(String::from_utf8_lossy(&out.stdout).lines().filter_map(parse_list_line).collect()))
}

/// Parse one `bsdtar -tv` line (ls -l style):
/// `-rw-r--r--  0 501    20          3 Nov 23 15:06 ./sub/b ü.txt`
fn parse_list_line(line: &str) -> Option<ArchiveEntry> {
    // The name is everything after the 8th field, spaces included.
    let mut fields = Vec::with_capacity(8);
    let mut rest = line;
    for _ in 0..8 {
        rest = rest.trim_start();
        let end = rest.find(char::is_whitespace)?;
        fields.push(&rest[..end]);
        rest = &rest[end..];
    }
    let mut name = rest.strip_prefix(' ')?.to_string();
    let mode = fields[0];
    if mode.starts_with('l') {
        if let Some(i) = name.find(" -> ") {
            name.truncate(i);
        }
    } else if let Some(i) = name.find(" link to ") {
        name.truncate(i); // hard links
    }
    let is_dir = mode.starts_with('d') || name.ends_with('/');
    let path = normalize(&name)?;
    Some(ArchiveEntry {
        path,
        is_dir,
        size: if is_dir { 0 } else { fields[4].parse().unwrap_or(0) },
        modified: parse_ls_date(fields[5], fields[6], fields[7]),
    })
}

fn normalize(name: &str) -> Option<String> {
    let trimmed = name.trim_start_matches("./").trim_start_matches('/').trim_end_matches('/');
    let parts: Vec<&str> = trimmed.split('/').filter(|p| !p.is_empty() && *p != ".").collect();
    if parts.is_empty() || parts.contains(&"..") {
        return None; // the root itself, or a path bsdtar would refuse to extract anyway
    }
    Some(parts.join("/"))
}

/// "Nov 23 15:06" (this year) or "Nov 23  2021".
fn parse_ls_date(month: &str, day: &str, time_or_year: &str) -> Option<i64> {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let m = MONTHS.iter().position(|x| *x == month)? as i64 + 1;
    let d: i64 = day.parse().ok()?;
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_millis() as i64;
    let (year, h, min) = match time_or_year.split_once(':') {
        Some((h, mi)) => (current_year(now_ms), h.parse().ok()?, mi.parse().ok()?),
        None => (time_or_year.parse().ok()?, 0, 0),
    };
    let ms = |y: i64| ((days_from_civil(y, m, d) * 24 + h) * 60 + min) * 60_000;
    // ls shows times for the last ~6 months; a date "in the future" was last year.
    let t = ms(year);
    Some(if time_or_year.contains(':') && t > now_ms + 86_400_000 { ms(year - 1) } else { t })
}

fn current_year(now_ms: i64) -> i64 {
    let mut y = 1970 + now_ms / 31_556_952_000;
    while days_from_civil(y + 1, 1, 1) * 86_400_000 <= now_ms {
        y += 1;
    }
    while days_from_civil(y, 1, 1) * 86_400_000 > now_ms {
        y -= 1;
    }
    y
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Add parent directories that only exist implicitly, and drop duplicates.
fn complete_dirs(entries: Vec<ArchiveEntry>) -> Vec<ArchiveEntry> {
    let mut map: BTreeMap<String, ArchiveEntry> = BTreeMap::new();
    for e in entries {
        let mut parent = e.path.as_str();
        while let Some((p, _)) = parent.rsplit_once('/') {
            map.entry(p.to_string()).or_insert_with(|| ArchiveEntry {
                path: p.to_string(),
                is_dir: true,
                size: 0,
                modified: e.modified,
            });
            parent = p;
        }
        map.insert(e.path.clone(), e);
    }
    map.into_values().collect()
}

/// Direct children of `dir` ("" for the root) within a full listing.
pub fn children<'a>(entries: &'a [ArchiveEntry], dir: &str) -> impl Iterator<Item = &'a ArchiveEntry> {
    let prefix = if dir.is_empty() { String::new() } else { format!("{dir}/") };
    entries.iter().filter(move |e| {
        e.path.strip_prefix(prefix.as_str()).is_some_and(|rest| !rest.is_empty() && !rest.contains('/'))
    })
}

// ---------- Paths inside archives ----------

/// Split a path that points *inside* an archive (`/x/photos.zip/2025/a.jpg`)
/// into the archive file and the inner path (`2025/a.jpg`). An archive file
/// itself gives an empty inner path. `None` for ordinary paths.
pub fn split_virtual(path: &Path) -> Option<(PathBuf, String)> {
    if fs::metadata(path).is_ok_and(|m| m.is_dir()) {
        return None;
    }
    let mut archive = path;
    let mut inner: Vec<String> = Vec::new();
    loop {
        if let Ok(meta) = fs::metadata(archive) {
            let name = archive.file_name()?.to_string_lossy();
            if meta.is_file() && is_archive(&name) {
                inner.reverse();
                return Some((archive.to_path_buf(), inner.join("/")));
            }
            return None; // an existing non-archive: an ordinary (missing) path
        }
        inner.push(archive.file_name()?.to_string_lossy().into_owned());
        archive = archive.parent()?;
    }
}

/// Listings are cached per archive (path + size + mtime), so moving around
/// inside a big archive doesn't re-read it every time.
fn cached_list(archive: &Path) -> Result<std::sync::Arc<Vec<ArchiveEntry>>> {
    use std::sync::{Arc, Mutex, OnceLock};
    type Key = (PathBuf, u64, Option<SystemTime>);
    type Cache = Mutex<Vec<(Key, Arc<Vec<ArchiveEntry>>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let meta = fs::metadata(archive).map_err(|e| CoreError::from_io(archive, e))?;
    let key: Key = (archive.to_path_buf(), meta.len(), meta.modified().ok());
    let cache = CACHE.get_or_init(Default::default);
    if let Some((_, hit)) = cache.lock().unwrap_or_else(|p| p.into_inner()).iter().find(|(k, _)| *k == key) {
        return Ok(hit.clone());
    }
    let entries = Arc::new(list(archive)?);
    let mut c = cache.lock().unwrap_or_else(|p| p.into_inner());
    c.retain(|(k, _)| k.0 != key.0);
    c.push((key, entries.clone()));
    if c.len() > 8 {
        c.remove(0);
    }
    Ok(entries)
}

/// A folder listing for `inner` inside `archive` (read-only). Paths of the
/// entries continue the archive's path, so they can be navigated into.
pub fn listing(archive: &Path, inner: &str, opts: &crate::ListOptions) -> Result<crate::Listing> {
    use crate::entry::{Entry, EntryKind};
    let all = cached_list(archive)?;
    let here = if inner.is_empty() { archive.to_path_buf() } else { archive.join(inner) };
    if !inner.is_empty() && !all.iter().any(|e| e.path == inner && e.is_dir) {
        return Err(CoreError::NotFound(here));
    }
    let mut entries = Vec::new();
    let mut hidden_count = 0;
    for e in children(&all, inner) {
        let name = e.path.rsplit('/').next().unwrap_or(&e.path).to_string();
        let kind = if e.is_dir { EntryKind::Directory } else { EntryKind::File };
        let path = here.join(&name);
        let ft = crate::filetype::describe_native(&name, &path, kind);
        let is_hidden = name.starts_with('.');
        if is_hidden && !opts.show_hidden {
            hidden_count += 1;
            continue;
        }
        entries.push(Entry {
            child_count: e.is_dir.then(|| children(&all, &e.path).count() as u64),
            size: (!e.is_dir).then_some(e.size),
            modified: e.modified,
            type_label: ft.label,
            icon: ft.icon,
            type_id: ft.type_id,
            icon_key: ft.icon_key,
            thumbnail: false,
            archive: false,
            is_symlink: false,
            is_hidden,
            readonly: true,
            kind,
            path,
            name,
        });
    }
    crate::sort::sort_entries(&mut entries, opts.sort);
    Ok(crate::Listing {
        parent: here.parent().map(Path::to_path_buf),
        path: here,
        entries,
        hidden_count,
        skipped_count: 0,
    })
}

/// Extract one file from an archive into a private temporary folder (to open
/// it with its app), returning the extracted path.
pub fn extract_for_opening(archive: &Path, inner: &str) -> Result<PathBuf> {
    let (dir, name) = inner.rsplit_once('/').unwrap_or(("", inner));
    let base = std::env::temp_dir().join("sonar-open").join(format!("{}", std::process::id()));
    fs::create_dir_all(&base).map_err(|e| CoreError::from_io(&base, e))?;
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dest = base.join(n.to_string());
    fs::create_dir(&dest).map_err(|e| CoreError::from_io(&dest, e))?;
    let names = [name.to_string()];
    let created =
        extract(archive, &dest, Some((dir, &names)), &AtomicBool::new(false), &mut |_, _, _| {})?.unwrap_or_default();
    created.into_iter().next().ok_or_else(|| CoreError::NotFound(archive.join(inner)))
}

// ---------- Extract / compress ----------

/// Progress through a job: entries done of total, and the current entry.
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64, &str);

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_dir_in(dir: &Path) -> Result<PathBuf> {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = dir.join(format!(".sonar-extract-{}-{n}", std::process::id()));
    fs::create_dir(&tmp).map_err(|e| CoreError::from_io(dir, e))?;
    Ok(tmp)
}

/// Run bsdtar, counting its `-v` lines (on stderr) as progress. Returns
/// stderr for error reporting, or `None` if cancelled.
fn run_with_progress(mut cmd: Command, total: u64, cancel: &AtomicBool, progress: Progress) -> Result<Option<String>> {
    let mut child: Child = cmd.stdout(Stdio::null()).stderr(Stdio::piped()).spawn().map_err(spawn_error)?;
    let stderr = child.stderr.take().expect("piped");
    let mut log = String::new();
    let mut done = 0;
    for line in BufReader::new(stderr).lines().map_while(std::result::Result::ok) {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        if let Some(name) = line.strip_prefix("x ").or_else(|| line.strip_prefix("a ")) {
            done += 1;
            progress(done.min(total), total, name.trim_start_matches("./"));
        } else {
            log.push_str(&line);
            log.push('\n');
        }
    }
    let status = child.wait().map_err(|e| CoreError::Other(format!("bsdtar failed: {e}")))?;
    if cancel.load(Ordering::Relaxed) {
        return Ok(None);
    }
    if !status.success() {
        return Err(CoreError::Other(log)); // turned into a readable message by the caller
    }
    Ok(Some(log))
}

/// Extract into `dest_dir` and return the top-level items created.
///
/// - `select = None`: the whole archive, Ark-style: a single top-level item
///   lands directly in `dest_dir`; several land in a new folder named after
///   the archive.
/// - `select = Some((dir, names))`: only those entries of the archive folder
///   `dir`, each placed directly in `dest_dir`.
///
/// Name clashes get "name (1)" instead of overwriting. Returns `Ok(None)` if
/// cancelled (nothing is left behind).
pub fn extract(
    archive: &Path,
    dest_dir: &Path,
    select: Option<(&str, &[String])>,
    cancel: &AtomicBool,
    progress: Progress,
) -> Result<Option<Vec<PathBuf>>> {
    let entries = list(archive)?;
    let patterns: Vec<String> = match select {
        Some((dir, names)) => {
            names.iter().map(|n| if dir.is_empty() { n.clone() } else { format!("{dir}/{n}") }).collect()
        }
        None => Vec::new(),
    };
    let total = if patterns.is_empty() {
        entries.len() as u64
    } else {
        entries.iter().filter(|e| patterns.iter().any(|p| e.path == *p || e.path.starts_with(&format!("{p}/")))).count()
            as u64
    };

    let tmp = temp_dir_in(dest_dir)?;
    let cleanup = |r| {
        let _ = fs::remove_dir_all(&tmp);
        r
    };
    let mut cmd = bsdtar();
    cmd.arg("-xvf").arg(archive).arg("-C").arg(&tmp).args(&patterns);
    match run_with_progress(cmd, total, cancel, progress) {
        Ok(Some(_)) => {}
        Ok(None) => return cleanup(Ok(None)),
        Err(CoreError::Other(log)) => return cleanup(Err(tool_error(archive, "extract", &log))),
        Err(e) => return cleanup(Err(e)),
    }

    let mut created = Vec::new();
    let mut place = |src: &Path, name: &str| -> Result<()> {
        let dst = available_name(dest_dir, name, src.is_dir());
        crate::platform::rename_noreplace(src, &dst).map_err(|e| CoreError::from_io(&dst, e))?;
        created.push(dst);
        Ok(())
    };
    let result = match select {
        Some((dir, names)) => names.iter().try_for_each(|n| {
            let src = tmp.join(dir).join(n);
            if fs::symlink_metadata(&src).is_ok() {
                place(&src, n)
            } else {
                Ok(())
            }
        }),
        None => {
            let roots: Vec<_> = fs::read_dir(&tmp).map_err(|e| CoreError::from_io(&tmp, e))?.flatten().collect();
            match roots.as_slice() {
                [only] => place(&only.path(), &only.file_name().to_string_lossy()),
                _ => {
                    let folder = stem(&file_name(archive)).to_string();
                    place(&tmp, &folder)
                }
            }
        }
    };
    cleanup(result.map(|()| Some(created)))
}

/// Compress `sources` (siblings in one folder) into `dest`, which must not
/// exist yet. Returns `Ok(None)` if cancelled.
pub fn compress(
    sources: &[PathBuf],
    dest: &Path,
    format: Format,
    cancel: &AtomicBool,
    progress: Progress,
) -> Result<Option<PathBuf>> {
    let parent =
        sources.first().and_then(|s| s.parent()).ok_or_else(|| CoreError::Other("Nothing to compress".into()))?;
    if sources.iter().any(|s| s.parent() != Some(parent)) {
        return Err(CoreError::Other("Items to compress must be in the same folder".into()));
    }
    let total: u64 = sources.iter().map(|s| count_items(s)).sum();
    let part = dest.with_file_name(format!(".{}.sonar-part", file_name(dest)));
    let _ = fs::remove_file(&part);

    let mut cmd = bsdtar();
    cmd.arg("-cvf").arg(&part).args(format.bsdtar_args());
    if cfg!(target_os = "macos") {
        cmd.arg("--no-mac-metadata"); // no "._file" AppleDouble entries
    }
    cmd.arg("-C").arg(parent).arg("--");
    for s in sources {
        cmd.arg(s.file_name().ok_or_else(|| CoreError::InvalidPath(s.display().to_string()))?);
    }
    let cleanup = |r| {
        let _ = fs::remove_file(&part);
        r
    };
    match run_with_progress(cmd, total, cancel, progress) {
        Ok(Some(_)) => {}
        Ok(None) => return cleanup(Ok(None)),
        Err(CoreError::Other(log)) => return cleanup(Err(tool_error(dest, "create", &log))),
        Err(e) => return cleanup(Err(e)),
    }
    match crate::platform::rename_noreplace(&part, dest) {
        Ok(()) => Ok(Some(dest.to_path_buf())),
        Err(e) => cleanup(Err(CoreError::from_io(dest, e))),
    }
}

fn count_items(path: &Path) -> u64 {
    let mut n = 0;
    let mut stack = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        n += 1;
        if fs::symlink_metadata(&p).is_ok_and(|m| m.is_dir()) {
            if let Ok(rd) = fs::read_dir(&p) {
                stack.extend(rd.flatten().map(|e| e.path()));
            }
        }
    }
    n
}

/// Default archive name for compressing `sources`: the item's own name for a
/// single item, otherwise "Archive", plus the format's extension.
pub fn default_name(sources: &[PathBuf], format: Format) -> String {
    let base = match sources {
        [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Archive".into()),
        _ => "Archive".into(),
    };
    format!("{base}.{}", format.extension())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_bsdtar() -> bool {
        Command::new(BSDTAR).arg("--version").output().is_ok_and(|o| o.status.success())
    }

    fn noop() -> impl FnMut(u64, u64, &str) {
        |_, _, _| {}
    }

    #[test]
    fn names() {
        assert!(is_archive("a.ZIP") && is_archive("x.tar.gz") && is_archive("y.rar") && is_archive("z.7z"));
        assert!(!is_archive("notes.txt") && !is_archive(".zip") && !is_archive("photo.gz.png"));
        assert_eq!(stem("photos.tar.gz"), "photos");
        assert_eq!(stem("My.Stuff.zip"), "My.Stuff");
        assert_eq!(default_name(&["/a/report.pdf".into()], Format::Zip), "report.pdf.zip");
        assert_eq!(default_name(&["/a/x".into(), "/a/y".into()], Format::TarXz), "Archive.tar.xz");
    }

    #[test]
    fn parses_listing_lines() {
        let e = parse_list_line("-rw-r--r--  0 501    20          3 Nov 23 15:06 ./sub/b ü  two spaces.txt").unwrap();
        assert_eq!(e.path, "sub/b ü  two spaces.txt");
        assert!(!e.is_dir);
        assert_eq!(e.size, 3);
        let d = parse_list_line("drwxr-xr-x  0 0      0           0 Jan  2  2021 docs/").unwrap();
        assert!(d.is_dir && d.path == "docs");
        assert_eq!(d.modified, Some(days_from_civil(2021, 1, 2) * 86_400_000));
        let l = parse_list_line("lrwxr-xr-x  0 me     staff       0 Mar  5  2020 link -> target").unwrap();
        assert_eq!(l.path, "link");
        assert!(parse_list_line("drwxr-xr-x  0 0 0 0 Nov 23 15:06 ./").is_none(), "root is skipped");
        assert!(parse_list_line("garbage").is_none());
        assert!(parse_list_line("-rw-r--r--  0 0 0 1 Nov 23 15:06 ../escape").is_none());
    }

    #[test]
    fn dates() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        let y = current_year(now);
        assert!(days_from_civil(y, 1, 1) * 86_400_000 <= now && now < days_from_civil(y + 1, 1, 1) * 86_400_000);
    }

    #[test]
    fn fills_in_implicit_dirs() {
        let e = |p: &str, d| ArchiveEntry { path: p.into(), is_dir: d, size: 0, modified: None };
        let all = complete_dirs(vec![e("a/b/c.txt", false), e("top.txt", false)]);
        let paths: Vec<_> = all.iter().map(|x| (x.path.as_str(), x.is_dir)).collect();
        assert_eq!(paths, [("a", true), ("a/b", true), ("a/b/c.txt", false), ("top.txt", false)]);
        let root: Vec<_> = children(&all, "").map(|x| x.path.as_str()).collect();
        assert_eq!(root, ["a", "top.txt"]);
        assert_eq!(children(&all, "a").map(|x| x.path.as_str()).collect::<Vec<_>>(), ["a/b"]);
    }

    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("src");
        fs::create_dir_all(src.join("photos/2025")).unwrap();
        fs::write(src.join("photos/2025/ü beach.jpg"), b"jpg").unwrap();
        fs::write(src.join("photos/readme.txt"), b"hello").unwrap();
        fs::write(src.join("notes.txt"), b"notes").unwrap();
        (d, src)
    }

    #[test]
    fn round_trips_every_format() {
        if !have_bsdtar() {
            return eprintln!("bsdtar not installed; skipping");
        }
        let (d, src) = fixture();
        for format in [Format::Zip, Format::SevenZip, Format::TarGz, Format::TarXz, Format::TarBz2] {
            let out = d.path().join(format!("test.{}", format.extension()));
            let mut steps = 0;
            let created = compress(
                &[src.join("photos"), src.join("notes.txt")],
                &out,
                format,
                &AtomicBool::new(false),
                &mut |_, _, _| steps += 1,
            )
            .unwrap()
            .unwrap();
            assert_eq!(created, out);
            assert!(steps >= 4, "{format:?}: progress reported ({steps})");
            let names: Vec<_> = list(&out).unwrap().into_iter().map(|e| e.path).collect();
            // macOS's bsdtar stores names decomposed (u + U+0308), like Finder's archiver.
            assert!(
                names.iter().any(|n| n.starts_with("photos/2025/") && n.ends_with(" beach.jpg")),
                "{format:?}: {names:?}"
            );
            assert!(names.contains(&"notes.txt".to_string()));
            assert!(!names.iter().any(|n| n.contains("._")), "no AppleDouble files: {names:?}");
        }
    }

    #[test]
    fn extract_here_detects_a_single_root() {
        if !have_bsdtar() {
            return;
        }
        let (d, src) = fixture();
        let single = d.path().join("photos.zip");
        compress(&[src.join("photos")], &single, Format::Zip, &AtomicBool::new(false), &mut noop()).unwrap();
        let multi = d.path().join("bundle.tar.gz");
        compress(
            &[src.join("photos"), src.join("notes.txt")],
            &multi,
            Format::TarGz,
            &AtomicBool::new(false),
            &mut noop(),
        )
        .unwrap();
        let out = d.path().join("out");
        fs::create_dir(&out).unwrap();

        // One top-level folder: extracted as is.
        let a = extract(&single, &out, None, &AtomicBool::new(false), &mut noop()).unwrap().unwrap();
        assert_eq!(a, [out.join("photos")]);
        assert_eq!(fs::read(out.join("photos/2025/ü beach.jpg")).unwrap(), b"jpg");
        // Again: never overwrites.
        let b = extract(&single, &out, None, &AtomicBool::new(false), &mut noop()).unwrap().unwrap();
        assert_eq!(b, [out.join("photos (1)")]);
        // Several top-level items: into a folder named after the archive.
        let c = extract(&multi, &out, None, &AtomicBool::new(false), &mut noop()).unwrap().unwrap();
        assert_eq!(c, [out.join("bundle")]);
        assert!(out.join("bundle/notes.txt").is_file() && out.join("bundle/photos/readme.txt").is_file());
        let leftovers: Vec<_> = fs::read_dir(&out)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "temporary folders cleaned up");
    }

    #[test]
    fn extracts_selected_entries_and_browses() {
        if !have_bsdtar() {
            return;
        }
        let (d, src) = fixture();
        let zip = d.path().join("photos.zip");
        compress(&[src.join("photos")], &zip, Format::Zip, &AtomicBool::new(false), &mut noop()).unwrap();

        // Browsing: virtual paths split into archive + inner path.
        assert_eq!(split_virtual(&zip), Some((zip.clone(), String::new())));
        assert_eq!(split_virtual(&zip.join("photos/2025")), Some((zip.clone(), "photos/2025".into())));
        assert_eq!(split_virtual(&src.join("photos")), None);
        assert_eq!(split_virtual(&src.join("missing/x")), None);
        let all = list(&zip).unwrap();
        let inner: Vec<_> = children(&all, "photos").map(|e| e.path.as_str()).collect();
        assert_eq!(inner, ["photos/2025", "photos/readme.txt"]);

        // Folder listings inside the archive.
        let opts = crate::ListOptions::default();
        let root = listing(&zip, "", &opts).unwrap();
        assert_eq!(root.path, zip);
        assert_eq!(root.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["photos"]);
        assert_eq!(root.entries[0].child_count, Some(2));
        let sub = listing(&zip, "photos", &opts).unwrap();
        assert_eq!(sub.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["2025", "readme.txt"]);
        assert_eq!(sub.entries[1].path, zip.join("photos/readme.txt"));
        assert_eq!(sub.entries[1].size, Some(5));
        assert!(sub.entries.iter().all(|e| e.readonly));
        assert_eq!(sub.parent.as_deref(), Some(zip.as_path()));
        assert!(matches!(listing(&zip, "nope", &opts), Err(CoreError::NotFound(_))));
        let opened = extract_for_opening(&zip, "photos/readme.txt").unwrap();
        assert_eq!(fs::read(&opened).unwrap(), b"hello");

        // Copying entries out of the browsed folder "photos".
        let out = d.path().join("out");
        fs::create_dir(&out).unwrap();
        let names = vec!["2025".to_string(), "readme.txt".to_string()];
        let got = extract(&zip, &out, Some(("photos", &names)), &AtomicBool::new(false), &mut noop()).unwrap().unwrap();
        assert_eq!(got, [out.join("2025"), out.join("readme.txt")]);
        assert_eq!(fs::read(out.join("2025/ü beach.jpg")).unwrap(), b"jpg");
    }

    #[test]
    fn errors_and_cancel() {
        if !have_bsdtar() {
            return;
        }
        let d = tempfile::tempdir().unwrap();
        let bad = d.path().join("broken.zip");
        fs::write(&bad, b"this is not a zip").unwrap();
        let err = extract(&bad, d.path(), None, &AtomicBool::new(false), &mut noop()).unwrap_err().to_string();
        assert!(err.starts_with("Could not open \"broken.zip\": ") && err.contains("format"), "{err}");
        assert_eq!(fs::read_dir(d.path()).unwrap().count(), 1, "nothing left behind");

        let (d2, src) = fixture();
        let zip = d2.path().join("x.zip");
        compress(&[src.join("photos")], &zip, Format::Zip, &AtomicBool::new(false), &mut noop()).unwrap();
        let cancelled = extract(&zip, d2.path(), None, &AtomicBool::new(true), &mut noop()).unwrap();
        assert_eq!(cancelled, None);
        assert!(!d2.path().join("photos").exists());
    }
}
