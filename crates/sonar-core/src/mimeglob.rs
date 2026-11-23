//! File name -> MIME type from shared-mime-info `globs2` files, indexed.
//!
//! `xdg-mime` matches a name against every glob in turn (~1700 on a typical
//! system, ~40 µs per file); a folder of 100k files took seconds. Almost all
//! globs are literal names or `*suffix`, so those go into hash maps, and only
//! the handful of real wildcard patterns are tried one by one. Matching and
//! ranking follow `xdg-mime` 0.4: literal names ignore case, `*suffix` also
//! matches the lowercased name unless the glob is case-sensitive, and of all
//! matches the highest weight wins, then the longest pattern. Pure; used on
//! Linux and tested everywhere.

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone)]
struct Rule {
    mime: String,
    weight: i32,
    case_sensitive: bool,
    /// Pattern length as `xdg-mime` counts it (a `*suffix` glob without its `*`).
    len: usize,
    /// Position in the files, to break ties the same way every time.
    order: usize,
}

#[derive(Default)]
pub struct GlobIndex {
    /// Lowercased literal name ("makefile") -> rules.
    literal: HashMap<String, Vec<Rule>>,
    /// Suffix as written (".tar.gz", "~") -> rules.
    suffix: HashMap<String, Vec<Rule>>,
    /// Patterns with wildcards beyond a leading `*`.
    full: Vec<(glob::Pattern, Rule)>,
    /// Lengths of the suffixes in `suffix`, so lookups only try those.
    suffix_lens: Vec<usize>,
}

impl GlobIndex {
    /// The `mime/globs2` (or v1 `mime/globs`) files under each data dir, in order.
    pub fn load(data_dirs: &[impl AsRef<Path>]) -> Self {
        let mut index = Self::default();
        for dir in data_dirs {
            let mime = dir.as_ref().join("mime");
            if let Ok(text) = std::fs::read_to_string(mime.join("globs2")) {
                index.add_globs2(&text);
            } else if let Ok(text) = std::fs::read_to_string(mime.join("globs")) {
                index.add_globs1(&text);
            }
        }
        index
    }

    /// `weight:mime:glob[:flags]` lines.
    pub fn add_globs2(&mut self, text: &str) {
        for line in text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let mut parts = line.split(':');
            let (Some(weight), Some(mime), Some(glob)) = (parts.next(), parts.next(), parts.next()) else { continue };
            let Some(weight) = weight.parse::<i32>().ok().filter(|w| *w >= 0) else { continue };
            let cs = parts.next().is_some_and(|flags| flags.split(',').any(|f| f == "cs"));
            self.add(mime, glob, weight, cs);
        }
    }

    /// `mime:glob` lines (weight 50).
    pub fn add_globs1(&mut self, text: &str) {
        for line in text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')) {
            let mut parts = line.split(':');
            let (Some(mime), Some(glob), None) = (parts.next(), parts.next(), parts.next()) else { continue };
            self.add(mime, glob, 50, false);
        }
    }

    fn add(&mut self, mime: &str, glob: &str, weight: i32, case_sensitive: bool) {
        if mime.is_empty() || glob.is_empty() || !mime.contains('/') {
            return;
        }
        let order = self.literal.values().chain(self.suffix.values()).map(Vec::len).sum::<usize>() + self.full.len();
        let rule = |len| Rule { mime: mime.to_lowercase(), weight, case_sensitive, len, order };
        let wild = |s: &str| s.contains(['\\', '[', '*', '?']);
        if let Some(tail) = glob.strip_prefix('*').filter(|t| !wild(t)) {
            let rules = self.suffix.entry(tail.to_string()).or_default();
            rules.push(rule(tail.len()));
            if !self.suffix_lens.contains(&tail.len()) {
                self.suffix_lens.push(tail.len());
            }
        } else if !wild(glob) {
            self.literal.entry(glob.to_lowercase()).or_default().push(rule(glob.len()));
        } else if let Ok(p) = glob::Pattern::new(glob) {
            self.full.push((p, rule(glob.len())));
        }
    }

    /// The best-matching MIME type for a file name, if any glob matches.
    pub fn lookup(&self, name: &str) -> Option<&str> {
        let lower = name.to_lowercase();
        let mut found: Vec<&Rule> = self.literal.get(&lower).into_iter().flatten().collect();
        for &len in &self.suffix_lens {
            let exact = tail(name, len);
            found.extend(exact.and_then(|t| self.suffix.get(t)).into_iter().flatten());
            let folded = tail(&lower, len);
            if folded != exact {
                let rules = folded.and_then(|t| self.suffix.get(t)).into_iter().flatten();
                found.extend(rules.filter(|r| !r.case_sensitive));
            }
        }
        found.extend(self.full.iter().filter(|(p, _)| p.matches(name)).map(|(_, r)| r));
        found.into_iter().max_by_key(|r| (r.weight, r.len, std::cmp::Reverse(r.order))).map(|r| r.mime.as_str())
    }
}

/// The last `len` bytes of `s`, if that is a char boundary.
fn tail(s: &str, len: usize) -> Option<&str> {
    s.len().checked_sub(len).and_then(|i| s.get(i..))
}

#[cfg(test)]
mod tests {
    use super::GlobIndex;

    const GLOBS2: &str = "\
# This file was automatically generated
50:text/plain:*.txt
50:application/x-compressed-tar:*.tar.gz
50:application/gzip:*.gz
50:text/x-c++src:*.C:cs
50:text/x-csrc:*.c
50:text/x-makefile:makefile
50:text/x-makefile:[Mm]akefile
50:text/x-readme:README*
10:text/x-readme:readme*
50:application/x-trash:*~
80:text/x-cmake:CMakeLists.txt
55:text/x-weird:*.TXT:cs
50:broken line
-1:text/negative:*.neg
";

    fn index() -> GlobIndex {
        let mut i = GlobIndex::default();
        i.add_globs2(GLOBS2);
        i
    }

    #[test]
    fn suffixes_literals_and_wildcards() {
        let i = index();
        assert_eq!(i.lookup("notes.txt"), Some("text/plain"));
        assert_eq!(i.lookup("NOTES.Txt"), Some("text/plain"), "case-insensitive suffix");
        assert_eq!(i.lookup("backup.tar.gz"), Some("application/x-compressed-tar"), "longest suffix wins");
        assert_eq!(i.lookup("x.gz"), Some("application/gzip"));
        assert_eq!(i.lookup("file.txt~"), Some("application/x-trash"));
        assert_eq!(i.lookup("Makefile"), Some("text/x-makefile"));
        assert_eq!(i.lookup("MAKEFILE"), Some("text/x-makefile"), "literal names ignore case");
        assert_eq!(i.lookup("README.md"), Some("text/x-readme"), "wildcard pattern");
        assert_eq!(i.lookup("CMakeLists.txt"), Some("text/x-cmake"), "higher weight beats the suffix");
        assert_eq!(i.lookup("photo.unknownext"), None);
        assert_eq!(i.lookup("x.neg"), None, "negative weights are invalid");
        assert_eq!(i.lookup(""), None);
        assert_eq!(i.lookup("é.txt"), Some("text/plain"), "multi-byte names");
    }

    #[test]
    fn case_sensitive_globs() {
        let i = index();
        assert_eq!(i.lookup("main.C"), Some("text/x-c++src"));
        assert_eq!(i.lookup("main.c"), Some("text/x-csrc"));
        // *.TXT:cs has the higher weight but only matches that exact case.
        assert_eq!(i.lookup("A.TXT"), Some("text/x-weird"));
        assert_eq!(i.lookup("a.Txt"), Some("text/plain"));
    }

    #[test]
    fn v1_globs_and_loading() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("a/mime")).unwrap();
        std::fs::create_dir_all(d.path().join("b/mime")).unwrap();
        std::fs::write(d.path().join("a/mime/globs"), "text/x-v1:*.vone\nbad\n").unwrap();
        std::fs::write(d.path().join("b/mime/globs2"), "50:text/x-v2:*.vtwo\n").unwrap();
        let i = GlobIndex::load(&[d.path().join("a"), d.path().join("b"), d.path().join("missing")]);
        assert_eq!(i.lookup("x.vone"), Some("text/x-v1"));
        assert_eq!(i.lookup("x.vtwo"), Some("text/x-v2"));
    }

    /// Same answers as `xdg-mime` for the real database, where there is one.
    #[cfg(target_os = "linux")]
    #[test]
    fn agrees_with_xdg_mime() {
        let db = xdg_mime::SharedMimeInfo::new();
        let mut dirs: Vec<std::path::PathBuf> = dirs::data_dir().into_iter().collect();
        dirs.extend(["/usr/local/share", "/usr/share"].map(std::path::PathBuf::from));
        let i = GlobIndex::load(&dirs);
        if i.lookup("x.png").is_none() {
            return; // no shared-mime-info installed
        }
        let names = [
            "a.png",
            "A.PNG",
            "x.tar.gz",
            "x.tar.xz",
            "Makefile",
            "makefile.am",
            "README",
            "README.md",
            "x.c",
            "x.C",
            "x.cpp",
            "x.h",
            "x.rs",
            "x.py",
            "x.js",
            "x.mjs",
            "x.json",
            "x.desktop",
            "core",
            "x.pdf",
            "x.odt",
            "x.docx",
            "x.mkv",
            "x.mp3",
            "x.flac",
            "x.html",
            "x.svg",
            "x.svgz",
            "x.iso",
            "x.AppImage",
            "x.deb",
            "x.txt~",
            "#x#",
            "x.bak",
            "Cargo.toml",
            "Cargo.lock",
            "CMakeLists.txt",
            "x.7z",
            "x.zip",
            "x.unknownext",
            "noext",
            ".bashrc",
            "x.tar.bz2",
            "x.sqlite",
            "x.ics",
            "x.vcf",
            "x.ttf",
            "x.epub",
            "x.jar",
            "x.apk",
            "x.SH",
            "Dockerfile",
        ];
        for n in names {
            let theirs = db.get_mime_types_from_file_name(n);
            let ours = i.lookup(n).unwrap_or("application/octet-stream");
            assert!(
                theirs.iter().any(|m| m.essence_str() == ours),
                "{n}: ours {ours}, xdg-mime {:?}",
                theirs.iter().map(|m| m.essence_str()).collect::<Vec<_>>()
            );
        }
    }
}
