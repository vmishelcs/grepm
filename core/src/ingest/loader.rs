use rusqlite::Connection;

use crate::db::queries;
use crate::ingest::parse::{parse_conversation_file, RawMessage};
use crate::ingest::scan::ConversationDir;
use crate::Result;

/// Loads a single conversation into the database: for every message file,
/// upserts the thread (`conversations` row), inserts and links its
/// participants, and loads its messages, all within one transaction.
pub fn load_conversation(conn: &mut Connection, conversation_dir: &ConversationDir) -> Result<()> {
    let tx = conn.transaction()?;

    for message_file in &conversation_dir.message_files {
        let raw_file = parse_conversation_file(message_file)?;

        let conversation_id = queries::upsert_conversation(&tx, &raw_file)?;

        for raw_participant in &raw_file.participants {
            queries::find_or_create_participant(&tx, conversation_id, &raw_participant.name)?;
        }

        load_messages(&tx, conversation_id, &raw_file.messages)?;
    }

    tx.commit()?;

    Ok(())
}

/// Loads a conversation's messages, resolving each message's sender name to
/// a participant id (scoped to this conversation, see
/// [`queries::find_or_create_participant`]) along the way. A sender who
/// isn't in the file's `participants` list is created and linked like any
/// other, so they resolve to one participant row rather than a fresh one
/// per message. Duplicate messages (per the `messages` table's UNIQUE
/// constraint) are silently skipped, along with their attachments — a
/// skipped message's attachments were already persisted the first time
/// around.
pub fn load_messages(
    conn: &Connection,
    conversation_id: i64,
    messages: &[RawMessage],
) -> Result<()> {
    for message in messages {
        let sender_id = message
            .sender_name
            .as_deref()
            .map(|name| queries::find_or_create_participant(conn, conversation_id, name))
            .transpose()?;
        if let Some(message_id) =
            queries::insert_message(conn, conversation_id, sender_id, message)?
        {
            queries::insert_attachments(conn, message_id, message)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::*;
    use crate::db::schema;

    fn migrated_connection() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::configure(&conn).unwrap();
        schema::migrate(&mut conn).unwrap();
        conn
    }

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn conversation_dir(folder: PathBuf, message_files: Vec<PathBuf>) -> ConversationDir {
        ConversationDir {
            folder,
            message_files,
        }
    }

    const MESSAGE_1: &str = r#"{
        "participants": [{"name": "Alice"}, {"name": "Bob"}],
        "messages": [
            {"sender_name": "Alice", "timestamp_ms": 1000, "content": "hi"},
            {"sender_name": "Bob", "timestamp_ms": 2000, "content": "hello"}
        ],
        "title": "Alice and Bob",
        "is_still_participant": true,
        "thread_path": "inbox/alice_and_bob"
    }"#;

    const MESSAGE_2: &str = r#"{
        "participants": [{"name": "Alice"}, {"name": "Bob"}],
        "messages": [
            {"sender_name": "Alice", "timestamp_ms": 3000, "content": "how are you"}
        ],
        "title": "Alice and Bob",
        "is_still_participant": true,
        "thread_path": "inbox/alice_and_bob"
    }"#;

    // A different conversation that happens to also have a participant
    // named "Alice" — a different real person, not the same "Alice" as
    // MESSAGE_1/MESSAGE_2.
    const MESSAGE_OTHER_CONVERSATION: &str = r#"{
        "participants": [{"name": "Alice"}, {"name": "Carol"}],
        "messages": [
            {"sender_name": "Alice", "timestamp_ms": 4000, "content": "hey"}
        ],
        "title": "Alice and Carol",
        "is_still_participant": true,
        "thread_path": "inbox/alice_and_carol"
    }"#;

    // Facebook replaces a deleted account's name with the placeholder
    // "Facebook User" in both the title and the participants list, so two
    // conversations with two different deleted-account counterparts end up
    // with an identical title and participant name, differing only by
    // thread_path (derived from the export's per-conversation numeric id).
    const MESSAGE_DELETED_ACCOUNT_1: &str = r#"{
        "participants": [{"name": "Facebook User"}],
        "messages": [
            {"sender_name": "Facebook User", "timestamp_ms": 5000, "content": "hey"}
        ],
        "title": "Facebook User",
        "is_still_participant": false,
        "thread_path": "inbox/facebookuser_1122334455"
    }"#;

    const MESSAGE_DELETED_ACCOUNT_2: &str = r#"{
        "participants": [{"name": "Facebook User"}],
        "messages": [
            {"sender_name": "Facebook User", "timestamp_ms": 6000, "content": "hi"}
        ],
        "title": "Facebook User",
        "is_still_participant": false,
        "thread_path": "inbox/facebookuser_6677889900"
    }"#;

    #[test]
    fn load_conversation_inserts_the_conversation_row() {
        let export = tempdir().unwrap();
        let folder = export.path().join("alice_and_bob_123");
        let message_file = folder.join("message_1.json");
        write_file(&message_file, MESSAGE_1);

        let mut conn = migrated_connection();
        load_conversation(&mut conn, &conversation_dir(folder, vec![message_file])).unwrap();

        let (title, is_still_participant, thread_path): (
            Option<String>,
            Option<bool>,
            Option<String>,
        ) = conn
            .query_row(
                "SELECT title, is_still_participant, thread_path FROM conversations",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();

        assert_eq!(title.as_deref(), Some("Alice and Bob"));
        assert_eq!(is_still_participant, Some(true));
        assert_eq!(thread_path.as_deref(), Some("inbox/alice_and_bob"));
    }

    #[test]
    fn load_conversation_inserts_and_links_participants() {
        let export = tempdir().unwrap();
        let folder = export.path().join("conv");
        let message_file = folder.join("message_1.json");
        write_file(&message_file, MESSAGE_1);

        let mut conn = migrated_connection();
        load_conversation(&mut conn, &conversation_dir(folder, vec![message_file])).unwrap();

        let participant_count: i64 = conn
            .query_row("SELECT count(*) FROM participants", [], |row| row.get(0))
            .unwrap();
        assert_eq!(participant_count, 2);

        let link_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM conversation_participants",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(link_count, 2);
    }

    #[test]
    fn load_conversation_does_not_merge_same_named_participants_across_conversations() {
        let export = tempdir().unwrap();

        let folder_1 = export.path().join("conv_1");
        let message_1 = folder_1.join("message_1.json");
        write_file(&message_1, MESSAGE_1);

        let folder_2 = export.path().join("conv_2");
        let message_2 = folder_2.join("message_1.json");
        write_file(&message_2, MESSAGE_OTHER_CONVERSATION);

        let mut conn = migrated_connection();
        load_conversation(&mut conn, &conversation_dir(folder_1, vec![message_1])).unwrap();
        load_conversation(&mut conn, &conversation_dir(folder_2, vec![message_2])).unwrap();

        let alice_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM participants WHERE name = 'Alice'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            alice_count, 2,
            "two different conversations' same-named participants should get separate rows"
        );
    }

    #[test]
    fn load_conversation_keeps_same_titled_deleted_account_conversations_separate() {
        let export = tempdir().unwrap();

        let folder_1 = export.path().join("facebookuser_1122334455");
        let message_1 = folder_1.join("message_1.json");
        write_file(&message_1, MESSAGE_DELETED_ACCOUNT_1);

        let folder_2 = export.path().join("facebookuser_6677889900");
        let message_2 = folder_2.join("message_1.json");
        write_file(&message_2, MESSAGE_DELETED_ACCOUNT_2);

        let mut conn = migrated_connection();
        load_conversation(&mut conn, &conversation_dir(folder_1, vec![message_1])).unwrap();
        load_conversation(&mut conn, &conversation_dir(folder_2, vec![message_2])).unwrap();

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

        let message_counts: Vec<i64> = conn
            .prepare("SELECT message_count FROM conversations ORDER BY thread_path")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(message_counts, vec![1, 1]);

        let total_messages: i64 = conn
            .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(total_messages, 2);

        let misattributed_senders: i64 = conn
            .query_row(
                "SELECT count(*) FROM messages m \
                 WHERE NOT EXISTS ( \
                     SELECT 1 FROM conversation_participants cp \
                     WHERE cp.conversation_id = m.conversation_id \
                     AND cp.participant_id = m.sender_id \
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            misattributed_senders, 0,
            "each message's sender should be linked to its own conversation, not the \
             other same-named participant"
        );
    }

    #[test]
    fn load_conversation_inserts_messages_from_every_file() {
        let export = tempdir().unwrap();
        let folder = export.path().join("conv");
        let message_1 = folder.join("message_1.json");
        let message_2 = folder.join("message_2.json");
        write_file(&message_1, MESSAGE_1);
        write_file(&message_2, MESSAGE_2);

        let mut conn = migrated_connection();
        load_conversation(
            &mut conn,
            &conversation_dir(folder, vec![message_1, message_2]),
        )
        .unwrap();

        let message_count: i64 = conn
            .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(message_count, 3);
    }

    #[test]
    fn load_conversation_accumulates_message_count_across_files() {
        let export = tempdir().unwrap();
        let folder = export.path().join("conv");
        let message_1 = folder.join("message_1.json");
        let message_2 = folder.join("message_2.json");
        write_file(&message_1, MESSAGE_1);
        write_file(&message_2, MESSAGE_2);

        let mut conn = migrated_connection();
        load_conversation(
            &mut conn,
            &conversation_dir(folder, vec![message_1, message_2]),
        )
        .unwrap();

        let message_count: i64 = conn
            .query_row("SELECT message_count FROM conversations", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(message_count, 3);
    }

    #[test]
    fn load_conversation_merges_every_file_into_one_conversation_row() {
        // Both files carry the same (title, thread_path), which is what the
        // upsert conflicts on. Since neither can be NULL — NULLs are
        // distinct in a SQLite unique index, so they'd never match — this
        // holds for every file the parser accepts.
        let export = tempdir().unwrap();
        let folder = export.path().join("conv");
        let message_1 = folder.join("message_1.json");
        let message_2 = folder.join("message_2.json");
        write_file(&message_1, MESSAGE_1);
        write_file(&message_2, MESSAGE_2);

        let mut conn = migrated_connection();
        load_conversation(
            &mut conn,
            &conversation_dir(folder, vec![message_1, message_2]),
        )
        .unwrap();

        let conversation_count: i64 = conn
            .query_row("SELECT count(*) FROM conversations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(conversation_count, 1);
    }

    #[test]
    fn load_conversation_does_not_duplicate_participants_across_files() {
        // Both fixture files list the same two participants; they should
        // resolve to the same two participant rows, not four.
        let export = tempdir().unwrap();
        let folder = export.path().join("conv");
        let message_1 = folder.join("message_1.json");
        let message_2 = folder.join("message_2.json");
        write_file(&message_1, MESSAGE_1);
        write_file(&message_2, MESSAGE_2);

        let mut conn = migrated_connection();
        load_conversation(
            &mut conn,
            &conversation_dir(folder, vec![message_1, message_2]),
        )
        .unwrap();

        let participant_count: i64 = conn
            .query_row("SELECT count(*) FROM participants", [], |row| row.get(0))
            .unwrap();
        assert_eq!(participant_count, 2);
    }

    #[test]
    fn load_conversation_resolves_message_sender_to_a_participant_id() {
        let export = tempdir().unwrap();
        let folder = export.path().join("conv");
        let message_file = folder.join("message_1.json");
        write_file(&message_file, MESSAGE_1);

        let mut conn = migrated_connection();
        load_conversation(&mut conn, &conversation_dir(folder, vec![message_file])).unwrap();

        let sender_name: String = conn
            .query_row(
                "SELECT p.name FROM messages m \
                 JOIN participants p ON p.id = m.sender_id \
                 WHERE m.content = 'hi'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(sender_name, "Alice");
    }

    #[test]
    fn load_conversation_stores_the_message_type_classified_for_each_message() {
        let export = tempdir().unwrap();
        let folder = export.path().join("conv");
        let message_file = folder.join("message_1.json");
        write_file(
            &message_file,
            r#"{
                "participants": [{"name": "Alice"}],
                "messages": [
                    {"sender_name": "Alice", "timestamp_ms": 1000, "content": "hi"},
                    {
                        "sender_name": "Alice",
                        "timestamp_ms": 2000,
                        "photos": [{"uri": "photos/1.jpg"}]
                    },
                    {
                        "sender_name": "Alice",
                        "timestamp_ms": 3000,
                        "audio_files": [{"uri": "audio/1.aac"}]
                    }
                ],
                "title": "Alice",
                "is_still_participant": true,
                "thread_path": "inbox/conv"
            }"#,
        );

        let mut conn = migrated_connection();
        load_conversation(&mut conn, &conversation_dir(folder, vec![message_file])).unwrap();

        let types: Vec<String> = conn
            .prepare("SELECT type FROM messages ORDER BY timestamp_ms")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(types, vec!["text", "photos", "audio_files"]);
    }

    // Two photos and one gif across two messages; the gif entry has no
    // creation_timestamp, matching how real exports shape gifs.
    const MESSAGE_WITH_ATTACHMENTS: &str = r#"{
        "participants": [{"name": "Alice"}],
        "messages": [
            {
                "sender_name": "Alice",
                "timestamp_ms": 1000,
                "content": "vacation pics",
                "photos": [
                    {"uri": "photos/1.jpg", "creation_timestamp": 1712345678},
                    {"uri": "photos/2.jpg", "creation_timestamp": 1712345679}
                ]
            },
            {
                "sender_name": "Alice",
                "timestamp_ms": 2000,
                "gifs": [{"uri": "gifs/1.gif"}]
            }
        ],
        "title": "Alice",
        "is_still_participant": true,
        "thread_path": "inbox/conv"
    }"#;

    #[test]
    fn load_conversation_persists_each_attachment_with_its_kind_and_metadata() {
        let export = tempdir().unwrap();
        let folder = export.path().join("conv");
        let message_file = folder.join("message_1.json");
        write_file(&message_file, MESSAGE_WITH_ATTACHMENTS);

        let mut conn = migrated_connection();
        load_conversation(&mut conn, &conversation_dir(folder, vec![message_file])).unwrap();

        let attachments: Vec<(String, Option<String>, Option<i64>)> = conn
            .prepare(
                "SELECT a.type, a.uri, a.creation_timestamp FROM attachments a \
                 JOIN messages m ON m.id = a.message_id \
                 ORDER BY m.timestamp_ms, a.uri",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();

        assert_eq!(
            attachments,
            vec![
                (
                    "photos".to_string(),
                    Some("photos/1.jpg".to_string()),
                    Some(1712345678)
                ),
                (
                    "photos".to_string(),
                    Some("photos/2.jpg".to_string()),
                    Some(1712345679)
                ),
                ("gifs".to_string(), Some("gifs/1.gif".to_string()), None),
            ]
        );
    }

    #[test]
    fn load_conversation_keeps_same_millisecond_photo_batches_of_different_sizes_separate() {
        // Same sender, same timestamp_ms, both attachment-only photo
        // messages — only their attachment counts differ. Before
        // attachment_count joined the dedup key, the second batch would
        // have been silently dropped as a duplicate.
        let export = tempdir().unwrap();
        let folder = export.path().join("conv");
        let message_file = folder.join("message_1.json");
        write_file(
            &message_file,
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
                "thread_path": "inbox/conv"
            }"#,
        );

        let mut conn = migrated_connection();
        let dir = conversation_dir(folder, vec![message_file]);
        load_conversation(&mut conn, &dir).unwrap();

        let message_count: i64 = conn
            .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(message_count, 2);

        // And a reload must still recognize both as duplicates.
        load_conversation(&mut conn, &dir).unwrap();

        let counts: (i64, i64) = conn
            .query_row(
                "SELECT (SELECT count(*) FROM messages), \
                        (SELECT count(*) FROM attachments)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(counts, (2, 3));
    }

    #[test]
    fn load_conversation_does_not_duplicate_attachments_on_reload() {
        let export = tempdir().unwrap();
        let folder = export.path().join("conv");
        let message_file = folder.join("message_1.json");
        write_file(&message_file, MESSAGE_WITH_ATTACHMENTS);

        let mut conn = migrated_connection();
        let dir = conversation_dir(folder, vec![message_file]);
        load_conversation(&mut conn, &dir).unwrap();
        load_conversation(&mut conn, &dir).unwrap();

        let attachment_count: i64 = conn
            .query_row("SELECT count(*) FROM attachments", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            attachment_count, 3,
            "a reload skips duplicate messages, so their attachments must \
             not be inserted a second time"
        );
    }

    #[test]
    fn load_conversation_is_idempotent_on_reload() {
        let export = tempdir().unwrap();
        let folder = export.path().join("conv");
        let message_file = folder.join("message_1.json");
        write_file(&message_file, MESSAGE_1);

        let mut conn = migrated_connection();
        let dir = conversation_dir(folder, vec![message_file]);
        load_conversation(&mut conn, &dir).unwrap();
        load_conversation(&mut conn, &dir).unwrap();

        let conversation_count: i64 = conn
            .query_row("SELECT count(*) FROM conversations", [], |row| row.get(0))
            .unwrap();
        assert_eq!(conversation_count, 1);

        let message_count: i64 = conn
            .query_row("SELECT count(*) FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(message_count, 2);
    }

    #[test]
    fn load_conversation_errors_on_malformed_first_file() {
        let export = tempdir().unwrap();
        let folder = export.path().join("conv");
        let message_file = folder.join("message_1.json");
        write_file(&message_file, "{ not json");

        let mut conn = migrated_connection();
        let result = load_conversation(&mut conn, &conversation_dir(folder, vec![message_file]));

        assert!(result.is_err());
    }

    #[test]
    fn find_or_create_participant_links_the_new_participant_to_the_conversation() {
        let conn = migrated_connection();
        let conversation_id = empty_conversation(&conn);

        let participant_id =
            queries::find_or_create_participant(&conn, conversation_id, "Alice").unwrap();

        let links: i64 = conn
            .query_row(
                "SELECT count(*) FROM conversation_participants \
                 WHERE conversation_id = ?1 AND participant_id = ?2",
                rusqlite::params![conversation_id, participant_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            links, 1,
            "creating a participant must link it in the same step — the link \
             is what the next lookup searches by"
        );
    }

    #[test]
    fn find_or_create_participant_returns_the_existing_id_when_called_again() {
        let conn = migrated_connection();
        let conversation_id = empty_conversation(&conn);

        let first = queries::find_or_create_participant(&conn, conversation_id, "Alice").unwrap();
        let second = queries::find_or_create_participant(&conn, conversation_id, "Alice").unwrap();

        assert_eq!(first, second);

        let participant_count: i64 = conn
            .query_row("SELECT count(*) FROM participants", [], |row| row.get(0))
            .unwrap();
        assert_eq!(participant_count, 1);
    }

    #[test]
    fn insert_message_reports_a_duplicate_as_none_rather_than_an_error() {
        let conn = migrated_connection();
        let conversation_id = empty_conversation(&conn);
        let messages = parse_message_json(
            r#"{
                "participants": [],
                "messages": [{"timestamp_ms": 1000, "content": "hi"}],
                "title": "Alice",
                "thread_path": "inbox/conv_a"
            }"#,
        )
        .messages;

        let first = queries::insert_message(&conn, conversation_id, None, &messages[0]).unwrap();
        let second = queries::insert_message(&conn, conversation_id, None, &messages[0]).unwrap();

        assert!(first.is_some());
        assert_eq!(second, None);
    }

    #[test]
    fn insert_message_errors_on_a_constraint_violation_that_is_not_a_duplicate() {
        // The extra index stands in for a constraint a future schema might
        // add. The insert's conflict handling is scoped to the dedup index,
        // so violating anything else has to surface as an error — a
        // silently dropped row would be indistinguishable from a duplicate.
        let conn = migrated_connection();
        let conversation_id = empty_conversation(&conn);
        conn.execute(
            "CREATE UNIQUE INDEX one_message_per_millisecond ON messages (timestamp_ms)",
            [],
        )
        .unwrap();

        let messages = parse_message_json(
            r#"{
                "participants": [],
                "messages": [
                    {"timestamp_ms": 1000, "content": "hi"},
                    {"timestamp_ms": 1000, "content": "not a duplicate"}
                ],
                "title": "Alice",
                "thread_path": "inbox/conv_a"
            }"#,
        )
        .messages;

        queries::insert_message(&conn, conversation_id, None, &messages[0])
            .unwrap()
            .expect("the first message is new");

        let result = queries::insert_message(&conn, conversation_id, None, &messages[1]);

        assert!(
            result.is_err(),
            "a non-dedup constraint violation must not be reported as a duplicate"
        );
    }

    #[test]
    fn load_messages_leaves_sender_id_null_when_sender_name_is_absent() {
        let conn = migrated_connection();
        let conversation_id = empty_conversation(&conn);

        let messages = parse_message_json(
            r#"{
                "participants": [],
                "messages": [{"timestamp_ms": 1000, "content": "hi"}],
                "title": "Alice",
                "thread_path": "inbox/conv_a"
            }"#,
        )
        .messages;
        load_messages(&conn, conversation_id, &messages).unwrap();

        let sender_id: Option<i64> = conn
            .query_row("SELECT sender_id FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sender_id, None);
    }

    fn empty_conversation(conn: &Connection) -> i64 {
        let json = r#"{
            "participants": [],
            "title": "Alice",
            "thread_path": "inbox/conv_a"
        }"#;
        queries::upsert_conversation(conn, &parse_message_json(json)).unwrap()
    }

    fn parse_message_json(json: &str) -> crate::ingest::parse::RawConversationFile {
        let dir = tempdir().unwrap();
        let file = dir.path().join("message_1.json");
        write_file(&file, json);
        parse_conversation_file(&file).unwrap()
    }
}
