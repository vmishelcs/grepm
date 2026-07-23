use std::error::Error;

use rusqlite::Connection;

use crate::db;
use crate::ingest::parse::parse_conversation_file;
use crate::ingest::scan::ConversationDir;

/// Loads a single conversation into the database: inserts the thread
/// (`conversations` row) and its participants, all within one transaction.
/// Message loading is not implemented yet.
pub fn load_conversation(
    conn: &mut Connection,
    conversation_dir: &ConversationDir,
) -> Result<i64, Box<dyn Error>> {
    let first_file = conversation_dir
        .message_files
        .first()
        .ok_or("conversation has no message files")?;
    let raw = parse_conversation_file(first_file)?;

    let tx = conn.transaction()?;

    let conversation_id = db::upsert_conversation(&tx, &conversation_dir.raw_name, &raw)?;

    for raw_participant in &raw.participants {
        let participant_id = db::insert_participant(&tx, raw_participant)?;
        db::link_conversation_participant(&tx, conversation_id, participant_id)?;
    }

    tx.commit()?;

    Ok(conversation_id)
}
