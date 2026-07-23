use rusqlite::Row;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: i64,
    pub raw_name: String,
    pub title: Option<String>,
    pub is_still_participant: bool,
    pub message_count: i64,
}

impl Conversation {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            raw_name: row.get("raw_name")?,
            title: row.get("title")?,
            is_still_participant: row.get("is_still_participant")?,
            message_count: row.get("message_count")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: i64,
    pub conversation_id: i64,
    pub participant_id: i64,
    pub timestamp_ms: i64,
    pub content: Option<String>,
}

impl Message {
    pub fn from_row(row: &Row) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            conversation_id: row.get("conversation_id")?,
            participant_id: row.get("participant_id")?,
            timestamp_ms: row.get("timestamp_ms")?,
            content: row.get("content")?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
