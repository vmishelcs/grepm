use std::path::Path;

use rusqlite::Connection;

use crate::{Error, Result};

/// The schema, as an append-only list of migration steps: index `n` takes a
/// database from `user_version` `n` to `n + 1`. Never edit or reorder an
/// existing entry — a shipped database has already run it — only append.
pub const MIGRATIONS: &[&str] = &[r#"
    CREATE TABLE IF NOT EXISTS conversations (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        title TEXT NOT NULL,
        is_still_participant INTEGER,
        thread_path TEXT NOT NULL,
        message_count INTEGER NOT NULL DEFAULT 0,
        UNIQUE (title, thread_path)
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
        sender_id INTEGER,
        timestamp_ms INTEGER NOT NULL,
        content TEXT,
        type TEXT NOT NULL,
        attachment_count INTEGER NOT NULL DEFAULT 0,
        FOREIGN KEY (conversation_id) REFERENCES conversations (id),
        FOREIGN KEY (sender_id) REFERENCES participants (id)
    );

    CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_dedup ON messages (
        conversation_id,
        COALESCE(sender_id, -1),
        timestamp_ms,
        COALESCE(content, ''),
        type,
        attachment_count
    );

    CREATE TABLE IF NOT EXISTS attachments (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        message_id INTEGER NOT NULL,
        type TEXT NOT NULL,
        uri TEXT,
        creation_timestamp INTEGER,
        FOREIGN KEY (message_id) REFERENCES messages (id)
    );

    CREATE TABLE IF NOT EXISTS reactions (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        message_id INTEGER NOT NULL,
        actor_id INTEGER NOT NULL,
        reaction TEXT NOT NULL,
        FOREIGN KEY (message_id) REFERENCES messages (id),
        FOREIGN KEY (actor_id) REFERENCES participants (id)
    );

    CREATE VIRTUAL TABLE IF NOT EXISTS messages_fts USING fts5(
        content,
        content='messages',
        content_rowid='id',
        tokenize='unicode61 remove_diacritics 2'
    );
    "#];

/// The schema version a fully migrated database sits at — one step per
/// entry in [`MIGRATIONS`], so adding a migration is a one-place change
/// that can't be left half-done by forgetting to bump a hand-written
/// constant.
pub const LATEST_VERSION: i32 = MIGRATIONS.len() as i32;

pub fn open(path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    configure(&conn)?;
    migrate(&mut conn)?;
    Ok(conn)
}

pub fn configure(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

pub fn migrate(conn: &mut Connection) -> Result<()> {
    let current_version: i32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;

    // `user_version` is a signed pragma, so a corrupt or hand-edited file
    // can hold a negative value that no build of this app ever wrote.
    // Refusing it here also keeps `version as usize` below from wrapping to
    // a huge index and panicking on MIGRATIONS.
    if current_version < 0 {
        return Err(Error::InvalidSchemaVersion {
            found: current_version,
        });
    }

    // A version beyond LATEST_VERSION means the database was created by a
    // newer build (or isn't ours at all). Proceeding would mean reading and
    // writing a schema this code doesn't understand — refuse instead of
    // silently treating it as up to date.
    if current_version > LATEST_VERSION {
        return Err(Error::UnsupportedSchemaVersion {
            found: current_version,
            supported: LATEST_VERSION,
        });
    }

    for version in current_version..LATEST_VERSION {
        // An uncommitted `Transaction` rolls back when dropped, so a failed
        // migration can't strand the connection inside an open transaction.
        let tx = conn.transaction()?;
        tx.execute_batch(MIGRATIONS[version as usize])?;
        tx.pragma_update(None, "user_version", version + 1)?;
        tx.commit()?;
    }

    Ok(())
}

pub fn populate_fts(conn: &Connection) -> Result<usize> {
    Ok(conn.execute(
        "INSERT INTO messages_fts(rowid, content) \
         SELECT id, content FROM messages \
         WHERE content IS NOT NULL AND content != ''",
        [],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::migrated_connection;

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
        conn.execute(
            "INSERT INTO conversations (title, thread_path, is_still_participant) \
             VALUES ('test', 'inbox/test', 1)",
            [],
        )
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
                "attachments",
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
        let mut conn = migrated_connection();

        migrate(&mut conn).unwrap();

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, LATEST_VERSION);
    }

    #[test]
    fn migrate_applies_nothing_once_already_at_latest_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        conn.pragma_update(None, "user_version", LATEST_VERSION)
            .unwrap();

        // If this didn't stop at LATEST_VERSION, it would index past the end
        // of MIGRATIONS and panic.
        migrate(&mut conn).unwrap();

        assert!(table_names(&conn).is_empty());
    }

    #[test]
    fn migrate_rejects_a_database_from_a_newer_app_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        conn.pragma_update(None, "user_version", LATEST_VERSION + 1)
            .unwrap();

        let err = migrate(&mut conn).unwrap_err();

        assert!(
            matches!(
                err,
                Error::UnsupportedSchemaVersion { found, supported }
                    if found == LATEST_VERSION + 1 && supported == LATEST_VERSION
            ),
            "a user_version beyond LATEST_VERSION should be refused, got: {err:?}"
        );
    }

    #[test]
    fn conversations_reject_a_null_title_or_thread_path() {
        let conn = migrated_connection();

        for (column, sql) in [
            (
                "title",
                "INSERT INTO conversations (title, thread_path) VALUES (NULL, 'inbox/a')",
            ),
            (
                "thread_path",
                "INSERT INTO conversations (title, thread_path) VALUES ('Alice', NULL)",
            ),
        ] {
            assert!(
                conn.execute(sql, []).is_err(),
                "a NULL {column} would be distinct from every other row in the \
                 UNIQUE index, so the conversation upsert could never merge it"
            );
        }
    }

    #[test]
    fn migrate_rejects_a_negative_user_version_instead_of_panicking() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        // No build writes this; only a corrupt or hand-tampered file has it.
        // Unguarded, `version as usize` would wrap and index MIGRATIONS far
        // out of bounds.
        conn.pragma_update(None, "user_version", -1).unwrap();

        let err = migrate(&mut conn).unwrap_err();

        assert!(
            matches!(err, Error::InvalidSchemaVersion { found } if found == -1),
            "a negative user_version should be refused as invalid, got: {err:?}"
        );
    }

    #[test]
    fn migrate_rolls_back_a_failed_migration_and_leaves_the_connection_usable() {
        let mut conn = Connection::open_in_memory().unwrap();
        configure(&conn).unwrap();
        // Sabotage the migration partway through: with a pre-existing
        // `messages` table already holding duplicate rows, the migration's
        // CREATE UNIQUE INDEX idx_messages_dedup genuinely fails (IF NOT
        // EXISTS only skips creation when the *index* already exists).
        conn.execute_batch(
            "CREATE TABLE messages (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id INTEGER NOT NULL,
                sender_id INTEGER,
                timestamp_ms INTEGER NOT NULL,
                content TEXT NOT NULL DEFAULT ''
            );
            INSERT INTO messages (conversation_id, sender_id, timestamp_ms, content)
                VALUES (1, 1, 0, 'dup'), (1, 1, 0, 'dup');",
        )
        .unwrap();

        let result = migrate(&mut conn);

        assert!(result.is_err(), "the sabotaged migration should fail");

        let version: i32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(
            version, 0,
            "a failed migration should not bump user_version"
        );

        assert!(
            conn.transaction().is_ok(),
            "the failed migration should not leave an open transaction behind"
        );
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
            "INSERT INTO messages (conversation_id, sender_id, timestamp_ms, type) \
             VALUES (1, 1, 0, 'text')",
            [],
        );

        assert!(
            result.is_err(),
            "insert referencing a nonexistent conversation/participant should fail"
        );
    }

    #[test]
    fn duplicate_messages_are_rejected() {
        let conn = migrated_connection();
        let (conversation_id, sender_id) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, 0, 'hello', 'text')",
            rusqlite::params![conversation_id, sender_id],
        )
        .unwrap();

        let result = conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, 0, 'hello', 'text')",
            rusqlite::params![conversation_id, sender_id],
        );

        assert!(
            result.is_err(),
            "inserting the same conversation_id/sender_id/timestamp_ms/content \
             combination twice should fail"
        );
    }

    #[test]
    fn duplicate_messages_with_null_sender_id_are_rejected() {
        let conn = migrated_connection();
        let (conversation_id, _) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages (conversation_id, timestamp_ms, content, type) \
             VALUES (?1, 0, 'hello', 'text')",
            rusqlite::params![conversation_id],
        )
        .unwrap();

        let result = conn.execute(
            "INSERT INTO messages (conversation_id, timestamp_ms, content, type) \
             VALUES (?1, 0, 'hello', 'text')",
            rusqlite::params![conversation_id],
        );

        assert!(
            result.is_err(),
            "two messages with the same conversation_id/timestamp_ms/content \
             and a NULL sender_id should be recognized as duplicates"
        );
    }

    #[test]
    fn messages_differing_only_in_content_are_not_duplicates() {
        let conn = migrated_connection();
        let (conversation_id, sender_id) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, 0, 'hello', 'text')",
            rusqlite::params![conversation_id, sender_id],
        )
        .unwrap();

        let result = conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, 0, 'goodbye', 'text')",
            rusqlite::params![conversation_id, sender_id],
        );

        assert!(result.is_ok());
    }

    #[test]
    fn attachments_referencing_a_nonexistent_message_are_rejected() {
        let conn = migrated_connection();

        let result = conn.execute(
            "INSERT INTO attachments (message_id, type, uri) \
             VALUES (999, 'photos', 'photos/1.jpg')",
            [],
        );

        assert!(
            result.is_err(),
            "attachments.message_id has a FOREIGN KEY to messages"
        );
    }

    #[test]
    fn messages_differing_only_in_message_type_are_not_duplicates() {
        // Two attachment-only messages share content = '' — without
        // type in the dedup index, a photo and a video sent by the
        // same person in the same millisecond would collide.
        let conn = migrated_connection();
        let (conversation_id, sender_id) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, 0, '', 'photos')",
            rusqlite::params![conversation_id, sender_id],
        )
        .unwrap();

        let result = conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, 0, '', 'videos')",
            rusqlite::params![conversation_id, sender_id],
        );

        assert!(result.is_ok());
    }

    #[test]
    fn messages_differing_only_in_attachment_count_are_not_duplicates() {
        // e.g. one photo vs. two photos sent by the same person in the same
        // millisecond: same content (''), same type ('photos').
        let conn = migrated_connection();
        let (conversation_id, sender_id) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type, \
              attachment_count) \
             VALUES (?1, ?2, 0, '', 'photos', 1)",
            rusqlite::params![conversation_id, sender_id],
        )
        .unwrap();

        let result = conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type, \
              attachment_count) \
             VALUES (?1, ?2, 0, '', 'photos', 2)",
            rusqlite::params![conversation_id, sender_id],
        );

        assert!(result.is_ok());
    }

    #[test]
    fn messages_require_an_explicit_message_type() {
        let conn = migrated_connection();
        let (conversation_id, sender_id) = seed_conversation_and_participant(&conn);

        let result = conn.execute(
            "INSERT INTO messages (conversation_id, sender_id, timestamp_ms, content) \
             VALUES (?1, ?2, 0, 'hello')",
            rusqlite::params![conversation_id, sender_id],
        );

        assert!(
            result.is_err(),
            "type has no DEFAULT; an insert that omits it should fail"
        );
    }

    #[test]
    fn messages_reject_a_null_message_type() {
        let conn = migrated_connection();
        let (conversation_id, sender_id) = seed_conversation_and_participant(&conn);

        let result = conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, 0, 'hello', NULL)",
            rusqlite::params![conversation_id, sender_id],
        );

        assert!(
            result.is_err(),
            "type is NOT NULL; an explicit NULL insert should fail"
        );
    }

    #[test]
    fn messages_store_an_explicit_message_type() {
        let conn = migrated_connection();
        let (conversation_id, sender_id) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, 0, '', 'photos')",
            rusqlite::params![conversation_id, sender_id],
        )
        .unwrap();

        let message_type: String = conn
            .query_row("SELECT type FROM messages", [], |row| row.get(0))
            .unwrap();
        assert_eq!(message_type, "photos");
    }

    #[test]
    fn populate_fts_indexes_existing_messages() {
        let conn = migrated_connection();
        let (conversation_id, sender_id) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, 0, 'hello world', 'text')",
            rusqlite::params![conversation_id, sender_id],
        )
        .unwrap();
        let message_id = conn.last_insert_rowid();

        populate_fts(&conn).unwrap();

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
    fn populate_fts_does_not_match_absent_terms() {
        let conn = migrated_connection();
        let (conversation_id, sender_id) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, 0, 'apples and oranges', 'text')",
            rusqlite::params![conversation_id, sender_id],
        )
        .unwrap();

        populate_fts(&conn).unwrap();

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
    fn populate_fts_skips_empty_content() {
        let conn = migrated_connection();
        let (conversation_id, sender_id) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, 0, '', 'text')",
            rusqlite::params![conversation_id, sender_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, 0, 'has content', 'text')",
            rusqlite::params![conversation_id, sender_id],
        )
        .unwrap();
        // A NULL-content (e.g. attachment-only) message must be skipped too.
        conn.execute(
            "INSERT INTO messages (conversation_id, sender_id, timestamp_ms, type) \
             VALUES (?1, ?2, 1, 'photos')",
            rusqlite::params![conversation_id, sender_id],
        )
        .unwrap();

        // A plain SELECT against an external-content FTS5 table (no MATCH)
        // reads through to the backing `messages` table rather than the
        // search index, so it can't be used to check what got indexed.
        // The affected-row count from the populating INSERT is the only
        // reliable signal here.
        let indexed_count = populate_fts(&conn).unwrap();
        assert_eq!(indexed_count, 1);
    }

    #[test]
    fn duplicate_messages_with_null_content_are_rejected() {
        // NULLs are distinct in a UNIQUE index, so without the
        // COALESCE(content, '') in idx_messages_dedup, re-importing an
        // attachment-only (NULL content) message would duplicate it.
        let conn = migrated_connection();
        let (conversation_id, sender_id) = seed_conversation_and_participant(&conn);

        conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, type, attachment_count) \
             VALUES (?1, ?2, 0, 'photos', 1)",
            rusqlite::params![conversation_id, sender_id],
        )
        .unwrap();

        let result = conn.execute(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, type, attachment_count) \
             VALUES (?1, ?2, 0, 'photos', 1)",
            rusqlite::params![conversation_id, sender_id],
        );

        assert!(
            result.is_err(),
            "two identical NULL-content messages should be recognized as duplicates"
        );
    }
}
