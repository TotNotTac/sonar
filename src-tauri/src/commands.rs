//! Thin Tauri layer over `sonar_core`. Keep logic in the core crate.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Serialize;
use sonar_core::{breadcrumbs, CoreError, Crumb, DirWatcher, ListOptions, Listing, Place, SortKey};
use tauri::{AppHandle, Emitter, Manager, State};

/// Emitted when the folder a pane shows changes on disk.
pub const EVENT_DIR_CHANGED: &str = "dir-changed";
/// Emitted in batches as folder item counts are computed after a listing.
pub const EVENT_CHILD_COUNTS: &str = "child-counts";
/// Emitted with the full list when drives are attached, mounted or removed.
pub const EVENT_PLACES_CHANGED: &str = "places-changed";

/// Per-pane backend state. A pane is one folder view; a tab holds one or two
/// (split view). Pane ids are chosen by the frontend.
#[derive(Default)]
pub struct ViewState {
    panes: Mutex<HashMap<String, PaneSlot>>,
}

#[derive(Default)]
struct PaneSlot {
    /// Bumped on every `open_dir` for this pane; background count jobs from
    /// an older generation stop as soon as they notice.
    generation: Arc<AtomicU64>,
    watcher: Option<DirWatcher>,
}

impl ViewState {
    fn panes(&self) -> MutexGuard<'_, HashMap<String, PaneSlot>> {
        self.panes.lock().unwrap_or_else(|p| p.into_inner())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirView {
    #[serde(flatten)]
    listing: Listing,
    crumbs: Vec<Crumb>,
    /// Matches `generation` on the `child-counts` events for this listing.
    generation: u64,
    /// False if folder item counts will arrive later via `child-counts`.
    counts_complete: bool,
    /// Set when live updates couldn't be enabled; the listing is still valid.
    watch_error: Option<String>,
    /// When browsing inside an archive: the archive file (contents are read-only).
    archive: Option<PathBuf>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ChildCounts {
    pane: String,
    generation: u64,
    /// `(entry name, count)`; count is `None` for unreadable folders.
    counts: Vec<(String, Option<u64>)>,
    done: bool,
}

#[derive(Clone, Serialize)]
struct DirChanged {
    pane: String,
    path: PathBuf,
}

#[tauri::command]
pub async fn get_places() -> Result<Vec<Place>, CoreError> {
    blocking(sonar_core::places).await
}

/// Mount an attached device (Linux, via udisks). Returns where it was mounted.
#[tauri::command]
pub async fn mount_device(device: String) -> Result<PathBuf, CoreError> {
    blocking(move || sonar_core::platform::mount_device(&device)).await?
}

/// Eject a removable volume (or unmount a network share).
#[tauri::command]
pub async fn eject_volume(path: PathBuf) -> Result<(), CoreError> {
    blocking(move || sonar_core::platform::eject_volume(&path)).await?
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, CoreError> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| CoreError::Other(format!("Background task failed: {e}")))
}

/// Poll the list of places and tell the UI when drives come or go. Free
/// space changes constantly, so usage-only changes are sent at most every 30 s.
pub fn watch_places(app: AppHandle) {
    std::thread::Builder::new()
        .name("sonar-places".into())
        .spawn(move || {
            let structure = |places: &[Place]| {
                let bare: Vec<Place> = places
                    .iter()
                    .cloned()
                    .map(|mut p| {
                        p.usage = None;
                        p
                    })
                    .collect();
                serde_json::to_string(&bare).unwrap_or_default()
            };
            let mut last = structure(&sonar_core::places());
            let mut last_full = Instant::now();
            loop {
                std::thread::sleep(Duration::from_secs(3));
                let places = sonar_core::places();
                let now = structure(&places);
                if now != last || last_full.elapsed() >= Duration::from_secs(30) {
                    let _ = app.emit(EVENT_PLACES_CHANGED, &places);
                    last = now;
                    last_full = Instant::now();
                }
            }
        })
        .expect("failed to spawn places watcher");
}

/// List a folder for a pane, watch it for changes, and count its
/// subfolders' items in the background.
#[tauri::command]
pub async fn open_dir(
    app: AppHandle,
    state: State<'_, ViewState>,
    pane: String,
    path: PathBuf,
    options: Option<ListOptions>,
) -> Result<DirView, CoreError> {
    let counter = state.panes().entry(pane.clone()).or_default().generation.clone();
    let generation = counter.fetch_add(1, Ordering::SeqCst) + 1;
    let mut options = options.unwrap_or_default();
    // Sorting folders by size needs every count up front.
    options.count_children = options.sort.key == SortKey::Size;

    // An archive, or a folder inside one, is listed from the archive itself.
    let (listing, archive) = {
        let path = path.clone();
        tauri::async_runtime::spawn_blocking(move || match sonar_core::archive::split_virtual(&path) {
            Some((archive, inner)) => {
                sonar_core::archive::listing(&archive, &inner, &options).map(|l| (l, Some(archive)))
            }
            None => sonar_core::list_dir(&path, &options).map(|l| (l, None)),
        })
        .await
        .map_err(|e| CoreError::InvalidPath(format!("listing task failed: {e}")))??
    };

    // A newer navigation in this pane started while we were reading; don't
    // touch the watcher or start counting for a folder nobody is looking at.
    let current = counter.load(Ordering::SeqCst) == generation;
    let watch_error = match (&archive, current) {
        (_, false) => None,
        (None, true) => ensure_watching(&app, &state, &pane, &listing.path).err(),
        (Some(_), true) => {
            // Archive contents don't change underneath us; stop watching the old folder.
            if let Some(slot) = state.panes().get_mut(&pane) {
                slot.watcher = None;
            }
            None
        }
    };

    let pending: Vec<(String, PathBuf)> = listing
        .entries
        .iter()
        .filter(|e| e.is_folder() && e.child_count.is_none())
        .map(|e| (e.name.clone(), e.path.clone()))
        .collect();
    let counts_complete = pending.is_empty();
    if current && !counts_complete {
        let pane = pane.clone();
        tauri::async_runtime::spawn_blocking(move || count_children(app, pane, counter, generation, pending));
    }

    let crumbs = breadcrumbs(&listing.path, dirs::home_dir().as_deref());
    Ok(DirView {
        listing,
        crumbs,
        generation,
        counts_complete,
        watch_error: watch_error.map(|e| e.to_string()),
        archive,
    })
}

/// Forget a closed pane: stops its watcher and any count job.
#[tauri::command]
pub fn close_pane(state: State<'_, ViewState>, pane: String) {
    if let Some(slot) = state.panes().remove(&pane) {
        slot.generation.fetch_add(1, Ordering::SeqCst);
    }
}

/// Point the pane's watcher at `path`, reusing it if it's already there.
fn ensure_watching(app: &AppHandle, state: &ViewState, pane: &str, path: &Path) -> Result<(), CoreError> {
    let mut panes = state.panes();
    let Some(slot) = panes.get_mut(pane) else { return Ok(()) }; // closed meanwhile
    if slot.watcher.as_ref().is_some_and(|w| w.path() == path) {
        return Ok(());
    }
    slot.watcher = None; // stop watching the old folder even if the new watch fails
    let app = app.clone();
    let changed = DirChanged { pane: pane.to_string(), path: path.to_path_buf() };
    slot.watcher = Some(DirWatcher::new(path, sonar_core::watch::DEFAULT_DEBOUNCE, move || {
        let _ = app.emit(EVENT_DIR_CHANGED, changed.clone());
    })?);
    Ok(())
}

/// Count items in each folder, emitting results in small batches so the UI
/// fills in progressively. Stops early if the pane navigates elsewhere.
fn count_children(
    app: AppHandle,
    pane: String,
    current: Arc<AtomicU64>,
    generation: u64,
    folders: Vec<(String, PathBuf)>,
) {
    const FLUSH_EVERY: Duration = Duration::from_millis(100);
    let mut batch = Vec::new();
    let mut last_flush = Instant::now();
    let emit = |counts, done| {
        let _ = app.emit(EVENT_CHILD_COUNTS, ChildCounts { pane: pane.clone(), generation, counts, done });
    };

    for (name, path) in folders {
        if current.load(Ordering::Relaxed) != generation {
            return;
        }
        batch.push((name, sonar_core::child_count(&path)));
        if last_flush.elapsed() >= FLUSH_EVERY {
            emit(std::mem::take(&mut batch), false);
            last_flush = Instant::now();
        }
    }
    emit(batch, true);
}

/// Open a file (or macOS app bundle) with the system's default handler.
#[tauri::command]
pub async fn open_path(path: PathBuf) -> Result<(), CoreError> {
    tauri::async_runtime::spawn_blocking(move || {
        // A file inside an archive: extract just that file to a temp folder first.
        let real = match sonar_core::archive::split_virtual(&path) {
            Some((archive, inner)) if !inner.is_empty() => sonar_core::archive::extract_for_opening(&archive, &inner)?,
            _ => path,
        };
        sonar_core::open_path(&real)
    })
    .await
    .map_err(|e| CoreError::InvalidPath(format!("open task failed: {e}")))?
}

/// Resolve text typed into the editable location bar.
#[tauri::command]
pub fn resolve_path(input: String, cwd: PathBuf) -> Result<PathBuf, CoreError> {
    sonar_core::resolve_user_path(&input, &cwd)
}

#[tauri::command]
pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

// ---------- Search ----------

pub const EVENT_SEARCH_RESULTS: &str = "search-results";

/// One running search per pane; starting another cancels it.
#[derive(Default)]
pub struct SearchState(Mutex<HashMap<String, (u64, Arc<std::sync::atomic::AtomicBool>)>>);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SearchResults {
    pane: String,
    generation: u64,
    hits: Vec<sonar_core::search::Hit>,
    folders_scanned: u64,
    done: bool,
    summary: Option<sonar_core::search::SearchSummary>,
}

/// Search below `root` for `query`; results stream as `search-results`
/// events tagged with the returned generation.
#[tauri::command]
pub fn start_search(
    app: AppHandle,
    state: State<'_, SearchState>,
    pane: String,
    root: PathBuf,
    query: String,
    show_hidden: bool,
) -> u64 {
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let generation = {
        let mut map = state.0.lock().unwrap_or_else(|p| p.into_inner());
        let entry = map.entry(pane.clone()).or_insert((0, cancel.clone()));
        entry.1.store(true, Ordering::Relaxed); // stop the previous search
        *entry = (entry.0 + 1, cancel.clone());
        entry.0
    };
    std::thread::Builder::new()
        .name("sonar-search".into())
        .spawn(move || {
            let emit = |hits, folders_scanned, done, summary| {
                let payload = SearchResults { pane: pane.clone(), generation, hits, folders_scanned, done, summary };
                let _ = app.emit(EVENT_SEARCH_RESULTS, payload);
            };
            let summary = sonar_core::search::search(&root, &query, show_hidden, &cancel, &mut |hits, scanned| {
                if !hits.is_empty() {
                    emit(hits, scanned, false, None);
                }
            });
            if !summary.cancelled {
                emit(Vec::new(), summary.folders_scanned, true, Some(summary));
            }
        })
        .expect("failed to spawn search thread");
    generation
}

#[tauri::command]
pub fn cancel_search(state: State<'_, SearchState>, pane: String) {
    if let Some((_, cancel)) = state.0.lock().unwrap_or_else(|p| p.into_inner()).get(&pane) {
        cancel.store(true, Ordering::Relaxed);
    }
}

// ---------- Properties ----------

pub const EVENT_SIZE_PROGRESS: &str = "size-progress";

/// Running size computations, by a token the frontend chooses.
#[derive(Default)]
pub struct SizeJobs(Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SizeProgress {
    token: String,
    totals: sonar_core::properties::SizeTotals,
    done: bool,
}

#[tauri::command]
pub async fn get_properties(paths: Vec<PathBuf>) -> Result<Vec<sonar_core::properties::Properties>, CoreError> {
    tauri::async_runtime::spawn_blocking(move || paths.iter().map(|p| sonar_core::properties::properties(p)).collect())
        .await
        .map_err(|e| CoreError::Other(e.to_string()))?
}

/// Total size and item counts of `paths`, streamed as `size-progress` events.
#[tauri::command]
pub fn start_size(app: AppHandle, jobs: State<'_, SizeJobs>, token: String, paths: Vec<PathBuf>) {
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    jobs.0.lock().unwrap_or_else(|p| p.into_inner()).insert(token.clone(), cancel.clone());
    std::thread::spawn(move || {
        let emit = |totals, done| {
            let _ = app.emit(EVENT_SIZE_PROGRESS, SizeProgress { token: token.clone(), totals, done });
        };
        if let Some(t) = sonar_core::properties::total_size(&paths, &cancel, &mut |t| emit(t, false)) {
            emit(t, true);
        }
        app.state::<SizeJobs>().0.lock().unwrap_or_else(|p| p.into_inner()).remove(&token);
    });
}

#[tauri::command]
pub fn cancel_size(jobs: State<'_, SizeJobs>, token: String) {
    if let Some(c) = jobs.0.lock().unwrap_or_else(|p| p.into_inner()).get(&token) {
        c.store(true, Ordering::Relaxed);
    }
}
