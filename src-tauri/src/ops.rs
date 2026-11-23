//! File-operation commands: rename, new folder, trash, delete, background
//! copy/move jobs, and undo. The logic lives in `sonar_core::ops`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use sonar_core::ops::{self, Conflict, ConflictChoice, Progress, TransferKind, TransferObserver, UndoAction};
use sonar_core::CoreError;
use tauri::{AppHandle, Emitter, Manager, State};

pub const EVENT_JOB_PROGRESS: &str = "job-progress";
pub const EVENT_JOB_CONFLICT: &str = "job-conflict";
pub const EVENT_JOB_FINISHED: &str = "job-finished";

const UNDO_LIMIT: usize = 50;

#[derive(Default)]
pub struct OpsState {
    undo: Mutex<Vec<UndoAction>>,
    jobs: Mutex<HashMap<u64, JobControl>>,
    next_job: AtomicU64,
}

struct JobControl {
    cancel: Arc<AtomicBool>,
    answers: Sender<(ConflictChoice, bool)>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl OpsState {
    fn push_undo(&self, action: UndoAction) {
        let mut stack = lock(&self.undo);
        stack.push(action);
        if stack.len() > UNDO_LIMIT {
            stack.remove(0);
        }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, CoreError> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| CoreError::Other(format!("Background task failed: {e}")))
}

/// Result of an operation over several items: how many succeeded, and a
/// message per failure.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchReport {
    done: usize,
    errors: Vec<String>,
}

// ---------- Simple operations ----------

#[tauri::command]
pub async fn rename_entry(app: AppHandle, path: PathBuf, new_name: String) -> Result<PathBuf, CoreError> {
    let from = path.clone();
    let to = blocking(move || ops::rename(&path, &new_name)).await??;
    if to != from {
        app.state::<OpsState>().push_undo(UndoAction::Rename { from, to: to.clone() });
    }
    Ok(to)
}

/// Create "New Folder" (or "New Folder (n)") in `dir`; the UI then starts an inline rename.
#[tauri::command]
pub async fn create_folder(app: AppHandle, dir: PathBuf) -> Result<PathBuf, CoreError> {
    let path = blocking(move || ops::new_folder(&dir)).await??;
    app.state::<OpsState>().push_undo(UndoAction::NewFolder(path.clone()));
    Ok(path)
}

#[tauri::command]
pub async fn trash_entries(app: AppHandle, paths: Vec<PathBuf>) -> Result<BatchReport, CoreError> {
    let (receipts, errors) = blocking(move || {
        let mut receipts = Vec::new();
        let mut errors = Vec::new();
        for p in &paths {
            match ops::trash(p) {
                Ok(r) => receipts.push(r),
                Err(e) => errors.push(e.to_string()),
            }
        }
        (receipts, errors)
    })
    .await?;
    let done = receipts.len();
    if done > 0 {
        app.state::<OpsState>().push_undo(UndoAction::Trash(receipts));
    }
    Ok(BatchReport { done, errors })
}

/// Permanent delete. The UI confirms first; this cannot be undone.
#[tauri::command]
pub async fn delete_entries(paths: Vec<PathBuf>) -> Result<BatchReport, CoreError> {
    blocking(move || {
        let mut report = BatchReport { done: 0, errors: Vec::new() };
        for p in &paths {
            match ops::delete_permanently(p) {
                Ok(()) => report.done += 1,
                Err(e) => report.errors.push(e.to_string()),
            }
        }
        report
    })
    .await
}

// ---------- Undo ----------

#[tauri::command]
pub fn undo_label(state: State<'_, OpsState>) -> Option<String> {
    lock(&state.undo).last().map(|a| a.label().to_string())
}

/// Undo the most recent operation. Returns its label, or `None` if there was
/// nothing to undo.
#[tauri::command]
pub async fn undo(app: AppHandle) -> Result<Option<String>, CoreError> {
    let Some(action) = lock(&app.state::<OpsState>().undo).pop() else { return Ok(None) };
    let label = action.label().to_string();
    blocking(move || action.undo()).await??;
    Ok(Some(label))
}

// ---------- Copy / move jobs ----------

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobProgress {
    id: u64,
    #[serde(flatten)]
    progress: Progress,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobConflict {
    id: u64,
    #[serde(flatten)]
    conflict: Conflict,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobFinished {
    id: u64,
    kind: TransferKind,
    cancelled: bool,
    /// Top-level items that were fully transferred.
    completed: usize,
    errors: Vec<String>,
}

struct EventObserver {
    app: AppHandle,
    id: u64,
    answers: Receiver<(ConflictChoice, bool)>,
}

impl TransferObserver for EventObserver {
    fn progress(&mut self, progress: &Progress) {
        let _ = self.app.emit(EVENT_JOB_PROGRESS, JobProgress { id: self.id, progress: progress.clone() });
    }

    /// Ask the UI and block this job's thread until it answers (or cancels).
    fn conflict(&mut self, conflict: &Conflict) -> (ConflictChoice, bool) {
        let _ = self.app.emit(EVENT_JOB_CONFLICT, JobConflict { id: self.id, conflict: conflict.clone() });
        self.answers.recv().unwrap_or((ConflictChoice::Cancel, false))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferRequest {
    kind: TransferKind,
    sources: Vec<PathBuf>,
    dest_dir: PathBuf,
}

/// Start a copy or move on its own thread. Returns the job id; progress,
/// conflicts and completion arrive as `job-*` events carrying that id.
#[tauri::command]
pub fn start_transfer(app: AppHandle, state: State<'_, OpsState>, request: TransferRequest) -> u64 {
    let id = state.next_job.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    lock(&state.jobs).insert(id, JobControl { cancel: cancel.clone(), answers: tx });

    std::thread::Builder::new()
        .name(format!("sonar-job-{id}"))
        .spawn(move || {
            let TransferRequest { kind, sources, dest_dir } = request;
            let mut observer = EventObserver { app: app.clone(), id, answers: rx };
            let out = ops::run_transfer(kind, &sources, &dest_dir, &cancel, &mut observer);

            let ops_state = app.state::<OpsState>();
            lock(&ops_state.jobs).remove(&id);
            let completed = out.created.len() + out.moved.len();
            match kind {
                TransferKind::Copy if !out.created.is_empty() => ops_state.push_undo(UndoAction::Copy(out.created)),
                TransferKind::Move if !out.moved.is_empty() => ops_state.push_undo(UndoAction::Move(out.moved)),
                _ => {}
            }
            let errors = out.errors.iter().map(ToString::to_string).collect();
            let _ = app.emit(EVENT_JOB_FINISHED, JobFinished { id, kind, cancelled: out.cancelled, completed, errors });
        })
        .expect("failed to spawn job thread");
    id
}

#[tauri::command]
pub fn cancel_job(state: State<'_, OpsState>, id: u64) {
    if let Some(job) = lock(&state.jobs).get(&id) {
        job.cancel.store(true, Ordering::Relaxed);
        // Unblock a job that is waiting on a conflict answer.
        let _ = job.answers.send((ConflictChoice::Cancel, false));
    }
}

#[tauri::command]
pub fn resolve_conflict(state: State<'_, OpsState>, id: u64, choice: ConflictChoice, apply_to_all: bool) {
    if let Some(job) = lock(&state.jobs).get(&id) {
        let _ = job.answers.send((choice, apply_to_all));
    }
}
