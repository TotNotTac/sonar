//! Platform code that is bound to the UI toolkit (AppKit, GTK) and therefore
//! must run on the main thread: the system clipboard and native drag-out.
//! Toolkit-free platform code lives in `sonar_core::platform`; the clipboard
//! data formats live in `sonar_core::clipboard`.

use std::sync::mpsc;

use serde::Serialize;
use sonar_core::clipboard::FileClip;
use sonar_core::uipath::{self, UiPath};
use sonar_core::CoreError;
use tauri::{AppHandle, Emitter, WebviewWindow};

#[cfg(target_os = "linux")]
#[path = "clipboard_gtk.rs"]
mod clipboard;
#[cfg(target_os = "macos")]
#[path = "clipboard_macos.rs"]
mod clipboard;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
#[path = "clipboard_none.rs"]
mod clipboard;

pub const EVENT_DRAG_OUT_ENDED: &str = "drag-out-ended";

/// Run `f` on the main (UI) thread and wait for its result without blocking
/// the async runtime. Callers must be `async` commands: sync commands already
/// run on the main thread and would deadlock here.
async fn on_main<T: Send + 'static>(app: &AppHandle, f: impl FnOnce() -> T + Send + 'static) -> Result<T, CoreError> {
    let (tx, rx) = mpsc::channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|e| CoreError::Other(format!("UI thread unavailable: {e}")))?;
    tauri::async_runtime::spawn_blocking(move || rx.recv())
        .await
        .map_err(|e| CoreError::Other(e.to_string()))?
        .map_err(|_| CoreError::Other("UI thread dropped the request".into()))
}

fn clip_err(e: String) -> CoreError {
    CoreError::Other(format!("Clipboard: {e}"))
}

/// Put files on the system clipboard (other file managers can paste them).
#[tauri::command]
pub async fn clipboard_write_files(app: AppHandle, clip: FileClip) -> Result<(), CoreError> {
    on_main(&app, move || clipboard::write_files(&clip)).await?.map_err(clip_err)
}

/// Files on the system clipboard, from Sonar or any other app.
#[tauri::command]
pub async fn clipboard_read_files(app: AppHandle) -> Result<Option<FileClip>, CoreError> {
    on_main(&app, clipboard::read_files).await
}

#[tauri::command]
pub async fn clipboard_write_text(app: AppHandle, text: String) -> Result<(), CoreError> {
    on_main(&app, move || clipboard::write_text(&text)).await?.map_err(clip_err)
}

/// After a cut-and-paste completes, empty the clipboard if it still holds
/// our cut (pasting again would fail: the sources are gone).
#[tauri::command]
pub async fn clipboard_clear_cut(app: AppHandle) -> Result<(), CoreError> {
    on_main(&app, clipboard::clear_cut).await
}

#[derive(Clone, Serialize)]
struct DragOutEnded {
    dropped: bool,
}

/// Hand an in-progress drag over to the OS so the files can be dropped on
/// other apps (Finder, Dolphin, mail...). Offers a copy; the receiving app
/// decides what to do with it. Drops back onto Sonar arrive as Tauri's
/// regular drag-drop events.
#[tauri::command]
pub async fn start_drag_out(app: AppHandle, window: WebviewWindow, paths: Vec<UiPath>) -> Result<(), CoreError> {
    let paths = uipath::paths(paths);
    let icon = include_bytes!("../../icons/64x64.png").to_vec();
    let notify = app.clone();
    let result = on_main(&app, move || {
        let on_end = move |result: drag::DragResult, _pos: drag::CursorPosition| {
            let dropped = matches!(result, drag::DragResult::Dropped);
            let _ = notify.emit(EVENT_DRAG_OUT_ENDED, DragOutEnded { dropped });
        };
        let item = drag::DragItem::Files(paths);
        let image = drag::Image::Raw(icon);
        let options = drag::Options { skip_animatation_on_cancel_or_failure: false, mode: drag::DragMode::Copy };
        #[cfg(target_os = "linux")]
        {
            let gtk_window = window.gtk_window().map_err(|e| e.to_string())?;
            drag::start_drag(&gtk_window, item, image, on_end, options).map_err(|e| e.to_string())
        }
        #[cfg(not(target_os = "linux"))]
        {
            drag::start_drag(&window, item, image, on_end, options).map_err(|e| e.to_string())
        }
    })
    .await?;
    result.map_err(|e| CoreError::Other(format!("Could not start dragging: {e}")))
}
