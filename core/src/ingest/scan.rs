use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::Result;

const MESSAGE_FILE_PREFIX: &str = "message_";
const MESSAGE_FILE_EXTENSION: &str = "json";
const MESSAGES_DIR_NAME: &str = "messages";
const INBOX_DIR_NAME: &str = "inbox";

/// How deep under the export root to search for `messages/inbox`. Real
/// exports put it at depth 2 (`messages/inbox` right at the root) or 3
/// (nested under a dated export folder); the bound keeps the discovery walk
/// from descending into the export's large media trees.
const MESSAGES_ROOT_MAX_DEPTH: usize = 4;

#[derive(Debug)]
pub struct ConversationDir {
    pub folder: PathBuf,
    pub message_files: Vec<PathBuf>,
}

/// Cheaply counts the conversation folders directly under `messages/inbox`,
/// trusting that every direct subdirectory there is a conversation rather
/// than opening each one to look for message files. Fast enough to run as
/// a first pass before the more expensive [`scan`] function.
///
/// Callers that also need [`scan`] should locate the inbox once with
/// [`find_messages_root`] and use [`count_inbox`]/[`scan_inbox`] instead,
/// so the discovery walk isn't repeated.
pub fn count(root: impl AsRef<Path>) -> Result<usize> {
    count_inbox(find_messages_root(root)?)
}

/// [`count`], for an already-located `messages/inbox` directory.
pub fn count_inbox(inbox: impl AsRef<Path>) -> Result<usize> {
    let total = fs::read_dir(inbox.as_ref())?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .count();
    Ok(total)
}

/// Lazily walks the conversation folders directly under `messages/inbox`,
/// yielding one [`ConversationDir`] per conversation with its
/// `message_N.json` files collected and sorted numerically. Errors
/// encountered mid-walk are yielded as `Err` items rather than dropped, so
/// callers can distinguish a partial failure from a conversation that
/// legitimately has no message files (which is simply skipped).
pub fn scan(root: impl AsRef<Path>) -> Result<impl Iterator<Item = Result<ConversationDir>>> {
    Ok(scan_inbox(find_messages_root(root)?))
}

/// [`scan`], for an already-located `messages/inbox` directory. A missing
/// or unreadable inbox surfaces as the iterator's first `Err` item rather
/// than an eager error.
pub fn scan_inbox(inbox: impl AsRef<Path>) -> impl Iterator<Item = Result<ConversationDir>> {
    WalkDir::new(inbox.as_ref())
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => return Some(Err(io::Error::from(err).into())),
            };

            if !entry.file_type().is_dir() {
                return None;
            }

            match message_files_in(entry.path()) {
                Ok(message_files) if message_files.is_empty() => None,
                Ok(mut message_files) => {
                    message_files.sort_unstable_by_key(|&(number, _)| number);
                    Some(Ok(ConversationDir {
                        folder: entry.into_path(),
                        message_files: message_files.into_iter().map(|(_, path)| path).collect(),
                    }))
                }
                Err(err) => Some(Err(err.into())),
            }
        })
}

/// Locates the `messages/inbox` directory somewhere under `root` (searching
/// down to [`MESSAGES_ROOT_MAX_DEPTH`] levels). Facebook's export format
/// doesn't guarantee a fixed depth for it (e.g. it may be nested under a
/// dated export folder), so this searches for it rather than assuming a
/// fixed relative path.
///
/// A directory that can't be read doesn't abort the search — the inbox may
/// still be found elsewhere — but if the search comes up empty, the first
/// such walk error is reported instead of a misleading "not found".
pub fn find_messages_root(root: impl AsRef<Path>) -> Result<PathBuf> {
    let root = root.as_ref();
    validate_root(root)?;

    let mut first_walk_error: Option<walkdir::Error> = None;
    for entry in WalkDir::new(root).max_depth(MESSAGES_ROOT_MAX_DEPTH) {
        match entry {
            Ok(entry) if is_messages_inbox(&entry) => return Ok(entry.into_path()),
            Ok(_) => {}
            Err(err) => {
                if first_walk_error.is_none() {
                    first_walk_error = Some(err);
                }
            }
        }
    }

    Err(match first_walk_error {
        Some(err) => io::Error::from(err).into(),
        None => io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "could not find a {MESSAGES_DIR_NAME}/{INBOX_DIR_NAME} directory under {}",
                root.display()
            ),
        )
        .into(),
    })
}

fn is_messages_inbox(entry: &walkdir::DirEntry) -> bool {
    entry.file_type().is_dir()
        && entry.file_name() == OsStr::new(INBOX_DIR_NAME)
        && entry.path().parent().and_then(Path::file_name) == Some(OsStr::new(MESSAGES_DIR_NAME))
}

fn validate_root(root: &Path) -> io::Result<()> {
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{} is not a directory", root.display()),
        ));
    }
    Ok(())
}

/// Collects `dir`'s `message_N.json` files keyed by `N`. Parsing the number
/// here, where files are admitted, means every returned entry has one by
/// construction — the sort in [`scan`] can key on it with no failure path.
fn message_files_in(dir: &Path) -> io::Result<Vec<(u64, PathBuf)>> {
    let mut message_files = Vec::new();

    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        if let Some(number) = message_number(&path) {
            message_files.push((number, path));
        }
    }

    Ok(message_files)
}

/// Extracts `N` from a `message_N.json` path, so files can be sorted in
/// their real chronological order (`message_2.json` < `message_10.json`)
/// instead of lexicographically.
fn message_number(path: &Path) -> Option<u64> {
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    let extension = path.extension().and_then(|e| e.to_str())?;

    if extension != MESSAGE_FILE_EXTENSION {
        return None;
    }

    stem.strip_prefix(MESSAGE_FILE_PREFIX)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn make_dir(path: &Path) {
        fs::create_dir_all(path).unwrap();
    }

    // --- find_messages_root ---------------------------------------------

    #[test]
    fn find_messages_root_locates_inbox_at_the_expected_depth() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        make_dir(&inbox);

        let found = find_messages_root(export.path()).unwrap();
        assert_eq!(found, inbox);
    }

    #[test]
    fn find_messages_root_locates_inbox_nested_under_extra_directories() {
        // Facebook exports aren't guaranteed to put `messages/` right at the
        // root; it's sometimes nested under a dated export folder.
        let export = tempdir().unwrap();
        let inbox = export
            .path()
            .join("facebook-jdoe-20240101")
            .join("messages")
            .join("inbox");
        make_dir(&inbox);

        let found = find_messages_root(export.path()).unwrap();
        assert_eq!(found, inbox);
    }

    #[test]
    fn find_messages_root_errors_when_inbox_is_missing() {
        let export = tempdir().unwrap();
        make_dir(&export.path().join("messages").join("archived_threads"));

        let result = find_messages_root(export.path());
        assert!(result.is_err());
    }

    #[test]
    fn find_messages_root_errors_when_root_does_not_exist() {
        let missing = std::env::temp_dir().join("grepm_core_definitely_missing_root");

        let result = find_messages_root(&missing);
        assert!(result.is_err());
    }

    #[test]
    fn find_messages_root_does_not_search_below_the_depth_bound() {
        // An inbox buried deeper than MESSAGES_ROOT_MAX_DEPTH is outside
        // anything a real export produces; the bounded walk shouldn't find
        // it (that's the trade for not descending into media trees).
        let export = tempdir().unwrap();
        let inbox = export
            .path()
            .join("a")
            .join("b")
            .join("c")
            .join("d")
            .join("messages")
            .join("inbox");
        make_dir(&inbox);

        assert!(find_messages_root(export.path()).is_err());
    }

    /// Makes `dir` unreadable and returns a guard that restores the
    /// permissions on drop, so the tempdir can be cleaned up even if the
    /// test's assertions panic first.
    #[cfg(unix)]
    fn make_unreadable(dir: &Path) -> impl Drop + '_ {
        use std::os::unix::fs::PermissionsExt;

        struct RestorePermissions<'a>(&'a Path);
        impl Drop for RestorePermissions<'_> {
            fn drop(&mut self) {
                let _ = fs::set_permissions(self.0, fs::Permissions::from_mode(0o755));
            }
        }

        fs::set_permissions(dir, fs::Permissions::from_mode(0o000)).unwrap();
        RestorePermissions(dir)
    }

    #[cfg(unix)]
    #[test]
    fn find_messages_root_reports_a_walk_error_instead_of_a_misleading_not_found() {
        let export = tempdir().unwrap();
        let locked = export.path().join("locked");
        make_dir(&locked);
        let _guard = make_unreadable(&locked);
        if fs::read_dir(&locked).is_ok() {
            // Running as root, where mode 000 is still readable; the
            // scenario can't be constructed, so there's nothing to test.
            return;
        }

        let err = find_messages_root(export.path()).unwrap_err();

        assert!(
            matches!(
                &err,
                crate::Error::Io(io_err) if io_err.kind() == io::ErrorKind::PermissionDenied
            ),
            "an unreadable directory should surface as a permission error, \
             not be swallowed into 'could not find': {err:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn find_messages_root_keeps_searching_past_an_unreadable_directory() {
        let export = tempdir().unwrap();
        let locked = export.path().join("locked");
        make_dir(&locked);
        let inbox = export.path().join("messages").join("inbox");
        make_dir(&inbox);
        let _guard = make_unreadable(&locked);

        let found = find_messages_root(export.path()).unwrap();

        assert_eq!(
            found, inbox,
            "an unreadable sibling directory shouldn't stop the inbox from being found"
        );
    }

    // --- count -------------------------------------------------------------

    #[test]
    fn count_returns_the_number_of_conversation_folders() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(&inbox.join("conv_a").join("message_1.json"), "{}");
        write_file(&inbox.join("conv_b").join("message_1.json"), "{}");
        write_file(&inbox.join("conv_c").join("message_1.json"), "{}");

        assert_eq!(count(export.path()).unwrap(), 3);
    }

    #[test]
    fn count_ignores_stray_files_directly_inside_inbox() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(&inbox.join("conv_a").join("message_1.json"), "{}");
        // Not every real export is guaranteed to be spotless; a stray file
        // sitting directly in inbox/ shouldn't be counted as a conversation.
        write_file(&inbox.join(".DS_Store"), "");

        assert_eq!(count(export.path()).unwrap(), 1);
    }

    #[test]
    fn count_trusts_conversation_folders_without_checking_for_message_files() {
        // count() is documented to be a cheap pass that trusts every
        // directory under inbox/ is a conversation, unlike scan().
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        make_dir(&inbox.join("empty_conversation"));

        assert_eq!(count(export.path()).unwrap(), 1);
    }

    #[test]
    fn count_errors_when_inbox_is_missing() {
        let export = tempdir().unwrap();
        make_dir(export.path());

        assert!(count(export.path()).is_err());
    }

    #[test]
    fn count_errors_when_root_does_not_exist() {
        let missing = std::env::temp_dir().join("grepm_core_definitely_missing_root_2");
        assert!(count(&missing).is_err());
    }

    // --- scan ----------------------------------------------------------

    #[test]
    fn scan_yields_a_conversation_dir_per_conversation_folder() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(&inbox.join("conv_a").join("message_1.json"), "{}");
        write_file(&inbox.join("conv_b").join("message_1.json"), "{}");

        let mut folders: Vec<PathBuf> = scan(export.path())
            .unwrap()
            .map(|c| c.unwrap().folder)
            .collect();
        folders.sort();

        assert_eq!(folders, vec![inbox.join("conv_a"), inbox.join("conv_b")]);
    }

    #[test]
    fn scan_collects_all_message_files_for_a_conversation() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        let conv = inbox.join("conv_a");
        write_file(&conv.join("message_1.json"), "{}");
        write_file(&conv.join("message_2.json"), "{}");
        write_file(&conv.join("message_3.json"), "{}");

        let conversations: Vec<ConversationDir> =
            scan(export.path()).unwrap().map(Result::unwrap).collect();

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].message_files.len(), 3);
    }

    #[test]
    fn scan_sorts_message_files_numerically_not_lexicographically() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        let conv = inbox.join("conv_a");
        // Written out of order, and deliberately including a double-digit
        // index: lexicographic sort would put message_10 before message_2.
        write_file(&conv.join("message_10.json"), "{}");
        write_file(&conv.join("message_1.json"), "{}");
        write_file(&conv.join("message_2.json"), "{}");

        let conversations: Vec<ConversationDir> =
            scan(export.path()).unwrap().map(Result::unwrap).collect();

        assert_eq!(
            conversations[0].message_files,
            vec![
                conv.join("message_1.json"),
                conv.join("message_2.json"),
                conv.join("message_10.json"),
            ]
        );
    }

    #[test]
    fn scan_ignores_files_that_do_not_match_the_message_naming_pattern() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        let conv = inbox.join("conv_a");
        write_file(&conv.join("message_1.json"), "{}");
        write_file(&conv.join("not_a_message.json"), "{}");
        write_file(&conv.join("message_1.txt"), "not json");
        write_file(&conv.join("message_.json"), "{}"); // no digits after prefix

        let conversations: Vec<ConversationDir> =
            scan(export.path()).unwrap().map(Result::unwrap).collect();

        assert_eq!(conversations.len(), 1);
        assert_eq!(
            conversations[0].message_files,
            vec![conv.join("message_1.json")]
        );
    }

    #[test]
    fn scan_does_not_descend_into_conversation_subdirectories() {
        // A conversation folder's photos/gifs/files/etc. subdirectories
        // shouldn't be walked looking for message files.
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        let conv = inbox.join("conv_a");
        write_file(&conv.join("message_1.json"), "{}");
        write_file(&conv.join("photos").join("message_1.json"), "{}");

        let conversations: Vec<ConversationDir> =
            scan(export.path()).unwrap().map(Result::unwrap).collect();

        assert_eq!(conversations.len(), 1);
        assert_eq!(
            conversations[0].message_files,
            vec![conv.join("message_1.json")]
        );
    }

    #[test]
    fn scan_skips_conversation_folders_with_no_message_files() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(
            &inbox.join("conv_with_messages").join("message_1.json"),
            "{}",
        );
        make_dir(&inbox.join("conv_without_messages"));

        let folders: Vec<PathBuf> = scan(export.path())
            .unwrap()
            .map(|c| c.unwrap().folder)
            .collect();

        assert_eq!(folders, vec![inbox.join("conv_with_messages")]);
    }

    #[test]
    fn scan_errors_eagerly_when_inbox_is_missing() {
        let export = tempdir().unwrap();
        make_dir(export.path());

        assert!(scan(export.path()).is_err());
    }

    #[test]
    fn scan_errors_eagerly_when_root_does_not_exist() {
        let missing = std::env::temp_dir().join("grepm_core_definitely_missing_root_3");
        assert!(scan(&missing).is_err());
    }

    #[test]
    fn count_inbox_and_scan_inbox_work_from_a_single_discovery_walk() {
        // The intended count-then-scan flow: locate the inbox once, then
        // run both passes against it directly.
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(&inbox.join("conv_a").join("message_1.json"), "{}");
        write_file(&inbox.join("conv_b").join("message_1.json"), "{}");

        let found = find_messages_root(export.path()).unwrap();

        assert_eq!(count_inbox(&found).unwrap(), 2);

        let mut folders: Vec<PathBuf> = scan_inbox(&found).map(|c| c.unwrap().folder).collect();
        folders.sort();
        assert_eq!(folders, vec![inbox.join("conv_a"), inbox.join("conv_b")]);
    }

    #[test]
    fn count_and_scan_agree_when_every_conversation_has_message_files() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(&inbox.join("conv_a").join("message_1.json"), "{}");
        write_file(&inbox.join("conv_b").join("message_1.json"), "{}");
        write_file(&inbox.join("conv_c").join("message_1.json"), "{}");

        let counted = count(export.path()).unwrap();
        let scanned = scan(export.path()).unwrap().count();

        assert_eq!(counted, scanned);
    }
}
