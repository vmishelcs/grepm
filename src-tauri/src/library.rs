//! The library: a managed folder of imported databases, plus the index file
//! that names them.
//!
//! Each import is one `<id>.sqlite3`; `index.json` beside them records what
//! each one is called. The index is the source of truth for the launch
//! screen's list, which means it can drift from what's actually on disk — a
//! database deleted by hand stays listed until someone tries to open it. That
//! is a deliberate trade for not opening every file at launch just to read a
//! name; the drift surfaces as [`AppError::ImportFileMissing`].
//!
//! Because the user's chosen name lives in the index rather than in a
//! filename, it never has to satisfy any filesystem's rules — ids are opaque
//! and always safe.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use grepm_core::db::queries::Stats;
use grepm_core::ingest::{self, Progress};
use grepm_core::{db, ingest::import_export_with_progress};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

const INDEX_FILE: &str = "index.json";
const INDEX_TEMP_FILE: &str = ".index.json.tmp";
const DB_EXTENSION: &str = "sqlite3";
const TEMP_DB_PREFIX: &str = ".tmp-";

/// Bumped if the on-disk shape below ever changes. An index written by a
/// newer build is refused rather than misread.
const INDEX_VERSION: u32 = 1;

/// One imported export. Mirrored in `src/lib/ipc/types.ts`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ImportEntry {
    /// Opaque, and also the database's filename stem.
    pub id: String,
    pub name: String,
    pub created_at_ms: i64,
    pub source_path: String,
    pub message_count: i64,
    pub conversation_count: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Index {
    pub version: u32,
    pub imports: Vec<ImportEntry>,
}

impl Default for Index {
    fn default() -> Self {
        Self {
            version: INDEX_VERSION,
            imports: Vec::new(),
        }
    }
}

/// Reads the index, treating "no file yet" as an empty library rather than an
/// error — that's every first launch.
pub fn read_index(dir: &Path) -> Result<Index, AppError> {
    let path = dir.join(INDEX_FILE);
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Index::default()),
        Err(err) => return Err(err.into()),
    };

    let index: Index = serde_json::from_str(&contents).map_err(|err| AppError::CorruptIndex {
        path: path.display().to_string(),
        message: err.to_string(),
    })?;

    if index.version != INDEX_VERSION {
        return Err(AppError::CorruptIndex {
            path: path.display().to_string(),
            message: format!(
                "index version {}, this build understands {INDEX_VERSION}",
                index.version
            ),
        });
    }

    Ok(index)
}

/// Writes the index by replacing it wholesale: to a temp file first, then
/// renamed over the original. A crash mid-write therefore leaves the previous
/// index intact rather than a half-written one — losing the most recent import
/// from the list is recoverable, a truncated index is not.
pub fn write_index(dir: &Path, index: &Index) -> Result<(), AppError> {
    let json = serde_json::to_string_pretty(index).map_err(|err| AppError::Io {
        message: err.to_string(),
    })?;

    let temp = dir.join(INDEX_TEMP_FILE);
    fs::write(&temp, json)?;
    fs::rename(&temp, dir.join(INDEX_FILE))?;
    Ok(())
}

/// Where the database for `id` lives.
pub fn database_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.{DB_EXTENSION}"))
}

/// Where an in-flight import writes before it has earned its real name. The
/// leading dot keeps it out of the way, and nothing lists it, so an import
/// that dies half-done never shows up as a real one.
pub fn temp_database_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{TEMP_DB_PREFIX}{id}.{DB_EXTENSION}"))
}

/// Trims surrounding whitespace. Everything that compares or stores a name
/// goes through here first, so " Work " and "Work" can't become two imports.
pub fn normalize_name(name: &str) -> &str {
    name.trim()
}

/// Finds an existing import whose name collides with `name`, comparing
/// case-insensitively: "Work" and "work" are close enough that having both
/// would be a UI trap rather than a useful distinction.
pub fn find_name_conflict<'a>(index: &'a Index, name: &str) -> Option<&'a ImportEntry> {
    let name = normalize_name(name).to_lowercase();
    index
        .imports
        .iter()
        .find(|entry| entry.name.to_lowercase() == name)
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

/// Picks an id no database in `dir` is using. The timestamp alone is
/// effectively unique; the suffix loop only matters for two imports started in
/// the same millisecond, and for a clock that jumped backwards.
pub fn allocate_id(dir: &Path, created_at_ms: i64) -> String {
    let mut id = created_at_ms.to_string();
    let mut suffix = 1;
    while database_path(dir, &id).exists() || temp_database_path(dir, &id).exists() {
        id = format!("{created_at_ms}-{suffix}");
        suffix += 1;
    }
    id
}

/// Imports `source` into a new database in `dir`, and lists it under `name`.
///
/// Blocking, and slow — minutes for a large export. The caller runs it off the
/// main thread and holds whatever lock stops two imports interleaving on the
/// index; everything here assumes it has the library to itself.
///
/// Nothing is left behind on failure: the database is built under a temporary
/// name and only moved into place, and into the index, once the import has
/// actually succeeded.
pub fn import_into_library(
    dir: &Path,
    source: &Path,
    name: &str,
    created_at_ms: i64,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<ImportEntry, AppError> {
    let name = normalize_name(name);
    if name.is_empty() {
        return Err(AppError::EmptyName);
    }

    // Pre-flight before anything is created: bounded to four levels and
    // touching no conversation folders, so the wrong folder fails in
    // milliseconds instead of after a database exists.
    if ingest::scan::find_messages_root(source).is_err() {
        return Err(AppError::NotAnExport {
            path: source.display().to_string(),
        });
    }

    let mut index = read_index(dir)?;
    if let Some(existing) = find_name_conflict(&index, name) {
        return Err(AppError::NameInUse {
            name: existing.name.clone(),
        });
    }

    let id = allocate_id(dir, created_at_ms);
    let temp_path = temp_database_path(dir, &id);

    let stats = match import_database(&temp_path, source, on_progress) {
        Ok(stats) => stats,
        Err(err) => {
            // A half-imported database is worse than none: it would sit in the
            // folder unreferenced, indistinguishable from a good one.
            let _ = fs::remove_file(&temp_path);
            return Err(err);
        }
    };

    let entry = ImportEntry {
        id,
        name: name.to_string(),
        created_at_ms,
        source_path: source.display().to_string(),
        message_count: stats.message_count,
        conversation_count: stats.conversation_count,
    };

    // Moved into place *before* being listed, so the index never names a file
    // that isn't there yet.
    fs::rename(&temp_path, database_path(dir, &entry.id))?;
    index.imports.push(entry.clone());
    write_index(dir, &index)?;

    Ok(entry)
}

fn import_database(
    db_path: &Path,
    source: &Path,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<Stats, AppError> {
    let mut conn = db::schema::open(db_path)?;
    import_export_with_progress(&mut conn, source, on_progress)?;
    Ok(db::queries::stats(&conn)?)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    /// The synthetic export the repo keeps for exactly this.
    fn samples() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../samples")
    }

    fn import(dir: &Path, name: &str) -> Result<ImportEntry, AppError> {
        import_into_library(dir, &samples(), name, now_ms(), &mut |_| {})
    }

    fn entry(id: &str, name: &str) -> ImportEntry {
        ImportEntry {
            id: id.into(),
            name: name.into(),
            created_at_ms: 1_786_041_256_217,
            source_path: "/home/vm/export".into(),
            message_count: 12,
            conversation_count: 3,
        }
    }

    #[test]
    fn a_library_with_no_index_file_reads_as_empty() {
        let dir = tempdir().unwrap();

        let index = read_index(dir.path()).unwrap();

        assert!(index.imports.is_empty());
    }

    #[test]
    fn an_index_round_trips_through_disk() {
        let dir = tempdir().unwrap();
        let written = Index {
            version: INDEX_VERSION,
            imports: vec![entry("1", "Work chats"), entry("2", "Family")],
        };

        write_index(dir.path(), &written).unwrap();
        let read = read_index(dir.path()).unwrap();

        assert_eq!(read.imports, written.imports);
    }

    #[test]
    fn writing_the_index_leaves_no_temp_file_behind() {
        let dir = tempdir().unwrap();

        write_index(dir.path(), &Index::default()).unwrap();

        assert!(!dir.path().join(INDEX_TEMP_FILE).exists());
    }

    #[test]
    fn a_malformed_index_is_reported_rather_than_silently_emptied() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(INDEX_FILE), "{ not json").unwrap();

        let err = read_index(dir.path()).unwrap_err();

        assert!(
            matches!(err, AppError::CorruptIndex { .. }),
            "an unreadable index must not look like an empty library — that \
             would hide every existing import behind a fresh-install screen"
        );
    }

    #[test]
    fn an_index_from_a_newer_build_is_refused() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join(INDEX_FILE),
            r#"{"version": 99, "imports": []}"#,
        )
        .unwrap();

        assert!(matches!(
            read_index(dir.path()).unwrap_err(),
            AppError::CorruptIndex { .. }
        ));
    }

    #[test]
    fn a_name_conflict_ignores_case_and_surrounding_space() {
        let index = Index {
            version: INDEX_VERSION,
            imports: vec![entry("1", "Work chats")],
        };

        assert!(find_name_conflict(&index, "  work CHATS ").is_some());
        assert!(find_name_conflict(&index, "Work chat").is_none());
    }

    #[test]
    fn an_allocated_id_skips_one_already_on_disk() {
        let dir = tempdir().unwrap();
        fs::write(database_path(dir.path(), "500"), "").unwrap();

        assert_eq!(allocate_id(dir.path(), 500), "500-1");
    }

    #[test]
    fn an_allocated_id_also_skips_an_import_still_in_flight() {
        let dir = tempdir().unwrap();
        fs::write(temp_database_path(dir.path(), "500"), "").unwrap();

        assert_eq!(allocate_id(dir.path(), 500), "500-1");
    }

    #[test]
    fn importing_the_sample_export_lists_it_with_what_it_holds() {
        let dir = tempdir().unwrap();

        let entry = import(dir.path(), "Work chats").unwrap();

        assert_eq!(entry.name, "Work chats");
        assert_eq!(entry.conversation_count, 4);
        assert_eq!(entry.message_count, 51);
        assert!(database_path(dir.path(), &entry.id).exists());
        assert_eq!(read_index(dir.path()).unwrap().imports, vec![entry]);
    }

    #[test]
    fn an_import_reports_progress_as_it_goes() {
        let dir = tempdir().unwrap();
        let mut reports = Vec::new();

        import_into_library(
            dir.path(),
            &samples(),
            "Work chats",
            now_ms(),
            &mut |progress| reports.push((progress.done, progress.total)),
        )
        .unwrap();

        assert_eq!(reports, vec![(0, 4), (1, 4), (2, 4), (3, 4), (4, 4)]);
    }

    #[test]
    fn a_second_import_gets_its_own_database_and_both_stay_listed() {
        let dir = tempdir().unwrap();

        let first = import(dir.path(), "Work chats").unwrap();
        let second = import(dir.path(), "Family").unwrap();

        assert_ne!(first.id, second.id);
        assert!(database_path(dir.path(), &first.id).exists());
        assert!(database_path(dir.path(), &second.id).exists());

        let names: Vec<_> = read_index(dir.path())
            .unwrap()
            .imports
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, vec!["Work chats", "Family"]);
    }

    #[test]
    fn a_name_already_in_use_is_refused_before_anything_is_written() {
        let dir = tempdir().unwrap();
        import(dir.path(), "Work chats").unwrap();

        let err = import(dir.path(), "  WORK CHATS  ").unwrap_err();

        assert!(matches!(err, AppError::NameInUse { .. }));
        assert_eq!(read_index(dir.path()).unwrap().imports.len(), 1);
        assert_eq!(databases_in(dir.path()), 1);
    }

    #[test]
    fn a_folder_that_is_not_an_export_leaves_no_database_behind() {
        let dir = tempdir().unwrap();
        let not_an_export = tempdir().unwrap();

        let err = import_into_library(
            dir.path(),
            not_an_export.path(),
            "Work chats",
            now_ms(),
            &mut |_| {},
        )
        .unwrap_err();

        assert!(matches!(err, AppError::NotAnExport { .. }));
        assert!(read_index(dir.path()).unwrap().imports.is_empty());
        assert_eq!(
            databases_in(dir.path()),
            0,
            "a rejected folder must not leave a database, temporary or otherwise"
        );
    }

    #[test]
    fn an_import_that_fails_partway_removes_its_half_built_database() {
        let dir = tempdir().unwrap();
        let export = tempdir().unwrap();
        let conversation = export.path().join("messages").join("inbox").join("broken");
        fs::create_dir_all(&conversation).unwrap();
        // Enough to satisfy `find_messages_root`, so the import gets past the
        // pre-flight and has created its database before the loader chokes.
        fs::write(conversation.join("message_1.json"), "{ not json").unwrap();

        let err = import_into_library(
            dir.path(),
            export.path(),
            "Work chats",
            now_ms(),
            &mut |_| {},
        )
        .unwrap_err();

        assert!(matches!(err, AppError::Parse { .. }));
        assert!(read_index(dir.path()).unwrap().imports.is_empty());
        assert_eq!(
            databases_in(dir.path()),
            0,
            "the temporary database has to go, or it lingers in the folder \
             forever — unreferenced, and taking an export's worth of disk"
        );
    }

    #[test]
    fn an_empty_name_is_refused() {
        let dir = tempdir().unwrap();

        assert!(matches!(
            import(dir.path(), "   ").unwrap_err(),
            AppError::EmptyName
        ));
    }

    /// Counts every `.sqlite3` in `dir`, temporary ones included — so a test
    /// can assert a failed import cleaned up after itself.
    fn databases_in(dir: &Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == DB_EXTENSION)
            })
            .count()
    }
}
