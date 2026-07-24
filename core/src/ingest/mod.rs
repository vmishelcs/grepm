use std::error::Error;
use std::path::Path;

use rusqlite::Connection;

use crate::db;

pub mod loader;
pub mod parse;
pub mod scan;

/// Imports every conversation found under `export_root` (a Facebook data
/// export's root directory, i.e. the directory containing `messages/`) into
/// the database at `conn`, then (re)builds the full-text search index over
/// every imported message.
pub fn import_export(
    conn: &mut Connection,
    export_root: impl AsRef<Path>,
) -> Result<(), Box<dyn Error>> {
    for conversation_dir in scan::scan(export_root)? {
        loader::load_conversation(conn, &conversation_dir?)?;
    }

    db::populate_fts(conn)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::*;
    use crate::db::schema;

    fn migrated_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        schema::configure(&conn).unwrap();
        schema::migrate(&conn).unwrap();
        conn
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

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
