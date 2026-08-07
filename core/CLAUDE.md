# The engine

`grepm_core`: scan → parse → load, then FTS5 search over the result. Pure
Rust, no UI dependencies, and the most heavily tested part of the project.
Read this before editing anything under `core/`.

Most of what's here is load-bearing in a quiet way — a rule whose violation
produces wrong data rather than a failed build. The review findings cited by
ID are the long-form versions — open ones in `ai-code-reviews/CODE_REVIEW.md`,
resolved ones in `CODE_REVIEW-addressed.md`.

## 1. Migrations are append-only

`MIGRATIONS` (`src/db/schema.rs`) is a list where index `n` takes a database
from `user_version` `n` to `n + 1`. **Never edit or reorder an existing
entry** — a shipped database has already run it, and changing it changes only
what *new* databases get. Append a new one instead.

`LATEST_VERSION` is `MIGRATIONS.len()`, deliberately not a hand-maintained
constant: adding a migration is then a one-place change that can't be left
half-done.

`migrate` refuses two kinds of version rather than proceeding:

- **negative** → `InvalidSchemaVersion`. `user_version` is a signed pragma, so
  a corrupt file can hold a value no build ever wrote. Rejecting it also stops
  `version as usize` from wrapping to a huge index and panicking on the slice.
- **greater than `LATEST_VERSION`** → `UnsupportedSchemaVersion`. The file
  came from a newer build; reading it would mean guessing at a schema this
  code doesn't have.

Each step runs inside a `Transaction`, which rolls back if dropped without a
commit, so a migration that fails partway can't strand the connection inside
an open transaction.

## 2. NULLs are distinct in a SQLite unique index

This one fact explains three otherwise-odd bits of the schema, and it is the
single easiest way to silently corrupt an import. Two rows that are `NULL` in
the same column do **not** conflict, so a nullable column in a unique key
means "this row matches nothing, not even its own duplicate".

- `messages` dedups on `COALESCE(sender_id, -1)` and `COALESCE(content, '')`
  rather than on the raw columns (`idx_messages_dedup`). A message with no
  sender or no text would otherwise re-insert on every import of the same
  file.
- `conversations` is keyed `UNIQUE (title, thread_path)` with both columns
  `NOT NULL`, and `RawConversationFile` takes both as `String` rather than
  `Option<String>` — so a file missing either is refused at parse time, by
  name, instead of quietly becoming a second conversation. (C3, K5)

If you add a column to a unique key, decide what its NULL means before you
add it.

## 3. The dedup key includes `attachment_count`, and that's not incidental

Two attachment-only messages from the same sender in the same millisecond
have the same `conversation_id`, `sender_id`, `timestamp_ms`, empty `content`
and the same `type`. Without the attachment count in the key they collapse
into one message, and the second one's attachments are never stored.

Same-millisecond messages are routine in chat exports. Treat them as a normal
case, not an edge case — `search/fts.rs` does too (see §6).

## 4. `insert_message` names its conflict target on purpose

```rust
ON CONFLICT (conversation_id, COALESCE(sender_id, -1), timestamp_ms,
             COALESCE(content, ''), type, attachment_count)
DO NOTHING
```

`INSERT OR IGNORE` is shorter and wrong. It swallows `NOT NULL`, `CHECK`, and
every constraint added later — and here a swallowed row is indistinguishable
from a duplicate, so a bug would surface as messages quietly missing from
search results. Naming the index scopes the "ignore" to actual duplicates and
lets everything else surface as an error.

`insert_message` returns `Option<i64>`: `None` means it was a duplicate. Only
call `insert_attachments` for a `Some` — a duplicate's attachments are already
stored, and re-inserting doubles them.

## 5. Participants are scoped to a conversation, and creating one links it

Facebook's export gives no stable per-person id, only a display name. Two
different people can share one, so `find_or_create_participant` looks up a
name *within one conversation* (via `conversation_participants`) rather than
globally.

Creating and linking are one operation because **the link is how a
participant is found.** An unlinked `participants` row can never be found
again, so a caller who created without linking would mint a fresh row for
every message that sender sent. There is no `create_participant` to call by
mistake, and there shouldn't be. (A2, K3)

The cost, accepted knowingly: the same real person in five conversations gets
five `participants` rows, so "everything from Bob, everywhere" needs
reconciliation this crate doesn't do.

## 6. Search: three rules that each fix a real bug

All in `src/search/fts.rs`, and each looks like a detail until it isn't.

- **The query text is wrapped as a quoted phrase**, with `"` doubled. FTS5
  reads `-`, `*`, `AND`, `OR`, `NEAR` and quotes as syntax, so an unwrapped
  user string is a boolean expression, not a search for what they typed.
- **Every `ORDER BY` ends in `m.id`.** None of the three leading expressions
  is unique — bm25 ties constantly on short chat messages, and same-timestamp
  messages are a first-class case (§3). Each page is a separate query, so
  without a total tiebreaker SQLite may order tied rows differently per page
  and a message shows up twice, or never.
- **The count and the page of hits share one transaction.** They are two
  reads; a write landing between them (an import on another connection) would
  otherwise make `count` disagree with the hits the caller got.
  `unchecked_transaction` is used because `search` holds only a shared borrow.

The tokenizer is `unicode61 remove_diacritics 2`, so "cafe" matches "café".
That lives in the migration, which means changing it needs a new migration —
and a reindex, since existing rows were tokenized under the old rules.

## 7. Snippets are marked with format characters, not brackets

`MATCH_START` (U+FFF9) and `MATCH_END` (U+FFFB) are Unicode's interlinear
annotation anchors. Message text is arbitrary — it contains brackets, quotes
and angle brackets freely — so any ASCII marker would be ambiguous with
something the sender typed.

The consumer's obligation is in `src/CLAUDE.md` §1, and it is a security
boundary: split on the sentinels, or escape before substituting. Substituting
first is an XSS hole. If you change either constant, `src/lib/ipc/types.ts`
mirrors them by hand.

## 8. `populate_fts` is not idempotent — one import per database

It's a plain `INSERT ... SELECT` over every message with content, with no
conflict handling, run once at the end of `import_export`. Calling it twice
re-inserts every rowid. The app layer avoids the problem structurally by
giving each import its own file; don't add a code path that imports into a
populated database without fixing this first. (C1, C2)

## 9. `scan` counts and walks differently, and that's the `Progress` gap

`count_inbox` counts direct subdirectories of `messages/inbox` without opening
them. `scan_inbox` skips any folder that holds no `message_N.json`. So the
count is an **upper bound** and a finished import can end at `done < total` —
which is why completion is "the call returned", never `done == total`.

Two more things that walk is quiet about:

- **Symlinks are not followed.** A symlinked conversation folder is skipped,
  not imported: an export is self-contained, so a link points somewhere the
  user didn't ask for, and following links lets a cycle run forever. The
  export root itself is the exception — the caller named it.
- **`find_messages_root` searches to depth 4** rather than assuming a fixed
  path, because exports sometimes nest the inbox under a dated folder. The
  bound is what keeps discovery out of the export's media trees. A directory
  that can't be read doesn't abort the search, but if nothing is found the
  first walk error is reported instead of a misleading "not found".

## 10. `repair_mojibake` is unconditional and deliberately not idempotent

Messenger exports encode non-ASCII as UTF-8 and then mis-decode those bytes as
Latin-1; `parse.rs` reverses that on every non-ASCII string, without trying to
detect whether a given string needs it. The bug is systemic, so detection
would be guesswork with worse failure modes.

It is defensive — a `char` outside Latin-1, or bytes that aren't valid UTF-8
once reinterpreted, leave the text alone — but it cannot distinguish
already-correct Latin-1 text from corrupted text. `"Ã©"` becomes `"é"`, and
that is pinned by a test rather than treated as a bug. Don't reuse this
function against a cleaner data source. (K10)

## 11. Tests

`./scripts/verify-rust.sh` from the repo root is the definition of done —
`cargo fmt --all --check`, the 80-column comment check, clippy at
`-D warnings`, then `cargo test --workspace`. The engine is the reason that
suite has to stay fast.

Two test layers, and the duplication between them is structural:

- **Unit tests** live in a `#[cfg(test)] mod tests` at the bottom of the file
  they test, and share `src/test_util.rs` (`migrated_connection`,
  `write_file`, `make_unreadable`).
- **Integration tests** in `tests/` each compile as their own crate linking
  the published library — so they *cannot* reach `test_util`, which is
  `cfg(test)` within it. `tests/common/mod.rs` is the deliberate copy, with
  `#![allow(dead_code)]` because each test crate uses only part of it.

`make_unreadable` returns `None` when the permissions didn't take, which is
what happens as root — mode 000 is still readable there. A test that needs it
must skip, not fail, on `None`.

Name tests as sentences about behaviour
(`import_export_with_progress_treats_total_as_an_upper_bound`), and follow the
house habit: sabotage the fix, watch the new test fail, then put it back.

## 12. Things not to add here

- **No UI dependencies, and no Tauri.** This crate is why the whole import
  path is testable without a window.
- **No FTS5 SQL outside `src/search/`.** Callers go through the `SearchIndex`
  trait or `search::run`; that boundary is what makes the index swappable and
  is repeated in the root `CLAUDE.md` for a reason.
- **No `unsafe`** — denied at the workspace level.
- Reactions parse into `RawReaction` and are then dropped; the `reactions`
  table exists but nothing writes it. That's a known gap (C4), not an
  oversight to fix in passing.
