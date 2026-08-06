//! Helpers shared by the integration tests in this directory.
//!
//! These mirror `src/test_util.rs`, which serves the unit tests. The
//! duplication is structural, not an oversight: `test_util` is `cfg(test)`
//! within the library, and each file here compiles as its own crate linking
//! the real, published library — so there is nothing to import.

// Each integration test crate pulls in this whole module but uses only part
// of it, which would otherwise warn in whichever crate doesn't use a given
// helper.
#![allow(dead_code)]

use std::fs;
use std::path::Path;

use rusqlite::Connection;

use grepm_core::db;

/// Writes `contents` to `path`, creating parent directories as needed, so a
/// test can lay out an export tree in one line per file.
pub fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Opens a real on-disk SQLite database (as opposed to the in-memory ones
/// the unit tests use), migrated and ready to import into.
pub fn open_db(dir: &Path) -> Connection {
    db::schema::open(&dir.join("grepm.sqlite3")).unwrap()
}

/// Makes `dir` unreadable and returns a guard that restores the permissions
/// on drop, so the tempdir can be cleaned up even if the test's assertions
/// panic first.
///
/// Returns `None` when the permissions didn't actually take effect, which
/// happens when the tests run as root: mode 000 is still readable there, so
/// the scenario can't be constructed and the caller should skip.
#[cfg(unix)]
pub fn make_unreadable(dir: &Path) -> Option<impl Drop + '_> {
    use std::os::unix::fs::PermissionsExt;

    struct RestorePermissions<'a>(&'a Path);
    impl Drop for RestorePermissions<'_> {
        fn drop(&mut self) {
            let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
        }
    }

    fs::set_permissions(dir, fs::Permissions::from_mode(0o000)).unwrap();
    let guard = RestorePermissions(dir);

    fs::read_dir(dir).err().map(|_| guard)
}
