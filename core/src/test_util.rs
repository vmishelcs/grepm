//! Helpers shared by the unit tests co-located with each module.
//!
//! Compiled only under `cfg(test)`, so none of this ships. The integration
//! tests in `tests/` can't reach it for exactly that reason — each is its
//! own crate linking the real library — so they keep their own copies in
//! `tests/common/mod.rs`.

use std::fs;
use std::path::Path;

use rusqlite::Connection;

use crate::db::schema;

/// Writes `contents` to `path`, creating parent directories as needed, so a
/// test can lay out an export tree in one line per file.
pub fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// An in-memory database configured and migrated the way [`schema::open`]
/// would leave a real one.
pub fn migrated_connection() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    schema::configure(&conn).unwrap();
    schema::migrate(&mut conn).unwrap();
    conn
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
