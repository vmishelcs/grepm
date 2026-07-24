use rusqlite::{params, Connection, OptionalExtension};

use crate::ingest::parse::{RawConversationFile, RawMessage};

/// Inserts a conversation, or, if a row with the same `title`/`thread_path`
/// already exists (e.g. a conversation split across multiple
/// `message_N.json` files), updates its is_still_participant and adds this
/// file's message count onto the running total. Returns the conversation's
/// id either way.
pub fn upsert_conversation(
    conn: &Connection,
    conversation: &RawConversationFile,
) -> rusqlite::Result<i64> {
    let message_count = conversation.messages.len() as i64;
    conn.query_row(
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
    )
}

/// Finds or creates a participant named `name` scoped to `conversation_id`:
/// if a participant with that name is already linked to this conversation
/// (via `conversation_participants`), returns its id; otherwise inserts a
/// new participant row and returns the new id.
///
/// Participants are deliberately *not* deduped globally by name — Facebook's
/// export gives no stable per-person id, only a display name, and two
/// different real people can share one (e.g. "John Smith" in unrelated
/// conversations). Scoping the lookup to a single conversation avoids
/// merging them, at the cost of a person who's in several conversations
/// getting a separate `participants` row in each.
pub fn insert_participant(
    conn: &Connection,
    conversation_id: i64,
    name: &str,
) -> rusqlite::Result<i64> {
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

    conn.query_row(
        "INSERT INTO participants (name) VALUES (?1) RETURNING id",
        params![name],
        |row| row.get(0),
    )
}

pub fn link_conversation_participant(
    conn: &Connection,
    conversation_id: i64,
    participant_id: i64,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO conversation_participants (conversation_id, participant_id) \
         VALUES (?1, ?2)",
        params![conversation_id, participant_id],
    )?;
    Ok(())
}

/// Inserts a message, ignoring it if a message with the same
/// conversation_id, sender_id, timestamp_ms, and content already exists (see
/// the `idx_messages_dedup` unique index). Returns the new row's id, or
/// `None` if the insert was ignored as a duplicate.
///
/// A missing `content` is stored as `''` rather than `NULL`, so the dedup
/// index can compare it directly without a `COALESCE`.
pub fn insert_message(
    conn: &Connection,
    conversation_id: i64,
    sender_id: Option<i64>,
    message: &RawMessage,
) -> rusqlite::Result<Option<i64>> {
    conn.execute(
        "INSERT OR IGNORE INTO messages (conversation_id, sender_id, timestamp_ms, content) \
         VALUES (?1, ?2, ?3, ?4)",
        params![
            conversation_id,
            sender_id,
            message.timestamp_ms,
            message.content.as_deref().unwrap_or(""),
        ],
    )?;
    Ok(if conn.changes() == 0 {
        None
    } else {
        Some(conn.last_insert_rowid())
    })
}
