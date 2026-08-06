use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::ingest::parse::{MessageType, RawConversationFile, RawMessage};
use crate::Result;

/// How much a database holds. Exists so the app layer can show the size of an
/// import without writing SQL against this crate's schema.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct Stats {
    pub message_count: i64,
    pub conversation_count: i64,
}

/// Counts the rows [`Stats`] describes. Cheap on a database that was just
/// imported into, since both tables are already in the page cache.
pub fn stats(conn: &Connection) -> Result<Stats> {
    Ok(Stats {
        message_count: conn.query_row("SELECT count(*) FROM messages", [], |row| row.get(0))?,
        conversation_count: conn
            .query_row("SELECT count(*) FROM conversations", [], |row| row.get(0))?,
    })
}

/// Inserts a conversation, or, if a row with the same `title`/`thread_path`
/// already exists (e.g. a conversation split across multiple
/// `message_N.json` files), updates its is_still_participant and adds this
/// file's message count onto the running total. Returns the conversation's
/// id either way.
pub fn upsert_conversation(conn: &Connection, conversation: &RawConversationFile) -> Result<i64> {
    let message_count = conversation.messages.len() as i64;
    Ok(conn.query_row(
        "INSERT INTO conversations (title, is_still_participant, thread_path, message_count) \
         VALUES (?1, ?2, ?3, ?4) \
         ON CONFLICT (title, thread_path) DO UPDATE SET \
             is_still_participant = excluded.is_still_participant, \
             message_count = message_count + excluded.message_count \
         RETURNING id",
        params![
            conversation.title,
            conversation.is_still_participant,
            conversation.thread_path,
            message_count,
        ],
        |row| row.get(0),
    )?)
}

/// Finds or creates a participant named `name` scoped to `conversation_id`:
/// if a participant with that name is already linked to this conversation
/// (via `conversation_participants`), returns its id; otherwise inserts a
/// new participant row, links it to the conversation, and returns the new
/// id.
///
/// Creating and linking are deliberately one operation: membership in
/// `conversation_participants` is *how* a participant is found, so a caller
/// that created a participant without linking it would leave a row this
/// function can never find again — and would then insert a fresh one for
/// every later mention of the same name.
///
/// Participants are deliberately *not* deduped globally by name — Facebook's
/// export gives no stable per-person id, only a display name, and two
/// different real people can share one (e.g. "John Smith" in unrelated
/// conversations). Scoping the lookup to a single conversation avoids
/// merging them, at the cost of a person who's in several conversations
/// getting a separate `participants` row in each.
pub fn find_or_create_participant(
    conn: &Connection,
    conversation_id: i64,
    name: &str,
) -> Result<i64> {
    let existing_id: Option<i64> = conn
        .query_row(
            "SELECT p.id FROM participants p \
             JOIN conversation_participants cp ON cp.participant_id = p.id \
             WHERE cp.conversation_id = ?1 AND p.name = ?2",
            params![conversation_id, name],
            |row| row.get(0),
        )
        .optional()?;

    if let Some(id) = existing_id {
        return Ok(id);
    }

    let participant_id: i64 = conn.query_row(
        "INSERT INTO participants (name) VALUES (?1) RETURNING id",
        params![name],
        |row| row.get(0),
    )?;

    // The id was just minted, so it can't already be linked to anything —
    // a plain INSERT can't conflict here, and shouldn't be told to ignore
    // it if it somehow does.
    conn.execute(
        "INSERT INTO conversation_participants (conversation_id, participant_id) \
         VALUES (?1, ?2)",
        params![conversation_id, participant_id],
    )?;

    Ok(participant_id)
}

/// Inserts a message, ignoring it if a message with the same
/// conversation_id, sender_id, timestamp_ms, content, type, and
/// attachment_count already exists (see the `idx_messages_dedup` unique
/// index). Returns the new row's id, or `None` if the insert was ignored as
/// a duplicate.
///
/// A missing `content` is stored as `NULL` (the dedup index compares it via
/// `COALESCE(content, '')`, since NULLs are distinct in a UNIQUE index).
/// The message's type and attachment count are derived from its attachment
/// lists (see [`RawMessage::message_type`] and
/// [`RawMessage::attachment_count`]).
///
/// The conflict target spells the dedup index out rather than using
/// `INSERT OR IGNORE`, which would also swallow `NOT NULL`, `CHECK`, and
/// any future constraint — turning a bug into silently missing messages,
/// since a swallowed row is indistinguishable from a duplicate here.
/// Naming the index scopes the "ignore" to duplicates and lets everything
/// else surface as an error.
pub fn insert_message(
    conn: &Connection,
    conversation_id: i64,
    sender_id: Option<i64>,
    message: &RawMessage,
) -> Result<Option<i64>> {
    conn.execute(
        "INSERT INTO messages \
         (conversation_id, sender_id, timestamp_ms, content, type, attachment_count) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT (conversation_id, COALESCE(sender_id, -1), timestamp_ms, \
                      COALESCE(content, ''), type, attachment_count) \
         DO NOTHING",
        params![
            conversation_id,
            sender_id,
            message.timestamp_ms,
            message.content.as_deref(),
            message.message_type().as_str(),
            message.attachment_count(),
        ],
    )?;
    Ok(if conn.changes() == 0 {
        None
    } else {
        Some(conn.last_insert_rowid())
    })
}

/// Inserts one `attachments` row per entry in `message`'s attachment lists,
/// tagged with the list it came from (`photos`/`videos`/`audio_files`/
/// `gifs`). Callers should only invoke this for a *newly inserted* message
/// (a `Some` id from [`insert_message`]) — a duplicate message's attachments
/// are already in the database, and re-inserting them would double them up.
pub fn insert_attachments(conn: &Connection, message_id: i64, message: &RawMessage) -> Result<()> {
    let lists = [
        (MessageType::Photos, &message.photos),
        (MessageType::Videos, &message.videos),
        (MessageType::AudioFiles, &message.audio_files),
        (MessageType::Gifs, &message.gifs),
    ];

    for (kind, attachments) in lists {
        for attachment in attachments {
            conn.execute(
                "INSERT INTO attachments (message_id, type, uri, creation_timestamp) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    message_id,
                    kind.as_str(),
                    attachment.uri.as_deref(),
                    attachment.creation_timestamp,
                ],
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest::import_export;
    use crate::test_util::{migrated_connection, write_file};

    #[test]
    fn stats_are_zero_for_an_empty_database() {
        let conn = migrated_connection();

        let stats = stats(&conn).unwrap();

        assert_eq!(stats.message_count, 0);
        assert_eq!(stats.conversation_count, 0);
    }

    #[test]
    fn stats_count_what_was_imported() {
        let export = tempfile::tempdir().unwrap();
        let inbox = export.path().join("messages").join("inbox");
        write_file(
            &inbox.join("alice_and_bob").join("message_1.json"),
            r#"{
                "participants": [{"name": "Alice"}, {"name": "Bob"}],
                "messages": [
                    {"sender_name": "Alice", "timestamp_ms": 1000, "content": "hi"},
                    {"sender_name": "Bob", "timestamp_ms": 2000, "content": "hello"}
                ],
                "title": "Alice and Bob",
                "is_still_participant": true,
                "thread_path": "inbox/alice_and_bob"
            }"#,
        );

        let mut conn = migrated_connection();
        import_export(&mut conn, export.path()).unwrap();

        let stats = stats(&conn).unwrap();

        assert_eq!(stats.message_count, 2);
        assert_eq!(stats.conversation_count, 1);
    }
}
