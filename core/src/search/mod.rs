use serde::{Deserialize, Serialize};

use crate::search::fts::FtsIndex;

pub mod fts;

#[derive(Debug, Deserialize, Serialize)]
pub struct Page {
    pub limit: i64,
    pub offset: i64,
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

#[derive(Debug, Serialize)]
pub struct SearchHit {
    pub message_id: i64,
    pub conversation_id: i64,
    pub conversation_title: Option<String>,
    pub sender_name: Option<String>,
    pub timestamp_ms: i64,
    pub snippet: String,
}

#[derive(Debug, Serialize)]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub count: usize,
    pub page: Page,
}

pub trait SearchIndex {
    fn search(&self, query: &SearchQuery, page: Page) -> rusqlite::Result<SearchResults>;
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
) -> rusqlite::Result<SearchResults> {
    let query = build_query(text, filters, sort);
    FtsIndex::new(conn).search(&query, page)
}
