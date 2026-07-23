use std::path::Path;

use rusqlite::Connection;

pub const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE IF NOT EXISTS conversations (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        title TEXT NOT NULL,
        message_count INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS participants (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS conversation_participants (
        conversation_id INTEGER NOT NULL,
        participant_id INTEGER NOT NULL,
        PRIMARY KEY (conversation_id, participant_id),
        FOREIGN KEY (conversation_id) REFERENCES conversations (id),
        FOREIGN KEY (participant_id) REFERENCES participants (id)
    );

    CREATE TABLE IF NOT EXISTS messages (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        conversation_id INTEGER NOT NULL,
        participant_id INTEGER NOT NULL,
        timestamp_ms INTEGER NOT NULL,
        content TEXT,
        FOREIGN KEY (conversation_id) REFERENCES conversations (id),
        FOREIGN KEY (participant_id) REFERENCES participants (id)
    );

    CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
        content,
        content='messages',
        content_rowid='id'
    );

    CREATE TABLE IF NOT EXISTS reactions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        message_id INTEGER NOT NULL,
        actor_id INTEGER NOT NULL,
        reaction TEXT NOT NULL,
        FOREIGN KEY (message_id) REFERENCES messages (id),
        FOREIGN KEY (actor_id) REFERENCES participants (id)
    );
    "#,
];

pub const LATEST_VERSION: i32 = 1;

pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    let conn = Connection::open(path)?;
    configure(&conn)?;
    migrate(&conn)?;
    Ok(conn)
}

pub fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let current_version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for version in current_version..LATEST_VERSION {
        conn.execute_batch("BEGIN;")?;
        conn.execute_batch(MIGRATIONS[version as usize])?;
        conn.pragma_update(None, "user_version", version + 1)?;
        conn.execute_batch("COMMIT;")?;
    }

    Ok(())
}

pub fn populate_messages_fts(conn: &Connection) -> rusqlite::Result<usize> {
    conn.execute(
        "INSERT INTO messages_fts(rowid, content) \
         SELECT id, content FROM messages WHERE content IS NOT NULL",
        [],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn migrated_connection() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn table_names(conn: &Connection) -> Vec<String> {
        conn.prepare(
            // messages_fts's shadow tables (messages_fts_data, _idx, _docsize,
            // _config) are FTS5 implementation detail, not schema we own.
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' \
             AND name NOT LIKE 'sqlite_%' \
             AND name NOT LIKE 'messages\\_fts\\_%' ESCAPE '\\' \
             ORDER BY name",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
    }

    fn seed_conversation_and_participant(conn: &Connection) -> (i64, i64) {
        conn.execute("INSERT INTO conversations (title) VALUES ('test')", [])
            .unwrap();
        let conversation_id = conn.last_insert_rowid();

        conn.execute("INSERT INTO participants (name) VALUES ('alice')", [])
            .unwrap();
        let participant_id = conn.last_insert_rowid();

        (conversation_id, participant_id)
    }

    #[test]
    fn migrate_creates_all_tables() {
        let conn = migrated_connection();

        assert_eq!(
            table_names(&conn),
            vec![
                "conversation_participants",
                "conversations",
                "messages",
                "messages_fts",
                "participants",
                "reactions",
            ]
        );
    }

    #[test]
    fn migrate_sets_user_version_to_latest() {
        let conn = migrated_connection();

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, LATEST_VERSION);
    }

    #[test]
    fn migrate_is_idempotent() {
        let conn = migrated_connection();

        migrate(&conn).unwrap();

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, LATEST_VERSION);
    }

    #[test]
    fn migrate_applies_nothing_once_already_at_latest_version() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        conn.pragma_update(None, "user_version", LATEST_VERSION)
            .unwrap();

        // If this didn't stop at LATEST_VERSION, it would index past the end
        // of MIGRATIONS and panic.
        migrate(&conn).unwrap();

        assert!(table_names(&conn).is_empty());
    }

    #[test]
    fn configure_sets_synchronous_normal() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();

        // NORMAL == 1, see https://www.sqlite.org/pragma.html#pragma_synchronous
        let synchronous: i64 = conn
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .unwrap();
        assert_eq!(synchronous, 1);
    }

    #[test]
    fn configure_sets_foreign_keys_on() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();

        let foreign_keys: bool = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert!(foreign_keys);
    }

    #[test]
    fn configure_requests_wal_journal_mode() {
        let conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();

        // SQLite silently keeps in-memory databases on "memory" journal mode
        // regardless of what's requested, so this only proves the pragma call
        // itself succeeds; WAL-on-disk behavior isn't exercisable in-memory.
        let journal_mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        assert_eq!(journal_mode, "memory");
    }

    #[test]
    fn foreign_keys_are_enforced_after_migration() {
        let conn = migrated_connection();

        let result = conn.execute(
            "INSERT INTO messages (conversation_id, participant_id, timestamp_ms) \
             VALUES (1, 1, 0)",
            [],
        );

        assert!(
            result.is_err(),
            "insert referencing a nonexistent conversation/participant should fail"
        );
    }

    #[test]
    fn populate_messages_fts_indexes_existing_messages() {
        let conn = migrated_connection();
        let (conversation_id, participant_id) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages (conversation_id, participant_id, timestamp_ms, content) \
             VALUES (?1, ?2, 0, 'hello world')",
            rusqlite::params![conversation_id, participant_id],
        )
        .unwrap();
        let message_id = conn.last_insert_rowid();

        populate_messages_fts(&conn).unwrap();

        let matched: i64 = conn
            .query_row(
                "SELECT rowid FROM messages_fts WHERE messages_fts MATCH 'hello'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(matched, message_id);
    }

    #[test]
    fn populate_messages_fts_does_not_match_absent_terms() {
        let conn = migrated_connection();
        let (conversation_id, participant_id) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages (conversation_id, participant_id, timestamp_ms, content) \
             VALUES (?1, ?2, 0, 'apples and oranges')",
            rusqlite::params![conversation_id, participant_id],
        )
        .unwrap();

        populate_messages_fts(&conn).unwrap();

        let match_count: i64 = conn
            .query_row(
                "SELECT count(*) FROM messages_fts WHERE messages_fts MATCH 'bananas'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(match_count, 0);
    }

    #[test]
    fn populate_messages_fts_skips_null_content() {
        let conn = migrated_connection();
        let (conversation_id, participant_id) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages (conversation_id, participant_id, timestamp_ms, content) \
             VALUES (?1, ?2, 0, NULL)",
            rusqlite::params![conversation_id, participant_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages (conversation_id, participant_id, timestamp_ms, content) \
             VALUES (?1, ?2, 0, 'has content')",
            rusqlite::params![conversation_id, participant_id],
        )
        .unwrap();

        // A plain SELECT against an external-content FTS5 table (no MATCH)
        // reads through to the backing `messages` table rather than the
        // search index, so it can't be used to check what got indexed.
        // The affected-row count from the populating INSERT is the only
        // reliable signal here.
        let indexed_count = populate_messages_fts(&conn).unwrap();
        assert_eq!(indexed_count, 1);
    }
}
