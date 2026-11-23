//! freedesktop.org `.desktop` files: parsing, and expanding `Exec=` lines
//! into commands (Desktop Entry Specification, "The Exec key"). Pure
//! functions; used by the Linux "Open With" support and tested everywhere.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopEntry {
    pub name: String,
    pub exec: String,
    pub icon: Option<String>,
    pub mime_types: Vec<String>,
    /// Hidden from menus (`NoDisplay`) or deleted (`Hidden`).
    pub hidden: bool,
    pub terminal: bool,
}

/// Parse the `[Desktop Entry]` group of an application entry. `None` for
/// other types or entries without a name or command.
pub fn parse(text: &str) -> Option<DesktopEntry> {
    let mut in_main = false;
    let mut e = DesktopEntry::default();
    let mut kind = None;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim(), unescape_value(value.trim()));
        match key {
            "Type" => kind = Some(value),
            "Name" => e.name = value,
            "Exec" => e.exec = value,
            "Icon" => e.icon = Some(value).filter(|v| !v.is_empty()),
            "MimeType" => e.mime_types = value.split(';').filter(|m| !m.is_empty()).map(String::from).collect(),
            "NoDisplay" | "Hidden" => e.hidden |= value == "true",
            "Terminal" => e.terminal = value == "true",
            _ => {}
        }
    }
    (kind.as_deref() == Some("Application") && !e.name.is_empty() && !e.exec.is_empty()).then_some(e)
}

/// String escapes that apply to every value: `\s \n \t \r \\`.
fn unescape_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                // Not a value escape: keep it for the Exec quoting rules.
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Split an `Exec` value into arguments: spaces separate, double quotes
/// group, and inside quotes `\"`, `` \` ``, `\$` and `\\` are escapes.
pub fn split_exec(exec: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_arg = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_arg = true;
            }
            '\\' if in_quotes => match chars.next() {
                Some(e @ ('"' | '`' | '$' | '\\')) => cur.push(e),
                Some(other) => {
                    cur.push('\\');
                    cur.push(other);
                }
                None => cur.push('\\'),
            },
            ' ' | '\t' if !in_quotes => {
                if has_arg || !cur.is_empty() {
                    args.push(std::mem::take(&mut cur));
                    has_arg = false;
                }
            }
            _ => {
                cur.push(c);
                has_arg = true;
            }
        }
    }
    if has_arg || !cur.is_empty() {
        args.push(cur);
    }
    args
}

/// The commands to run to open `files` with this entry. `%f`/`%u` take one
/// file, so several files mean several commands; `%F`/`%U` take them all.
/// Arguments are `OsString`s so file names that aren't UTF-8 pass through intact.
pub fn commands(entry: &DesktopEntry, desktop_file: &Path, files: &[PathBuf]) -> Vec<Vec<OsString>> {
    let template = split_exec(&entry.exec);
    let single = template.iter().any(|a| a.contains("%f") || a.contains("%u"));
    let groups: Vec<&[PathBuf]> = if single && files.len() > 1 { files.chunks(1).collect() } else { vec![files] };
    groups.into_iter().map(|group| expand(&template, entry, desktop_file, group)).collect()
}

fn expand(template: &[String], entry: &DesktopEntry, desktop_file: &Path, files: &[PathBuf]) -> Vec<OsString> {
    let paths = || files.iter().map(|f| f.as_os_str().to_owned());
    let uris = || files.iter().map(|f| OsString::from(crate::clipboard::path_to_uri(f)));
    let mut out = Vec::new();
    for arg in template {
        match arg.as_str() {
            // Field codes standing alone expand to zero or more arguments.
            "%F" => out.extend(paths()),
            "%U" => out.extend(uris()),
            "%f" => out.extend(paths().take(1)),
            "%u" => out.extend(uris().take(1)),
            "%i" => {
                if let Some(icon) = &entry.icon {
                    out.push("--icon".into());
                    out.push(icon.into());
                }
            }
            _ => {
                // Field codes inside an argument; deprecated ones vanish.
                let mut s = OsString::new();
                let mut chars = arg.chars();
                while let Some(c) = chars.next() {
                    if c != '%' {
                        s.push(c.encode_utf8(&mut [0; 4]));
                        continue;
                    }
                    match chars.next() {
                        Some('%') => s.push("%"),
                        Some('f') => s.extend(paths().next()),
                        Some('u') => s.extend(uris().next()),
                        Some('c') => s.push(&entry.name),
                        Some('k') => s.push(desktop_file),
                        _ => {}
                    }
                }
                if !s.is_empty() {
                    out.push(s);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const OKULAR: &str = "[Desktop Entry]\nType=Application\nName=Okular\nName[nl]=Okular NL\nExec=okular %U\nIcon=okular\n\
        MimeType=application/pdf;application/postscript;\n\n[Desktop Action new]\nName=New window\nExec=okular --new";

    #[test]
    fn parses_entries() {
        let e = parse(OKULAR).unwrap();
        assert_eq!(e.name, "Okular");
        assert_eq!(e.exec, "okular %U", "only the main group counts");
        assert_eq!(e.mime_types, ["application/pdf", "application/postscript"]);
        assert!(!e.hidden && !e.terminal);
        assert!(parse("[Desktop Entry]\nType=Link\nName=x\nExec=y").is_none());
        assert!(parse("[Desktop Entry]\nType=Application\nName=x\nExec=y\nNoDisplay=true").unwrap().hidden);
    }

    #[test]
    fn splits_exec_lines() {
        assert_eq!(split_exec("vlc --started-from-file %U"), ["vlc", "--started-from-file", "%U"]);
        assert_eq!(
            split_exec(r#""/opt/My App/bin" --arg "a \"quoted\" \$x" %f"#),
            ["/opt/My App/bin", "--arg", "a \"quoted\" $x", "%f"]
        );
        assert_eq!(split_exec("a  \"\"  b"), ["a", "", "b"]);
    }

    #[test]
    fn expands_field_codes() {
        let e = parse(OKULAR).unwrap();
        let files = vec![PathBuf::from("/tmp/a b.pdf"), PathBuf::from("/tmp/c.pdf")];
        let k = Path::new("/usr/share/applications/org.kde.okular.desktop");
        assert_eq!(commands(&e, k, &files), [vec!["okular", "file:///tmp/a%20b.pdf", "file:///tmp/c.pdf"]]);

        let single = DesktopEntry { exec: "gimp %f".into(), ..e.clone() };
        assert_eq!(commands(&single, k, &files), [vec!["gimp", "/tmp/a b.pdf"], vec!["gimp", "/tmp/c.pdf"]]);

        let fancy = DesktopEntry { exec: "app %i --title=%c --file=%f %d 100%%".into(), ..e };
        assert_eq!(
            commands(&fancy, k, &files[..1]),
            [vec!["app", "--icon", "okular", "--title=Okular", "--file=/tmp/a b.pdf", "100%"]]
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_files_pass_through() {
        use std::os::unix::ffi::OsStringExt;
        let bad = PathBuf::from(OsString::from_vec(b"/tmp/caf\xe9.pdf".to_vec()));
        let k = Path::new("/x.desktop");
        let e = DesktopEntry { name: "App".into(), exec: "app %f --file=%f %U".into(), ..Default::default() };
        let cmd = commands(&e, k, std::slice::from_ref(&bad)).remove(0);
        let mut joined = b"--file=".to_vec();
        joined.extend(bad.as_os_str().as_encoded_bytes());
        assert_eq!(cmd[1], bad.as_os_str());
        assert_eq!(cmd[2], OsString::from_vec(joined));
        assert_eq!(cmd[3], "file:///tmp/caf%E9.pdf");
    }
}
