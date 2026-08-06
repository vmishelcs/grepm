use rusqlite::{params, Connection};

use crate::search::{Page, SearchHit, SearchIndex, SearchQuery, SearchResults, SortOrder};
use crate::Result;

pub struct FtsIndex<'a> {
    conn: &'a Connection,
}

impl<'a> FtsIndex<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }
}

/// Filters shared by the count query and the paginated results query: an
/// optional `conversation_id`, an optional sender `participant` name, and an
/// optional inclusive `timestamp_ms` range (`?4`/`?5`), each a no-op when its
/// parameter is NULL.
const FILTER_SQL: &str = " \
    AND (?2 IS NULL OR m.conversation_id = ?2) \
    AND (?3 IS NULL OR p.name = ?3) \
    AND (?4 IS NULL OR m.timestamp_ms >= ?4) \
    AND (?5 IS NULL OR m.timestamp_ms <= ?5)";

impl<'a> SearchIndex for FtsIndex<'a> {
    fn search(&self, query: &SearchQuery, page: Page) -> Result<SearchResults> {
        // FTS5 treats quotes, `-`, `*`, `AND`/`OR`, etc. as query syntax;
        // wrapping the text as a quote-escaped phrase makes the match literal
        // instead of a boolean/operator expression.
        let match_text = format!("\"{}\"", query.text.replace('"', "\"\""));
        let conversation_id = query.filters.conversation_id;
        let participant = query.filters.participant.as_deref();
        let (date_from, date_to) = query
            .filters
            .date_range
            .map_or((None, None), |(from, to)| (Some(from), Some(to)));

        // bm25() scores lower for a better match, so ascending order ranks
        // the best matches first; the other two orders are chronological.
        //
        // Every arm ends in `m.id`, because none of the three leading
        // expressions is unique: same-millisecond messages are a first-class
        // case here (see the dedup index), and bm25 ties are routine for
        // short chat messages. Each page is a separate query, so without a
        // total tiebreaker SQLite could order tied rows differently per page
        // and a message would show up twice or not at all.
        let order_by = match query.sort {
            SortOrder::Relevance => "bm25(messages_fts) ASC, m.id DESC",
            SortOrder::Latest => "m.timestamp_ms DESC, m.id DESC",
            SortOrder::Oldest => "m.timestamp_ms ASC, m.id ASC",
        };

        // The total count and the page of hits are two separate reads; the
        // transaction makes them see one snapshot, so a write landing in
        // between (e.g. an import on another connection) can't make `count`
        // disagree with the hits. `unchecked_transaction` because `self`
        // only holds a shared borrow; this is safe here since no other
        // transaction can be entangled with these two read-only queries.
        let tx = self.conn.unchecked_transaction()?;

        let count: i64 = tx.query_row(
            &format!(
                "SELECT count(*) \
                 FROM messages_fts \
                 JOIN messages m ON m.id = messages_fts.rowid \
                 LEFT JOIN participants p ON p.id = m.sender_id \
                 WHERE messages_fts MATCH ?1{FILTER_SQL}"
            ),
            params![match_text, conversation_id, participant, date_from, date_to],
            |row| row.get(0),
        )?;

        let mut stmt = tx.prepare(&format!(
            "SELECT m.id, m.conversation_id, c.title, p.name, m.timestamp_ms, \
                    snippet(messages_fts, 0, '[', ']', '...', 8) \
             FROM messages_fts \
             JOIN messages m ON m.id = messages_fts.rowid \
             JOIN conversations c ON c.id = m.conversation_id \
             LEFT JOIN participants p ON p.id = m.sender_id \
             WHERE messages_fts MATCH ?1{FILTER_SQL} \
             ORDER BY {order_by} \
             LIMIT ?6 OFFSET ?7"
        ))?;

        let hits = stmt
            .query_map(
                params![
                    match_text,
                    conversation_id,
                    participant,
                    date_from,
                    date_to,
                    page.limit,
                    page.offset
                ],
                |row| {
                    Ok(SearchHit {
                        message_id: row.get(0)?,
                        conversation_id: row.get(1)?,
                        conversation_title: row.get(2)?,
                        sender_name: row.get(3)?,
                        timestamp_ms: row.get(4)?,
                        snippet: row.get(5)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        drop(stmt);
        tx.commit()?;

        Ok(SearchResults {
            hits,
            count: count as usize,
            page,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::schema;
    use crate::search::UiFilters;

    fn migrated_connection() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        schema::configure(&conn).unwrap();
        schema::migrate(&mut conn).unwrap();
        conn
    }

    fn insert_conversation(conn: &Connection, title: &str, thread_path: &str) -> i64 {
        conn.query_row(
            "INSERT INTO conversations (title, thread_path, is_still_participant) \
             VALUES (?1, ?2, 1) RETURNING id",
            params![title, thread_path],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn insert_participant(conn: &Connection, conversation_id: i64, name: &str) -> i64 {
        let participant_id: i64 = conn
            .query_row(
                "INSERT INTO participants (name) VALUES (?1) RETURNING id",
                params![name],
                |row| row.get(0),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO conversation_participants (conversation_id, participant_id) \
             VALUES (?1, ?2)",
            params![conversation_id, participant_id],
        )
        .unwrap();
        participant_id
    }

    fn insert_message(
        conn: &Connection,
        conversation_id: i64,
        sender_id: i64,
        timestamp_ms: i64,
        content: &str,
    ) -> i64 {
        conn.query_row(
            "INSERT INTO messages \
             (conversation_id, sender_id, timestamp_ms, content, type) \
             VALUES (?1, ?2, ?3, ?4, 'text') RETURNING id",
            params![conversation_id, sender_id, timestamp_ms, content],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn text_query(text: &str, filters: UiFilters) -> SearchQuery {
        sorted_query(text, filters, SortOrder::Latest)
    }

    fn sorted_query(text: &str, filters: UiFilters, sort: SortOrder) -> SearchQuery {
        SearchQuery {
            text: text.to_string(),
            filters,
            sort,
        }
    }

    #[test]
    fn search_matches_text_across_conversations_with_no_filters() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let dm = insert_conversation(&conn, "DM", "inbox/dm");
        let lunch = insert_conversation(&conn, "Lunch Plans", "inbox/lunch");
        let alice = insert_participant(&conn, team, "Alice");
        let carol = insert_participant(&conn, dm, "Carol");
        let dave = insert_participant(&conn, lunch, "Dave");
        insert_message(&conn, team, alice, 1000, "let's grab coffee");
        insert_message(&conn, dm, carol, 2000, "coffee run?");
        insert_message(&conn, lunch, dave, 3000, "let's get lunch");
        schema::populate_fts(&conn).unwrap();

        let index = FtsIndex::new(&conn);
        let results = index
            .search(&text_query("coffee", UiFilters::default()), Page::default())
            .unwrap();

        assert_eq!(results.count, 2);
        assert_eq!(results.hits.len(), 2);
    }

    #[test]
    fn search_filters_by_conversation_id() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let dm = insert_conversation(&conn, "DM", "inbox/dm");
        let alice = insert_participant(&conn, team, "Alice");
        let carol = insert_participant(&conn, dm, "Carol");
        insert_message(&conn, team, alice, 1000, "let's grab coffee");
        insert_message(&conn, dm, carol, 2000, "coffee run?");
        schema::populate_fts(&conn).unwrap();

        let filters = UiFilters {
            conversation_id: Some(team),
            ..Default::default()
        };
        let index = FtsIndex::new(&conn);
        let results = index
            .search(&text_query("coffee", filters), Page::default())
            .unwrap();

        assert_eq!(results.count, 1);
        assert_eq!(results.hits[0].conversation_id, team);
    }

    #[test]
    fn search_filters_by_participant() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        let bob = insert_participant(&conn, team, "Bob");
        insert_message(&conn, team, alice, 1000, "coffee?");
        let bobs_message = insert_message(&conn, team, bob, 2000, "coffee please");
        schema::populate_fts(&conn).unwrap();

        let filters = UiFilters {
            participant: Some("Bob".to_string()),
            ..Default::default()
        };
        let index = FtsIndex::new(&conn);
        let results = index
            .search(&text_query("coffee", filters), Page::default())
            .unwrap();

        assert_eq!(results.count, 1);
        assert_eq!(results.hits[0].message_id, bobs_message);
        assert_eq!(results.hits[0].sender_name.as_deref(), Some("Bob"));
    }

    #[test]
    fn search_filters_by_date_range() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        insert_message(&conn, team, alice, 1000, "coffee at 1000");
        let in_range = insert_message(&conn, team, alice, 2000, "coffee at 2000");
        insert_message(&conn, team, alice, 3000, "coffee at 3000");
        schema::populate_fts(&conn).unwrap();

        let filters = UiFilters {
            date_range: Some((1500, 2500)),
            ..Default::default()
        };
        let index = FtsIndex::new(&conn);
        let results = index
            .search(&text_query("coffee", filters), Page::default())
            .unwrap();

        assert_eq!(results.count, 1);
        assert_eq!(results.hits[0].message_id, in_range);
    }

    #[test]
    fn search_combines_conversation_participant_and_date_range_filters() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let dm = insert_conversation(&conn, "DM", "inbox/dm");
        let alice = insert_participant(&conn, team, "Alice");
        let bob = insert_participant(&conn, team, "Bob");
        let alice_in_dm = insert_participant(&conn, dm, "Alice");

        let target = insert_message(&conn, team, alice, 2000, "coffee break");
        // Wrong participant.
        insert_message(&conn, team, bob, 2000, "coffee break");
        // Wrong conversation (a separate "Alice" row scoped to `dm`, see
        // KNOWN_ISSUES.md #3).
        insert_message(&conn, dm, alice_in_dm, 2000, "coffee break");
        // Wrong timestamp.
        insert_message(&conn, team, alice, 9000, "coffee break");
        schema::populate_fts(&conn).unwrap();

        let filters = UiFilters {
            conversation_id: Some(team),
            participant: Some("Alice".to_string()),
            date_range: Some((1000, 3000)),
        };
        let index = FtsIndex::new(&conn);
        let results = index
            .search(&text_query("coffee", filters), Page::default())
            .unwrap();

        assert_eq!(results.count, 1);
        assert_eq!(results.hits[0].message_id, target);
    }

    #[test]
    fn search_returns_no_results_when_a_filter_excludes_every_match() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        insert_message(&conn, team, alice, 1000, "coffee break");
        schema::populate_fts(&conn).unwrap();

        let filters = UiFilters {
            participant: Some("Nobody".to_string()),
            ..Default::default()
        };
        let index = FtsIndex::new(&conn);
        let results = index
            .search(&text_query("coffee", filters), Page::default())
            .unwrap();

        assert_eq!(results.count, 0);
        assert!(results.hits.is_empty());
    }

    #[test]
    fn search_paginates_results_independently_of_total_count() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        let ids: Vec<i64> = (0..5)
            .map(|i| insert_message(&conn, team, alice, 1000 + i, "coffee"))
            .collect();
        schema::populate_fts(&conn).unwrap();

        let index = FtsIndex::new(&conn);
        let first_page = index
            .search(
                &text_query("coffee", UiFilters::default()),
                Page {
                    limit: 2,
                    offset: 0,
                },
            )
            .unwrap();
        let second_page = index
            .search(
                &text_query("coffee", UiFilters::default()),
                Page {
                    limit: 2,
                    offset: 2,
                },
            )
            .unwrap();

        assert_eq!(first_page.count, 5);
        assert_eq!(second_page.count, 5);

        // ORDER BY timestamp_ms DESC, so the newest 2 come first, then the
        // next 2.
        let first_ids: Vec<i64> = first_page.hits.iter().map(|h| h.message_id).collect();
        let second_ids: Vec<i64> = second_page.hits.iter().map(|h| h.message_id).collect();
        assert_eq!(first_ids, vec![ids[4], ids[3]]);
        assert_eq!(second_ids, vec![ids[2], ids[1]]);
    }

    #[test]
    fn search_orders_oldest_first() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        let newest = insert_message(&conn, team, alice, 3000, "coffee");
        let oldest = insert_message(&conn, team, alice, 1000, "coffee");
        let middle = insert_message(&conn, team, alice, 2000, "coffee");
        schema::populate_fts(&conn).unwrap();

        let index = FtsIndex::new(&conn);
        let results = index
            .search(
                &sorted_query("coffee", UiFilters::default(), SortOrder::Oldest),
                Page::default(),
            )
            .unwrap();

        let ids: Vec<i64> = results.hits.iter().map(|h| h.message_id).collect();
        assert_eq!(ids, vec![oldest, middle, newest]);
    }

    #[test]
    fn search_orders_latest_first() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        let newest = insert_message(&conn, team, alice, 3000, "coffee");
        let oldest = insert_message(&conn, team, alice, 1000, "coffee");
        let middle = insert_message(&conn, team, alice, 2000, "coffee");
        schema::populate_fts(&conn).unwrap();

        let index = FtsIndex::new(&conn);
        let results = index
            .search(
                &sorted_query("coffee", UiFilters::default(), SortOrder::Latest),
                Page::default(),
            )
            .unwrap();

        let ids: Vec<i64> = results.hits.iter().map(|h| h.message_id).collect();
        assert_eq!(ids, vec![newest, middle, oldest]);
    }

    #[test]
    fn search_orders_by_relevance_using_bm25() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        // Timestamps are deliberately the reverse of the expected relevance
        // order, so this test can't accidentally pass because the results
        // happen to also be in chronological order.
        let less_relevant = insert_message(&conn, team, alice, 2000, "I had one coffee today");
        let more_relevant = insert_message(
            &conn,
            team,
            alice,
            1000,
            "coffee coffee coffee, all I think about is coffee",
        );
        schema::populate_fts(&conn).unwrap();

        let index = FtsIndex::new(&conn);
        let results = index
            .search(
                &sorted_query("coffee", UiFilters::default(), SortOrder::Relevance),
                Page::default(),
            )
            .unwrap();

        let ids: Vec<i64> = results.hits.iter().map(|h| h.message_id).collect();
        assert_eq!(
            ids,
            vec![more_relevant, less_relevant],
            "the message mentioning 'coffee' more often should rank first under BM25"
        );
    }

    #[test]
    fn search_breaks_timestamp_ties_by_message_id() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        // All four land in the same millisecond, so `timestamp_ms` alone
        // leaves the order up to SQLite.
        let ids: Vec<i64> = (0..4)
            .map(|i| insert_message(&conn, team, alice, 1000, &format!("coffee {i}")))
            .collect();
        schema::populate_fts(&conn).unwrap();

        let index = FtsIndex::new(&conn);
        let latest = index
            .search(
                &sorted_query("coffee", UiFilters::default(), SortOrder::Latest),
                Page::default(),
            )
            .unwrap();
        let oldest = index
            .search(
                &sorted_query("coffee", UiFilters::default(), SortOrder::Oldest),
                Page::default(),
            )
            .unwrap();

        let latest_ids: Vec<i64> = latest.hits.iter().map(|h| h.message_id).collect();
        let oldest_ids: Vec<i64> = oldest.hits.iter().map(|h| h.message_id).collect();
        assert_eq!(latest_ids, vec![ids[3], ids[2], ids[1], ids[0]]);
        assert_eq!(oldest_ids, ids);
    }

    #[test]
    fn search_breaks_relevance_ties_by_message_id() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        // One "coffee" each in equal-length messages scores identically
        // under bm25 (the trailing word is not a query term), so relevance
        // alone leaves the order up to SQLite. The bodies still differ so
        // the dedup index doesn't reject them.
        let ids: Vec<i64> = ["alpha", "bravo", "delta", "gamma"]
            .iter()
            .map(|word| insert_message(&conn, team, alice, 1000, &format!("coffee {word}")))
            .collect();
        schema::populate_fts(&conn).unwrap();

        let index = FtsIndex::new(&conn);
        let results = index
            .search(
                &sorted_query("coffee", UiFilters::default(), SortOrder::Relevance),
                Page::default(),
            )
            .unwrap();

        let hit_ids: Vec<i64> = results.hits.iter().map(|h| h.message_id).collect();
        assert_eq!(hit_ids, vec![ids[3], ids[2], ids[1], ids[0]]);
    }

    #[test]
    fn paging_through_tied_results_visits_every_hit_exactly_once() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        // Same millisecond and same bm25 score: every ordering expression
        // ties, so only the `m.id` tiebreaker separates these rows.
        let ids: Vec<i64> = ["alpha", "bravo", "delta", "gamma", "hotel", "india"]
            .iter()
            .map(|word| insert_message(&conn, team, alice, 1000, &format!("coffee {word}")))
            .collect();
        schema::populate_fts(&conn).unwrap();

        // Each page is its own query and its own read transaction, so this
        // only holds if the ordering is total.
        let index = FtsIndex::new(&conn);
        let paged: Vec<i64> = (0..3)
            .flat_map(|page| {
                index
                    .search(
                        &sorted_query("coffee", UiFilters::default(), SortOrder::Relevance),
                        Page {
                            limit: 2,
                            offset: page * 2,
                        },
                    )
                    .unwrap()
                    .hits
                    .into_iter()
                    .map(|hit| hit.message_id)
            })
            .collect();

        let mut sorted = paged.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, ids, "every hit should appear on exactly one page");
    }

    #[test]
    fn search_commits_its_read_transaction_and_leaves_the_connection_free() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        insert_message(&conn, team, alice, 1000, "coffee");
        schema::populate_fts(&conn).unwrap();

        let index = FtsIndex::new(&conn);
        index
            .search(&text_query("coffee", UiFilters::default()), Page::default())
            .unwrap();

        assert!(
            conn.unchecked_transaction().is_ok(),
            "search should commit its internal read transaction, leaving \
             the connection free to start another one"
        );
    }

    #[test]
    fn search_does_not_interpret_query_text_as_fts5_boolean_operators() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        // Neither message contains the phrase "coffee or tea"; if "OR" were
        // parsed as the FTS5 boolean operator instead of a literal word,
        // this query would match both (one has "coffee", the other "tea").
        insert_message(&conn, team, alice, 1000, "I really like coffee");
        insert_message(&conn, team, alice, 2000, "I really like tea");
        schema::populate_fts(&conn).unwrap();

        let index = FtsIndex::new(&conn);
        let results = index
            .search(
                &text_query("coffee OR tea", UiFilters::default()),
                Page::default(),
            )
            .unwrap();

        assert_eq!(results.count, 0);
    }

    #[test]
    fn search_matches_multi_word_query_text_as_a_contiguous_phrase() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        let phrase_present =
            insert_message(&conn, team, alice, 1000, "coffee or tea, either is fine");
        // Contains both words, but not contiguous/in order.
        insert_message(&conn, team, alice, 2000, "tea before coffee");
        schema::populate_fts(&conn).unwrap();

        let index = FtsIndex::new(&conn);
        let results = index
            .search(
                &text_query("coffee or tea", UiFilters::default()),
                Page::default(),
            )
            .unwrap();

        assert_eq!(results.count, 1);
        assert_eq!(results.hits[0].message_id, phrase_present);
    }

    #[test]
    fn search_does_not_error_on_query_text_containing_a_double_quote() {
        let conn = migrated_connection();
        let team = insert_conversation(&conn, "Team Chat", "inbox/team");
        let alice = insert_participant(&conn, team, "Alice");
        insert_message(&conn, team, alice, 1000, "coffee");
        schema::populate_fts(&conn).unwrap();

        // An unescaped `"` in the query text would produce a mismatched-quote
        // FTS5 syntax error; the doubled-quote escaping in `search` should
        // keep this a valid (if unusual) query instead.
        let index = FtsIndex::new(&conn);
        let result = index.search(
            &text_query("coffee\"", UiFilters::default()),
            Page::default(),
        );

        assert!(result.is_ok());
    }
}
