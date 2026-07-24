//! End-to-end search tests: like `ingestion.rs`, these run against a real
//! SQLite file on disk (via `import_export`), then exercise `FtsIndex`
//! exactly as the rest of the app would through the `SearchIndex` trait.
//!
//! Several scenarios below (token-exact vs. substring matching, and
//! diacritic-insensitive matching) only pass if `search` is actually going
//! through the `messages_fts` index and its `unicode61 remove_diacritics 2`
//! tokenizer — a query run against the raw `messages` table (e.g. a `LIKE
//! '%text%'` scan) would give different results, which is what these are
//! checking for.

use std::fs;
use std::path::Path;

use rusqlite::Connection;
use tempfile::{tempdir, TempDir};

use grepm_core::db;
use grepm_core::ingest::import_export;
use grepm_core::search::fts::FtsIndex;
use grepm_core::search::{Page, SearchIndex, SearchQuery, SearchResults, SortOrder, UiFilters};

fn write_file(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// Imports a small, deliberately varied export into a real on-disk database:
/// several conversations, several participants, and messages chosen so that
/// filter/sort/tokenizer scenarios below have exactly one correct answer.
///
/// Matches for the text "coffee" (the term most tests search for), by
/// conversation/sender/timestamp_ms:
///   - Team Chat / Alice   / 1000 "let's grab coffee before the meeting"
///   - Team Chat / Bob     / 2000 "coffee sounds good, I'll bring donuts"
///   - Weekend Trip / Carol / 1500 "who's up for a coffeehouse crawl" — does
///     NOT match: "coffeehouse" is a different token than "coffee".
///   - Weekend Trip / Alice / 2500 "count me in for coffee"
///   - Coffee Talk / Frank  / 1750 "coffee coffee coffee, I love coffee" —
///     four occurrences, timestamp deliberately between the other matches
///     (neither newest nor oldest) so a relevance sort can't be confused
///     with a chronological one.
///   - DM with Dave / Dave  / 4000 "café was great..." (mojibake-repaired) —
///     does not match "coffee", but does match a diacritic-insensitive
///     search for "cafe".
fn seeded_db() -> (TempDir, Connection) {
    let export = tempdir().unwrap();
    let db_dir = tempdir().unwrap();
    let inbox = export.path().join("messages").join("inbox");

    write_file(
        &inbox.join("team_1111").join("message_1.json"),
        r#"{
            "participants": [{"name": "Alice"}, {"name": "Bob"}],
            "messages": [
                {"sender_name": "Alice", "timestamp_ms": 1000, "content": "let's grab coffee before the meeting"},
                {"sender_name": "Bob", "timestamp_ms": 2000, "content": "coffee sounds good, I'll bring donuts"},
                {"sender_name": "Alice", "timestamp_ms": 3000, "content": "actually let's do tea instead"}
            ],
            "title": "Team Chat",
            "is_still_participant": true,
            "thread_path": "inbox/team_1111"
        }"#,
    );

    write_file(
        &inbox.join("trip_2222").join("message_1.json"),
        r#"{
            "participants": [{"name": "Alice"}, {"name": "Carol"}],
            "messages": [
                {"sender_name": "Carol", "timestamp_ms": 1500, "content": "who's up for a coffeehouse crawl"},
                {"sender_name": "Alice", "timestamp_ms": 2500, "content": "count me in for coffee"}
            ],
            "title": "Weekend Trip",
            "is_still_participant": true,
            "thread_path": "inbox/trip_2222"
        }"#,
    );

    write_file(
        &inbox.join("dm_3333").join("message_1.json"),
        r#"{
            "participants": [{"name": "Dave"}],
            "messages": [
                {"sender_name": "Dave", "timestamp_ms": 4000, "content": "cafÃ© was great, thanks for the recommendation"}
            ],
            "title": "DM with Dave",
            "is_still_participant": true,
            "thread_path": "inbox/dm_3333"
        }"#,
    );

    write_file(
        &inbox.join("coffee_4444").join("message_1.json"),
        r#"{
            "participants": [{"name": "Frank"}],
            "messages": [
                {"sender_name": "Frank", "timestamp_ms": 1750, "content": "coffee coffee coffee, I love coffee"}
            ],
            "title": "Coffee Talk",
            "is_still_participant": true,
            "thread_path": "inbox/coffee_4444"
        }"#,
    );

    let mut conn = db::schema::open(&db_dir.path().join("grepm.sqlite3")).unwrap();
    import_export(&mut conn, export.path()).unwrap();

    (db_dir, conn)
}

fn query(text: &str, filters: UiFilters, sort: SortOrder) -> SearchQuery {
    SearchQuery {
        text: text.to_string(),
        filters,
        sort,
    }
}

fn search(conn: &Connection, query: SearchQuery, page: Page) -> SearchResults {
    FtsIndex::new(conn).search(&query, page).unwrap()
}

#[test]
fn matches_text_across_every_conversation_with_no_filters() {
    let (_db_dir, conn) = seeded_db();

    let results = search(
        &conn,
        query("coffee", UiFilters::default(), SortOrder::Latest),
        Page::default(),
    );

    assert_eq!(results.count, 4);
    assert_eq!(results.hits.len(), 4);
}

#[test]
fn matches_are_token_exact_not_a_raw_substring_scan() {
    let (_db_dir, conn) = seeded_db();

    let results = search(
        &conn,
        query("coffee", UiFilters::default(), SortOrder::Latest),
        Page::default(),
    );

    // "coffeehouse" contains "coffee" as a substring (a `LIKE '%coffee%'`
    // scan over the raw messages table would match it), but it's a
    // different FTS5 token, so it must not appear among the hits.
    assert!(
        results
            .hits
            .iter()
            .all(|hit| !hit.snippet.to_lowercase().contains("coffeehouse")),
        "a search for 'coffee' matched 'coffeehouse', meaning it isn't going \
         through FTS5 token matching"
    );
    assert_eq!(results.count, 4);
}

#[test]
fn matches_diacritic_insensitively_via_the_fts_tokenizer() {
    let (_db_dir, conn) = seeded_db();

    // Only matches if the query goes through messages_fts's
    // `unicode61 remove_diacritics 2` tokenizer: the stored content is
    // "café" (after mojibake repair) but the query text has no diacritic.
    let results = search(
        &conn,
        query("cafe", UiFilters::default(), SortOrder::Latest),
        Page::default(),
    );

    assert_eq!(results.count, 1);
    let hit = &results.hits[0];
    assert_eq!(hit.conversation_title.as_deref(), Some("DM with Dave"));
    assert_eq!(hit.sender_name.as_deref(), Some("Dave"));
    assert_eq!(hit.timestamp_ms, 4000);
    assert!(
        hit.snippet.contains("[café]"),
        "snippet should highlight the matched word using the '[' ']' markers: {}",
        hit.snippet
    );
}

#[test]
fn returns_no_results_for_text_that_matches_nothing() {
    let (_db_dir, conn) = seeded_db();

    let results = search(
        &conn,
        query("xylophone", UiFilters::default(), SortOrder::Latest),
        Page::default(),
    );

    assert_eq!(results.count, 0);
    assert!(results.hits.is_empty());
}

#[test]
fn filters_by_conversation_id() {
    let (_db_dir, conn) = seeded_db();

    let team_chat_id: i64 = conn
        .query_row(
            "SELECT id FROM conversations WHERE title = 'Team Chat'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let filters = UiFilters {
        conversation_id: Some(team_chat_id),
        ..Default::default()
    };
    let results = search(
        &conn,
        query("coffee", filters, SortOrder::Latest),
        Page::default(),
    );

    assert_eq!(results.count, 2);
    assert!(results
        .hits
        .iter()
        .all(|hit| hit.conversation_id == team_chat_id));
}

#[test]
fn filters_by_participant() {
    let (_db_dir, conn) = seeded_db();

    let filters = UiFilters {
        participant: Some("Alice".to_string()),
        ..Default::default()
    };
    let results = search(
        &conn,
        query("coffee", filters, SortOrder::Latest),
        Page::default(),
    );

    assert_eq!(results.count, 2);
    assert!(results
        .hits
        .iter()
        .all(|hit| hit.sender_name.as_deref() == Some("Alice")));
}

#[test]
fn filters_by_date_range() {
    let (_db_dir, conn) = seeded_db();

    let filters = UiFilters {
        date_range: Some((1800, 2600)),
        ..Default::default()
    };
    let results = search(
        &conn,
        query("coffee", filters, SortOrder::Latest),
        Page::default(),
    );

    let mut timestamps: Vec<i64> = results.hits.iter().map(|hit| hit.timestamp_ms).collect();
    timestamps.sort();
    assert_eq!(timestamps, vec![2000, 2500]);
}

#[test]
fn combines_conversation_participant_and_date_range_filters() {
    let (_db_dir, conn) = seeded_db();

    let weekend_trip_id: i64 = conn
        .query_row(
            "SELECT id FROM conversations WHERE title = 'Weekend Trip'",
            [],
            |row| row.get(0),
        )
        .unwrap();

    let filters = UiFilters {
        conversation_id: Some(weekend_trip_id),
        participant: Some("Alice".to_string()),
        date_range: Some((2000, 3000)),
    };
    let results = search(
        &conn,
        query("coffee", filters, SortOrder::Latest),
        Page::default(),
    );

    assert_eq!(results.count, 1);
    assert_eq!(results.hits[0].timestamp_ms, 2500);
}

#[test]
fn a_filter_that_matches_nothing_returns_an_empty_page_not_an_error() {
    let (_db_dir, conn) = seeded_db();

    let filters = UiFilters {
        participant: Some("Nobody".to_string()),
        ..Default::default()
    };
    let results = search(
        &conn,
        query("coffee", filters, SortOrder::Latest),
        Page::default(),
    );

    assert_eq!(results.count, 0);
    assert!(results.hits.is_empty());
}

#[test]
fn paginates_results_independently_of_the_total_count() {
    let (_db_dir, conn) = seeded_db();

    let first_page = search(
        &conn,
        query("coffee", UiFilters::default(), SortOrder::Latest),
        Page {
            limit: 2,
            offset: 0,
        },
    );
    let second_page = search(
        &conn,
        query("coffee", UiFilters::default(), SortOrder::Latest),
        Page {
            limit: 2,
            offset: 2,
        },
    );

    assert_eq!(first_page.count, 4);
    assert_eq!(second_page.count, 4);

    let first_timestamps: Vec<i64> = first_page.hits.iter().map(|hit| hit.timestamp_ms).collect();
    let second_timestamps: Vec<i64> = second_page
        .hits
        .iter()
        .map(|hit| hit.timestamp_ms)
        .collect();
    assert_eq!(first_timestamps, vec![2500, 2000]);
    assert_eq!(second_timestamps, vec![1750, 1000]);
}

#[test]
fn sorts_latest_first() {
    let (_db_dir, conn) = seeded_db();

    let results = search(
        &conn,
        query("coffee", UiFilters::default(), SortOrder::Latest),
        Page::default(),
    );

    let timestamps: Vec<i64> = results.hits.iter().map(|hit| hit.timestamp_ms).collect();
    assert_eq!(timestamps, vec![2500, 2000, 1750, 1000]);
}

#[test]
fn sorts_oldest_first() {
    let (_db_dir, conn) = seeded_db();

    let results = search(
        &conn,
        query("coffee", UiFilters::default(), SortOrder::Oldest),
        Page::default(),
    );

    let timestamps: Vec<i64> = results.hits.iter().map(|hit| hit.timestamp_ms).collect();
    assert_eq!(timestamps, vec![1000, 1750, 2000, 2500]);
}

#[test]
fn sorts_by_relevance_using_bm25() {
    let (_db_dir, conn) = seeded_db();

    let results = search(
        &conn,
        query("coffee", UiFilters::default(), SortOrder::Relevance),
        Page::default(),
    );

    // The message with four occurrences of "coffee" (timestamp 1750, neither
    // the newest nor the oldest match) should rank first: this can only
    // happen under a genuine relevance sort, not a chronological one.
    assert_eq!(
        results.hits[0].timestamp_ms, 1750,
        "the message mentioning 'coffee' most often should rank first under BM25"
    );
}
