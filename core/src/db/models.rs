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
    /// `None` for messages without text (e.g. attachment-only messages);
    /// stored as `NULL` in the schema.
    pub content: Option<String>,
    /// One of `text`, `photos`, `videos`, `audio_files`, or `gifs`
    /// (`NOT NULL` in the schema, no default; ingestion classifies every
    /// message via `RawMessage::message_type`). Named `type` in the schema;
    /// renamed here because `type` is a Rust keyword.
    #[serde(rename = "type")]
    pub message_type: String,
    /// Total attachments across all lists (`0` for a plain text message);
    /// part of the dedup key, see `RawMessage::attachment_count`.
    pub attachment_count: i64,
}

impl Message {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            conversation_id: row.get("conversation_id")?,
            sender_id: row.get("sender_id")?,
            timestamp_ms: row.get("timestamp_ms")?,
            content: row.get("content")?,
            message_type: row.get("type")?,
            attachment_count: row.get("attachment_count")?,
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Attachment {
    pub id: i64,
    pub message_id: i64,
    /// The attachment list this entry came from: `photos`, `videos`,
    /// `audio_files`, or `gifs`. Named `type` in the schema; renamed here
    /// because `type` is a Rust keyword.
    #[serde(rename = "type")]
    pub attachment_type: String,
    pub uri: Option<String>,
    /// Unix timestamp in *seconds* (as in the export), unlike
    /// `Message::timestamp_ms`.
    pub creation_timestamp: Option<i64>,
}

impl Attachment {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            message_id: row.get("message_id")?,
            attachment_type: row.get("type")?,
            uri: row.get("uri")?,
            creation_timestamp: row.get("creation_timestamp")?,
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
    // apart — in particular, `content`'s nullability and the `type` column
    // name.
    #[test]
    fn message_from_row_maps_a_row_with_null_content_and_null_sender() {
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
            "INSERT INTO messages (conversation_id, timestamp_ms, type) \
             VALUES (?1, 5000, 'text')",
            [conversation_id],
        )
        .unwrap();

        let message = conn
            .query_row("SELECT * FROM messages", [], Message::from_row)
            .unwrap();

        assert_eq!(message.conversation_id, conversation_id);
        assert_eq!(message.sender_id, None);
        assert_eq!(message.timestamp_ms, 5000);
        assert_eq!(message.content, None);
        assert_eq!(message.message_type, "text");
        assert_eq!(message.attachment_count, 0);
    }
}
