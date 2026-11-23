//! File-operation commands: rename, new folder, trash, delete, background
//! copy/move jobs, and undo. The logic lives in `sonar_core::ops`.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use sonar_core::archive;
use sonar_core::ops::{self, Conflict, ConflictChoice, Progress, TransferKind, TransferObserver, UndoAction};
use sonar_core::uipath::{self, UiPath};
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
pub async fn rename_entry(app: AppHandle, path: UiPath, new_name: String) -> Result<UiPath, CoreError> {
    let path = path.0;
    let from = path.clone();
    // The UI shows undecodable bytes as U+FFFD, so a typed name is normally
    // plain text; one that still carries encoded bytes is decoded like a path.
    let new_name = uipath::decode(&new_name)
        .into_string()
        .map_err(|_| CoreError::InvalidName("The name contains characters that are not valid text".into()))?;
    let to = blocking(move || ops::rename(&path, &new_name)).await??;
    if to != from {
        app.state::<OpsState>().push_undo(UndoAction::Rename { from, to: to.clone() });
    }
    Ok(to.into())
}

/// Create "New Folder" (or "New Folder (n)") in `dir`; the UI then starts an inline rename.
#[tauri::command]
pub async fn create_folder(app: AppHandle, dir: UiPath) -> Result<UiPath, CoreError> {
    let dir = dir.0;
    let path = blocking(move || ops::new_folder(&dir)).await??;
    app.state::<OpsState>().push_undo(UndoAction::NewFolder(path.clone()));
    Ok(path.into())
}

#[tauri::command]
pub async fn trash_entries(app: AppHandle, paths: Vec<UiPath>) -> Result<BatchReport, CoreError> {
    let paths = uipath::paths(paths);
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
pub async fn delete_entries(paths: Vec<UiPath>) -> Result<BatchReport, CoreError> {
    let paths = uipath::paths(paths);
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
    /// "copy", "move", "extract" or "compress".
    kind: &'static str,
    cancelled: bool,
    /// Top-level items that were fully transferred.
    completed: usize,
    errors: Vec<String>,
    /// Top-level items the job created (for selecting them afterwards).
    #[serde(with = "uipath::vec")]
    created: Vec<PathBuf>,
    /// An encrypted archive stopped the job: ask for its password and retry.
    #[serde(skip_serializing_if = "Option::is_none")]
    needs_password: Option<NeedsPassword>,
}

#[derive(Clone, Serialize)]
struct NeedsPassword {
    #[serde(with = "uipath::path")]
    path: PathBuf,
    wrong: bool,
}

impl JobFinished {
    fn new(id: u64, kind: &'static str) -> Self {
        JobFinished { id, kind, cancelled: false, completed: 0, errors: vec![], created: vec![], needs_password: None }
    }

    fn failed(id: u64, kind: &'static str, err: CoreError) -> Self {
        let needs_password = match &err {
            CoreError::PasswordRequired { archive, wrong } => {
                Some(NeedsPassword { path: archive.clone(), wrong: *wrong })
            }
            _ => None,
        };
        let errors = if needs_password.is_some() { vec![] } else { vec![err.to_string()] };
        JobFinished { errors, needs_password, ..JobFinished::new(id, kind) }
    }
}

const fn kind_name(kind: TransferKind) -> &'static str {
    match kind {
        TransferKind::Copy => "copy",
        TransferKind::Move => "move",
    }
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
    #[serde(with = "uipath::vec")]
    sources: Vec<PathBuf>,
    #[serde(with = "uipath::path")]
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
            let ops_state = app.state::<OpsState>();
            // Items browsed inside an archive are copied out by extracting them.
            if sources.first().and_then(|s| archive::split_virtual(s)).is_some_and(|(_, inner)| !inner.is_empty()) {
                let fin = extract_entries(&app, id, &cancel, &sources, &dest_dir);
                lock(&ops_state.jobs).remove(&id);
                let _ = app.emit(EVENT_JOB_FINISHED, fin);
                return;
            }
            if archive::split_virtual(&dest_dir).is_some() {
                lock(&ops_state.jobs).remove(&id);
                let errors = vec![format!("“{}” is an archive; its contents are read-only", dest_dir.display())];
                let fin = JobFinished { errors, ..JobFinished::new(id, kind_name(kind)) };
                let _ = app.emit(EVENT_JOB_FINISHED, fin);
                return;
            }
            let mut observer = EventObserver { app: app.clone(), id, answers: rx };
            let out = ops::run_transfer(kind, &sources, &dest_dir, &cancel, &mut observer);

            lock(&ops_state.jobs).remove(&id);
            let completed = out.created.len() + out.moved.len();
            let created: Vec<PathBuf> =
                out.created.iter().cloned().chain(out.moved.iter().map(|(_, to)| to.clone())).collect();
            match kind {
                TransferKind::Copy if !out.created.is_empty() => ops_state.push_undo(UndoAction::Copy(out.created)),
                TransferKind::Move if !out.moved.is_empty() => ops_state.push_undo(UndoAction::Move(out.moved)),
                _ => {}
            }
            let errors = out.errors.iter().map(ToString::to_string).collect();
            let fin = JobFinished {
                cancelled: out.cancelled,
                completed,
                errors,
                created,
                ..JobFinished::new(id, kind_name(kind))
            };
            let _ = app.emit(EVENT_JOB_FINISHED, fin);
        })
        .expect("failed to spawn job thread");
    id
}

// ---------- Archive jobs ----------

/// Emits `job-progress` for archive jobs (entry counts), at most every 100 ms.
fn archive_progress(app: &AppHandle, id: u64) -> impl FnMut(u64, u64, &str) + '_ {
    let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
    move |done, total, current| {
        if done < total && last.elapsed() < std::time::Duration::from_millis(100) {
            return;
        }
        last = std::time::Instant::now();
        let progress =
            Progress { total_items: total, done_items: done, current: Some(current.to_string()), ..Default::default() };
        let _ = app.emit(EVENT_JOB_PROGRESS, JobProgress { id, progress });
    }
}

/// Copy items out of a browsed archive folder into `dest_dir`.
fn extract_entries(
    app: &AppHandle,
    id: u64,
    cancel: &AtomicBool,
    sources: &[PathBuf],
    dest_dir: &std::path::Path,
) -> JobFinished {
    let fail = |msg: &str| JobFinished::failed(id, "copy", CoreError::Other(msg.into()));
    let Some(dir) = sources.first().and_then(|s| s.parent()) else { return fail("Nothing to copy") };
    if sources.iter().any(|s| s.parent() != Some(dir)) {
        return fail("Items from different archive folders can't be copied together");
    }
    let names: Vec<String> =
        sources.iter().filter_map(|s| s.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
    match archive::extract_items(dir, &names, dest_dir, cancel, &mut archive_progress(app, id)) {
        Ok(Some(created)) => {
            if !created.is_empty() {
                app.state::<OpsState>().push_undo(UndoAction::Created("Extract", created.clone()));
            }
            JobFinished { completed: created.len(), created, ..JobFinished::new(id, "copy") }
        }
        Ok(None) => JobFinished { cancelled: true, ..JobFinished::new(id, "copy") },
        Err(e) => JobFinished::failed(id, "copy", e),
    }
}

/// Run `work` as a cancellable background job reporting through `job-*` events.
fn spawn_job(
    app: AppHandle,
    state: &OpsState,
    work: impl FnOnce(&AppHandle, u64, &AtomicBool) -> JobFinished + Send + 'static,
) -> u64 {
    let id = state.next_job.fetch_add(1, Ordering::Relaxed) + 1;
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, _rx) = mpsc::channel(); // archive jobs never ask about conflicts
    lock(&state.jobs).insert(id, JobControl { cancel: cancel.clone(), answers: tx });
    std::thread::Builder::new()
        .name(format!("sonar-job-{id}"))
        .spawn(move || {
            let fin = work(&app, id, &cancel);
            lock(&app.state::<OpsState>().jobs).remove(&id);
            let _ = app.emit(EVENT_JOB_FINISHED, fin);
        })
        .expect("failed to spawn job thread");
    id
}

/// "Extract Here": each archive next to itself (or into `dest_dir`), with
/// Ark-style subfolder detection. Never overwrites.
#[tauri::command]
pub fn start_extract(
    app: AppHandle,
    state: State<'_, OpsState>,
    archives: Vec<UiPath>,
    dest_dir: Option<UiPath>,
) -> u64 {
    let archives = uipath::paths(archives);
    let dest_dir = dest_dir.map(PathBuf::from);
    spawn_job(app, &state, move |app, id, cancel| {
        let mut created = Vec::new();
        let mut errors = Vec::new();
        let mut needs_password = None;
        for a in &archives {
            let Some(dest) = dest_dir.clone().or_else(|| a.parent().map(PathBuf::from)) else { continue };
            match archive::extract(a, &dest, cancel, &mut archive_progress(app, id)) {
                Ok(Some(items)) => created.extend(items),
                Ok(None) => break,
                Err(e @ CoreError::PasswordRequired { .. }) => {
                    // Stop here; the UI asks for the password and retries
                    // from this archive on.
                    needs_password = JobFinished::failed(id, "extract", e).needs_password;
                    break;
                }
                Err(e) => errors.push(e.to_string()),
            }
        }
        if !created.is_empty() {
            app.state::<OpsState>().push_undo(UndoAction::Created("Extract", created.clone()));
        }
        let cancelled = cancel.load(Ordering::Relaxed);
        JobFinished {
            cancelled,
            completed: created.len(),
            errors,
            created,
            needs_password,
            ..JobFinished::new(id, "extract")
        }
    })
}

/// "Compress": `sources` (siblings) into a new archive next to them, named
/// after the item (or "Archive") without overwriting anything.
#[tauri::command]
pub fn start_compress(
    app: AppHandle,
    state: State<'_, OpsState>,
    sources: Vec<UiPath>,
    format: archive::Format,
) -> u64 {
    let sources = uipath::paths(sources);
    spawn_job(app, &state, move |app, id, cancel| {
        let new = || JobFinished::new(id, "compress");
        let Some(parent) = sources.first().and_then(|s| s.parent()) else {
            return JobFinished::failed(id, "compress", CoreError::Other("Nothing to compress".into()));
        };
        let dest = ops::available_name(parent, &archive::default_name(&sources, format), false);
        match archive::compress(&sources, &dest, format, cancel, &mut archive_progress(app, id)) {
            Ok(Some(path)) => {
                app.state::<OpsState>().push_undo(UndoAction::Created("Compress", vec![path.clone()]));
                JobFinished { completed: 1, created: vec![path], ..new() }
            }
            Ok(None) => JobFinished { cancelled: true, ..new() },
            Err(e) => JobFinished::failed(id, "compress", e),
        }
    })
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
