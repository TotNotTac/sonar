use std::path::{Component, Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Crumb {
    pub label: String,
    #[serde(with = "crate::uipath::path")]
    pub path: PathBuf,
    /// Icon hint for the first crumb: "root" or "home". `None` for the rest.
    pub icon: Option<&'static str>,
}

/// Split `path` into clickable segments. Paths inside `home` start with a
/// single "Home" crumb instead of `/`, `Users`, `name` — like Dolphin does
/// when browsing from the Home place.
pub fn breadcrumbs(path: &Path, home: Option<&Path>) -> Vec<Crumb> {
    let mut crumbs = Vec::new();
    let (mut acc, rest) = match home.and_then(|h| path.strip_prefix(h).ok().map(|r| (h, r))) {
        Some((h, rest)) => {
            crumbs.push(Crumb { label: "Home".into(), path: h.to_path_buf(), icon: Some("home") });
            (h.to_path_buf(), rest)
        }
        None => (PathBuf::new(), path),
    };

    for comp in rest.components() {
        match comp {
            Component::RootDir => {
                acc.push("/");
                crumbs.push(Crumb { label: "/".into(), path: acc.clone(), icon: Some("root") });
            }
            Component::Normal(name) => {
                acc.push(name);
                crumbs.push(Crumb { label: name.to_string_lossy().into_owned(), path: acc.clone(), icon: None });
            }
            // Paths reaching us are absolute and normalised; ignore the rest.
            Component::CurDir | Component::ParentDir | Component::Prefix(_) => {}
        }
    }
    crumbs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outside_home() {
        let c = breadcrumbs(Path::new("/usr/share"), Some(Path::new("/home/me")));
        let labels: Vec<_> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["/", "usr", "share"]);
        assert_eq!(c[2].path, Path::new("/usr/share"));
    }

    #[test]
    fn inside_home() {
        let c = breadcrumbs(Path::new("/home/me/Documents/x"), Some(Path::new("/home/me")));
        let labels: Vec<_> = c.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, ["Home", "Documents", "x"]);
        assert_eq!(c[1].path, Path::new("/home/me/Documents"));
    }

    #[test]
    fn home_itself_and_sibling_prefix() {
        assert_eq!(breadcrumbs(Path::new("/home/me"), Some(Path::new("/home/me"))).len(), 1);
        // "/home/meow" must not be treated as inside "/home/me".
        assert_eq!(breadcrumbs(Path::new("/home/meow"), Some(Path::new("/home/me")))[0].label, "/");
    }
}
