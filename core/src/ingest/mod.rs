use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::db::schema;
use crate::Result;

pub mod loader;
pub mod parse;
pub mod scan;

/// How far along an import is, in conversations.
///
/// `total` is an *upper bound*, not a promise: it comes from
/// [`scan::count_inbox`], which counts every folder under `messages/inbox`
/// without opening any of them, while the import itself skips folders that
/// turn out to hold no `message_N.json` files. So a finished import can end
/// with `done < total`, and a caller driving a progress bar should treat the
/// call returning — not `done == total` — as completion.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

/// Imports every conversation found under `export_root` (a Facebook data
/// export's root directory, i.e. the directory containing `messages/`) into
/// the database at `conn`, then (re)builds the full-text search index over
/// every imported message.
pub fn import_export(conn: &mut Connection, export_root: impl AsRef<Path>) -> Result<()> {
    import_export_with_progress(conn, export_root, &mut |_| {})
}

/// [`import_export`], reporting progress as it goes.
///
/// `on_progress` is called once with `done: 0` before any work starts (so a
/// caller learns `total` up front, and can show a bar rather than a spinner
/// immediately), then once after each conversation is committed. It is called
/// on this thread, synchronously, between conversations — an expensive
/// callback slows the import down.
///
/// No progress is reported for the final [`schema::populate_fts`] pass, which
/// is a single bulk statement with nothing to subdivide.
pub fn import_export_with_progress(
    conn: &mut Connection,
    export_root: impl AsRef<Path>,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    // Locate the inbox once and reuse it for both the count and the walk;
    // `scan`/`count` would each repeat the discovery walk.
    let inbox = scan::find_messages_root(export_root)?;
    let total = scan::count_inbox(&inbox)?;
    on_progress(Progress { done: 0, total });

    for (index, conversation_dir) in scan::scan_inbox(&inbox).enumerate() {
        loader::load_conversation(conn, &conversation_dir?)?;
        on_progress(Progress {
            done: index + 1,
            total,
        });
    }

    schema::populate_fts(conn)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;
    use crate::test_util::{migrated_connection, write_file};

    const MESSAGE_ALICE_AND_BOB: &str = r#"{
        "participants": [{"name": "Alice"}, {"name": "Bob"}],
        "messages": [
            {"sender_name": "Alice", "timestamp_ms": 1000, "content": "hi"},
            {"sender_name": "Bob", "timestamp_ms": 2000, "content": "hello"}
        ],
        "title": "Alice and Bob",
        "is_still_participant": true,
        "thread_path": "inbox/alice_and_bob"
    }"#;

    const MESSAGE_ALICE_AND_CAROL: &str = r#"{
        "participants": [{"name": "Alice"}, {"name": "Carol"}],
        "messages": [
            {"sender_name": "Carol", "timestamp_ms": 3000, "content": "hey"}
        ],
        "title": "Alice and Carol",
        "is_still_participant": true,
        "thread_path": "inbox/alice_and_carol"
    }"#;

    const MESSAGE_WITH_DIACRITICS: &str = r#"{
        "participants": [{"name": "Alice"}],
        "messages": [
            {"sender_name": "Alice", "timestamp_ms": 4000, "content": "let's grab café"}
        ],
        "title": "Alice",
        "is_still_participant": true,
        "thread_path": "inbox/alice_solo"
    }"#;

    #[test]
    fn import_export_loads_every_conversation_found_under_the_export_root() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(
            &inbox.join("alice_and_bob").join("message_1.json"),
            MESSAGE_ALICE_AND_BOB,
        );
        write_file(
            &inbox.join("alice_and_carol").join("message_1.json"),
            MESSAGE_ALICE_AND_CAROL,
        );

        let mut conn = migrated_connection();
        import_export(&mut conn, export.path()).unwrap();

        let conversation_count: i64 = conn
            .query_row("SELECT count(*) FROM conversations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(conversation_count, 2);

        let message_count: i64 = conn
            .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(message_count, 3);
    }

    #[test]
    fn import_export_builds_a_diacritic_insensitive_fts_index() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(
            &inbox.join("alice_and_bob").join("message_1.json"),
            MESSAGE_ALICE_AND_BOB,
        );
        write_file(
            &inbox.join("alice_solo").join("message_1.json"),
            MESSAGE_WITH_DIACRITICS,
        );

        let mut conn = migrated_connection();
        import_export(&mut conn, export.path()).unwrap();

        let match_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH 'hi'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(match_count, 1);

        let diacritic_insensitive_match_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH 'cafe'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            diacritic_insensitive_match_count, 1,
            "searching 'cafe' should match content containing 'café' \
             (unicode61 remove_diacritics 2)"
        );
    }

    #[test]
    fn import_export_with_progress_reports_before_starting_and_after_each_conversation() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(
            &inbox.join("alice_and_bob").join("message_1.json"),
            MESSAGE_ALICE_AND_BOB,
        );
        write_file(
            &inbox.join("alice_and_carol").join("message_1.json"),
            MESSAGE_ALICE_AND_CAROL,
        );

        let mut conn = migrated_connection();
        let mut reports = Vec::new();
        import_export_with_progress(&mut conn, export.path(), &mut |progress| {
            reports.push((progress.done, progress.total))
        })
        .unwrap();

        // The leading (0, 2) is what lets a caller show a determinate bar
        // before the first conversation has been committed.
        assert_eq!(reports, vec![(0, 2), (1, 2), (2, 2)]);
    }

    #[test]
    fn import_export_with_progress_treats_total_as_an_upper_bound() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(
            &inbox.join("alice_and_bob").join("message_1.json"),
            MESSAGE_ALICE_AND_BOB,
        );
        // A conversation folder with no `message_N.json` files. `count_inbox`
        // counts it without looking inside; `scan_inbox` skips it. The gap is
        // documented on `Progress` and is why callers must not wait for
        // `done == total` to call an import finished.
        fs::create_dir_all(inbox.join("empty_conv")).unwrap();

        let mut conn = migrated_connection();
        let mut reports = Vec::new();
        import_export_with_progress(&mut conn, export.path(), &mut |progress| {
            reports.push((progress.done, progress.total))
        })
        .unwrap();

        assert_eq!(reports, vec![(0, 2), (1, 2)]);
        let (done, total) = *reports.last().unwrap();
        assert!(done < total, "the skipped folder should leave a shortfall");
    }

    #[test]
    fn import_export_errors_when_the_export_root_has_no_messages_inbox() {
        let export = tempdir().unwrap();

        let mut conn = migrated_connection();
        let result = import_export(&mut conn, export.path());

        assert!(result.is_err());
    }

    #[test]
    fn import_export_propagates_a_malformed_conversation_file_error() {
        let export = tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(
            &inbox.join("good_conv").join("message_1.json"),
            MESSAGE_ALICE_AND_BOB,
        );
        write_file(&inbox.join("bad_conv").join("message_1.json"), "{ not json");

        let mut conn = migrated_connection();
        let result = import_export(&mut conn, export.path());

        assert!(result.is_err());
    }
}
