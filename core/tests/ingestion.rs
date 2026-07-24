//! End-to-end ingestion tests: unlike the unit tests co-located with each
//! module (which mostly use in-memory databases), these exercise the full
//! `import_export` pipeline against a real Facebook-export-shaped directory
//! tree and a real SQLite file on disk, closer to how the Tauri app will
//! actually use `grepm_core`.

use std::fs;
use std::path::Path;

use rusqlite::Connection;
use tempfile::tempdir;

use grepm_core::db;
use grepm_core::ingest::import_export;

fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Opens a real on-disk SQLite database (as opposed to the in-memory
/// databases the unit tests use), migrated and ready to import into.
fn open_db(dir: &Path) -> Connection {
    db::schema::open(&dir.join("grepm.sqlite3")).unwrap()
}

#[test]
fn imports_multiple_conversations_including_a_group_chat() {
    let export = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let inbox = export.path().join("messages").join("inbox");

    write_file(
        &inbox
            .join("alice_and_bob_1122334455")
            .join("message_1.json"),
        r#"{
            "participants": [{"name": "Alice"}, {"name": "Bob"}],
            "messages": [
                {"sender_name": "Alice", "timestamp_ms": 1000, "content": "hi"},
                {"sender_name": "Bob", "timestamp_ms": 2000, "content": "hello"}
            ],
            "title": "Alice and Bob",
            "is_still_participant": true,
            "thread_path": "inbox/alice_and_bob_1122334455"
        }"#,
    );

    write_file(
        &inbox.join("weekend_trip_9988776655").join("message_1.json"),
        r#"{
            "participants": [{"name": "Alice"}, {"name": "Bob"}, {"name": "Carol"}],
            "messages": [
                {"sender_name": "Carol", "timestamp_ms": 3000, "content": "who's driving"},
                {"sender_name": "Alice", "timestamp_ms": 3100, "content": "I can"},
                {"sender_name": "Bob", "timestamp_ms": 3200, "content": "count me in"}
            ],
            "title": "Weekend Trip",
            "is_still_participant": true,
            "thread_path": "inbox/weekend_trip_9988776655"
        }"#,
    );

    let mut conn = open_db(db_dir.path());
    import_export(&mut conn, export.path()).unwrap();

    let conversation_count: i64 = conn
        .query_row("SELECT count(*) FROM conversations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(conversation_count, 2);

    let message_count: i64 = conn
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(message_count, 5);

    // Alice and Bob are each in both conversations, but participants are
    // scoped per conversation (see KNOWN_ISSUES.md #3), so that's 2 rows
    // apiece; Carol is only in the group chat, so 1 row for her.
    let participant_count: i64 = conn
        .query_row("SELECT count(*) FROM participants", [], |row| row.get(0))
        .unwrap();
    assert_eq!(participant_count, 5);

    let group_participant_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM conversation_participants cp \
             JOIN conversations c ON c.id = cp.conversation_id \
             WHERE c.title = 'Weekend Trip'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(group_participant_count, 3);
}

#[test]
fn accumulates_a_conversation_split_across_multiple_files_and_links_late_arriving_participants() {
    let export = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let conv = export
        .path()
        .join("messages")
        .join("inbox")
        .join("book_club_5566778899");

    write_file(
        &conv.join("message_1.json"),
        r#"{
            "participants": [{"name": "Dave"}, {"name": "Erin"}],
            "messages": [
                {"sender_name": "Dave", "timestamp_ms": 1000, "content": "starting chapter 3"}
            ],
            "title": "Book Club",
            "is_still_participant": true,
            "thread_path": "inbox/book_club_5566778899"
        }"#,
    );

    // A later page whose participants list includes someone ("Frank") who
    // never appeared in an earlier page and never sends a message here
    // either — only their presence in this page's participants list should
    // be enough to link them to the conversation.
    write_file(
        &conv.join("message_2.json"),
        r#"{
            "participants": [{"name": "Dave"}, {"name": "Erin"}, {"name": "Frank"}],
            "messages": [
                {"sender_name": "Erin", "timestamp_ms": 2000, "content": "loved the twist"}
            ],
            "title": "Book Club",
            "is_still_participant": true,
            "thread_path": "inbox/book_club_5566778899"
        }"#,
    );

    let mut conn = open_db(db_dir.path());
    import_export(&mut conn, export.path()).unwrap();

    let stored_message_count: i64 = conn
        .query_row("SELECT message_count FROM conversations", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(
        stored_message_count, 2,
        "message_count should accumulate across both files"
    );

    let total_messages: i64 = conn
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(total_messages, 2);

    let frank_is_linked: bool = conn
        .query_row(
            "SELECT EXISTS( \
                 SELECT 1 FROM participants p \
                 JOIN conversation_participants cp ON cp.participant_id = p.id \
                 WHERE p.name = 'Frank' \
             )",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        frank_is_linked,
        "a participant who only appears in a later file's participants list, \
         and never sends a message, should still be linked to the conversation"
    );
}

#[test]
fn keeps_same_titled_deleted_account_conversations_separate() {
    // Facebook replaces a deleted account's name with the placeholder
    // "Facebook User" in both the title and the participants list, so two
    // conversations with two different deleted-account counterparts end up
    // with an identical title and participant name, differing only by
    // thread_path (derived from the export's per-conversation numeric id).
    let export = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let inbox = export.path().join("messages").join("inbox");

    write_file(
        &inbox.join("facebookuser_1122334455").join("message_1.json"),
        r#"{
            "participants": [{"name": "Facebook User"}],
            "messages": [
                {"sender_name": "Facebook User", "timestamp_ms": 1000, "content": "hey"}
            ],
            "title": "Facebook User",
            "is_still_participant": false,
            "thread_path": "inbox/facebookuser_1122334455"
        }"#,
    );

    write_file(
        &inbox.join("facebookuser_6677889900").join("message_1.json"),
        r#"{
            "participants": [{"name": "Facebook User"}],
            "messages": [
                {"sender_name": "Facebook User", "timestamp_ms": 2000, "content": "hi"}
            ],
            "title": "Facebook User",
            "is_still_participant": false,
            "thread_path": "inbox/facebookuser_6677889900"
        }"#,
    );

    let mut conn = open_db(db_dir.path());
    import_export(&mut conn, export.path()).unwrap();

    let conversation_count: i64 = conn
        .query_row("SELECT count(*) FROM conversations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        conversation_count, 2,
        "same-titled conversations with different thread_path should stay separate"
    );

    let participant_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM participants WHERE name = 'Facebook User'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        participant_count, 2,
        "each conversation's 'Facebook User' participant should be a separate row"
    );
}

#[test]
fn repairs_mojibake_content_and_makes_it_diacritic_insensitively_searchable() {
    let export = tempdir().unwrap();
    let db_dir = tempdir().unwrap();

    write_file(
        &export
            .path()
            .join("messages")
            .join("inbox")
            .join("garbled_convo")
            .join("message_1.json"),
        // "café" mojibake-corrupted the way real exports do it: its UTF-8
        // bytes (C3 A9) reinterpreted as two Latin-1 characters (Ã, ©).
        // Also includes a null-content (e.g. attachment-only) message.
        r#"{
            "participants": [{"name": "Alice"}],
            "messages": [
                {"sender_name": "Alice", "timestamp_ms": 1000, "content": "let's grab cafÃ©"},
                {"sender_name": "Alice", "timestamp_ms": 2000, "content": null}
            ],
            "title": "Alice",
            "is_still_participant": true,
            "thread_path": "inbox/garbled_convo"
        }"#,
    );

    let mut conn = open_db(db_dir.path());
    import_export(&mut conn, export.path()).unwrap();

    let repaired_content: String = conn
        .query_row(
            "SELECT content FROM messages WHERE timestamp_ms = 1000",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(repaired_content, "let's grab café");

    let content_for_null_message: String = conn
        .query_row(
            "SELECT content FROM messages WHERE timestamp_ms = 2000",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        content_for_null_message, "",
        "a null content field should be stored as '' rather than NULL"
    );

    // A plain SELECT (no MATCH) against an external-content FTS5 table reads
    // through to the backing `messages` table rather than the search index,
    // so it can't be used to check what actually got indexed — a MATCH
    // query is required, which the assertion below relies on.
    let diacritic_insensitive_match_count: i64 = conn
        .query_row(
            "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH 'cafe'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        diacritic_insensitive_match_count, 1,
        "searching 'cafe' should match the repaired 'café' content \
         (unicode61 remove_diacritics 2)"
    );
}

#[test]
fn a_malformed_conversation_file_fails_the_whole_import_but_leaves_earlier_conversations_committed()
{
    let export = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let inbox = export.path().join("messages").join("inbox");

    // "aaa_good_conv" sorts before "zzz_bad_conv", and scan() walks
    // directory entries in the order the filesystem yields them; on most
    // filesystems that's not guaranteed to be alphabetical, but this at
    // least matches the common case without depending on it for the
    // assertions below (which don't assume an order).
    write_file(
        &inbox.join("aaa_good_conv").join("message_1.json"),
        r#"{
            "participants": [{"name": "Alice"}],
            "messages": [
                {"sender_name": "Alice", "timestamp_ms": 1000, "content": "hi"}
            ],
            "title": "Alice",
            "is_still_participant": true,
            "thread_path": "inbox/aaa_good_conv"
        }"#,
    );
    write_file(
        &inbox.join("zzz_bad_conv").join("message_1.json"),
        "{ not valid json",
    );

    let mut conn = open_db(db_dir.path());
    let result = import_export(&mut conn, export.path());

    assert!(result.is_err());

    // load_conversation commits its own transaction per conversation, so a
    // later conversation's malformed file doesn't roll back a conversation
    // that was already fully loaded and committed earlier in the scan.
    let conversation_count: i64 = conn
        .query_row("SELECT count(*) FROM conversations", [], |row| row.get(0))
        .unwrap();
    assert!(
        conversation_count <= 1,
        "only the conversation that parsed successfully before the error \
         should ever be committed"
    );
}
