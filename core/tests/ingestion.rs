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

    let content_for_null_message: Option<String> = conn
        .query_row(
            "SELECT content FROM messages WHERE timestamp_ms = 2000",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        content_for_null_message, None,
        "a null content field should be stored as NULL"
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

/// A conversation exercising every message shape ingestion distinguishes:
/// a plain text message, one of each attachment kind (photos/videos/
/// audio_files/gifs), a photo message that also carries a text caption, and
/// a multi-attachment photo batch. Reused across the attachment tests below.
const CONVERSATION_WITH_ATTACHMENTS: &str = r#"{
    "participants": [{"name": "Alice"}],
    "messages": [
        {"sender_name": "Alice", "timestamp_ms": 1000, "content": "morning!"},
        {
            "sender_name": "Alice",
            "timestamp_ms": 2000,
            "photos": [{"uri": "photos/1.jpg", "creation_timestamp": 1712345678}]
        },
        {
            "sender_name": "Alice",
            "timestamp_ms": 3000,
            "videos": [{"uri": "videos/1.mp4", "creation_timestamp": 1712345679}]
        },
        {
            "sender_name": "Alice",
            "timestamp_ms": 4000,
            "audio_files": [{"uri": "audio/1.aac"}]
        },
        {
            "sender_name": "Alice",
            "timestamp_ms": 5000,
            "gifs": [{"uri": "gifs/1.gif"}]
        },
        {
            "sender_name": "Alice",
            "timestamp_ms": 6000,
            "content": "check these out",
            "photos": [
                {"uri": "photos/2.jpg", "creation_timestamp": 1712345680},
                {"uri": "photos/3.jpg", "creation_timestamp": 1712345681}
            ]
        }
    ],
    "title": "Alice",
    "is_still_participant": true,
    "thread_path": "inbox/alice_attachments"
}"#;

/// Imports [`CONVERSATION_WITH_ATTACHMENTS`] into a real on-disk database.
/// Returns the `db_dir` guard alongside the connection so the database file
/// outlives the call (dropping it would delete the file out from under the
/// still-open connection); the export dir is only needed during the import.
fn import_conversation_with_attachments() -> (tempfile::TempDir, Connection) {
    let export = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    write_file(
        &export
            .path()
            .join("messages")
            .join("inbox")
            .join("alice_attachments")
            .join("message_1.json"),
        CONVERSATION_WITH_ATTACHMENTS,
    );

    let mut conn = open_db(db_dir.path());
    import_export(&mut conn, export.path()).unwrap();
    (db_dir, conn)
}

#[test]
fn classifies_each_message_by_its_attachment_kind_and_counts_attachments() {
    let (_db_dir, conn) = import_conversation_with_attachments();

    let rows: Vec<(i64, String, i64, Option<String>)> = conn
        .prepare(
            "SELECT timestamp_ms, type, attachment_count, content \
             FROM messages ORDER BY timestamp_ms",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    assert_eq!(
        rows,
        vec![
            (1000, "text".to_string(), 0, Some("morning!".to_string())),
            (2000, "photos".to_string(), 1, None),
            (3000, "videos".to_string(), 1, None),
            (4000, "audio_files".to_string(), 1, None),
            (5000, "gifs".to_string(), 1, None),
            // An attachment message can carry a caption: its type is still
            // the attachment kind, but the caption rides along in content.
            (
                6000,
                "photos".to_string(),
                2,
                Some("check these out".to_string())
            ),
        ]
    );
}

#[test]
fn persists_each_attachment_row_with_its_kind_uri_and_creation_timestamp() {
    let (_db_dir, conn) = import_conversation_with_attachments();

    let attachments: Vec<(i64, String, Option<String>, Option<i64>)> = conn
        .prepare(
            "SELECT m.timestamp_ms, a.type, a.uri, a.creation_timestamp \
             FROM attachments a JOIN messages m ON m.id = a.message_id \
             ORDER BY m.timestamp_ms, a.uri",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();

    assert_eq!(
        attachments,
        vec![
            (2000, "photos".to_string(), Some("photos/1.jpg".to_string()), Some(1712345678)),
            (3000, "videos".to_string(), Some("videos/1.mp4".to_string()), Some(1712345679)),
            (4000, "audio_files".to_string(), Some("audio/1.aac".to_string()), None),
            (5000, "gifs".to_string(), Some("gifs/1.gif".to_string()), None),
            (6000, "photos".to_string(), Some("photos/2.jpg".to_string()), Some(1712345680)),
            (6000, "photos".to_string(), Some("photos/3.jpg".to_string()), Some(1712345681)),
        ]
    );
}

#[test]
fn indexes_only_messages_that_have_text_content() {
    let (_db_dir, conn) = import_conversation_with_attachments();

    // A plain `count(*)` on an external-content FTS5 table reads through to
    // `messages`, so it can't tell us what was indexed; the `%_docsize`
    // shadow table has exactly one row per *indexed* document, so its count
    // is the real index size. Only the two text-bearing messages (the plain
    // "morning!" and the captioned photo batch) should be there — the four
    // attachment-only (NULL content) messages must be excluded.
    let indexed: i64 = conn
        .query_row("SELECT count(*) FROM messages_fts_docsize", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(indexed, 2);

    // And each text-bearing message really is matchable by its own words,
    // including the caption on the attachment message.
    for term in ["morning", "check"] {
        let matches: i64 = conn
            .query_row(
                "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH ?1",
                [term],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(matches, 1, "expected exactly one message matching '{term}'");
    }
}

#[test]
fn re_importing_a_conversation_does_not_duplicate_its_attachments() {
    let export = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    write_file(
        &export
            .path()
            .join("messages")
            .join("inbox")
            .join("alice_attachments")
            .join("message_1.json"),
        CONVERSATION_WITH_ATTACHMENTS,
    );

    let mut conn = open_db(db_dir.path());
    import_export(&mut conn, export.path()).unwrap();
    import_export(&mut conn, export.path()).unwrap();

    // Re-import skips duplicate messages, so their attachments — which are
    // only inserted for a newly-inserted message — must not be doubled.
    let (message_count, attachment_count): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM messages), \
                    (SELECT count(*) FROM attachments)",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(message_count, 6);
    assert_eq!(attachment_count, 6);
}

#[test]
fn keeps_same_millisecond_attachment_messages_of_different_sizes_separate() {
    // Two photo-only messages from the same sender in the same millisecond
    // that differ only in how many photos they carry. They share content
    // (NULL) and type ('photos'); only attachment_count tells them apart, so
    // this pins that attachment_count is part of the dedup key end-to-end.
    let export = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    write_file(
        &export
            .path()
            .join("messages")
            .join("inbox")
            .join("batch_convo")
            .join("message_1.json"),
        r#"{
            "participants": [{"name": "Alice"}],
            "messages": [
                {
                    "sender_name": "Alice",
                    "timestamp_ms": 1000,
                    "photos": [{"uri": "photos/1.jpg"}]
                },
                {
                    "sender_name": "Alice",
                    "timestamp_ms": 1000,
                    "photos": [{"uri": "photos/2.jpg"}, {"uri": "photos/3.jpg"}]
                }
            ],
            "title": "Alice",
            "is_still_participant": true,
            "thread_path": "inbox/batch_convo"
        }"#,
    );

    let mut conn = open_db(db_dir.path());
    import_export(&mut conn, export.path()).unwrap();

    let message_count: i64 = conn
        .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        message_count, 2,
        "same-millisecond photo batches of different sizes are distinct messages"
    );
}

#[test]
fn a_stray_file_directly_in_the_inbox_is_skipped_not_treated_as_a_conversation() {
    // Real exports sometimes drop loose files into messages/inbox (a
    // .DS_Store, an autogenerated index, etc.). The scan only treats
    // directories as conversations, so such a file must be ignored rather
    // than derailing the import of the real conversation beside it.
    let export = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let inbox = export.path().join("messages").join("inbox");

    write_file(&inbox.join(".DS_Store"), "not a conversation");
    write_file(
        &inbox.join("real_convo").join("message_1.json"),
        r#"{
            "participants": [{"name": "Alice"}],
            "messages": [
                {"sender_name": "Alice", "timestamp_ms": 1000, "content": "hi"}
            ],
            "title": "Alice",
            "is_still_participant": true,
            "thread_path": "inbox/real_convo"
        }"#,
    );

    let mut conn = open_db(db_dir.path());
    import_export(&mut conn, export.path()).unwrap();

    let conversation_count: i64 = conn
        .query_row("SELECT count(*) FROM conversations", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        conversation_count, 1,
        "the loose file should be skipped, leaving only the real conversation"
    );
}

/// Makes `dir` unreadable and returns a guard that restores its permissions
/// on drop, so the tempdir can still be cleaned up if an assertion panics.
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
fn an_unreadable_conversation_folder_surfaces_as_an_import_error() {
    // A conversation directory whose contents can't be listed (e.g. a
    // permissions problem in the export) should fail the import with the
    // underlying I/O error rather than being silently treated as empty.
    let export = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let locked = export
        .path()
        .join("messages")
        .join("inbox")
        .join("locked_convo");
    fs::create_dir_all(&locked).unwrap();
    let _guard = make_unreadable(&locked);
    if fs::read_dir(&locked).is_ok() {
        // Running as root, where mode 000 is still readable; the scenario
        // can't be constructed, so there's nothing to test.
        return;
    }

    let mut conn = open_db(db_dir.path());
    let result = import_export(&mut conn, export.path());

    assert!(
        result.is_err(),
        "an unreadable conversation folder should fail the import, not be \
         mistaken for an empty conversation"
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
