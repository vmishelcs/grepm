use serde::{Deserialize, Serialize};

use crate::search::fts::FtsIndex;
use crate::Result;

pub mod fts;

/// A pagination window. The fields are unsigned so that a negative value
/// coming from the UI fails at deserialization instead of reaching SQL,
/// where a negative LIMIT means "no limit at all".
#[derive(Debug, Deserialize, Serialize)]
pub struct Page {
    pub limit: u32,
    pub offset: u32,
}

impl Default for Page {
    fn default() -> Self {
        Self {
            limit: 32,
            offset: 0,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct UiFilters {
    pub participant: Option<String>,
    pub conversation_id: Option<i64>,
    pub date_range: Option<(i64, i64)>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub enum SortOrder {
    #[default]
    Relevance,
    Latest,
    Oldest,
}

#[derive(Debug, Serialize)]
pub struct SearchQuery {
    pub text: String,
    pub filters: UiFilters,
    pub sort: SortOrder,
}

/// Wraps each matched term in [`SearchHit::snippet`], paired with
/// [`MATCH_END`].
///
/// Unicode's interlinear annotation anchor: a format character meant for
/// exactly this "the following run of text is annotated" role, so it can't
/// be mistaken for something a person typed. Marking matches with ordinary
/// punctuation instead (`[`…`]`, say) would leave a consumer unable to tell
/// a highlight from a literal bracket in the message — chat text contains
/// those freely, and the distinction is lost once the marked-up string is
/// built.
pub const MATCH_START: char = '\u{FFF9}';

/// Closes a match opened by [`MATCH_START`]. Unicode's interlinear
/// annotation terminator.
pub const MATCH_END: char = '\u{FFFB}';

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub message_id: i64,
    pub conversation_id: i64,
    pub conversation_title: Option<String>,
    pub sender_name: Option<String>,
    pub timestamp_ms: i64,
    /// An excerpt of the message with each matched term wrapped in
    /// [`MATCH_START`]/[`MATCH_END`], and `...` where the excerpt was
    /// clipped. A consumer is expected to replace the markers with its own
    /// markup — and to escape the surrounding text first, since this is
    /// message content the sender controls.
    pub snippet: String,
}

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub count: usize,
    pub page: Page,
}

pub trait SearchIndex {
    fn search(&self, query: &SearchQuery, page: Page) -> Result<SearchResults>;
}

fn build_query(text: &str, filters: &UiFilters, sort: SortOrder) -> SearchQuery {
    SearchQuery {
        text: text.trim().to_string(),
        filters: filters.clone(),
        sort,
    }
}

pub fn run(
    conn: &rusqlite::Connection,
    text: &str,
    filters: &UiFilters,
    sort: SortOrder,
    page: Page,
) -> Result<SearchResults> {
    let query = build_query(text, filters, sort);
    FtsIndex::new(conn).search(&query, page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_deserialization_rejects_a_negative_limit() {
        let result = serde_json::from_str::<Page>(r#"{"limit": -1, "offset": 0}"#);

        assert!(
            result.is_err(),
            "a negative limit must fail at deserialization; in SQL it would \
             mean 'no limit at all'"
        );
    }

    #[test]
    fn page_deserialization_rejects_a_negative_offset() {
        let result = serde_json::from_str::<Page>(r#"{"limit": 32, "offset": -5}"#);

        assert!(result.is_err());
    }

    #[test]
    fn page_deserializes_from_valid_input() {
        let page: Page = serde_json::from_str(r#"{"limit": 10, "offset": 20}"#).unwrap();

        assert_eq!(page.limit, 10);
        assert_eq!(page.offset, 20);
    }
}
