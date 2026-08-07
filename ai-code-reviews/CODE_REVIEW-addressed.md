# Code Review — grepm — Addressed Findings

**Extracted from:** `CODE_REVIEW.md` (review dated 2026-08-05)
**Extracted:** 2026-08-06

The findings from the grepm code review that have been resolved. Each keeps
the resolution block written when it was fixed, recording what actually
changed, where the fix differs from what was recommended, and what was
deliberately left undone.

Open findings stay in `CODE_REVIEW.md`. **IDs are stable across both files** —
an ID that appears here has been resolved, not retired, and the commits and
code comments citing it still point at the right thing. Section numbers are
kept from the original too, so a cross-reference like "see §8" resolves in
either document.

The review method — what was demonstrated by running code versus by reading
it — is documented in `CODE_REVIEW.md` §1.

**Index:**

| ID | Severity | Area | Title | Resolved |
|---|---|---|---|---|
| A9 | High | db | A failed migration left the connection inside an open transaction | 2026-07-24 |
| C3 | Medium | db | `NULL` title/thread_path defeats the conversation upsert | 2026-08-05 |
| A1 | Medium | search | No deterministic tiebreaker in `ORDER BY` → unstable pagination | 2026-08-05 |
| A2 | Medium | db API | Participant find-or-create and linking had to be called in lockstep | 2026-08-05 |
| A10 | Medium | api | `Box<dyn Error>`, lost file context, leaked `rusqlite::Result` | 2026-07-24 |
| A11 | Medium | scan | `find_messages_root` swallowed I/O errors and walked unbounded | 2026-07-24 |
| D1 | Medium | docs | `KNOWN_ISSUES.md` #2 described a fix that was not the one implemented | 2026-08-05 |
| A3 | Low | db | `INSERT OR IGNORE` swallows more than dedup conflicts | 2026-08-05 |
| A4 | Low | db | Negative `user_version` panics `migrate` | 2026-08-05 |
| A5 | Low | scan | `count` and `scan` disagree about symlinked conversation dirs | 2026-08-05 |
| A6 | Low | search | Snippet delimiters `[`/`]` collide with literal brackets | 2026-08-05 |
| A12 | Low | scan | `unwrap()` in the production sort path | 2026-07-24 |
| A13 | Low | search | `Page` accepted a negative limit, disabling `LIMIT` | 2026-07-24 |
| A14 | Low | search | Count and page queries were not one snapshot | 2026-07-24 |
| A15 | Low | db | Migration runner accepted a newer-versioned database | 2026-07-24 |
| A16 | Low | api | Duplicate public paths into `db` | 2026-07-24 |
| T3 | — | tests | Shared test helpers | 2026-08-05 |
| T4 | — | tests | Property test for `repair_mojibake` | 2026-08-05 |
| I1 | — | infra | No Cargo workspace | 2026-08-05 |
| I2 | — | infra | No CI | 2026-08-05 |
| I3 | — | infra | fmt drift | 2026-08-05 |
| T5 | — | tests/ci | CI never ran the shell crate's tests | 2026-08-06 |
| I4 | Low | infra | `npm run verify` failed on a machine-local file | 2026-08-06 |
| — | — | style | `LATEST_VERSION` drift risk (§8) | 2026-08-05 |
| K1 | — | ingest/db | `message_count` was not accumulated across a conversation's files | — |
| K3 | — | db | Participants were deduped globally by exact name match | — |
| K4 | — | ingest | Participants appearing only in a later file were never linked | — |
| K5 | — | db | Conversation metadata was unconditionally overwritten on conflict | — |
| K7 | — | ingest/fts | `messages_fts` was never populated during import | — |

The `K` findings came from `core/KNOWN_ISSUES.md`, which was folded into these
documents and removed; they keep their original entry numbers, so an older
citation of "KNOWN_ISSUES #3" resolves to K3. That file recorded neither
severities nor fix dates, hence the dashes. `CODE_REVIEW.md` carries the full
map, including the three entries that were already covered by C4, A9, and
C1 + C2 and so were not duplicated.

Resolution blocks below still name `KNOWN_ISSUES.md` where the work at the
time included editing it. Those are left as written — they record what was
done, not where to look now.

---


## 3. Correctness findings

### C3 — `NULL` title/thread_path defeats the conversation upsert — **Medium** ✅ **Addressed**

> **Resolution (2026-08-05).** Neither recommended option; the author's call
> was to assume the fields are never null and make that assumption enforced
> rather than hoped for, at both layers:
>
> - `RawConversationFile.title` and `.thread_path` are now `String`, not
>   `Option<String>`. A file missing either is refused by serde with an
>   `Error::Parse` naming the file — the degenerate input can no longer reach
>   the storage layer at all, which is the gap this finding describes.
> - `conversations.title` and `.thread_path` are `NOT NULL`, so the upsert's
>   conflict target fires for every row by construction. Since nothing has
>   shipped, this edits the initial migration rather than adding a rebuild
>   step. `models::Conversation` follows suit (`String`, not `Option`).
>
> KNOWN_ISSUES #5 corrected — it claimed two files must agree on the fields
> "including both being present or both absent", and as this finding showed,
> both-absent never merged.
>
> Tests: `parse_conversation_file_rejects_a_file_without_the_conversation_key`
> (each field missing in turn, asserting the error names the file),
> `conversations_reject_a_null_title_or_thread_path` (the constraint itself),
> and `load_conversation_merges_every_file_into_one_conversation_row` (the
> positive case the finding is really about).
>
> Trade-off worth naming: a real export with one conversation missing a title
> now fails the whole import instead of silently splitting that thread. The
> evidence says real exports always populate both fields; if one ever
> doesn't, the fallback-to-folder-name option below is the way out.

`conversations` is keyed by `UNIQUE (title, thread_path)` and
`upsert_conversation` conflicts on that pair. In SQLite, `NULL`s are distinct
in a unique index, so `ON CONFLICT` **never fires** when either field is
`NULL` — every file inserts a brand-new conversation row, and because the
message dedup index is scoped by `conversation_id`, all of that
conversation's messages are duplicated along with it.

**[verified]** Two `message_N.json` files in the same conversation folder,
both lacking `title`/`thread_path` (legal per `RawConversationFile`, both
fields were `Option`), produce **two** `conversations` rows on one import.
Importing such a conversation twice yields two conversation rows and two
copies of its single message.

`KNOWN_ISSUES.md` #5 got this wrong: it said two files must agree on the
fields "including both being present or both absent" to merge. Both-absent
never merged. Existing test coverage proved the parser accepts such files, so
the pipeline admitted inputs the storage layer silently split.

**Recommendation.** Pick one:

- Make the key total: since `scan` already hands `load_conversation` the
  conversation folder (`ConversationDir::folder`), fall back to the folder
  name when `thread_path` is `NULL` (and `''` for `title`), storing the
  fallback rather than `NULL`. Real exports always populate both fields, so
  this only changes behavior for the degenerate case.
- Or mirror the messages fix: a
  `CREATE UNIQUE INDEX ... ON conversations (COALESCE(title,''), COALESCE(thread_path,''))`
  with a matching expression conflict target.

Either way, correct KNOWN_ISSUES #5.

### K1 — `message_count` was not accumulated across a conversation's files — **Fixed**

> **Resolution.** `load_conversation` (`core/src/ingest/loader.rs`) loops over
> every file in `conversation_dir.message_files` and calls
> `upsert_conversation` (`core/src/db/queries.rs`) once per file, so each
> file's count is added onto the running total via
> `message_count = message_count + excluded.message_count`. Files are matched
> to the same conversation row by the `(title, thread_path)` unique key (see
> K5) rather than the removed `raw_name` folder name.

`conversations.message_count` did not accumulate across the files a
conversation is split into.

Note what the fix does *not* provide: an idempotence story. The accumulation
is right within a single import and inflates across repeated ones — that is
C1, still open.

### K3 — Participants were deduped globally by exact name match — **Fixed**

> **Resolution.** `participants.name` is no longer globally `UNIQUE`
> (`core/src/db/schema.rs`). `find_or_create_participant`
> (`core/src/db/queries.rs`) finds-or-creates a participant scoped to a single
> `conversation_id`: it looks for an existing participant with that name
> already linked to the conversation via `conversation_participants`, and only
> inserts a new row if none is found — linking it in the same call. Two
> different real people who happen to share a display name in unrelated
> conversations ("John Smith") now get separate `participants` rows instead of
> being merged.
>
> Creating and linking are one operation on purpose: the link is *how* a
> participant is found, so an unlinked participant row could never be found
> again. That matters for a message sender who, for whatever reason, isn't in
> the file's `participants` list — without the link they would get a fresh
> `participants` row for every single message they sent. Folding the link into
> the lookup means no caller can get that pairing wrong. A2 is the finding
> that made that shape mandatory rather than conventional.

**The trade-off, accepted deliberately.** Someone who is genuinely the same
person across several conversations now gets a separate `participants` row in
each, so there is no built-in way to query "all messages from Bob" across
conversations without additional reconciliation logic. This was already
unreliable under global name-based dedup — the export carries no stable
per-person id, only a display name — so it is a deliberate trade for
correctness within a conversation.

Two participants sharing the exact same name *within one* conversation remain
indistinguishable. That is a limitation of the source data, not something this
fix, or any fix, can address.

### K4 — Participants appearing only in a later file were never linked — **Fixed**

> **Resolution.** `load_conversation` (`core/src/ingest/loader.rs`) inserts and
> links every file's `participants` list, not just the first file's, so
> someone who appears only in a later page's list — and never sends a message
> — is still linked via `conversation_participants`.

For a conversation split across several `message_N.json` files, only the first
file's `participants` list was persisted.

### K5 — Conversation metadata was unconditionally overwritten on conflict — **Fixed**

> **Resolution.** `conversations` no longer has a `raw_name` column; a
> conversation is identified by `UNIQUE (title, thread_path)`
> (`core/src/db/schema.rs`), and `upsert_conversation`
> (`core/src/db/queries.rs`) conflicts on that pair. Because `title` and
> `thread_path` are the conflict *key* rather than fields the `DO UPDATE`
> touches, on conflict they already equal what is stored — there is no
> overwrite to blank them out. Only `is_still_participant` and `message_count`
> are updated on conflict.
>
> Both fields are also required rather than optional, which closes the worse
> version of the same hole. C3 is the finding that established that, and
> records why NULLs made it worse.

**The residual caveat.** Two files for the same conversation must agree on
`title` *and* `thread_path` exactly to be recognised as the same conversation.
If a page ever reported a different `title` for the same thread they would
become two conversations — which, now that every file goes through
`upsert_conversation` (K1), surfaces as a duplicate `conversations` row with a
split `message_count`. Not observed in practice, but worth knowing if it ever
comes up.

### K7 — `messages_fts` was never populated during import — **Fixed**

> **Resolution.** `populate_fts` (`core/src/db/schema.rs`, renamed from
> `populate_messages_fts`) is called at the end of `ingest::import_export`
> (`core/src/ingest/mod.rs`), after every conversation has been loaded, so a
> fresh import leaves `messages_fts` fully indexed. The table also tokenizes
> with `unicode61 remove_diacritics 2`, so a search for "cafe" matches content
> containing "café".

A completed import left the full-text index empty, so search over a
freshly-imported database returned nothing.

The fix is correct for one import into a fresh database and no further: the
plain append it uses is exactly what C2 shows to corrupt the index on a second
import.

---

## 4. API and robustness findings

### A1 — No deterministic tiebreaker in search ordering — **Medium** ✅ **Addressed**

> **Resolution (2026-08-05).** Fixed as recommended: every `ORDER BY` arm now
> ends in `m.id` (`DESC` for `Relevance`/`Latest`, `ASC` for `Oldest`), with a
> comment recording why the ordering has to be total. Pinned by three new unit
> tests in `core/src/search/fts.rs` —
> `search_breaks_timestamp_ties_by_message_id`,
> `search_breaks_relevance_ties_by_message_id`, and
> `paging_through_tied_results_visits_every_hit_exactly_once` (six
> same-millisecond, same-bm25 hits paged 2 at a time must appear exactly once
> each). The first two were confirmed to fail against the pre-fix `ORDER BY`.

`FtsIndex::search` orders by exactly one expression: `bm25(...)`,
`timestamp_ms DESC`, or `timestamp_ms ASC`. None has a tiebreaker, and ties
are not exotic here:

- Same-millisecond messages are a first-class scenario in this codebase — the
  dedup index went through two revisions specifically to keep
  same-`timestamp_ms` messages apart.
- bm25 ties are routine for short chat messages; the probe observed the
  degenerate case where every hit scored an identical `-1e-6`.

Because each page is a separate query — and a separate transaction, since
pagination calls `search` once per page — SQLite is free to order tied rows
differently across pages, so a row can appear on two pages or on none.

**Recommendation:** append `, m.id DESC` (or `ASC` for `Oldest`) to every
arm. One-line change, makes pagination deterministic.

### A2 — Participant find-or-create and linking must be called in lockstep — **Medium** ✅ **Addressed**

> **Resolution (2026-08-05).** Folded as recommended:
> `insert_participant` + `link_conversation_participant` are replaced by a
> single `find_or_create_participant(conn, conversation_id, name) -> Result<i64>`
> that finds, or inserts *and* links. `link_conversation_participant` is gone
> — it had no other callers — so the trap is unrepresentable rather than
> merely documented. Both duplicated call sites in `loader.rs` collapsed to
> one call each (the sender path is now a `map(...).transpose()?` over
> `sender_name`). The link uses a plain `INSERT` rather than
> `INSERT OR IGNORE`: the id is freshly minted inside the function, so a
> conflict is impossible and shouldn't be silently swallowed if one somehow
> occurs (cf. A3). Pinned by two new unit tests in `core/src/ingest/loader.rs`
> — `find_or_create_participant_links_the_new_participant_to_the_conversation`
> and `find_or_create_participant_returns_the_existing_id_when_called_again`
> — both confirmed to fail if the link is removed from the function, along
> with seven existing loader tests. KNOWN_ISSUES #3 updated to match.
>
> Not done, deliberately: the per-conversation `HashMap` cache from P2. It's
> a performance change, and the find-or-create call is now the single place
> to add it when that's measured.

`insert_participant` found an existing participant *only via the
`conversation_participants` join*. If a caller inserted but forgot to
immediately call `link_conversation_participant`, the next lookup for the
same name missed and inserted another row — the exact failure KNOWN_ISSUES #3
describes for message senders. Nothing in the API enforced the pairing;
`loader.rs` just repeated the two-call sequence correctly in two places.

There is also no database backstop: `participants` has no unique constraint
that could catch a violation of this invariant — deliberately, since the same
name may repeat across conversations, but that means the invariant lived
entirely in caller discipline.

**Recommendation:** fold the link into the function — it already takes
`conversation_id`. A single
`find_or_create_participant(conn, conversation_id, name) -> Result<i64>`
that finds, or inserts *and links*, removes the trap and both duplicated call
sites. This is the type-driven "make invalid states unrepresentable" move at
API scale.

### A3 — `INSERT OR IGNORE` swallows more than dedup conflicts — **Low** ✅ **Addressed**

> **Resolution (2026-08-05).** Swapped for the recommended
> `ON CONFLICT (...) DO NOTHING`, spelling out the dedup index's columns and
> expressions (`COALESCE(sender_id, -1)`, `COALESCE(content, '')`) so the
> ignore is scoped to exactly that index. `changes() == 0` still means
> "duplicate", and now means only that.
>
> The conflict target is self-verifying: SQLite rejects a statement whose
> target doesn't match an index with `ON CONFLICT clause does not match any
> PRIMARY KEY or UNIQUE constraint` — confirmed by deliberately dropping one
> column from the target, which failed every dedup test. So the existing
> dedup suite passing proves it binds to `idx_messages_dedup`.
>
> Two new unit tests in `core/src/ingest/loader.rs`:
> `insert_message_reports_a_duplicate_as_none_rather_than_an_error` (the path
> that must keep working) and
> `insert_message_errors_on_a_constraint_violation_that_is_not_a_duplicate`,
> which adds a throwaway unique index to stand in for the future constraint
> this finding is about, then shows the violation now errors — confirmed to
> fail against `INSERT OR IGNORE`, where the row was silently dropped.

`insert_message` uses `INSERT OR IGNORE` and interprets `changes() == 0` as
"duplicate". `OR IGNORE` also silently drops rows that fail *any* non-FK
constraint (`NOT NULL`, `CHECK`), so a future schema constraint could turn a
coding bug into silently missing messages instead of an error — a swallowed
row is indistinguishable from a duplicate here.
`ON CONFLICT (conversation_id, ...) DO NOTHING` targeting `idx_messages_dedup`
scopes the ignore to exactly the dedup index and errors on everything else.
Cheap insurance.

### A4 — Negative `user_version` panics `migrate` — **Low** ✅ **Addressed**

> **Resolution (2026-08-05).** `migrate` now refuses `current_version < 0`
> before the migration loop, so a tampered file produces an `Err`, not a
> panic. Confirmed the panic was real: without the guard the new test aborts
> with `index out of bounds: the len is 1 but the index is
> 18446744073709551615` — exactly the `as usize` wrap described here.
>
> Took the "dedicated variant" option rather than reusing
> `UnsupportedSchemaVersion`: that variant's message says the database is
> *newer* than this build and suggests updating the app, which would be
> actively misleading here. The new `Error::InvalidSchemaVersion { found }`
> says the file is corrupt or isn't ours, and its doc comment records why the
> two are separate. Pinned by
> `migrate_rejects_a_negative_user_version_instead_of_panicking`.

`migrate` indexes `MIGRATIONS[version as usize]` over
`current_version..LATEST_VERSION`. A hand-tampered database with a negative
`user_version` — the pragma is signed — makes `version as usize` wrap and the
indexing panic. Guard `current_version < 0` with the same
`UnsupportedSchemaVersion`-style refusal (or a dedicated variant) so a
corrupt file produces an `Err`, not a panic, consistent with the crate's own
no-panic policy.

### A5 — `count` and `scan` disagree about symlinks — **Low** ✅ **Addressed**

> **Resolution (2026-08-05).** Aligned on *not* following symlinks, which is
> what `scan_inbox` and `find_messages_root` already did. `count_inbox` now
> filters on `DirEntry::file_type()` (reports the link itself) instead of
> `Path::is_dir()` (resolves it), and `message_files_in` likewise switched
> from `Path::is_file()` to `DirEntry::file_type()` so a symlinked
> `message_N.json` is skipped too — the same policy applied consistently
> rather than only at the conversation-folder level. The rationale is recorded
> on `scan_inbox`: an export is a self-contained tree, so a link inside it
> points somewhere the user didn't ask to import, and following links would
> let a cycle turn the walk infinite. The export root itself is exempt — the
> caller named it explicitly, so `validate_root` still resolves it. Pinned by
> three new unit tests in `core/src/ingest/scan.rs` (all `#[cfg(unix)]`):
> `count_and_scan_both_skip_a_symlinked_conversation_folder` (the finding's
> exact scenario, asserting the two passes agree),
> `scan_skips_symlinked_message_files`, and
> `find_messages_root_does_not_follow_a_symlink_to_the_messages_directory`.
> The first two were confirmed to fail against the pre-fix checks.

`count_inbox` used `entry.path().is_dir()`, which follows symlinks;
`scan_inbox` uses `entry.file_type().is_dir()` on a non-following `WalkDir`,
which doesn't. A symlinked conversation folder is counted by the cheap pass
but skipped by the real one, so a progress UI built on `count` would never
reach 100%. Align the two checks, and decide explicitly whether symlinked
conversation dirs are in or out.

### A6 — Snippet delimiters collide with message text — **Low** ✅ **Addressed**

> **Resolution (2026-08-05).** Switched to the suggested interlinear
> annotation pair, exposed as `search::MATCH_START` (`U+FFF9`) and
> `search::MATCH_END` (`U+FFFB`). They live in `search/mod.rs`, not
> `search/fts.rs`: how a match is marked is part of what `SearchIndex`
> promises about `SearchHit.snippet`, not an FTS5 detail, and a consumer has
> to name them to translate them — which it must do without reaching past the
> trait.
>
> `SearchHit::snippet` now documents the contract, including that the
> surrounding text is sender-controlled and needs escaping (see A8).
>
> Pinned by `match_markers_are_distinguishable_from_punctuation_in_the_message`,
> which searches "coffee" in `I read [1] and the coffee was fine` and asserts
> the full snippet — the message's own brackets come through untouched while
> only the match is wrapped. Under the old markers that snippet was
> `I read [1] and the [coffee] was fine`, with "1" indistinguishable from a
> highlight.
>
> Left alone deliberately: the `'...'` ellipsis argument has the same in-band
> ambiguity (a message can trail off with "..."), but a consumer has no reason
> to distinguish "clipped here" from a typed ellipsis, so it isn't worth a
> sentinel. Also not done: stripping `U+FFF9`/`U+FFFB` from `content` at
> ingest, which is what would make collisions impossible rather than merely
> implausible. That changes stored data for a case that doesn't arise in
> exported chat text.

`snippet(messages_fts, 0, '[', ']', '...', 8)` marks matches with characters
that legitimately occur in chat text, so a UI can't reliably distinguish a
highlight from a literal bracket, and can't safely transform the markers into
markup. Use improbable sentinels — the Unicode interlinear annotation pair
`\u{FFF9}`/`\u{FFFB}`, or a private-use pair — and translate at the UI
boundary.

### A9 — A failed migration leaves the connection inside an open transaction — **High** ✅ **Addressed**

> **Resolution (2026-07-24).** `schema::migrate` now takes `&mut Connection`
> and wraps each migration step in `Connection::transaction()` instead of
> manual `execute_batch("BEGIN;") … execute_batch("COMMIT;")`. If any
> statement fails, the `Transaction` guard is dropped uncommitted and
> rusqlite issues the `ROLLBACK` automatically, so the connection is never
> stranded. The `user_version` bump happens inside the same transaction, so a
> failed migration also can't leave the version pointing at schema that never
> applied.
>
> Regression test:
> `migrate_rolls_back_a_failed_migration_and_leaves_the_connection_usable`.
> It forces a *genuine* mid-migration failure — pre-creating a `messages`
> table containing duplicate rows so `CREATE UNIQUE INDEX idx_messages_dedup`
> fails, since `IF NOT EXISTS` only guards against the index existing, not the
> data violating it — then asserts the migration errors, `user_version` stays
> 0, and a fresh `conn.transaction()` succeeds afterward.

`schema::migrate` bracketed each migration with
`execute_batch("BEGIN;") … execute_batch("COMMIT;")`. If any statement in
between failed, the function returned early with the error and **never rolled
back**, leaving the connection with a dangling transaction. Every later
attempt to start a transaction on that connection then failed.

**[verified]**, by pre-seeding a `messages` table containing duplicates so
the `CREATE UNIQUE INDEX` genuinely fails:

```
migrate         => Err(… UNIQUE constraint failed: index 'idx_messages_dedup')
subsequent BEGIN => Err(… "cannot start a transaction within a transaction")
```

The single migration was all `IF NOT EXISTS`, so failures were unlikely in
practice — but the moment a second migration lands (an `ALTER TABLE`, a data
backfill), this becomes a real failure mode: the app "recovers" from a
migration error into a connection that can no longer write transactionally.

### A10 — `Box<dyn Error>` in a library, and lost file context — **Medium** ✅ **Addressed**

> **Resolution (2026-07-24).** `core/src/error.rs` defines a crate-level
> `thiserror` enum, re-exported from `lib.rs` with a `pub type Result<T>`
> alias, carrying `Io`, `Db`, `ReadFile { path, source }` and
> `Parse { path, source }` (later joined by `UnsupportedSchemaVersion` and
> `InvalidSchemaVersion`). Every public function in the crate returns it —
> `db`, `ingest`, and `search`, including the `SearchIndex` trait itself. The
> only remaining `rusqlite::Result`s are inside row-mapper closures, where
> rusqlite's API requires them.
>
> This resolves each sub-issue: `Box<dyn Error>` is gone and the enum is
> `Send + Sync`, pinned by a compile-time assertion test, so errors can cross
> thread and async boundaries in the Tauri layer; JSON errors no longer
> masquerade as `io::Error`; the offending file's path is carried in the
> variant and rendered in the message (`failed to parse /…/message_1.json:
> expected value at line 1 column 3`); and `SearchIndex::search` no longer
> hard-codes the FTS5 backend's error type.

Three issues compounded here:

- `import_export` and `load_conversation` returned
  `Result<_, Box<dyn Error>>`. For a library crate this erases all error
  structure — callers cannot distinguish "one file was malformed" from "the
  database is locked". It was also not `Box<dyn Error + Send + Sync>`, so
  those errors couldn't cross a thread or async boundary, which is exactly
  where they're headed once the Tauri layer runs imports off the main thread.
- `parse_conversation_file` returned `io::Result`, silently converting
  `serde_json::Error` into `io::Error` — a JSON syntax error masquerading as
  an I/O error.
- Most importantly for users: **the error carried no file path**. A malformed
  file in a 900-conversation export failed the whole import with
  `expected value at line 1 column 3` and no indication of which file.

The `SearchIndex` trait also leaked `rusqlite::Result` — a trait whose
purpose is to abstract the FTS5 backend shouldn't hard-code the backend's
error type in its signature.

### A11 — `find_messages_root` swallows I/O errors and walks unbounded — **Medium** ✅ **Addressed**

> **Resolution (2026-07-24).** All three sub-issues addressed:
>
> - *Swallowed errors*: `.filter_map(|entry| entry.ok())` replaced with an
>   explicit loop. An unreadable directory doesn't abort the search — the
>   inbox may still be findable elsewhere, deliberately more forgiving than
>   fail-fast — but the first walk error is remembered, and if the search
>   comes up empty that error is returned instead of the misleading "could not
>   find a messages/inbox directory".
> - *Unbounded walk*: discovery is capped at `MESSAGES_ROOT_MAX_DEPTH = 4`.
>   Real exports place `messages/inbox` at depth 2, or 3 under a dated export
>   folder; the bound keeps the walk out of the export's media trees. The
>   trade-off is documented at the constant.
> - *Double walk*: `count` and `scan` are now thin wrappers over `count_inbox`
>   / `scan_inbox`, so a caller can run `find_messages_root` once and reuse
>   the result for both passes.
>
> Tests: `find_messages_root_does_not_search_below_the_depth_bound`,
> `find_messages_root_reports_a_walk_error_instead_of_a_misleading_not_found`,
> `find_messages_root_keeps_searching_past_an_unreadable_directory`, and
> `count_inbox_and_scan_inbox_work_from_a_single_discovery_walk`.

The WalkDir traversal used `.filter_map(Result::ok)`, so a permission error
on the way to `messages/inbox` was silently dropped and the user saw the
misleading "could not find a messages/inbox directory" instead of "permission
denied". Two secondary issues: the walk was depth-unbounded over the whole
export, which for a real Facebook export means traversing tens of thousands
of media files; and `count()` and `scan()` each called `find_messages_root`,
so the count-then-scan flow performed the discovery walk twice.

### A12 — `unwrap()` in the production sort path — **Low** ✅ **Addressed**

> **Resolution (2026-07-24).** Took the `(number, path)` option — the one that
> makes the invariant hold *by construction* rather than masking a
> hypothetical `None` with a sentinel. `message_files_in` parses each file's
> number at the moment the file is admitted and returns `Vec<(u64, PathBuf)>`;
> `scan` sorts with `sort_unstable_by_key(|&(number, _)| number)`, for which
> no failure path exists, and strips the numbers when building
> `ConversationDir::message_files`, so the public type is unchanged. Side
> benefit: `message_number` used to run twice per file and now runs once.

`message_files.sort_by_key(|path| message_number(path).unwrap())` could not
panic, because `message_files_in` only admitted paths for which
`message_number(...).is_some()` — but that invariant lived in a different
function, and a future edit to the filter would break this at runtime.

### A13 — `Page` accepts a negative limit — **Low** ✅ **Addressed**

> **Resolution (2026-07-24).** Took the type-level option: `Page`'s fields are
> `u32`, so a negative value from the frontend fails at the serde boundary
> instead of reaching SQL, and the invalid state is unrepresentable for Rust
> callers too — no runtime clamping logic to keep in sync. `u32` binds
> cleanly to SQLite's `i64` parameters, so `FtsIndex::search` needed no
> changes. Deliberately *not* capped at the high end: a huge-but-positive
> limit is an explicit "give me everything", a policy question for the app
> layer, unlike the silent negative → unlimited surprise. Pinned by
> `page_deserialization_rejects_a_negative_limit`,
> `page_deserialization_rejects_a_negative_offset`, and
> `page_deserializes_from_valid_input`.

`Page { limit: i64, offset: i64 }` is deserialized from the frontend and
passed straight into `LIMIT ?6 OFFSET ?7`. In SQLite, `LIMIT -1` means
*unlimited* — a buggy or malicious frontend value dumps the entire result set
in one page.

### A14 — Count and page queries are not a consistent snapshot — **Low** ✅ **Addressed**

> **Resolution (2026-07-24).** `FtsIndex::search` wraps both reads in a single
> transaction via `Connection::unchecked_transaction()` and commits after
> collecting the hits. Under SQLite's deferred transaction semantics the
> snapshot is established at the first read and held until commit, so a write
> landing between the two queries can no longer make them disagree.
>
> `unchecked_transaction` rather than `transaction()` because `FtsIndex` holds
> `&Connection` and the `SearchIndex` trait takes `&self`; the borrow-checked
> variant would have forced `&mut` through the whole trait for no benefit. The
> "unchecked" caveat is safe here — the transaction is created, used for two
> read-only queries, and committed within one function. Pinned by
> `search_commits_its_read_transaction_and_leaves_the_connection_free`; true
> snapshot-isolation interleaving isn't deterministically testable without
> pausing `search` mid-function, so that part rests on SQLite's documented
> semantics.

`FtsIndex::search` ran the `count(*)` query and the paginated query as two
independent reads. A write between them — an import running while the user
searches — could make `count` disagree with the returned hits. Low impact for
a desktop app, but a read transaction around the pair is nearly free.

### A15 — Migration runner accepts a database from a newer app version — **Low** ✅ **Addressed**

> **Resolution (2026-07-24).** `schema::migrate` checks
> `current_version > LATEST_VERSION` before the migration loop and refuses
> with `Error::UnsupportedSchemaVersion { found, supported }`. Its message
> tells the user what happened — *"database schema version 2 is newer than
> this build supports (1); it was probably created by a newer version of the
> app"* — actionable for the downgraded-app case and a clear signal for a
> foreign database file. Because `open` runs `migrate`, the check guards every
> normal path to a connection. The structured fields let the future Tauri
> layer distinguish this from other `Db` errors and offer a sensible UX.
> Pinned by `migrate_rejects_a_database_from_a_newer_app_version`.

`migrate` iterates `current_version..LATEST_VERSION`; when a database's
`user_version` is *greater* than `LATEST_VERSION` — downgraded app, or a
foreign database file — the range is empty and the code proceeds as if all is
well against a schema it doesn't understand.

### A16 — Duplicate public paths into `db` — **Low** ✅ **Addressed**

> **Resolution (2026-07-24).** Went with plain public modules and no
> re-exports — the option matching how most existing code already addressed
> items (`db::schema::open` in every test helper and integration test).
> `db/mod.rs` is now three `pub mod` declarations; the re-export block is
> deleted, so every item has exactly one public path. The facade alternative
> was rejected because `schema::configure`/`schema::migrate` are used directly
> from test helpers in other modules, which would have forced the facade to
> re-export nearly all of `schema` anyway.

`db/mod.rs` re-exported `schema::populate_fts` and the query functions, but
`schema` and `models` were also `pub` — so `db::populate_fts` and
`db::schema::populate_fts` both worked, while `queries` alone was private.
Tests and integration code already mixed the styles.

---

## 5. Documentation drift

### D1 — `KNOWN_ISSUES.md` #2 describes a fix that isn't the implemented one — **Medium** ✅ **Addressed**

> **Resolution (2026-08-05).** Entry #2 deleted rather than corrected: it
> documented an already-fixed issue, and the design it was describing badly is
> stated accurately where it belongs — the NULL-content rationale lives on the
> `messages.content` column, `insert_message`, and the
> `duplicate_messages_with_null_content_are_rejected` test. A rewritten entry
> would only have restated those, with the same drift risk that made it wrong
> in the first place.
>
> The number is retired, not reused: `README.md`, `core/src/search/fts.rs`,
> and `core/tests/ingestion.rs` all cite entries by number, so renumbering
> would have silently broken them. The file's intro now records that numbering
> rule, so the gap doesn't read as an accident.

KNOWN_ISSUES #2 stated: "`content` is now `TEXT NOT NULL DEFAULT ''` …
`insert_message` stores a missing message body as `''` instead of `NULL`."
The code does the opposite: `content TEXT` is nullable, `insert_message` binds
`message.content.as_deref()` and stores `NULL`, `models::Message` documents
"stored as `NULL`", and the dedup index handles the NULL case with
`COALESCE(content, '')` — the same route taken for `sender_id`. The tests
pin the NULL-based behavior, so the code is consistent with itself; only the
document was wrong. Since this file functions as the project's decision
record, a wrong "fixed" entry is worth correcting promptly.

---

## 7. Tests

- **T3 — Shared test helpers** ✅ **Addressed (2026-08-05):** `write_file`,
  `migrated_connection`, and the `make_unreadable` guard were duplicated
  across five-plus files, unit and integration. A `tests/common/mod.rs`
  (integration) plus a `#[cfg(test)]` `test_util` module (unit) removes the
  drift risk.

  > **Resolution.** Both modules created as suggested. Collapsed: 5 copies
  > of `write_file`, 4 of `migrated_connection`, 2 of `make_unreadable`, and
  > `open_db` (which `search.rs` had open-coded inline twice instead of
  > copying). Helpers used by a single file — `table_names`,
  > `conversation_dir`, `fts.rs`'s raw-SQL seeders — stayed put; moving those
  > would trade duplication for indirection.
  >
  > The two `make_unreadable` copies had *already* drifted: their doc comments
  > no longer matched. Both call sites also carried an identical copy of the
  > run-as-root escape hatch, so the shared version folds that in and returns
  > `Option<impl Drop>` — `None` meaning "mode 000 didn't take, skip". A
  > `let Some(_guard) = … else { return }` replaces the guard-then-recheck
  > pair. Verified those tests don't now skip silently by making the
  > else-branch panic: it isn't taken, so the permission-denied paths still
  > run for real.
  >
  > The remaining duplication between `src/test_util.rs` and
  > `tests/common/mod.rs` is structural and documented in both: `test_util` is
  > `cfg(test)` inside the library, and each integration test is its own crate
  > linking the published library, so there is nothing for it to import.
- **T4 — Property test for `repair_mojibake`** ✅ **Addressed (2026-08-05):**
  the function is a perfect proptest target. For any `s: String`, encoding `s`
  as UTF-8 bytes and mapping each byte to its Latin-1 char must repair back to
  exactly `s`. This turns KNOWN_ISSUES #10's "low practical risk" reasoning
  into a checked invariant.

  > **Resolution.** `proptest` added as a dev-dependency, with three
  > properties in `core/src/ingest/parse.rs`:
  >
  > - `repair_mojibake_undoes_the_corruption_for_any_text` — the round-trip
  >   invariant. This is the one that matters: it turns "it always undoes the
  >   bug" from an argument into a check over every input, rather than the two
  >   hand-picked examples that were there.
  > - `repair_mojibake_leaves_any_text_with_a_non_latin1_char_alone` — pins
  >   the first escape hatch, over an arbitrary non-Latin-1 char in an
  >   arbitrary position.
  > - `repair_mojibake_leaves_any_ascii_text_alone` — pins the fast path.
  >
  > Verified the properties have teeth by sabotaging the implementation twice:
  > truncating with `c as u8` instead of bailing out on a non-Latin-1 char,
  > and refusing to decode past four bytes. Both were caught, the second
  > shrunk to the minimal input `"𞹤0"`. The sabotage-generated
  > `proptest-regressions/` seeds were deleted rather than committed — they
  > record artificial failures, not real ones.
  >
  > An idempotence property was considered and rejected as false.
  > `repair_mojibake` peels exactly one layer of the corruption, so feeding
  > its output back in peels another: `repair("Ã\u{83}Â©") == "Ã©"`, but
  > `repair("Ã©") == "é"`. That isn't a bug — it's the same accepted risk
  > KNOWN_ISSUES #10 records, since `"Ã©"` is indistinguishable by inspection
  > from a corrupted `"é"`, and the function's contract is that its input is
  > corrupted exactly once. The real behavior is pinned by
  > `repair_mojibake_peels_exactly_one_layer_of_corruption`, and KNOWN_ISSUES
  > #10 now cites it as a concrete instance of its residual risk.
- **T5 — CI never ran the shell crate's tests** ✅ **Addressed (2026-08-06):**
  the `app` job was clippy-only, justified by a comment that had rotted —
  "The desktop shell is still scaffold". It wasn't: `src-tauri` holds the
  library's atomic-rename logic and 25 tests, and `src-tauri/CLAUDE.md` §9
  openly warned that "a green CI doesn't prove it", with "remember to run it
  locally" as the mitigation — which is the job CI exists to do.

  > **Resolution.** One step added to the `app` job,
  > `cargo test --locked -p grepm`, after the existing clippy step. No new
  > setup: the Tauri system libraries were already installed there for clippy,
  > and the tests run against the repo's `samples/` export with no window, so
  > nothing needs a display.
  >
  > Both stale comments were corrected rather than deleted. The job comment
  > now says what the crate actually holds and why it stays a separate job
  > (the Tauri dependency tree is what keeps the engine job fast — the reason
  > the split existed all along, and the only one still true).
  > `src-tauri/CLAUDE.md` §9's warning became its inverse: both Rust jobs now
  > lint and test their own crate, so a green CI does cover these tests.
  >
  > Verified `cargo test --locked -p grepm` locally before committing —
  > 25 passed, 0 failed — so the first run of the new step starts green.
  > The job is still not a merge gate; that's I2's open "not done" note about
  > branch protection, unchanged.

---

## 8. Style and idiom

- **`LATEST_VERSION` drift risk:** ✅ **Addressed (2026-08-05)** —
  `pub const LATEST_VERSION: i32 = 1;` had to be bumped in lockstep with
  `MIGRATIONS`. It can be derived:
  `pub const LATEST_VERSION: i32 = MIGRATIONS.len() as i32;` (`<[T]>::len` is
  const), eliminating a whole class of forgot-to-bump bugs.

  > **Resolution.** Derived exactly as suggested, so adding a migration is a
  > one-place change. Both constants also picked up doc comments recording the
  > contract: index `n` takes a database from `user_version` `n` to `n + 1`,
  > and entries are append-only — never edit or reorder one a shipped database
  > has already run.
  >
  > Verified by temporarily appending a second migration step (probe written,
  > run, then reverted): `LATEST_VERSION` became `2` with no other edit,
  > `migrate` applied both steps, and the resulting `user_version` was `2`.
  > That also exercises the migration loop's multi-step path for the first
  > time — it has only ever run with a single entry.
  >
  > A permanent multi-step test would need `migrate` to take the migration
  > list as a parameter instead of reading the global; not done, as that's a
  > wider change than this item calls for.
- **Formatting:** ✅ **Addressed** — `cargo fmt --check` failed on three
  test-file spots (long hand-aligned tuple rows and one import line). See I3.

---

## 10. Infrastructure

- **I1 — No Cargo workspace** ✅ **Addressed (2026-08-05).** There was no root
  `Cargo.toml`; `core/` and `src-tauri/` built independently with **two
  separate lockfiles**, so they could silently resolve different dependency
  versions — both depend on `serde`/`serde_json`, and `src-tauri` depends on
  `core` by path. A root `[workspace]` manifest unifies the lockfile and
  enables shared `[workspace.lints]` in one place.

  > **Resolution.** Root `Cargo.toml` with both crates as members and
  > `resolver = "2"`. The two lockfiles are replaced by one at the root (482
  > packages, covering `core`'s dev-dependencies too), and `/target` moved
  > with it.
  >
  > `serde`/`serde_json` are declared once in `[workspace.dependencies]` and
  > referenced as `serde.workspace = true`, so the requirements can't drift at
  > the manifest level either — the shared lockfile only guarantees one
  > *resolved* version for a given requirement. Checked before the change: the
  > two lockfiles happened to agree on both crates at the time (`serde`
  > 1.0.229, `serde_json` 1.0.151), so the risk was live but not yet realized.
  >
  > `[workspace.lints]` set deliberately small, and both members opt in with
  > `[lints] workspace = true`: `unsafe_code = "deny"` (neither crate needs
  > any) and `clippy::redundant_clone = "warn"`. `clippy::perf` was not added
  > — it's already warn-by-default, so naming it would imply a change that
  > isn't one. `redundant_clone` found a real instance on its first run, a
  > needless `Vec` clone in `fts.rs`'s paging test, now fixed.
  >
  > Verified `cargo check --workspace`, `cargo clippy --workspace
  > --all-targets` (clean), and `cargo test --workspace`, including
  > `src-tauri`. The engine-only workflow (`cd core && cargo test`) still
  > works and is still worth keeping, since it skips the Tauri dependency
  > tree; the README documents both. The now-orphaned per-crate `target`
  > directories — 9 GB of dead cache — were removed.
- **I2 — No CI** ✅ **Addressed (2026-08-05).** There was no `.github/` (or
  equivalent). The suite is fast (~0.5 s) and clippy is already clean — this
  project was one small workflow away from enforcing `fmt --check` +
  `clippy -D warnings` + `cargo test`, and findings like the fmt drift (I3)
  only stay fixed with a gate.

  > **Resolution.** `.github/workflows/ci.yml`, on pushes to `main` and on
  > pull requests, in two jobs:
  >
  > - **engine** — `cargo fmt --all --check` (both crates; formatting needs no
  >   dependencies, so it's cheapest here), then `clippy -D warnings` and
  >   `cargo test` on `grepm_core`. Deliberately `-p grepm_core` rather than
  >   `--workspace`, so the job carrying all the tests doesn't wait on the
  >   Tauri dependency tree.
  > - **app** — installs the Tauri system libraries and type-checks `grepm`.
  >   It guards a crate that's still scaffold, but it's what catches a
  >   `grepm_core` API change breaking its one consumer.
  >
  > Both pass `--locked`, so a stale `Cargo.lock` fails the build instead of
  > being updated in place — cheap to add now that I1 left a single lockfile
  > at the root. `Swatinem/rust-cache` matters more than usual here:
  > `rusqlite`'s `bundled` feature compiles the SQLite amalgamation from
  > source on every cold build. All four commands were run locally before
  > committing, so the first run starts green.
  >
  > Not done: branch protection. CI on `main` reports *after* the push lands,
  > so this is a smoke alarm rather than a gate; making it block needs
  > PR-based work plus required checks in repo settings. That's the author's
  > call about how they want to work, not a code change.
- **I3 — fmt drift** ✅ **Addressed (2026-08-05)** — see §8; blocked on
  deciding intent, then trivially enforced by I2.

  > **Resolution.** Decided per-site rather than globally, since the two
  > remaining spots wanted opposite things. `tests/search.rs`'s over-long
  > `search::run(...)` call was simply reformatted — the file's *other*
  > `search::run` call was already in rustfmt's exploded form, so this made it
  > consistent. `tests/ingestion.rs`'s expected-attachments literal got
  > `#[rustfmt::skip]` and proper column alignment: it's a table of six
  > comparable rows, and rustfmt's one-field-per-line version buries the two
  > same-millisecond photo rows in thirty-odd lines.
  >
  > A `rustfmt.toml` was the wrong tool — it can't express "this one literal
  > is a table", and the settings that would stop rustfmt exploding it would
  > apply everywhere. `cargo fmt --all --check` is now clean and enforced by
  > I2.
- **I4 — `npm run verify` failed on a machine-local file** ✅ **Addressed
  (2026-08-06).** `npm run verify` stopped in its second gate —
  `prettier --check .` flagged `.claude/settings.local.json`, so eslint and
  vitest never ran. That file is machine-local state written by Claude Code,
  git-ignored globally and never committed, which meant one of the two
  definitions of done failed on any machine where the tooling had written it,
  over formatting the repo doesn't control. CI never sees the file, so the
  break was local-only — the same shape of drift I3 was about.

  > **Resolution.** Both `.claude` settings files added to `.prettierignore`,
  > under its existing "Owned by other tools" section — the same rule already
  > applied to `skills-lock.json` and `/src-tauri/`.
  >
  > The finding said `.prettierignore` "already excludes the tracked
  > `.claude/settings.json`". It did not; that file merely happened to be
  > prettier-clean, so it had never failed. Excluding only the local sibling
  > would have left the identical break waiting on the tracked one — and that
  > one *is* committed, so it would have failed CI rather than one machine.
  > Both are written by the same tool, which is the reason either is ignored.
  >
  > Verified by reproducing the failure first (`prettier --check .` flagging
  > exactly that one file), then running the full `npm run verify` after: all
  > three stages pass — svelte-check 0 errors, prettier and eslint clean,
  > 38 vitest tests passed.
