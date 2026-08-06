/**
 * Hand-written mirrors of the `grepm_core` types that cross the Tauri IPC
 * boundary.
 *
 * These correspond to the `Serialize`/`Deserialize` types in
 * `core/src/search/mod.rs`. There is no codegen, so **changing a type there
 * means changing this file in the same commit**. Field names are snake_case
 * because serde serializes them that way by default; `Option<T>` on the Rust
 * side arrives as `T | null`, not as a missing key.
 *
 * Only the search types live here. `core/src/db/models.rs` also derives
 * `Serialize`, but none of it crosses the boundary yet — mirroring it now
 * would be types for calls nobody makes.
 */

/** A pagination window. Mirrors `search::Page`. */
export interface Page {
	/** Rust-side `u32`: a negative value is rejected at deserialization. */
	limit: number;
	offset: number;
}

/** The default page the engine uses when none is supplied. */
export const DEFAULT_PAGE: Page = { limit: 32, offset: 0 };

/** Mirrors `search::UiFilters`. Any subset may be active. */
export interface UiFilters {
	participant: string | null;
	conversation_id: number | null;
	/** Inclusive `[from_ms, to_ms]`, mirroring Rust's `(i64, i64)` tuple. */
	date_range: [number, number] | null;
}

/**
 * Mirrors `search::SortOrder`. Serde serializes these unit variants as bare
 * strings, so the wire values are exactly the Rust variant names.
 */
export type SortOrder = 'Relevance' | 'Latest' | 'Oldest';

/** Mirrors `search::SearchQuery`. */
export interface SearchQuery {
	text: string;
	filters: UiFilters;
	sort: SortOrder;
}

/**
 * Wraps each matched term in {@link SearchHit.snippet}, paired with
 * {@link MATCH_END}. Unicode's interlinear annotation anchor (U+FFF9) — a
 * format character, so it cannot collide with anything a sender typed.
 */
export const MATCH_START = '￹';

/** Closes a match opened by {@link MATCH_START}. U+FFFB. */
export const MATCH_END = '￻';

/** Mirrors `search::SearchHit`. */
export interface SearchHit {
	message_id: number;
	conversation_id: number;
	conversation_title: string | null;
	sender_name: string | null;
	timestamp_ms: number;
	/**
	 * An excerpt of the message with each matched term wrapped in
	 * {@link MATCH_START}/{@link MATCH_END}, and `...` where it was clipped.
	 *
	 * This is message text a sender wrote. Never pass it to `{@html}` — split
	 * on the sentinels and render the pieces as text nodes. See `src/CLAUDE.md`.
	 */
	snippet: string;
}

/** Mirrors `search::SearchResults`. */
export interface SearchResults {
	hits: SearchHit[];
	/** Total matches, not the length of `hits` — this drives pagination. */
	count: number;
	page: Page;
}
