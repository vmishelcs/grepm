//! The Tauri command surface.
//!
//! Every command here goes through `grepm_core`'s public API — `db::schema`,
//! `db::queries`, `ingest` — and never writes SQL of its own. See `CLAUDE.md`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use grepm_core::db;
use grepm_core::db::queries::Stats;
use grepm_core::db::Connection;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::error::AppError;
use crate::library::{self, ImportEntry};

/// Emitted once before the first conversation and once after each one. The
/// payload is `grepm_core::ingest::Progress`.
pub const IMPORT_PROGRESS_EVENT: &str = "import:progress";

/// The database the user currently has open.
struct ActiveImport {
    entry: ImportEntry,
    conn: Connection,
}

pub struct AppState {
    /// The managed imports folder, kept *behind* the lock rather than beside
    /// it so there's no way to touch the index without serialising against
    /// everyone else who might be rewriting it. `Arc` so an import can carry
    /// the lock onto a blocking thread and hold it for its whole run.
    library: Arc<Mutex<PathBuf>>,
    /// `rusqlite::Connection` is `Send` but not `Sync`, so it has to sit
    /// behind a mutex to satisfy Tauri's `Send + Sync` bound on managed state.
    active: Mutex<Option<ActiveImport>>,
}

impl AppState {
    pub fn new(imports_dir: PathBuf) -> Self {
        Self {
            library: Arc::new(Mutex::new(imports_dir)),
            active: Mutex::new(None),
        }
    }
}

/// A panic while one of these locks is held leaves the guarded value perfectly
/// usable — a path, and a connection that can be reopened — so recovering beats
/// poisoning every later command in the process.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// What the open database holds, read from the live connection rather than
/// from the index — so a screen showing these has proven the file really is
/// open, which is the whole point of the placeholder route.
#[derive(Serialize)]
pub struct ActiveImportInfo {
    entry: ImportEntry,
    stats: Stats,
}

#[tauri::command]
pub fn list_imports(state: State<'_, AppState>) -> Result<Vec<ImportEntry>, AppError> {
    let dir = lock(&state.library);
    Ok(library::read_index(&dir)?.imports)
}

#[tauri::command]
pub fn active_import(state: State<'_, AppState>) -> Result<Option<ActiveImportInfo>, AppError> {
    let active = lock(&state.active);
    let Some(active) = active.as_ref() else {
        return Ok(None);
    };

    Ok(Some(ActiveImportInfo {
        entry: active.entry.clone(),
        stats: db::queries::stats(&active.conn)?,
    }))
}

#[tauri::command]
pub fn open_import(state: State<'_, AppState>, id: String) -> Result<ImportEntry, AppError> {
    let dir = lock(&state.library);
    let entry = library::read_index(&dir)?
        .imports
        .into_iter()
        .find(|entry| entry.id == id)
        .ok_or_else(|| AppError::UnknownImport { id: id.clone() })?;

    let path = library::database_path(&dir, &entry.id);
    // Checked before opening, because `schema::open` would happily *create* a
    // database at this path — an import deleted behind the app's back would
    // silently come back as an empty one instead of reporting itself missing.
    if !path.exists() {
        return Err(AppError::ImportFileMissing { name: entry.name });
    }

    let conn = db::schema::open(&path)?;
    *lock(&state.active) = Some(ActiveImport {
        entry: entry.clone(),
        conn,
    });

    Ok(entry)
}

#[tauri::command]
pub async fn start_import(
    app: AppHandle,
    state: State<'_, AppState>,
    source_path: String,
    name: String,
) -> Result<ImportEntry, AppError> {
    let library = Arc::clone(&state.library);
    let source = PathBuf::from(source_path);
    let created_at_ms = library::now_ms();

    tauri::async_runtime::spawn_blocking(move || {
        // The lock is held for the whole import, so two of them can't
        // interleave on the index. Nothing else needs it meanwhile: the launch
        // screen — the only caller of `list_imports` — is replaced by the
        // progress view for the duration.
        let dir = lock(&library);
        library::import_into_library(&dir, &source, &name, created_at_ms, &mut |progress| {
            // Progress is advisory. If the window has gone away there is
            // nobody left to tell, and the import should still run to
            // completion rather than fail on a dead channel.
            let _ = app.emit(IMPORT_PROGRESS_EVENT, progress);
        })
    })
    .await
    .map_err(|err| AppError::Io {
        message: format!("the import task did not finish: {err}"),
    })?
}
