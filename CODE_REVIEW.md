# Code review — `grepm`

Reviewed: 2026-07-23. Scope: the whole repository, with the depth on
`core/` (the `grepm_core` crate), which is where essentially all the real
logic currently lives. The Tauri/Svelte shell (`src-tauri/`, `src/`) is
still the unmodified `create-tauri-app` scaffold and is called out as such.

Baseline: everything compiles, `cargo clippy --all-targets` is clean, and
all 94 tests pass (`cargo test`). The findings below are about behavior
under inputs the tests don't yet cover, not regressions.

Severity legend: **[high]** can produce wrong results / data issues,
**[med]** matters under realistic data or future changes, **[low]**
polish / maintainability. Items already tracked in `core/KNOWN_ISSUES.md`
are cross-referenced rather than re-argued.

---

## Correctness

### C1. [med] Pagination has no deterministic tie-breaker

`core/src/search/fts.rs:40-44` — the three sort modes order by a single,
non-unique column:

```rust
SortOrder::Relevance => "bm25(messages_fts) ASC",
SortOrder::Latest    => "m.timestamp_ms DESC",
SortOrder::Oldest    => "m.timestamp_ms ASC",
```

None of these is a total order. When two rows compare equal (two messages
sharing a `timestamp_ms`, or — very common — two messages with the same
BM25 score), SQLite is free to return them in any order, and that order
is **not guaranteed to be stable across two separate statement
executions**. Pagination issues each page as its own `search()` call
(`LIMIT ?6 OFFSET ?7`), so a row sitting on a page boundary can be
duplicated onto two pages or skipped entirely between requests.

Facebook exports realistically contain equal `timestamp_ms` values
(rapid-fire messages, and the millisecond granularity is coarser than it
looks), and BM25 ties are the norm rather than the exception on short
messages. The current tests only ever use distinct timestamps, so they
can't surface this.

**Fix:** append a unique, stable tiebreaker to every `ORDER BY`, e.g.
`bm25(messages_fts) ASC, m.id ASC` / `m.timestamp_ms DESC, m.id DESC`.
`m.id` is the primary key, so it makes each ordering total and keeps
paging consistent. Cheap and low-risk.

### C2. [med] Empty / whitespace-only query text silently returns nothing

`core/src/search/fts.rs:30` wraps the text as an FTS5 phrase:

```rust
let match_text = format!("\"{}\"", query.text.replace('"', "\"\""));
```

For empty text this produces `MATCH '""'`, which I verified returns **zero
rows** (no error). So an empty search box yields "no results" rather than
"everything," and there is currently no code path that lists all messages.

Whether that's correct is a product decision, but right now it's implicit
and untested. Note also that `run()` (`mod.rs:73`) trims the text before
building the query, but `FtsIndex::search` is public and is called
directly (including by `core/tests/search.rs`), so a caller that skips
`run()` gets no trimming — `"   "` becomes `MATCH '"   "'`. Decide the
intended behavior (probably: reject/short-circuit empty text, or define a
distinct "browse all" path) and cover it with a test. See also D3 on where
the trimming policy lives.

### C3. [low] Negative `Page.limit` disables the limit entirely

`Page.limit`/`offset` are `i64` fed straight into `LIMIT ?6 OFFSET ?7`
(`fts.rs:67`, `78-79`) with no validation. In SQLite `LIMIT -1` means "no
limit," which I verified returns **all** matching rows. Since these values
originate from the frontend, a bug or hostile input of `limit: -1` turns a
paginated query into a full-result load into memory. A negative `offset`
is clamped to 0 by SQLite, so only `limit` is dangerous.

**Fix:** clamp/validate at the boundary (`limit.max(0)`, or a small
sanity ceiling) — ideally in `run()` or in the eventual Tauri command,
since `Page` is a plain data struct.

### C4. [low] `count` and results queries use different JOIN sets

The count query (`fts.rs:46-56`) joins only `messages` and `participants`;
the results query (`fts.rs:58-68`) additionally does `JOIN conversations c`
(an inner join). Today they can't diverge because `messages.conversation_id`
is `NOT NULL` with an enforced FK, so the inner join never drops a row —
`count` always matches the number of retrievable hits. But the two queries
duplicating the WHERE/JOIN logic by hand is fragile: a future schema change
(e.g. a nullable conversation, or a filter added to only one of them) would
make `count` silently disagree with `hits`. Consider deriving both from a
shared query fragment, or at least keeping the JOIN sets identical.

### C5. [low] Re-importing an export may corrupt the FTS index (already KI #9)

`populate_fts` (`core/src/db/schema.rs:91-97`) is a plain
`INSERT INTO messages_fts(rowid, content) SELECT ...` with no reset. A
second `import_export` re-inserts every already-indexed rowid. In a quick
probe this didn't visibly corrupt a trivial index, but duplicate-rowid
inserts into an external-content FTS5 table are exactly the situation
SQLite's docs warn leaves the index out of sync with its content table,
and the failure tends to surface later, not at insert time. This is worth
closing rather than leaving to chance.

**Fix:** make the index build idempotent. The canonical one-liner is
`INSERT INTO messages_fts(messages_fts) VALUES('rebuild');` (rebuilds the
whole index from the content table), or delete-all + repopulate. Either
removes the "import twice = undefined" caveat entirely and is trivially
testable.

---

## Robustness

### R1. [low] `migrate` can leave an open transaction on failure (already KI #8)

`schema.rs:78-88` runs `BEGIN` / migration / `user_version` bump / `COMMIT`
as four separate calls, so a mid-migration failure returns via `?` with the
transaction still open and no rollback. With a single migration today this
is theoretical, but the fix is easy and worth doing before a second
migration is added: use `rusqlite`'s `Transaction` (RAII rollback on drop),
or wrap the batch so a failure path runs `ROLLBACK`. A small
"a failing migration leaves the connection usable" test would lock it in.

### R2. [low] Reactions are parsed and then dropped (already KI #6)

`RawReaction` is fully parsed and mojibake-repaired (`parse.rs:71-74`,
`93-97`) and the `reactions` table exists (`schema.rs:45-52`), but nothing
ever inserts them. Either wire up an `insert_reaction` in the loader or
drop the parsing/table until it's needed — carrying half an unused feature
invites the assumption that reaction search/display already works.

---

## Performance

### P1. [low] No secondary indexes for the search filter/sort columns

The search queries filter on `m.conversation_id`, join `participants` and
filter on `p.name`, and sort on `m.timestamp_ms`. FTS5 narrows the row set
first, so this is fine at current scale, but on a large export:

- `p.name = ?3` scans `participants` (no index on `name`).
- Sorting the matched set by `timestamp_ms` has no supporting index.
- The `conversation_id` filter is partly served by the leftmost column of
  `idx_messages_dedup`, but a dedicated `messages(conversation_id,
  timestamp_ms)` index would help the common "filter to a thread, newest
  first" path.

Not worth adding speculatively, but keep it in mind once real
export-sized data is in play; measure before indexing.

### P2. [low] Each search runs two queries (count + page)

`search()` issues a separate `count(*)` and then the page query. That's a
reasonable, conventional way to get total-count + a page, and I'd keep it —
just noting it's two full evaluations of the FTS match per search. If it
ever shows up in profiling, the count can be made optional (only on page 1
/ when the filter set changes).

---

## Design / architecture

These are observations, not defects — mostly about the seams as the app
grows past `core`.

### D1. The Tauri/Svelte app is still 100% scaffold

`src-tauri/src/lib.rs` exposes only the template `greet` command and never
references `grepm_core`; `src/routes/+page.svelte` is the "Welcome to Tauri
+ Svelte" starter. So the `SearchIndex` abstraction, the ingest pipeline,
and the DB all exist and are tested, but **nothing in the actual
application is wired to them yet**. That's fine for the current stage, but
it's the single biggest gap between "what the crate can do" and "what the
app does," and the README should say so (it currently doesn't — see the
README rewrite in this pass).

### D2. `UiFilters` names the UI layer inside the core search type

`SearchQuery.filters: UiFilters` (`mod.rs:40`) puts a `Ui`-prefixed type at
the center of the backend-agnostic `SearchIndex` contract. It reads a
little oddly for a type that any search backend must accept. `SearchFilters`
(or just `Filters`) would keep the core search vocabulary layer-neutral.
Purely a naming call; no behavior implication.

### D3. Text-normalization policy is split between `run` and `search`

`run()` trims `text` (`mod.rs:67`) but `FtsIndex::search` does the FTS5
escaping (`fts.rs:30`). So "what counts as the query" depends on whether a
caller goes through `run()` or straight to the trait. Since the project's
stated rule is that everything routes through `SearchIndex`, consider
moving trimming (and the empty-text decision from C2) *into* `search()` (or
a shared normalization step the trait guarantees), so every backend and
every caller get identical semantics regardless of entry point.

### D4. `Serialize`/`Deserialize` derives match data-flow direction (good)

Worth affirming: `SearchHit`/`SearchResults` derive `Serialize` (Rust →
JS), while `UiFilters`/`SortOrder`/`Page` derive `Deserialize` (JS → Rust),
and `SearchQuery` derives only `Serialize`. That asymmetry is consistent
with `run()`'s signature (the command will take `text`/`filters`/`sort`/
`page` as separate args, not a whole `SearchQuery` from JS). Just confirm
that's the intended command shape before building the Tauri layer, so you
don't later need `Deserialize` on `SearchQuery`.

### D5. `Message.content: Option<String>` vs. `content TEXT NOT NULL DEFAULT ''`

The model (`db/models.rs:61`) makes `content` optional, but the schema
(`schema.rs:33`) can never store NULL — a missing body is written as `''`
(`queries.rs:102`). The `Option` is therefore always `Some` in practice and
is slightly misleading. Harmless, but tightening the model to `String` (or
documenting why it stays optional) removes a "can this be None?" question at
every read site.

---

## Testing

The test suite is a genuine strength — 94 tests, and several are
well-designed adversarial checks rather than happy-path confirmations:
the BM25 relevance test deliberately places the most-relevant message at a
*non-extreme* timestamp so it can't pass by accidentally coinciding with
chronological order (`fts.rs:404-435`, `search.rs:377-394`), and the
token-exact-vs-substring and diacritic tests only pass if search truly
routes through `messages_fts` rather than scanning the raw table
(`search.rs:138-185`). That's the right instinct.

Gaps worth closing (they line up with the findings above):

- **T1** — pagination stability with **tied** sort keys (C1): seed two
  messages with the same `timestamp_ms` (and/or identical text for a BM25
  tie), page through them, assert no row is dropped or duplicated. This is
  the test that would currently fail.
- **T2** — empty / whitespace-only query text (C2): pin down and assert the
  intended behavior instead of leaving it implicit.
- **T3** — a hit whose sender is `NULL` (message with no `sender_name`):
  the `LEFT JOIN participants` path and `sender_name: None` in a
  `SearchHit` is never exercised by a search test.
- **T4** — re-import / idempotent `populate_fts` (C5): once fixed, a test
  that imports twice and asserts match counts are unchanged.
- **T5** — negative/oversized `Page.limit` (C3), once validation exists.

---

## Minor / style

- **M1** `impl<'a> FtsIndex<'a>` / `impl<'a> SearchIndex for FtsIndex<'a>`
  (`fts.rs:9`, `25`) could elide to `impl FtsIndex<'_>`. Trivial; clippy
  doesn't flag it.
- **M2** `count as usize` (`fts.rs:96`) is safe (`count(*)` is always ≥ 0),
  just noting the cast.
- **M3** The SQL is assembled with `format!` interpolating `FILTER_SQL` and
  `order_by`. This is **not** an injection risk — `order_by` comes from a
  closed `match` on `SortOrder` and every user value is a bound parameter —
  but it's worth a one-line comment at the `format!` sites saying exactly
  that, so a future reader doesn't "fix" it by trying to parameterize the
  ORDER BY (which can't be a bind param anyway).
- **M4** `search::run` and `build_query` (`mod.rs:65-82`) are currently
  unused by any binary (only the tests exercise `FtsIndex` directly). Fine
  as the intended composition root, but they'll be dead until the Tauri
  command lands — don't lose track of wiring them in.

---

## Summary

The `core` crate is in good shape: clean separation of scan → parse →
load → index → search, thoughtful handling of the messy realities of
Facebook exports (mojibake, per-conversation identity, deleted-account
placeholders), and a test suite that actually tries to catch itself out.
The most valuable thing to fix now is **C1 (pagination tiebreaker)** —
it's a real correctness issue under ordinary data, and the fix is a few
characters. **C2/C3** (empty text and `limit` validation) are cheap
input-hardening at the same boundary. Everything else is either already
tracked in `KNOWN_ISSUES.md` or can wait until the Tauri/Svelte shell is
actually wired to the crate (**D1**), which is the real next milestone.
