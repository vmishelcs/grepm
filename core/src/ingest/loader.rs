use std::error::Error;

use rusqlite::Connection;

use crate::db;
use crate::ingest::parse::{parse_conversation_file, RawMessage};
use crate::ingest::scan::ConversationDir;

/// Loads a single conversation into the database: for every message file,
/// upserts the thread (`conversations` row), inserts and links its
/// participants, and loads its messages, all within one transaction.
pub fn load_conversation(
    conn: &mut Connection,
    conversation_dir: &ConversationDir,
) -> Result<(), Box<dyn Error>> {
    let tx = conn.transaction()?;

    for message_file in &conversation_dir.message_files {
        let raw_file = parse_conversation_file(message_file)?;

        let conversation_id = db::upsert_conversation(&tx, &raw_file)?;

        for raw_participant in &raw_file.participants {
            let participant_id = db::insert_participant(&tx, &raw_participant.name)?;
            db::link_conversation_participant(&tx, conversation_id, participant_id)?;
        }

        load_messages(&tx, conversation_id, &raw_file.messages)?;
    }

    tx.commit()?;

    Ok(())
}

/// Loads a conversation's messages, resolving each message's sender name to
/// a participant id along the way. Duplicate messages (per the `messages`
/// table's UNIQUE constraint) are silently skipped.
pub fn load_messages(
    conn: &Connection,
    conversation_id: i64,
    messages: &[RawMessage],
) -> rusqlite::Result<()> {
    for message in messages {
        let sender_id = match &message.sender_name {
            Some(name) => Some(db::insert_participant(conn, name)?),
            None => None,
        };
        db::insert_message(conn, conversation_id, sender_id, message)?;
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
        let conn = Connection::open_in_memory().unwrap();
        schema::configure(&conn).unwrap();
        schema::migrate(&conn).unwrap();
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
    fn load_messages_leaves_sender_id_null_when_sender_name_is_absent() {
        let conn = migrated_connection();
        let conversation_id =
            db::upsert_conversation(&conn, &parse_message_json(r#"{"participants": []}"#)).unwrap();

        let messages = parse_message_json(
            r#"{
                "participants": [],
                "messages": [{"timestamp_ms": 1000, "content": "hi"}]
            }"#,
        )
        .messages;
        load_messages(&conn, conversation_id, &messages).unwrap();

        let sender_id: Option<i64> = conn
            .query_row("SELECT sender_id FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(sender_id, None);
    }

    fn parse_message_json(json: &str) -> crate::ingest::parse::RawConversationFile {
        let dir = tempdir().unwrap();
        let file = dir.path().join("message_1.json");
        write_file(&file, json);
        parse_conversation_file(&file).unwrap()
    }
}
