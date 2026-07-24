use rusqlite::Row;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Conversation {
    pub id: i64,
    pub title: Option<String>,
    pub is_still_participant: Option<bool>,
    pub thread_path: Option<String>,
    pub message_count: i64,
}

impl Conversation {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            title: row.get("title")?,
            is_still_participant: row.get("is_still_participant")?,
            thread_path: row.get("thread_path")?,
            message_count: row.get("message_count")?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Participant {
    pub id: i64,
    pub name: String,
}

impl Participant {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            name: row.get("name")?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ConversationParticipant {
    pub conversation_id: i64,
    pub participant_id: i64,
}

impl ConversationParticipant {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            conversation_id: row.get("conversation_id")?,
            participant_id: row.get("participant_id")?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub conversation_id: i64,
    pub sender_id: Option<i64>,
    pub timestamp_ms: i64,
    /// Never absent: the schema declares `content TEXT NOT NULL DEFAULT ''`
    /// (ingestion stores a missing content as `''`), so an `Option` here
    /// would invite handling a `None` that can't occur.
    pub content: String,
}

impl Message {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            conversation_id: row.get("conversation_id")?,
            sender_id: row.get("sender_id")?,
            timestamp_ms: row.get("timestamp_ms")?,
            content: row.get("content")?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Reaction {
    pub id: i64,
    pub message_id: i64,
    pub actor_id: i64,
    pub reaction: String,
}

impl Reaction {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            message_id: row.get("message_id")?,
            actor_id: row.get("actor_id")?,
            reaction: row.get("reaction")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use super::*;
    use crate::db::schema;

    // Exercises `Message::from_row` against the real migrated schema (not a
    // hand-built row), so the mapper and the schema can't silently drift
    // apart — in particular, `content`'s NOT NULL DEFAULT ''.
    #[test]
    fn message_from_row_maps_a_row_with_defaulted_content_and_null_sender() {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::configure(&conn).unwrap();
        schema::migrate(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO conversations (title, thread_path) VALUES ('t', 'inbox/t')",
            [],
        )
        .unwrap();
        let conversation_id = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO messages (conversation_id, timestamp_ms) VALUES (?1, 5000)",
            [conversation_id],
        )
        .unwrap();

        let message = conn
            .query_row("SELECT * FROM messages", [], Message::from_row)
            .unwrap();

        assert_eq!(message.conversation_id, conversation_id);
        assert_eq!(message.sender_id, None);
        assert_eq!(message.timestamp_ms, 5000);
        assert_eq!(message.content, "");
    }
}
