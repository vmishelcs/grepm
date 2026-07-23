use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

const MESSAGE_FILE_PREFIX: &str = "message_";
const MESSAGE_FILE_EXTENSION: &str = "json";
const MESSAGES_DIR_NAME: &str = "messages";
const INBOX_DIR_NAME: &str = "inbox";

#[derive(Debug)]
pub struct ConversationDir {
    pub folder: PathBuf,
    pub raw_name: String,
    pub message_files: Vec<PathBuf>,
}

/// Cheaply counts the conversation folders directly under `messages/inbox`,
/// trusting that every direct subdirectory there is a conversation rather
/// than opening each one to look for message files. Fast enough to run as
/// a first pass before the more expensive [`scan`] function.
pub fn count(root: impl AsRef<Path>) -> io::Result<usize> {
    let inbox = find_messages_root(root)?;
    let total = std::fs::read_dir(inbox)?
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
pub fn scan(
    root: impl AsRef<Path>,
) -> io::Result<impl Iterator<Item = io::Result<ConversationDir>>> {
    let inbox = find_messages_root(root)?;

    Ok(WalkDir::new(inbox)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .filter_map(|entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => return Some(Err(err.into())),
            };

            if !entry.file_type().is_dir() {
                return None;
            }

            match message_files_in(entry.path()) {
                Ok(message_files) if message_files.is_empty() => None,
                Ok(mut message_files) => {
                    message_files.sort_by_key(|path| message_number(path).unwrap());
                    let raw_name = entry.file_name().to_string_lossy().into_owned();
                    Some(Ok(ConversationDir {
                        folder: entry.into_path(),
                        raw_name,
                        message_files,
                    }))
                }
                Err(err) => Some(Err(err)),
            }
        }))
}

/// Locates the `messages/inbox` directory somewhere under `root`. Facebook's
/// export format doesn't guarantee a fixed depth for it (e.g. it may be
/// nested under a dated export folder), so this searches for it rather than
/// assuming a fixed relative path.
pub fn find_messages_root(root: impl AsRef<Path>) -> io::Result<PathBuf> {
    let root = root.as_ref();
    validate_root(root)?;

    WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| {
            entry.file_type().is_dir()
                && entry.file_name() == OsStr::new(INBOX_DIR_NAME)
                && entry.path().parent().and_then(Path::file_name)
                    == Some(OsStr::new(MESSAGES_DIR_NAME))
        })
        .map(|entry| entry.into_path())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "could not find a {MESSAGES_DIR_NAME}/{INBOX_DIR_NAME} directory under {}",
                    root.display()
                ),
            )
        })
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

fn message_files_in(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut message_files = Vec::new();

    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if is_message_file(&path) {
            message_files.push(path);
        }
    }

    Ok(message_files)
}

fn is_message_file(path: &Path) -> bool {
    path.is_file() && message_number(path).is_some()
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
    fn scan_populates_raw_name_from_folder_name() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(
            &inbox
                .join("zoeyuan_1980919652003672")
                .join("message_1.json"),
            "{}",
        );

        let conversations: Vec<ConversationDir> =
            scan(export.path()).unwrap().map(Result::unwrap).collect();

        assert_eq!(conversations.len(), 1);
        assert_eq!(conversations[0].raw_name, "zoeyuan_1980919652003672");
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
