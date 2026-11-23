//! Requests from the OS to show folders in Sonar, so it can replace the
//! default file manager:
//!
//! - command-line arguments at startup (`sonar ~/Downloads`, Linux launchers),
//! - a second launch while Sonar runs (forwarded by the single-instance plugin),
//! - macOS "open" events (Finder, `open -a Sonar`, the default folder handler).
//!
//! Folders open in a new tab; for a file, its folder opens with the file selected.
//! Requests that arrive before the window is ready are queued.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

pub const EVENT_OPEN_PATHS: &str = "open-paths";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenRequest {
    pub folder: PathBuf,
    /// Name of an item in `folder` to select (when a file was requested).
    pub select: Option<String>,
}

#[derive(Default)]
pub struct OpenQueue(Mutex<(bool, Vec<OpenRequest>)>);

/// What to show for a requested path; `None` if it doesn't exist.
pub fn to_request(path: &Path) -> Option<OpenRequest> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.is_dir() {
        return Some(OpenRequest { folder: path.to_path_buf(), select: None });
    }
    let folder = path.parent()?.to_path_buf();
    let select = path.file_name().map(|n| n.to_string_lossy().into_owned());
    Some(OpenRequest { folder, select })
}

/// Show `paths` in the window, or queue them until it's ready.
pub fn handle(app: &AppHandle, paths: Vec<PathBuf>) {
    let requests: Vec<OpenRequest> = paths.iter().filter_map(|p| to_request(p)).collect();
    if requests.is_empty() {
        return;
    }
    let queue = app.state::<OpenQueue>();
    let mut q = queue.0.lock().unwrap_or_else(|p| p.into_inner());
    if q.0 {
        let _ = app.emit(EVENT_OPEN_PATHS, &requests);
    } else {
        q.1.extend(requests);
    }
    drop(q);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Called once by the frontend when it's ready: returns queued requests, and
/// later ones arrive as `open-paths` events.
#[tauri::command]
pub fn take_open_requests(queue: State<'_, OpenQueue>) -> Vec<OpenRequest> {
    let mut q = queue.0.lock().unwrap_or_else(|p| p.into_inner());
    q.0 = true;
    std::mem::take(&mut q.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_and_files() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("report.pdf");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(to_request(d.path()), Some(OpenRequest { folder: d.path().to_path_buf(), select: None }));
        assert_eq!(
            to_request(&file),
            Some(OpenRequest { folder: d.path().to_path_buf(), select: Some("report.pdf".into()) })
        );
        assert_eq!(to_request(&d.path().join("missing")), None);
    }
}
