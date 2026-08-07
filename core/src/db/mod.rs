pub mod models;
pub mod queries;
pub mod schema;

/// Re-exported because [`schema::open`] hands one back: a caller has to be
/// able to name the type it stores without taking its own `rusqlite`
/// dependency, which could then drift to a different version than the one
/// this crate is built against.
pub use rusqlite::Connection;
