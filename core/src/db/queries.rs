use rusqlite::{params, Connection};

use crate::ingest::parse::{RawConversationFile, RawMessage};

/// Inserts a conversation, or, if `raw_name` already has a row (e.g. a
/// conversation split across multiple `message_N.json` files), updates its
/// title/is_still_participant/thread_path and adds this file's message
/// count onto the running total. Returns the conversation's id either way.
pub fn upsert_conversation(
    conn: &Connection,
    raw_name: &str,
    conversation: &RawConversationFile,
) -> rusqlite::Result<i64> {
    let message_count = conversation.messages.len() as i64;
    conn.query_row(
        "INSERT INTO conversations (raw_name, title, is_still_participant, thread_path, message_count) \
         VALUES (?1, ?2, ?3, ?4, ?5) \
         ON CONFLICT (raw_name) DO UPDATE SET \
             title = excluded.title, \
             is_still_participant = excluded.is_still_participant, \
             thread_path = excluded.thread_path, \
             message_count = message_count + excluded.message_count \
         RETURNING id",
        params![
            raw_name,
            conversation.title,
            conversation.is_still_participant,
            conversation.thread_path,
            message_count,
        ],
        |row| row.get(0),
    )
}

/// Inserts a participant, or, if `name` already has a row (see the
/// `participants` table's UNIQUE constraint), leaves it untouched, so
/// importing the same person across many conversations doesn't create
/// duplicate rows. Returns the participant's id either way. The `DO
/// UPDATE` is a no-op (it just reassigns the same name) rather than `DO
/// NOTHING`, since SQLite's `RETURNING` doesn't produce a row for a
/// `DO NOTHING` conflict.
pub fn insert_participant(conn: &Connection, name: &str) -> rusqlite::Result<i64> {
    conn.query_row(
        "INSERT INTO participants (name) VALUES (?1) \
         ON CONFLICT (name) DO UPDATE SET name = excluded.name \
         RETURNING id",
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
