# Code Review — grepm core (`core/`)

Independent review of the `grepm_core` crate (ingest, db, search), performed
2026-07-24. Existing review documents were deliberately not consulted.

Method: full read of every source and test file in `core/`, `cargo clippy
--all-targets -- -D warnings` (clean), `cargo test` (all pass), plus targeted
throwaway tests written to empirically confirm or reject each suspected bug.
Findings marked **[verified]** were reproduced against the real code; the
repro evidence is quoted inline.

## Status of findings (updated 2026-07-24)

| # | Finding | Status |
|---|---------|--------|
| 1 | Second import corrupts the FTS index | Open |
| 2 | NULL title/thread_path duplicates conversations on re-import | Open |
| 3 | `message_count` inflates on re-import | Open |
| 4 | Failed migration leaves an open transaction | **Fixed** |
| 5 | Reactions parsed but never persisted | Open |
| 6 | `Box<dyn Error>` / lost file context / leaked `rusqlite::Result` | **Fixed** |
| 7 | Import is all-or-nothing per run, partial per conversation | Open |
| 8 | `find_messages_root` swallows I/O errors, walks unbounded | **Fixed** |
| 9 | `unwrap()` in production path in `scan` | **Fixed** |
| 10 | `Page` accepts negative limit/offset | **Fixed** |
| 11 | `db::models` dead code; `Message.content` disagrees with schema | **Partially fixed** |
| 12 | Count and page queries not a consistent snapshot | **Fixed** |
| 13 | Migration runner accepts a newer-versioned database | **Fixed** |
| 14 | Duplicate public paths into `db` | **Fixed** |
| 15 | Empty search text handled by accident | Open |

Fixed findings keep their original text below, followed by a **Resolution**
block describing exactly what changed.

---

## High severity

### 1. A second import corrupts the FTS index **[verified]**

`ingest::import_export` (`core/src/ingest/mod.rs:24`) calls
`db::populate_fts` at the end of every import, and `populate_fts`
(`core/src/db/schema.rs:91`) is a plain append:

```sql
INSERT INTO messages_fts(rowid, content)
SELECT id, content FROM messages WHERE content != ''
```

It selects **all** messages, not just newly inserted ones. On the second
`import_export` call against the same database — the natural "import my new
export next year" flow, or re-importing the same export (which the loader
otherwise handles idempotently) — every already-indexed rowid is inserted
into the external-content FTS5 table again. FTS5 does not deduplicate
external-content inserts; the shadow-table doclists end up inconsistent.

Reproduced: after importing export A then export B into the same database,
FTS5's own integrity check fails:

```
INSERT INTO messages_fts(messages_fts, rank) VALUES('integrity-check', 1)
=> SqliteFailure(DatabaseCorrupt, "database disk image is malformed")
```

Simple MATCH queries happen to still return plausible results, which makes
this worse — the corruption is silent and will surface later as wrong
results, snippet errors, or hard failures after merges/optimize.

The doc comment on `import_export` says it "(re)builds" the index, but the
code only appends. Fix options, in rough order of preference:

- Have `populate_fts` issue the FTS5 `rebuild` command instead:
  `INSERT INTO messages_fts(messages_fts) VALUES('rebuild')` — correct,
  idempotent, and simpler than the current INSERT…SELECT.
- Or keep the index in sync with `AFTER INSERT/UPDATE/DELETE` triggers on
  `messages` and drop `populate_fts` entirely.

### 2. Conversations with a NULL title or thread_path duplicate wholesale on re-import **[verified]**

The `conversations` dedup key is `UNIQUE (title, thread_path)`
(`core/src/db/schema.rs:12`), and `upsert_conversation`
(`core/src/db/queries.rs:10`) relies on `ON CONFLICT (title, thread_path)`.
In SQLite, NULLs are distinct in UNIQUE constraints, so a conversation file
with a missing `title` or `thread_path` — which `RawConversationFile`
explicitly allows (`Option<String>`, and
`parse_conversation_file_treats_missing_optional_fields_as_none` proves such
files are accepted) — never conflicts. Every re-import inserts a brand-new
`conversations` row, and because the message dedup index is scoped by
`conversation_id`, **all of that conversation's messages are duplicated
too**.

Reproduced: importing a title-less/thread_path-less conversation twice
yields 2 conversation rows and 2 copies of its single message.

Fix options: fall back to the conversation folder name (already available as
`ConversationDir::folder`) as a non-NULL dedup key; or store `''` instead of
NULL for the missing fields; or make the dedup key an expression index over
`COALESCE(...)` the way `idx_messages_dedup` already handles NULL
`sender_id`.

### 3. `conversations.message_count` inflates on every re-import **[verified]**

`upsert_conversation`'s conflict arm does
`message_count = message_count + excluded.message_count`
(`core/src/db/queries.rs:20`). That's correct for a conversation split
across `message_1.json`, `message_2.json`, … within one import, but on
re-import the messages themselves are deduplicated away by
`INSERT OR IGNORE` while the count is added again unconditionally.

Reproduced: import the same single-message export twice →
`message_count = 2`, actual `messages` rows = 1.

The existing idempotency test
(`load_conversation_is_idempotent_on_reload`, `core/src/ingest/loader.rs:394`)
checks conversation and message row counts but not `message_count`, which is
why this slipped through.

Fix: derive the increment from what was actually inserted —
`insert_message` already returns `None` for ignored duplicates, so the
loader can count `Some` results and apply one `UPDATE … SET message_count =
message_count + ?` per file (or simply recompute with `SELECT count(*)`).

### 4. A failed migration leaves the connection inside an open transaction **[verified]** ✅ FIXED

`schema::migrate` (`core/src/db/schema.rs:78`) brackets each migration with
`execute_batch("BEGIN;") … execute_batch("COMMIT;")`. If any statement in
between fails, the function returns early with the error and **never rolls
back**, leaving the connection with a dangling transaction. Every later
attempt to start a transaction on that connection fails.

Reproduced (by pre-seeding a `messages` table containing duplicates so the
`CREATE UNIQUE INDEX idx_messages_dedup` genuinely fails):

```
migrate => Err(… UNIQUE constraint failed: index 'idx_messages_dedup')
subsequent BEGIN => Err(… "cannot start a transaction within a transaction")
```

Today the single migration is all `IF NOT EXISTS`, so failures are unlikely
in practice — but the moment migration 2 lands (an `ALTER TABLE`, a data
backfill), this becomes a real failure mode: the app "recovers" from a
migration error into a connection that can no longer write transactionally.

Fix: take `&mut Connection` and use `Connection::transaction()`, whose Drop
guard rolls back on the error path; or explicitly `ROLLBACK` before
returning the error. Manual `BEGIN`/`COMMIT` via `execute_batch` on the
happy path only is the bug's root cause.

> **Resolution (2026-07-24).** `schema::migrate` now takes
> `&mut Connection` and wraps each migration step in
> `Connection::transaction()` instead of manual
> `execute_batch("BEGIN;") … execute_batch("COMMIT;")`. If any statement
> fails, the `Transaction` guard is dropped uncommitted and rusqlite issues
> the `ROLLBACK` automatically, so the connection is never stranded inside
> an open transaction. The `user_version` bump happens inside the same
> transaction, so a failed migration also can't leave the version pointing
> at schema that never applied. `schema::open` and the four
> `migrated_connection()` test helpers were adapted to the `&mut`
> signature.
>
> Regression test:
> `migrate_rolls_back_a_failed_migration_and_leaves_the_connection_usable`
> (`core/src/db/schema.rs`). It forces a *genuine* mid-migration failure —
> pre-creating a `messages` table containing duplicate rows so
> `CREATE UNIQUE INDEX idx_messages_dedup` fails (`IF NOT EXISTS` only
> guards against the index existing, not the data violating it) — then
> asserts the migration errors, `user_version` stays 0, and a fresh
> `conn.transaction()` succeeds afterward. Against the old code that last
> assertion fails with "cannot start a transaction within a transaction".

---

## Medium severity

### 5. Reactions are parsed and repaired, but never persisted

The schema defines a `reactions` table (`core/src/db/schema.rs:45`), and the
parser deserializes `RawReaction { reaction, actor }` and even runs mojibake
repair over both fields (`core/src/ingest/parse.rs:71-74`) — but no code
anywhere inserts them. There is no `insert_reaction` in `queries.rs`, and
`loader.rs` ignores `message.reactions` entirely. The table is always empty.

Either wire reactions into `load_messages` (resolving `actor` through the
same conversation-scoped participant lookup as senders) or delete the table
and the parsing/repair work until the feature exists. Right now it's dead
weight that misleadingly suggests reactions are captured.

### 6. Error handling: `Box<dyn Error>` in a library, and lost file context ✅ FIXED

Three separate issues compound here:

- `import_export` and `load_conversation` return
  `Result<_, Box<dyn Error>>` (`core/src/ingest/mod.rs:19`,
  `core/src/ingest/loader.rs:15`). For a library crate this erases all
  error structure — callers cannot distinguish "one file was malformed"
  from "the database is locked". Note it's also not
  `Box<dyn Error + Send + Sync>`, so these errors can't cross a thread or
  async boundary — which is exactly where they're headed once the Tauri
  layer invokes imports off the main thread.
- `parse_conversation_file` (`core/src/ingest/parse.rs:8`) returns
  `io::Result`, silently converting `serde_json::Error` into `io::Error` —
  a JSON syntax error masquerades as an I/O error.
- Most importantly for users: **the error carries no file path**. A
  malformed file in a 900-conversation export fails the whole import with
  something like `expected value at line 1 column 3` and no indication of
  which file. Both `parse_conversation_file` and the loop in
  `load_conversation` drop the path on the floor.

Recommended shape: a crate-level `thiserror` enum, e.g.
`GrepmError::{Io, Db, Parse { path: PathBuf, source: serde_json::Error }}`,
used consistently across `ingest`, `db`, and `search`. This also fixes the
`SearchIndex` trait leaking `rusqlite::Result` (`core/src/search/mod.rs:62`)
— a trait whose purpose is to abstract the FTS5 backend shouldn't hard-code
the backend's error type in its signature.

> **Resolution (2026-07-24).** New `core/src/error.rs` defines the
> crate-level type, re-exported from `lib.rs` along with a
> `pub type Result<T>` alias:
>
> ```rust
> #[derive(Debug, thiserror::Error)]
> pub enum Error {
>     #[error(transparent)]
>     Io(#[from] io::Error),
>     #[error(transparent)]
>     Db(#[from] rusqlite::Error),
>     #[error("failed to read {}: {source}", path.display())]
>     ReadFile { path: PathBuf, source: io::Error },
>     #[error("failed to parse {}: {source}", path.display())]
>     Parse { path: PathBuf, source: serde_json::Error },
> }
> ```
>
> Every public function in the crate now returns `crate::Result` — all
> layers, including the `db` module (`schema::open/configure/migrate/
> populate_fts`, all of `queries.rs`), `ingest`
> (`import_export`, `load_conversation`, `load_messages`,
> `parse_conversation_file`, `count`, `scan`, `find_messages_root`), and
> `search` (the `SearchIndex` trait itself plus `run` and the `FtsIndex`
> impl). The only remaining `rusqlite::Result`s are inside row-mapper
> closures, where rusqlite's API requires them. This resolves each
> sub-issue:
>
> - `Box<dyn Error>` is gone from `import_export`/`load_conversation`; the
>   enum is `Send + Sync` (pinned by a compile-time assertion test in
>   `error.rs`), so errors can cross thread/async boundaries in the Tauri
>   layer.
> - JSON errors no longer masquerade as `io::Error`:
>   `parse_conversation_file` maps read failures to
>   `Error::ReadFile { path, .. }` and malformed JSON to
>   `Error::Parse { path, .. }`.
> - The offending file's path is carried in the variant and rendered in
>   the message (`failed to parse /…/message_1.json: expected value at
>   line 1 column 3`), so a bad file in a large export is identifiable.
> - `SearchIndex::search` returns `crate::Result<SearchResults>`, no
>   longer hard-coding the FTS5 backend's error type.
>
> Tests: the two parse-failure tests now assert the specific variant and
> path via `matches!`, and that the rendered message names the file;
> `error.rs` adds the `Send + Sync` assertion and a message-format test.

### 7. Import is all-or-nothing per run, but partial per conversation

One malformed file anywhere aborts `import_export`, yet each conversation
commits its own transaction — so a failure mid-run leaves the database
partially populated **with no FTS index at all** (`populate_fts` only runs
at the very end). The integration test
`a_malformed_conversation_file_fails_the_whole_import_…` documents this as
intended, but from a product standpoint a single bad file (real exports do
contain oddities) shouldn't discard the other 99% of an import with no way
to resume, skip, or report progress.

Consider: collect per-conversation errors and continue, returning a summary
(`ImportReport { loaded, skipped: Vec<(PathBuf, Error)> }`); this pairs
naturally with the error-type work in finding 6. If fail-fast stays, at
least run `populate_fts` (as `rebuild`, per finding 1) even on the error
path so already-committed conversations remain searchable.

### 8. `find_messages_root` swallows I/O errors and walks unbounded ✅ FIXED

`core/src/ingest/scan.rs:79-88`: the WalkDir traversal uses
`.filter_map(Result::ok)`, so a permission error on the way to
`messages/inbox` is silently dropped and the user sees the misleading
"could not find a messages/inbox directory" instead of "permission denied".
Two secondary issues:

- The walk is depth-unbounded over the whole export, which for a real
  Facebook export means traversing tens of thousands of photo/video files
  before (or while) finding `messages/inbox`. A real export keeps it within
  the first couple of levels; a `max_depth` (say 4) would bound the cost and
  reduce the chance of matching a spurious `*/messages/inbox` deep in
  unrelated data.
- `count()` and `scan()` each call `find_messages_root`, so the
  count-then-scan flow the doc comments describe performs the full
  discovery walk twice.

> **Resolution (2026-07-24).** All three sub-issues addressed in
> `core/src/ingest/scan.rs`:
>
> - *Swallowed errors*: `find_messages_root` replaced
>   `.filter_map(|entry| entry.ok())` with an explicit loop. An unreadable
>   directory doesn't abort the search (the inbox may still be findable
>   elsewhere — deliberately more forgiving than fail-fast), but the first
>   walk error is remembered; if the search comes up empty, that error is
>   returned instead of the misleading "could not find a messages/inbox
>   directory". A permission failure now surfaces as
>   `Error::Io(PermissionDenied)`. The inbox-matching predicate moved to
>   an `is_messages_inbox` helper.
> - *Unbounded walk*: discovery is capped at
>   `MESSAGES_ROOT_MAX_DEPTH = 4`. Real exports place `messages/inbox` at
>   depth 2, or 3 under a dated export folder; the bound keeps the walk
>   out of the export's media trees. The trade-off (an inbox deeper than 4
>   is not found) is documented at the constant.
> - *Double walk*: `count` and `scan` are now thin wrappers over new
>   inbox-direct variants `count_inbox` / `scan_inbox`, so a caller can
>   run `find_messages_root` once and reuse the result for both passes.
>   `scan_inbox` returns the iterator directly (no outer `Result`); a bad
>   inbox path surfaces as the iterator's first `Err` item, per its doc
>   comment.
>
> Tests added: `find_messages_root_does_not_search_below_the_depth_bound`;
> `find_messages_root_reports_a_walk_error_instead_of_a_misleading_not_found`
> (Unix-only: mode-000 directory, drop-guard restores permissions so the
> tempdir cleans up even on assertion panic, no-op under root);
> `find_messages_root_keeps_searching_past_an_unreadable_directory`; and
> `count_inbox_and_scan_inbox_work_from_a_single_discovery_walk`.

---

## Low severity

### 9. `unwrap()` in production path in `scan` ✅ FIXED

`core/src/ingest/scan.rs:60`:
`message_files.sort_by_key(|path| message_number(path).unwrap())`. It cannot
panic today because `message_files_in` only admits paths for which
`message_number(...).is_some()` — but that invariant lives in a different
function, and a future edit to the filter breaks this at runtime. Sorting by
`message_number(path).unwrap_or(u64::MAX)` (or collecting `(number, path)`
pairs up front) removes the trap at zero cost.

> **Resolution (2026-07-24).** Took the `(number, path)` option — the one
> that makes the invariant hold *by construction* rather than masking a
> hypothetical `None` with a sentinel. `message_files_in` now parses each
> file's number at the moment the file is admitted and returns
> `Vec<(u64, PathBuf)>`; the `is_message_file` helper is gone (its check
> is the parse itself). `scan` sorts with
> `sort_unstable_by_key(|&(number, _)| number)` — no failure path exists
> for the key — and strips the numbers when building
> `ConversationDir::message_files`, so the public type is unchanged.
>
> Side benefit: `message_number` used to run twice per file (once in the
> filter, once in the sort key); it now runs once. Behavior is identical
> and was already pinned by existing tests
> (`scan_sorts_message_files_numerically_not_lexicographically`,
> `scan_ignores_files_that_do_not_match_the_message_naming_pattern`), so
> no new tests were needed.

### 10. `Page` accepts negative limit/offset from the UI ✅ FIXED

`Page { limit: i64, offset: i64 }` (`core/src/search/mod.rs:8`) is
deserialized from the frontend and passed straight into `LIMIT ?6 OFFSET ?7`.
In SQLite, `LIMIT -1` means *unlimited* — a buggy or malicious frontend value
dumps the entire result set in one page. Clamp to sane bounds in `search` or
make the fields unsigned (`u32`) so bad values fail at deserialization.

> **Resolution (2026-07-24).** Took the type-level option: `Page`'s
> fields are now `u32`, so a negative value coming from the frontend
> fails at the serde boundary instead of reaching SQL, and the invalid
> state is unrepresentable for Rust callers too — no runtime clamping
> logic to keep in sync. `u32` still binds cleanly to SQLite's `i64`
> parameters (rusqlite's `ToSql` handles the widening), so
> `FtsIndex::search` needed no changes. A doc comment on `Page` records
> why the fields are unsigned. Deliberately *not* capped at the high end:
> a huge-but-positive limit is an explicit "give me everything", which is
> a policy question for the app layer, unlike the silent negative →
> unlimited surprise.
>
> Tests (`core/src/search/mod.rs`):
> `page_deserialization_rejects_a_negative_limit`,
> `page_deserialization_rejects_a_negative_offset`, and
> `page_deserializes_from_valid_input` pin the boundary behavior.

### 11. `db::models` is dead code, and `Message.content` disagrees with the schema ⚠️ PARTIALLY FIXED

Nothing in the workspace references `models.rs` — no query maps rows through
`from_row`, and the Tauri crate doesn't use it yet. If it's staged for the
app layer, fine, but note `Message.content: Option<String>` contradicts the
schema (`content TEXT NOT NULL DEFAULT ''`): `from_row` will always yield
`Some`, and the `Option` invites the app layer to handle a `None` that can't
occur. Same drift risk applies to the whole file: hand-written `from_row`
mappers with no tests and no callers will silently rot as the schema evolves.

> **Resolution (2026-07-24, partial by design.)** Per the project's
> decision, the models themselves stay: they are staged for the app
> layer, so "dead code" is intentional and not being removed. What was
> fixed is the schema disagreement: `Message.content` is now `String`
> instead of `Option<String>`, with a doc comment explaining why it can
> never be absent (the schema's `NOT NULL DEFAULT ''`, plus ingestion
> storing missing content as `''`). The app layer can no longer be
> misled into handling a `None` that can't occur.
>
> The drift-risk concern also got a first guard:
> `message_from_row_maps_a_row_with_defaulted_content_and_null_sender`
> (`core/src/db/models.rs`) runs `Message::from_row` against a row from
> the *real migrated schema* — inserting a message with no `content` and
> no `sender_id` and asserting `content == ""` / `sender_id == None` —
> so at least the `Message` mapper now breaks loudly if the schema and
> the struct diverge again. Equivalent tests for the other four mappers
> are worth adding when the app layer starts using them.

### 12. Count query and page query are not a consistent snapshot ✅ FIXED

`FtsIndex::search` (`core/src/search/fts.rs:46-92`) runs the `count(*)`
query and the paginated query as two independent reads. A write between them
(e.g. an import running while the user searches) can make `count` disagree
with the returned hits. Low impact for a desktop app, but a read transaction
around the pair is nearly free.

> **Resolution (2026-07-24).** `FtsIndex::search` now wraps both reads in
> a single transaction via `Connection::unchecked_transaction()`, and
> commits it after collecting the hits. Under SQLite's deferred
> transaction semantics the snapshot is established at the first read and
> held until commit, so a write landing between the `count(*)` and the
> page query (e.g. an import on another connection, given WAL mode) can
> no longer make them disagree.
>
> `unchecked_transaction` rather than `transaction()` because `FtsIndex`
> holds `&Connection` and the `SearchIndex` trait takes `&self`; the
> borrow-checked variant would have forced `&mut` through the whole trait
> for no benefit. The "unchecked" caveat (rusqlite can't statically stop
> a second entangled transaction on the same connection) is safe here:
> the transaction is created, used for two read-only queries, and
> committed within the one function. If a statement fails mid-search, the
> `Transaction` drop guard rolls back, so the error path can't strand an
> open transaction (same pattern as the finding-4 fix in
> `schema::migrate`).
>
> Regression test:
> `search_commits_its_read_transaction_and_leaves_the_connection_free`
> (`core/src/search/fts.rs`) asserts a fresh transaction can be started
> on the connection after a search completes. True snapshot-isolation
> interleaving (a write injected between the two queries) isn't
> deterministically testable without pausing `search` mid-function, so
> the guarantee rests on SQLite's documented transaction semantics.

### 13. Migration runner silently accepts a database from a newer app version ✅ FIXED

`migrate` iterates `current_version..LATEST_VERSION`; when a database's
`user_version` is *greater* than `LATEST_VERSION` (downgraded app, or a
foreign database file), the range is empty and the code proceeds as if all
is well against a schema it doesn't understand. Erroring out on
`current_version > LATEST_VERSION` is one `if` and turns undefined behavior
into a clear message.

> **Resolution (2026-07-24).** `schema::migrate` now checks
> `current_version > LATEST_VERSION` before the migration loop and
> refuses with a new dedicated error variant,
> `Error::UnsupportedSchemaVersion { found, supported }` (added to the
> crate-level enum from the finding-6 fix). Its rendered message tells
> the user what actually happened: *"database schema version 2 is newer
> than this build supports (1); it was probably created by a newer
> version of the app"* — actionable for the downgraded-app case, and a
> clear signal for a foreign database file. Because `open` runs `migrate`,
> the check guards every normal path to a connection; a comment at the
> check explains why proceeding would be unsafe (reading/writing schema
> this code doesn't understand).
>
> The structured `{ found, supported }` fields (rather than a plain
> message) let the future Tauri layer distinguish this from other `Db`
> errors and offer a sensible UX ("this library was created by a newer
> version of grepm — please update").
>
> Test: `migrate_rejects_a_database_from_a_newer_app_version`
> (`core/src/db/schema.rs`) sets `user_version = LATEST_VERSION + 1` and
> asserts the exact variant and both version numbers via `matches!`.

### 14. Duplicate public paths into `db` ✅ FIXED

`db/mod.rs` re-exports `schema::populate_fts` and the four query functions,
but `schema` (and `models`) are also `pub` — so `db::populate_fts` and
`db::schema::populate_fts` both work, while `queries` alone is private.
Tests and integration code already mix the styles (`db::schema::open` vs
`db::populate_fts`). Pick one surface: either a curated facade (make
`schema` private too) or plain public modules with no re-exports.

> **Resolution (2026-07-24).** Went with plain public modules and no
> re-exports — the option that matched how most existing code already
> addressed items (`db::schema::open` in every test helper and
> integration test). `db/mod.rs` is now just three `pub mod`
> declarations (`models`, `queries`, `schema`); the re-export block is
> deleted, so every item has exactly one public path
> (`db::schema::populate_fts`, `db::queries::insert_message`, …).
> The facade alternative was rejected because `schema::configure` /
> `schema::migrate` are used directly from test helpers in other modules,
> which would have forced the facade to re-export nearly all of `schema`
> anyway.
>
> Callers updated to the canonical paths: `ingest/mod.rs` now imports
> `crate::db::schema` and calls `schema::populate_fts`, and
> `ingest/loader.rs` imports `crate::db::queries` and calls
> `queries::upsert_conversation` / `insert_participant` /
> `link_conversation_participant` / `insert_message` (its doc-comment
> cross-reference was updated too). No behavior change; the whole suite
> (101 tests) passes and the `src-tauri` crate, which doesn't touch `db`
> yet, still builds.

### 15. Empty search text is handled by accident, not by contract

An empty or whitespace-only query becomes `MATCH '""'`, which happens to
return zero rows without error (verified). That's a reasonable outcome, but
it's undocumented and untested, and `run()` trims text while direct
`SearchIndex::search` callers get untrimmed input — two subtly different
behaviors for the same trait. Worth a test pinning the empty-query behavior
and a decision on whether trimming belongs inside `search` itself.

---

## What's good (worth keeping as-is)

- **Test discipline is excellent**: descriptive names, one behavior per
  test, unit tests co-located with modules plus genuinely end-to-end
  integration tests on real files/DBs, and tests that encode *why* (e.g.
  relevance-vs-chronological ordering deliberately de-confounded, the
  FTS5-vs-LIKE distinguishing scenarios in `tests/search.rs`).
- **SQL hygiene**: every user-supplied value is parameterized; the only
  `format!`-interpolated SQL fragments (`FILTER_SQL`, `order_by`) are
  compile-time constants or enum-matched literals, so no injection surface.
- **FTS5 query escaping** (quote-doubling + phrase wrapping,
  `core/src/search/fts.rs:30`) is correct and covered by tests for the `OR`
  operator, phrases, and stray quotes.
- **The mojibake repair** (`core/src/ingest/parse.rs:28`) is a careful,
  well-documented solution to a real Facebook-export defect, with the
  all-or-nothing fallback reasoning spelled out and tested.
- **Domain decisions are documented where they live**: conversation-scoped
  participants (no global dedup by display name), the deleted-account
  "Facebook User" collision, numeric `message_N.json` ordering — each with a
  rationale comment and a test.
- Pragmas (WAL, `synchronous=NORMAL`, `foreign_keys=ON`) are appropriate for
  the workload, and the dedup index's `COALESCE(sender_id, -1)` correctly
  neutralizes SQLite's NULLs-are-distinct semantics for messages — which is
  exactly the trick the conversations table is missing (finding 2).

## Suggested priority

1. Findings 1–3 together make **any second import unsafe** (index
   corruption + data duplication + wrong counts). Fix before shipping any
   import UI; add "import twice" and "import two different exports"
   integration tests, which would have caught all three. **Still open —
   now the top remaining priority.**
2. ~~Finding 4 before writing migration 2.~~ Done (2026-07-24).
3. Findings 5 and 7 as part of wiring the Tauri layer. ~~Findings 6 and
   8~~ are done (2026-07-24), so the remaining Tauri-layer prep is
   persisting reactions (5) and deciding the partial-import/resume story
   (7) — finding 7 pairs naturally with the `ImportReport` idea now that
   the crate has a structured error type to report per-file failures
   with.
