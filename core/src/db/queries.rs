use rusqlite::{params, Connection};

use crate::db::models::{Conversation, ConversationParticipant, Message, Participant};

pub fn insert_conversation(
    conn: &Connection,
    conversation: &Conversation,
) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO conversations (raw_name, title, is_still_participant, thread_path, message_count) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            conversation.raw_name,
            conversation.title,
            conversation.is_still_participant,
            conversation.thread_path,
            conversation.message_count,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn insert_participant(conn: &Connection, participant: &Participant) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO participants (name) VALUES (?1)",
        params![participant.name],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn insert_conversation_participant(
    conn: &Connection,
    conversation_participant: &ConversationParticipant,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO conversation_participants (conversation_id, participant_id) VALUES (?1, ?2)",
        params![
            conversation_participant.conversation_id,
            conversation_participant.participant_id,
        ],
    )?;
    Ok(())
}

/// Inserts a message, ignoring it if a message with the same
/// conversation_id, participant_id, timestamp_ms, and content already
/// exists (see the `messages` table's UNIQUE constraint). Returns the new
/// row's id, or `None` if the insert was ignored as a duplicate.
pub fn insert_message(conn: &Connection, message: &Message) -> rusqlite::Result<Option<i64>> {
    conn.execute(
        "INSERT OR IGNORE INTO messages (conversation_id, participant_id, timestamp_ms, content) \
         VALUES (?1, ?2, ?3, ?4)",
        params![
            message.conversation_id,
            message.participant_id,
            message.timestamp_ms,
            message.content,
        ],
    )?;
    Ok(if conn.changes() == 0 {
        None
    } else {
        Some(conn.last_insert_rowid())
    })
}
