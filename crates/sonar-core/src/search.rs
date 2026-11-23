//! Recursive filename search.
//!
//! Breadth-first, so matches close to the starting folder arrive first.
//! Results are streamed in batches, the walk can be cancelled at any time,
//! symlinked folders are not followed (no loops), and hidden folders are
//! skipped unless hidden files are shown.

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Utf32Str};
use serde::Serialize;

use crate::entry::Entry;
use crate::platform::HiddenRules;

/// Stop after this many matches; a search this broad needs a better query.
pub const MAX_RESULTS: usize = 10_000;
const FLUSH_EVERY: Duration = Duration::from_millis(120);

/// fzf-style fuzzy matching (nucleo, the matcher behind the Helix editor)
/// against each item's path relative to the search folder, so `dlrep` finds
/// `Downloads/report.pdf`. fzf syntax works: space-separated terms must all
/// match, `'exact`, `^prefix`, `suffix$`, `!negation`. The match has to reach
/// into the item's own name, otherwise searching `notes` would list every
/// file inside a "Notes" folder. Queries with `*`/`?` are plain globs on the name.
pub struct Matcher {
    kind: Kind,
    /// Weakest fuzzy score kept (see [`MIN_QUALITY`]).
    min_score: u32,
    matcher: nucleo_matcher::Matcher,
    buf: Vec<char>,
    indices: Vec<u32>,
}

enum Kind {
    Fuzzy(Pattern),
    Glob(Vec<char>),
}

impl Matcher {
    pub fn new(query: &str) -> Option<Self> {
        let q = query.trim();
        if q.is_empty() {
            return None;
        }
        let mut matcher = nucleo_matcher::Matcher::new(Config::DEFAULT.match_paths());
        let mut buf = Vec::new();
        let (kind, min_score) = if q.contains(['*', '?']) {
            (Kind::Glob(q.to_lowercase().chars().collect()), 0)
        } else {
            // Case never matters, like Dolphin: "hva", "HVA" and "HvA" find the same files.
            let pattern = Pattern::parse(q, CaseMatching::Ignore, Normalization::Smart);
            // The query matched against itself is the best score possible.
            let ideal = pattern.score(Utf32Str::new(q, &mut buf), &mut matcher).unwrap_or(0);
            let min = (f64::from(ideal) * MIN_QUALITY) as u32;
            (Kind::Fuzzy(pattern), min)
        };
        Some(Self { kind, min_score, matcher, buf, indices: Vec::new() })
    }

    /// Score for an item at `relative` (path from the search root, `/`-separated);
    /// `None` if it doesn't match. Higher is better.
    pub fn score(&mut self, relative: &str) -> Option<u32> {
        let name = relative.rsplit('/').next().unwrap_or(relative);
        match &self.kind {
            Kind::Glob(p) => glob_match(p, &name.to_lowercase().chars().collect::<Vec<_>>()).then_some(0),
            Kind::Fuzzy(pattern) => {
                let hay = Utf32Str::new(relative, &mut self.buf);
                if pattern.score(hay, &mut self.matcher)? < self.min_score {
                    return None; // cheap rejection first
                }
                self.indices.clear();
                let hay = Utf32Str::new(relative, &mut self.buf);
                let score = pattern.indices(hay, &mut self.matcher, &mut self.indices)?;
                let name_start = (relative.chars().count() - name.chars().count()) as u32;
                let reaches_name = self.indices.is_empty() || self.indices.iter().any(|&i| i >= name_start);
                reaches_name.then_some(score)
            }
        }
    }
}

/// `*` = any run of characters, `?` = exactly one (iterative, no recursion).
fn glob_match(pattern: &[char], text: &[char]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|c| *c == '*')
}

/// Fuzzy matches scoring below this fraction of a perfect match are dropped.
/// Pure fzf-style matching accepts letters scattered across long paths;
/// measured on a real home folder, contained words score ~95% of ideal and
/// abbreviations like `dlrep` → Downloads/report ~85%, while scattered junk
/// scores ~50%. A 75% cut cut `report` from 7,257 hits to ~250.
pub const MIN_QUALITY: f64 = 0.75;

/// A search result with its match score (higher is better).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    #[serde(flatten)]
    pub entry: Entry,
    pub score: u32,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSummary {
    pub found: usize,
    pub folders_scanned: u64,
    /// Stopped at [`MAX_RESULTS`].
    pub truncated: bool,
    pub cancelled: bool,
}

/// Search below `root`. `on_batch(hits, folders_scanned)` is called with new
/// matches every ~120 ms (and at the end, possibly with none).
pub fn search(
    root: &Path,
    query: &str,
    show_hidden: bool,
    cancel: &AtomicBool,
    on_batch: &mut dyn FnMut(Vec<Hit>, u64),
) -> SearchSummary {
    let mut summary = SearchSummary::default();
    let Some(mut matcher) = Matcher::new(query) else { return summary };
    let mut queue: VecDeque<PathBuf> = VecDeque::from([root.to_path_buf()]);
    let mut batch = Vec::new();
    let mut last_flush = Instant::now();

    'walk: while let Some(dir) = queue.pop_front() {
        if cancel.load(Ordering::Relaxed) {
            summary.cancelled = true;
            break;
        }
        let Ok(read) = fs::read_dir(&dir) else { continue };
        summary.folders_scanned += 1;
        let rules = HiddenRules::for_dir(&dir);
        for de in read.flatten() {
            let name = de.file_name();
            let name = name.to_string_lossy();
            let Ok(ft) = de.file_type() else { continue };
            let path = de.path();
            let relative = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().into_owned();
            let score = matcher.score(&relative);
            let matched = score.is_some();
            // Cheap checks first: only build a full Entry (stat, type lookup) for matches
            // and for folders we may descend into.
            if !matched && !ft.is_dir() {
                continue;
            }
            let Ok(meta) = de.metadata() else { continue };
            let hidden = rules.is_hidden(&name, &meta);
            if hidden && !show_hidden {
                continue;
            }
            if ft.is_dir() {
                queue.push_back(path); // symlinks report !is_dir() here: not followed
            }
            if let Some(score) = score {
                if let Some(entry) = Entry::from_dir_entry(&de, &rules, false) {
                    batch.push(Hit { entry, score });
                    summary.found += 1;
                    if summary.found >= MAX_RESULTS {
                        summary.truncated = true;
                        break 'walk;
                    }
                }
            }
        }
        if last_flush.elapsed() >= FLUSH_EVERY && !batch.is_empty() {
            on_batch(std::mem::take(&mut batch), summary.folders_scanned);
            last_flush = Instant::now();
        }
    }
    on_batch(batch, summary.folders_scanned);
    summary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy_matching() {
        let mut m = Matcher::new("dlrep").unwrap();
        assert!(m.score("Downloads/report-2025.pdf").is_some(), "fzf-style across the path");
        assert!(m.score("Documents/old.txt").is_none());
        // Matches must reach into the item's own name.
        let mut notes = Matcher::new("notes").unwrap();
        assert!(notes.score("Notes/x.txt").is_none());
        assert!(notes.score("work/meeting-notes.md").is_some());
        assert!(notes.score("Notes").is_some(), "the folder itself matches");
        // Better matches score higher.
        let mut r = Matcher::new("report").unwrap();
        assert!(r.score("report.pdf").unwrap() > r.score("r-e-p-o-r-t.pdf").unwrap());
        // fzf syntax: all terms, exact, negation, suffix.
        let mut multi = Matcher::new("dl rep").unwrap();
        assert!(multi.score("Downloads/report.pdf").is_some());
        let mut neg = Matcher::new("report !draft").unwrap();
        assert!(neg.score("report.pdf").is_some() && neg.score("report-draft.pdf").is_none());
        let mut suffix = Matcher::new(".pdf$").unwrap();
        assert!(suffix.score("a/report.pdf").is_some() && suffix.score("report.pdf.bak").is_none());
        // Words inside long paths are kept; letters scattered all over are not.
        let mut word = Matcher::new("invoice").unwrap();
        assert!(word.score("Documents/2025/finance/Q3/acme-invoice-0042-final.pdf").is_some());
        assert!(word.score("src/lib/environment/config/service.rs").is_none(), "scattered letters");
        // Case-insensitive, and the casing of the query doesn't change the results.
        let names = ["HvA-rooster.pdf", "studie/HVA notes.md", "hva.txt", "Downloads/Have a look.txt"];
        let found = |q: &str| {
            let mut m = Matcher::new(q).unwrap();
            names.iter().map(|n| m.score(n).is_some()).collect::<Vec<_>>()
        };
        assert_eq!(found("hva")[..3], [true, true, true]);
        assert_eq!(found("HVA"), found("hva"));
        assert_eq!(found("HvA"), found("hva"));
        assert!(Matcher::new("   ").is_none());
    }

    #[test]
    fn glob_queries() {
        let mut g = Matcher::new("*.PNG").unwrap();
        assert!(g.score("a/photo.png").is_some() && g.score("photo.png.bak").is_none());
        let mut q = Matcher::new("img_??.jpg").unwrap();
        assert!(q.score("IMG_01.jpg").is_some() && q.score("img_1.jpg").is_none());
    }

    fn collect(root: &Path, query: &str, hidden: bool) -> (Vec<String>, SearchSummary) {
        let mut names = Vec::new();
        let s = search(root, query, hidden, &AtomicBool::new(false), &mut |batch, _| {
            names.extend(batch.into_iter().map(|h| h.entry.path.strip_prefix(root).unwrap().display().to_string()));
        });
        (names, s)
    }

    #[test]
    fn walks_breadth_first_and_skips_hidden() {
        let d = tempfile::tempdir().unwrap();
        let r = d.path();
        fs::create_dir_all(r.join("a/deep/deeper")).unwrap();
        fs::create_dir_all(r.join(".secret")).unwrap();
        fs::write(r.join("notes.txt"), "").unwrap();
        fs::write(r.join("a/deep/deeper/notes-old.txt"), "").unwrap();
        fs::write(r.join("a/notes-a.md"), "").unwrap();
        fs::write(r.join(".secret/notes-hidden.txt"), "").unwrap();
        fs::create_dir(r.join("a/Notes folder")).unwrap();

        let (names, s) = collect(r, "notes", false);
        assert_eq!(names[0], "notes.txt", "shallowest first (ranking happens in the UI)");
        assert_eq!(names.last().unwrap(), "a/deep/deeper/notes-old.txt");
        assert!(names.contains(&"a/Notes folder".to_string()), "folders match too");
        assert!(!names.iter().any(|n| n.contains("hidden")));
        assert_eq!(s.found, 4);
        let (with_hidden, _) = collect(r, "notes", true);
        assert!(with_hidden.contains(&".secret/notes-hidden.txt".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlink_loops() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir(d.path().join("x")).unwrap();
        std::os::unix::fs::symlink(d.path(), d.path().join("x/loop")).unwrap();
        fs::write(d.path().join("x/target.txt"), "").unwrap();
        let (names, _) = collect(d.path(), "target", false);
        assert_eq!(names, ["x/target.txt"]);
    }

    #[test]
    fn cancel_stops_early() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("hit"), "").unwrap();
        let s = search(d.path(), "hit", false, &AtomicBool::new(true), &mut |_, _| {});
        assert!(s.cancelled && s.found == 0);
    }
}
