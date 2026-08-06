# Code Review — grepm

**Reviewer:** Claude Fable 5 (xhigh effort) · **Date:** 2026-07-24
**Scope:** `core/` (grepm_core), `src-tauri/`, project layout. The SvelteKit
frontend and Tauri shell are still the starter scaffold and are reviewed only
as such. This review was produced independently, without reading any other
review documents in this repository.

**Acknowledged up front (per the author):** reactions are not persisted yet,
and re-imports / further imports into the same database are known-broken.
Those areas are still covered below, because the review verified *specific
mechanisms* of the breakage that are worth having on record — but they are
marked as acknowledged.

---

## 1. Method

Everything below labeled **[verified]** was demonstrated by running code, not
by reading it. Commands run against the repo:

| Check | Result |
|---|---|
| `cargo test` (core) | **139 passed** (111 unit + 12 ingestion + 16 search), 0 failed |
| `cargo clippy --all-targets` | clean, 0 warnings |
| `cargo fmt --check` | 3 diffs, all in test files (`tests/ingestion.rs:423`, `tests/search.rs:18`, `tests/search.rs:443`) |

In addition, a temporary probe test (written, run, then deleted) exercised the
suspected re-import failure modes against the real pipeline and against raw
SQLite (bundled version **3.53.2**). Probe results are cited inline as
evidence in the findings.

---

## 2. Summary

`grepm_core` is a genuinely well-built crate. The error type is a proper
`thiserror` enum with `Send + Sync` pinned by a test, there are no
`unwrap`/`expect` calls outside test code, public functions are documented
with *why*-comments rather than *what*-comments, the test suite is large,
behavior-named, and unusually sophisticated (it knows about FTS5 shadow
tables, read-through semantics of external-content tables, and permission
-denied edge cases). The hard domain problems — mojibake repair, per
-conversation participant identity, numeric file ordering, message dedup keys
— show careful, documented reasoning.

The problems cluster in one place: **the import pipeline is only correct for
a single import into a fresh database.** Running it twice (or importing a
newer export over an old database) inflates `conversations.message_count`
[verified] and appends duplicate rowids into the FTS index, which happens to
produce correct query results on the bundled SQLite today but is undocumented
behavior with a demonstrated corruption mode [verified]. There is also one
latent single-import bug: a conversation whose files carry `NULL`
`title`/`thread_path` is never recognized as the same conversation across
files, because `NULL` never equals `NULL` in a SQLite unique index
[verified]. Beyond that, the findings are doc drift, API-hardening, and
performance headroom for large exports.

**Finding index:**

| ID | Severity | Area | Title |
|---|---|---|---|
| C1 | High (acknowledged) | ingest/db | `message_count` inflates on every re-import |
| C2 | High (acknowledged) | ingest/fts | `populate_fts` appends duplicate rowids; correctness rests on undocumented FTS5 behavior |
| C3 | ~~Medium~~ **Addressed** | db | `NULL` title/thread_path defeats the conversation upsert |
| C4 | Medium (acknowledged) | ingest | Reactions parsed but dropped; note on the future dedup story |
| A1 | ~~Medium~~ **Addressed** | search | No deterministic tiebreaker in `ORDER BY` → unstable pagination |
| A2 | ~~Medium~~ **Addressed** | db API | `insert_participant` + `link_conversation_participant` must be called in lockstep, nothing enforces it |
| A3 | ~~Low~~ **Addressed** | db | `INSERT OR IGNORE` swallows more than dedup conflicts |
| A4 | ~~Low~~ **Addressed** | db | Negative `user_version` panics `migrate` |
| A5 | ~~Low~~ **Addressed** | scan | `count` and `scan` disagree about symlinked conversation dirs |
| A6 | ~~Low~~ **Addressed** | search | Snippet delimiters `[`/`]` collide with literal brackets |
| A7 | Low | search | Participant filter is case-sensitive; empty query silently returns 0 |
| A8 | Medium (when the UI lands) | ui/security | Rendering a snippet as HTML is an injection vector — *added after the original review* |
| D1 | ~~Medium~~ **Addressed** | docs | `KNOWN_ISSUES.md` #2 describes a fix that is not the one implemented |
| D2 | Low | docs | README drift: trait signature, test count, "(re)builds" wording |
| D3 | Low | code | `db/models.rs` is currently dead code |
| P1–P4 | — | perf | Statement caching, participant lookup caching, count-query duplication, parse-inside-transaction |
| T1–T4 | — | tests | Missing re-import assertions, ~~shared helpers~~ (T3), ~~property tests~~ (T4) |
| I1–I3 | — | infra | ~~No Cargo workspace~~ (I1 done), no CI, fmt drift |

---

## 3. Correctness findings

### C1 — `message_count` inflates on every re-import — **High** (acknowledged)

`upsert_conversation` (`core/src/db/queries.rs:11-28`) accumulates
unconditionally on conflict:

```sql
ON CONFLICT (title, thread_path) DO UPDATE SET
    ...
    message_count = message_count + excluded.message_count
```

This is the right accumulation rule *within* one import (a conversation split
across `message_1.json`, `message_2.json`, …), but it has no idempotence
story: on a second import of the same export, every file adds its full count
again while the actual `messages` rows are deduped by `idx_messages_dedup`.

**[verified]** Importing a 2-message conversation twice yields
`conversations.message_count = 4` with `count(*) FROM messages = 2`.

The existing reload tests (`loader.rs`
`load_conversation_is_idempotent_on_reload`, `tests/ingestion.rs`
`re_importing_a_conversation_does_not_duplicate_its_attachments`) assert
`count(*)` on `messages`/`attachments` but never re-check the stored
`message_count` column — which is exactly the value that goes wrong.

**Recommendation.** Stop trying to maintain the running total incrementally.
Two clean options:

1. Derive it: after `load_conversation` finishes its loop (still inside the
   transaction), issue one
   `UPDATE conversations SET message_count = (SELECT count(*) FROM messages WHERE conversation_id = ?1)`.
   This is self-healing for every past and future dedup interaction.
2. Or count what was *actually inserted*: `insert_message` already returns
   `Option<i64>`; sum the `Some`s in `load_messages` and add only that. This
   preserves incrementality but stays coupled to the dedup behavior.

Option 1 is simpler and immune to drift; the per-conversation `count(*)` is
served by the `idx_messages_dedup` prefix on `conversation_id`, so it's
cheap. Either way, add a test asserting `message_count` (not just row
counts) after a double `import_export`.

### C2 — `populate_fts` appends duplicate rowids on re-import — **High** (acknowledged)

`populate_fts` (`core/src/db/schema.rs:119-126`) runs a plain

```sql
INSERT INTO messages_fts(rowid, content)
SELECT id, content FROM messages WHERE content IS NOT NULL AND content != ''
```

after *every* import (`core/src/ingest/mod.rs:21`). On a second import it
re-inserts every already-indexed rowid. `KNOWN_ISSUES.md` #9 flags this as
"untested whether SQLite accepts, errors, or silently duplicates." The probe
answered that question, and the answer is subtler than any of those three:

**[verified]** on the bundled SQLite 3.53.2:

- After a double import, `MATCH` counts, phrase queries, `fts5vocab` term
  statistics (`coffee: 2 docs, 2 occurrences`), and `messages_fts_docsize`
  row counts are all **still correct**. Identical postings from the repeated
  insert collapse at query/merge time, so today's re-import *happens* to
  search correctly.
- But this is luck, not a contract. A raw probe on an external-content table
  inserted rowid 1 as `'apple'`, then re-inserted the same rowid as
  `'zebra'`: afterwards **both** `MATCH 'apple'` and `MATCH 'zebra'` return
  the row — the stale posting survives — and
  `INSERT INTO t_fts(t_fts) VALUES('integrity-check')` **still passes**, so
  the corruption is undetectable by FTS5's own checker.

Within the current schema the mismatched-content case can't happen
(`messages.id` is `AUTOINCREMENT`, so rowids are never reused, and a row's
content is never updated) — which is exactly why the double import looks
fine. The moment any future feature deletes or edits a message without
FTS-sync triggers, this becomes silent, checker-proof corruption. Meanwhile
each re-import also grows the index's segment data with redundant postings.

**Recommendation.** Replace the append with the canonical external-content
rebuild command:

```sql
INSERT INTO messages_fts(messages_fts) VALUES('rebuild');
```

It drops and rebuilds the whole index from the `messages` content table, is
idempotent by construction, fixes re-import, and even repairs an index left
stale by a previously failed import (today, a partial import commits
conversations but never indexes them; the current append-based retry only
works because of the same undocumented collapse behavior). For a
messages-scale table the rebuild cost is a full scan — the same order of
work `populate_fts` already does. If incremental indexing is ever needed,
the supported route is `content=''`-style delete commands or triggers, not
re-INSERTs.

### C3 — `NULL` title/thread_path defeats the conversation upsert — **Medium** ✅ **Addressed**

> **Resolution (2026-08-05).** Neither recommended option; the author's call
> was to assume the fields are never null and make that assumption enforced
> rather than hoped for, at both layers:
>
> - `RawConversationFile.title` and `.thread_path` are now `String`, not
>   `Option<String>`. A file missing either is refused by serde with an
>   `Error::Parse` naming the file — the degenerate input can no longer reach
>   the storage layer at all, which is the gap this finding describes ("the
>   pipeline admits inputs the storage layer silently splits").
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
> positive case the finding is really about). T2's suggested test — two
> no-metadata files producing one conversation — is moot, since such files
> are now rejected outright. Suite: 151 passing, clippy clean.
>
> Trade-off worth naming: a real export with one conversation missing a title
> now fails the whole import instead of silently splitting that thread. The
> review's evidence says real exports always populate both fields; if one
> ever doesn't, the fallback-to-folder-name option below is the way out.

`conversations` is keyed by `UNIQUE (title, thread_path)`
(`core/src/db/schema.rs:14`) and `upsert_conversation` conflicts on that pair
(`core/src/db/queries.rs:16`). In SQLite, `NULL`s are distinct in a unique
index, so `ON CONFLICT` **never fires** when either field is `NULL` — every
file inserts a brand-new conversation row.

**[verified]** Two `message_N.json` files in the same conversation folder,
both lacking `title`/`thread_path` (legal per `RawConversationFile`, both
fields are `Option` — `core/src/ingest/parse.rs:58-60`), produce **two**
`conversations` rows on one import.

`KNOWN_ISSUES.md` #5 gets this wrong: it says two files must agree on the
fields "including both being present or both absent" to merge. Both-absent
never merges. The existing test coverage
(`parse_conversation_file_treats_missing_optional_fields_as_none`) proves the
parser accepts such files, so the pipeline admits inputs the storage layer
silently splits.

**Recommendation.** Pick one:

- Make the key total: since `scan` already hands `load_conversation` the
  conversation folder (`ConversationDir::folder`), fall back to the folder
  name when `thread_path` is `NULL` (and `''` for `title`), storing the
  fallback rather than `NULL`. Real exports always populate both fields, so
  this only changes behavior for the degenerate case.
- Or mirror the messages fix: a
  `CREATE UNIQUE INDEX ... ON conversations (COALESCE(title,''), COALESCE(thread_path,''))`
  with a matching expression conflict target.

Either way, correct KNOWN_ISSUES #5 and add a two-files-no-metadata test.

### C4 — Reactions parsed but dropped — **Medium** (acknowledged)

`RawReaction` is fully parsed and even mojibake-repaired
(`core/src/ingest/parse.rs:80-83, 177-181`), the `reactions` table exists
(`core/src/db/schema.rs:60-67`), and `Reaction`/`from_row` exist in
`models.rs` — but no insert query exists and `load_messages` ignores
`message.reactions`. All of that work is currently thrown away per import.

Two design notes for when this is implemented, both cheaper to decide now:

1. **Dedup:** `reactions` has no unique constraint. If reactions follow the
   attachments pattern ("insert only for a newly inserted message",
   `core/src/db/queries.rs:113-117`), re-import safety inherits from the
   message dedup — but unlike attachments, reactions genuinely *change over
   time* on old messages (someone reacts to a year-old message between two
   exports). The attachments pattern will silently miss those. A
   delete-and-reinsert per message, or a
   `UNIQUE (message_id, actor_id, reaction)` index with `INSERT OR IGNORE`,
   handles both.
2. **Actor resolution:** the reaction `actor` is a display name; it should go
   through the same `find_or_create_participant` path a sender does (see
   A2), or a reactor who never sent a message will either fail the
   `actor_id NOT NULL` FK or force a dangling participant.

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
> Suite: 142 passing, clippy clean.

`FtsIndex::search` orders by exactly one expression
(`core/src/search/fts.rs:41-45`): `bm25(...)`, `timestamp_ms DESC`, or
`timestamp_ms ASC`. None has a tiebreaker, and ties are not exotic here:

- Same-millisecond messages are a first-class scenario in this codebase —
  the dedup index went through two revisions specifically to keep
  same-`timestamp_ms` messages apart.
- bm25 ties are routine for short chat messages; the probe observed the
  degenerate case where every hit scored an identical `-1e-6`.

Because each page is a separate query (and a separate transaction —
pagination calls `search` once per page), SQLite is free to order tied rows
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
> with seven existing loader tests. KNOWN_ISSUES #3 updated to match. Suite:
> 147 passing, clippy clean.
>
> Not done, deliberately: the per-conversation `HashMap` cache from P2. It's
> a performance change, and the find-or-create call is now the single place
> to add it when that's measured.

`insert_participant` (`core/src/db/queries.rs:41-61`) finds an existing
participant *only via the `conversation_participants` join*. If a caller
inserts but forgets to immediately call `link_conversation_participant`, the
next lookup for the same name misses and inserts another row — the exact
failure KNOWN_ISSUES #3 describes for message senders. Nothing in the API
enforces the pairing; `loader.rs` just repeats the two-call sequence
correctly in two places (`loader.rs:20-22` and `loader.rs:47-50`).

There is also no database backstop: `participants` has no unique constraint
that could catch a violation of this invariant (deliberately — the same name
may repeat across conversations — but that means the invariant lives
entirely in caller discipline).

**Recommendation:** fold the link into the function — it already takes
`conversation_id`. A single
`find_or_create_participant(conn, conversation_id, name) -> Result<i64>`
that finds, or inserts *and links*, removes the trap and both duplicated
call sites. (This is the type-driven "make invalid states unrepresentable"
move at API scale.)

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
> `insert_message_reports_a_duplicate_as_none_rather_than_an_error` (the
> path that must keep working) and
> `insert_message_errors_on_a_constraint_violation_that_is_not_a_duplicate`,
> which adds a throwaway unique index to stand in for the future constraint
> this finding is about, then shows the violation now errors — confirmed to
> fail against `INSERT OR IGNORE`, where the row was silently dropped.
> Suite: 153 passing, clippy clean.

`insert_message` (`core/src/db/queries.rs:93-105`) uses `INSERT OR IGNORE`
and interprets `changes() == 0` as "duplicate". `OR IGNORE` also silently
drops rows that fail *any* non-FK constraint (`NOT NULL`, `CHECK`), so a
future schema constraint could turn a coding bug into silently missing
messages instead of an error. `ON CONFLICT (conversation_id, ...) DO NOTHING`
targeting `idx_messages_dedup` scopes the ignore to exactly the dedup index
and errors on everything else. Cheap insurance.

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
> `migrate_rejects_a_negative_user_version_instead_of_panicking`
> (`core/src/db/schema.rs`). Suite: 148 passing, clippy clean.

`migrate` (`core/src/db/schema.rs:107-111`) indexes
`MIGRATIONS[version as usize]` over `current_version..LATEST_VERSION`. A
hand-tampered database with a negative `user_version` (the pragma is signed)
makes `version as usize` wrap and the indexing panic. Guard
`current_version < 0` with the same `UnsupportedSchemaVersion`-style refusal
(or a dedicated variant) so a corrupt file produces an `Err`, not a panic —
consistent with the crate's own no-panic policy.

### A5 — `count` and `scan` disagree about symlinks — **Low** ✅ **Addressed**

> **Resolution (2026-08-05).** Aligned on *not* following symlinks, which is
> what `scan_inbox` and `find_messages_root` already did. `count_inbox` now
> filters on `DirEntry::file_type()` (reports the link itself) instead of
> `Path::is_dir()` (resolves it), and `message_files_in` likewise switched
> from `Path::is_file()` to `DirEntry::file_type()` so a symlinked
> `message_N.json` is skipped too — the same policy applied consistently
> rather than only at the conversation-folder level. The rationale is
> recorded on `scan_inbox`: an export is a self-contained tree, so a link
> inside it points somewhere the user didn't ask to import, and following
> links would let a cycle turn the walk infinite. The export root itself is
> exempt — the caller named it explicitly, so `validate_root` still resolves
> it. Pinned by three new unit tests in `core/src/ingest/scan.rs` (all
> `#[cfg(unix)]`): `count_and_scan_both_skip_a_symlinked_conversation_folder`
> (the finding's exact scenario, asserting the two passes agree),
> `scan_skips_symlinked_message_files`, and
> `find_messages_root_does_not_follow_a_symlink_to_the_messages_directory`.
> The first two were confirmed to fail against the pre-fix checks. Suite: 145
> passing, clippy clean.

`count_inbox` uses `entry.path().is_dir()` (`core/src/ingest/scan.rs:43`),
which follows symlinks; `scan_inbox` uses `entry.file_type().is_dir()`
(`scan.rs:72`) on a non-following `WalkDir`, which doesn't. A symlinked
conversation folder is counted by the cheap pass but skipped by the real
one, so a progress UI built on `count` would never reach 100%. Align the two
checks (and decide explicitly whether symlinked conversation dirs are in or
out).

### A6 — Snippet delimiters collide with message text — **Low** ✅ **Addressed**

> **Resolution (2026-08-05).** Switched to the suggested interlinear
> annotation pair, exposed as `search::MATCH_START` (`U+FFF9`) and
> `search::MATCH_END` (`U+FFFB`). They live in `search/mod.rs`, not
> `search/fts.rs`: how a match is marked is part of what `SearchIndex`
> promises about `SearchHit.snippet`, not an FTS5 detail, and a consumer has
> to name them to translate them — which it must do without reaching past
> the trait.
>
> `SearchHit::snippet` now documents the contract, including that the
> surrounding text is sender-controlled and needs escaping (see A8).
>
> Pinned by `match_markers_are_distinguishable_from_punctuation_in_the_message`
> (`core/tests/search.rs`), which searches "coffee" in `I read [1] and the
> coffee was fine` and asserts the full snippet — the message's own brackets
> come through untouched while only the match is wrapped. Under the old
> markers that snippet was `I read [1] and the [coffee] was fine`, with "1"
> indistinguishable from a highlight. Suite: 154 passing, clippy clean.
>
> Left alone deliberately: the `'...'` ellipsis argument has the same in-band
> ambiguity (a message can trail off with "..."), but a consumer has no
> reason to distinguish "clipped here" from typed ellipsis, so it isn't worth
> a sentinel. Also not done: stripping `U+FFF9`/`U+FFFB` from `content` at
> ingest, which is what would make collisions impossible rather than merely
> implausible. That changes stored data for a case that doesn't arise in
> exported chat text.

`snippet(messages_fts, 0, '[', ']', '...', 8)` (`core/src/search/fts.rs:69`)
marks matches with characters that legitimately occur in chat text, so a UI
can't reliably distinguish a highlight from a literal bracket (and can't
safely transform the markers into markup). Use improbable sentinels (e.g.
the Unicode interlinear annotation pair `\u{FFF9}`/`\u{FFFB}`, or a private
-use pair) and translate at the UI boundary.

### A7 — Small search-behavior notes — **Low**

- **Empty query:** `run("")` (or all-whitespace, after the trim) becomes the
  FTS phrase `""`, which **[verified]** returns `Ok` with 0 hits. Reasonable,
  but it's an accident of FTS5 phrase semantics; consider short-circuiting
  empty text explicitly so the behavior is chosen, not inherited.
- **Trim location:** the trim lives in `search::run`'s `build_query`
  (`core/src/search/mod.rs:69-75`), not in `FtsIndex::search`. Any future
  caller constructing a `SearchQuery` directly (as every test does) silently
  gets phrase-with-whitespace semantics. Trimming inside `search` (or in a
  `SearchQuery` constructor) puts the invariant where it's enforced.
- **Case sensitivity:** the participant filter is `p.name = ?3`
  (`fts.rs:22`) — exact, case-sensitive match. Fine for a filter fed by a
  picker; wrong for a filter fed by a text box. Worth a `COLLATE NOCASE`
  decision when the UI lands.

### A8 — Rendering a snippet as HTML is an injection vector — **Medium (when the UI lands)**

*Added 2026-08-05, after the original review — surfaced while fixing A6, not
a finding of the review itself.*

`SearchHit.snippet` (`core/src/search/mod.rs`) is message text the sender
wrote, carrying `MATCH_START`/`MATCH_END` markers the UI is expected to turn
into markup. The obvious implementation of that step is also the wrong one:

```svelte
<!-- unsafe -->
{@html hit.snippet.replaceAll(MATCH_START, '<mark>').replaceAll(MATCH_END, '</mark>')}
```

Any message whose text contains markup is then interpreted as markup inside
the webview. A sender who wrote `<img src=x onerror=...>` — or any of the
usual payloads — gets script execution in a Tauri window, which is a more
serious boundary than a browser tab: the same context reaches the Tauri IPC
bridge and whatever commands are exposed on it. The attacker doesn't need to
compromise anything; they just need to have sent the victim a message years
ago, and the victim to search for a word in it.

Nothing is wrong today — nothing renders snippets yet, which is why this is
scoped to "when the UI lands" — but the bug is easy to write and easy to
miss in review, because the marker-to-markup replacement *looks* like the
whole job.

**The rule:** escape first, then substitute. HTML-escape the entire snippet,
and only afterwards replace the (now unescaped-by-construction) sentinels
with `<mark>`. Equivalently, and better: split the snippet on the sentinels
and let the framework render the pieces as text nodes, so no raw-HTML
insertion happens at all —

```svelte
{#each splitOnMarkers(hit.snippet) as part}{#if part.matched}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}
```

This is why the sentinels are non-ASCII format characters rather than
markup-adjacent punctuation: the split is unambiguous even when the message
contains angle brackets, quotes, or entities.

Note the ordering matters in one direction only. Escaping first is safe
because escaping can't produce a sentinel; substituting first is not,
because the escape pass would then mangle the `<mark>` tags it just
inserted, and a careless "escape everything except tags" pass reopens the
hole.

Worth deciding at the same time: whether `conversation_title` and
`sender_name` get the same treatment. Both are also export-derived,
sender-influenced strings, and both will be rendered next to the snippet.

---

## 5. Documentation drift

### D1 — `KNOWN_ISSUES.md` #2 describes a fix that isn't the implemented one — **Medium** ✅ **Addressed**

> **Resolution (2026-08-05).** Entry #2 deleted rather than corrected: it
> documented an already-fixed issue, and the design it was describing badly
> is stated accurately where it belongs — the NULL-content rationale lives on
> the `messages.content` column, `insert_message`, and the
> `duplicate_messages_with_null_content_are_rejected` test. A rewritten entry
> would only have restated those, with the same drift risk that made it wrong
> in the first place.
>
> The number is retired, not reused: `README.md`, `core/src/search/fts.rs`,
> and `core/tests/ingestion.rs` all cite entries by number, so renumbering
> would have silently broken them (and this review's own citations of #3, #5,
> #9, #10). The file's intro now records that numbering rule, so the gap
> doesn't read as an accident.

KNOWN_ISSUES #2 states: "`content` is now `TEXT NOT NULL DEFAULT ''` …
`insert_message` stores a missing message body as `''` instead of `NULL`."
The code does the opposite: `content TEXT` is nullable
(`core/src/db/schema.rs:35`), `insert_message` binds
`message.content.as_deref()` and stores `NULL`
(`core/src/db/queries.rs:101`), `models::Message` documents "stored as
`NULL`" (`core/src/db/models.rs:61-63`), and the dedup index handles the
NULL case with `COALESCE(content, '')` (`schema.rs:46`) — the same route
taken for `sender_id`. The tests
(`duplicate_messages_with_null_content_are_rejected`,
`populate_fts_skips_empty_content`) pin the NULL-based behavior, so the code
is consistent with itself; only the document is wrong. Since this file
functions as the project's decision record, a wrong "fixed" entry is worth
correcting promptly. (#5 also needs the C3 correction above.)

### D2 — README drift — **Low**

- The `SearchIndex` snippet (README "Search" section) shows
  `-> rusqlite::Result<SearchResults>`; the real trait returns the crate's
  own `Result` (`core/src/search/mod.rs:65-67`) — a meaningful difference,
  since not leaking backend error types is one of the crate's stated design
  points.
- "built and tested (94 tests)" — the suite is at 139 and this number will
  keep rotting; consider dropping the count or wording it loosely.
- `import_export`'s doc comment says it "(re)builds the full-text search
  index" (`core/src/ingest/mod.rs:12-15`); it appends (see C2). If the C2
  `rebuild` fix lands, the comment becomes true for free.

### D3 — `db/models.rs` is dead code — **Low**

Nothing outside `models.rs` references any of its types — no production
caller, no test outside its own module (checked by grep). These are clearly
the intended Tauri-boundary DTOs, so keeping them is fine, but until a
consumer exists they're untested-against-need API surface (e.g. nothing
guarantees `SearchHit` vs `Message` serialization stays coherent for the
UI). Either wire them up with the first Tauri command or mark the module
with a short comment stating its intended consumer, so it isn't mistaken
for abandoned code.

---

## 6. Performance (unmeasured — headroom, not observed bottlenecks)

Real exports reach hundreds of thousands of messages; none of these findings
were benchmarked, but all sit directly on the per-message hot path.

- **P1 — Statement re-preparation.** Every insert in `queries.rs` goes
  through `conn.execute`/`conn.query_row` with a SQL string, so SQLite
  re-parses each statement per message (`insert_message`, the three
  statements in `find_or_create_participant`, `insert_attachments`).
  rusqlite's `prepare_cached` is a drop-in change and
  typically a large constant-factor win on bulk ingest. (Same for the two
  statements in `FtsIndex::search`, though that path is per-keystroke, not
  per-million-rows.)
- **P2 — Per-message participant lookup.** `load_messages` runs the
  find-participant SELECT for *every* message (`loader.rs:46-51`). A
  `HashMap<String, i64>` per conversation in the loader (participants per
  conversation are few) removes ~one query per message and, combined with
  A2, centralizes the identity logic.
- **P3 — Double MATCH per search.** The count query and the page query each
  run the FTS match (`fts.rs:55-77`). A single query with
  `count(*) OVER ()` alongside the page columns halves the FTS work and
  removes the need for the snapshot transaction. Optional — the current
  two-query-in-one-transaction design is correct and clearer; do this only
  if search latency on a large index warrants it.
- **P4 — Parse inside the write transaction.** `load_conversation` opens the
  transaction before parsing any file (`loader.rs:12-15`), so JSON
  parse time (potentially tens of MB per conversation) is spent holding the
  write lock. Irrelevant while the app is single-connection; becomes real
  contention the moment a UI reads while an import runs. Parsing each file
  before `tx` starts (or all files before the loop) is a small reorder.

Genuinely fine as-is: WAL + `synchronous=NORMAL` for ingest; one transaction
per conversation (good batching granularity with a documented partial-import
semantics and a test pinning it); the dedup-index prefix doubling as the
`conversation_id` lookup index; bundled SQLite for reproducibility.

---

## 7. Tests

The suite is a strength: behavior-revealing names throughout, error paths
exercised (missing file, malformed JSON, permission-denied with a
restore-on-drop guard and a root-user escape hatch), integration tests on a
real on-disk DB, and FTS assertions that correctly route around
external-content read-through semantics (`messages_fts_docsize` counting —
a detail many teams get wrong). Tests document *why* fixtures are shaped as
they are.

Gaps, in priority order:

- **T1 — Re-import invariants** (would have caught C1): after two
  `import_export` runs, assert `conversations.message_count` equals
  `count(*) FROM messages` per conversation, and (post-C2) that the FTS
  index passes `integrity-check` with docsize == indexed-message count.
- ~~**T2 — NULL-metadata conversation** (C3): two files in one folder with no
  `title`/`thread_path` must produce one conversation.~~ Moot — such files
  are now rejected at parse time (see C3), and the rejection is tested.
- **T3 — Shared test helpers** ✅ **Addressed (2026-08-05):**
  `write_file`, `migrated_connection`, and the
  86-line `make_unreadable` guard are duplicated across four-plus files
  (`schema.rs`, `loader.rs`, `mod.rs`, `scan.rs` tests, and both integration
  files). A `tests/common/mod.rs` (integration) plus a `#[cfg(test)]`
  `test_util` module (unit) removes the drift risk — the two
  `make_unreadable` copies are already one refactor away from diverging.

  > **Resolution.** Both modules created as suggested. Consolidated: 5
  > copies of `write_file`, 4 of `migrated_connection`, 2 of
  > `make_unreadable`, and `open_db` (which `search.rs` had open-coded
  > inline twice instead of copying). Helpers used by a single file —
  > `table_names`, `conversation_dir`, `fts.rs`'s raw-SQL seeders — stayed
  > put; moving those would trade duplication for indirection.
  >
  > The two `make_unreadable` copies had *already* drifted: their doc
  > comments no longer matched. Both call sites also carried an identical
  > copy of the run-as-root escape hatch, so the shared version folds that in
  > and returns `Option<impl Drop>` — `None` meaning "mode 000 didn't take,
  > skip". A `let Some(_guard) = … else { return }` replaces the
  > guard-then-recheck pair. Verified those tests don't now skip silently by
  > making the else-branch panic: it isn't taken, so the permission-denied
  > paths still run for real.
  >
  > The remaining duplication between `src/test_util.rs` and
  > `tests/common/mod.rs` is structural and documented in both: `test_util`
  > is `cfg(test)` inside the library, and each integration test is its own
  > crate linking the published library, so there is nothing for it to
  > import. Suite: 154 passing, clippy clean.
- **T4 — Property test for `repair_mojibake`** ✅ **Addressed (2026-08-05):**
  the function is a perfect
  proptest target: for any `s: String`, encoding `s` as UTF-8 bytes and
  mapping each byte to its Latin-1 char must repair back to exactly `s`; and
  `repair_mojibake` must be idempotent on its own output. This turns
  KNOWN_ISSUES #10's "low practical risk" reasoning into a checked
  invariant.

  > **Resolution.** `proptest` added as a dev-dependency, with three
  > properties in `core/src/ingest/parse.rs`:
  >
  > - `repair_mojibake_undoes_the_corruption_for_any_text` — the round-trip
  >   invariant exactly as described. This is the one that matters: it turns
  >   "it always undoes the bug" from an argument into a check over every
  >   input, rather than the two hand-picked examples that were there.
  > - `repair_mojibake_leaves_any_text_with_a_non_latin1_char_alone` —
  >   pins the first escape hatch, over an arbitrary non-Latin-1 char in an
  >   arbitrary position.
  > - `repair_mojibake_leaves_any_ascii_text_alone` — pins the fast path.
  >
  > Verified the properties have teeth by sabotaging the implementation
  > twice: truncating with `c as u8` instead of bailing out on a
  > non-Latin-1 char, and refusing to decode past four bytes. Both were
  > caught, the second shrunk to the minimal input `"𞹤0"`. The
  > sabotage-generated `proptest-regressions/` seeds were deleted rather
  > than committed — they record artificial failures, not real ones.
  >
  > **Correction to this item:** the second suggested property is false.
  > `repair_mojibake` is *not* idempotent on its own output, and can't be —
  > it peels exactly one layer of the corruption, so feeding its output back
  > in peels another. Concretely, `repair("Ã\u{83}Â©") == "Ã©"`, but
  > `repair("Ã©") == "é"`. That isn't a bug: it's the same accepted risk
  > KNOWN_ISSUES #10 already records, since `"Ã©"` is indistinguishable by
  > inspection from a corrupted `"é"`, and the function's contract is that
  > its input is corrupted exactly once. Asserting idempotence would have
  > pinned behavior the function doesn't have. The real behavior is pinned
  > instead by `repair_mojibake_peels_exactly_one_layer_of_corruption`, and
  > KNOWN_ISSUES #10 now cites it as a concrete instance of its residual
  > risk. Suite: 158 passing, clippy clean.

---

## 8. Style and idiom

Measured against the Apollo Rust handbook conventions the project uses:

- **Conforms well:** no production `unwrap`/`expect`; `thiserror` with
  `#[from]` and contextual variants (`ReadFile`/`Parse` carrying the path);
  `?` propagation throughout; `let`/`else` and `.optional()` where
  appropriate; comments explain *why* (the `unchecked_transaction`
  justification in `fts.rs:48-52` and the FTS5 read-through comments are
  model examples); imports grouped std / external / crate; `Copy` derived
  on the small `MessageType`/`SortOrder` enums and not on the String-bearing
  structs.
- **Error context nit:** `Error::Io(#[from] io::Error)` is transparent, so a
  walk error surfaced from `scan_inbox`
  (`core/src/ingest/scan.rs:69`) loses the path that `walkdir` knew
  (`io::Error::from(err)` discards it). The crate already has the right
  pattern (`ReadFile { path, source }`); a `Scan { path, source }` variant
  would make mid-walk failures as diagnosable as parse failures.
- **`LATEST_VERSION` drift risk:** ✅ **Addressed (2026-08-05)** —
  `pub const LATEST_VERSION: i32 = 1;` must be bumped in lockstep with
  `MIGRATIONS`. It can be derived:
  `pub const LATEST_VERSION: i32 = MIGRATIONS.len() as i32;` (`<[T]>::len`
  is const), eliminating a whole class of forgot-to-bump bugs.

  > **Resolution.** Derived exactly as suggested, so adding a migration is a
  > one-place change. Both constants also picked up doc comments recording
  > the contract: index `n` takes a database from `user_version` `n` to
  > `n + 1`, and entries are append-only — never edit or reorder one a
  > shipped database has already run.
  >
  > Verified by temporarily appending a second migration step (probe written,
  > run, then reverted): `LATEST_VERSION` became `2` with no other edit,
  > `migrate` applied both steps, and the resulting `user_version` was `2`.
  > That also exercises the migration loop's multi-step path for the first
  > time — it has only ever run with a single entry. The only test that
  > failed under the probe was `migrate_creates_all_tables`, precisely
  > because the second migration's table existed.
  >
  > A permanent multi-step test would need `migrate` to take the migration
  > list as a parameter instead of reading the global; not done, as that's a
  > wider change than this item calls for.
- **Formatting:** `cargo fmt --check` fails on three test-file spots (long
  hand-aligned tuple rows and one import line). Either run `cargo fmt` and
  accept its output, or add a `rustfmt.toml` codifying the intended
  exceptions — a check that's *almost* clean is one nobody can put in CI.

---

## 9. Tauri / frontend (scaffold)

`src-tauri/src/lib.rs` is the untouched template (`greet` command);
`src/routes/+page.svelte` likewise. Nothing to review yet beyond flagging,
for the wiring step:

- Keep the app layer behind the `SearchIndex` trait / `search::run`
  composition root — the boundary is already designed for exactly this; the
  Tauri command should never see FTS5 SQL.
- `rusqlite::Connection` is `!Sync`; under Tauri it will need to live in
  managed state behind a `Mutex` (or a dedicated DB thread/channel).
  `import_export` is long-running and must not run on the main thread —
  use an async command with `spawn_blocking`, and `scan::count_inbox` +
  `find_messages_root` already exist to drive a progress UI (mind A5).
- `grepm_core::Error` will need a serializable mapping for command results
  (Tauri requires `Serialize` errors); the structured error enum makes that
  straightforward — resist flattening to `String` at the boundary.
- Snippet rendering is the one step in the wiring with a security
  consequence: escape before substituting the match markers, or skip raw-HTML
  insertion entirely. See A8.

---

## 10. Infrastructure

- **I1 — No Cargo workspace** ✅ **Addressed (2026-08-05).** There is no root
  `Cargo.toml`; `core/` and
  `src-tauri/` build independently with **two separate lockfiles**, so they
  can silently resolve different dependency versions (both depend on
  `serde`/`serde_json`, and `src-tauri` depends on `core` by path). A
  two-line root `[workspace]` manifest unifies the lockfile and enables
  shared `[workspace.lints]` (e.g. `clippy::perf`, `redundant_clone`) in one
  place.

  > **Resolution.** Root `Cargo.toml` with both crates as members and
  > `resolver = "2"`. The two lockfiles are replaced by one at the root (482
  > packages, covering `core`'s dev-dependencies too), and `/target` moved
  > with it.
  >
  > `serde`/`serde_json` are declared once in `[workspace.dependencies]` and
  > referenced as `serde.workspace = true`, so the requirements can't drift
  > at the manifest level either — the shared lockfile only guarantees one
  > *resolved* version for a given requirement. Checked before the change:
  > the two lockfiles happened to agree on both crates at the time
  > (`serde` 1.0.229, `serde_json` 1.0.151), so the risk was live but not
  > yet realized.
  >
  > `[workspace.lints]` set deliberately small, and both members opt in with
  > `[lints] workspace = true`: `unsafe_code = "deny"` (neither crate needs
  > any) and `clippy::redundant_clone = "warn"`. `clippy::perf` was not
  > added — it's already warn-by-default, so naming it would imply a change
  > that isn't one. `redundant_clone` found a real instance on its first
  > run, a needless `Vec` clone in `fts.rs`'s paging test, now fixed.
  >
  > Verified `cargo check --workspace`, `cargo clippy --workspace
  > --all-targets` (clean), and `cargo test --workspace` (158 passing) —
  > including `src-tauri`, which builds here. The engine-only workflow
  > (`cd core && cargo test`) still works and is still worth keeping, since
  > it skips the Tauri dependency tree; the README now documents both.
  >
  > The now-orphaned `core/target` and `src-tauri/target` were removed on the
  > author's go-ahead — cargo builds into the workspace root instead, so
  > together they were 9 GB of dead cache. The members' `.gitignore` entries
  > for `/target/` were left in place; they cost nothing and still apply if a
  > crate is ever built outside the workspace.
- **I2 — No CI.** There is no `.github/` (or equivalent). The suite is fast
  (~0.5 s) and clippy is already clean — this project is one small workflow
  away from enforcing `fmt --check` + `clippy -D warnings` + `cargo test`,
  and findings like the fmt drift (I3) only stay fixed with a gate.
- **I3 — fmt drift** — see §8; blocked on deciding intent, then trivially
  enforced by I2.

---

## 11. What's done well

Called out explicitly, because most of this review's length is complaints
about a codebase that is above average:

- **Mojibake repair** (`parse.rs:37-51`) is the correct inverse transform,
  applied with two defensive escape hatches, an ASCII fast path, a
  pre-sized buffer, and honest documentation of its one residual false
  -positive risk (KNOWN_ISSUES #10).
- **The message dedup key** (`schema.rs:42-49`) shows real iteration:
  NULL-safe via `COALESCE`, extended by `type` and `attachment_count` for
  same-millisecond attachment batches, each extension pinned by a test
  explaining the collision it prevents.
- **External-content FTS5** with the diacritic-folding tokenizer, literal
  -phrase query escaping (with tests proving `OR` is not an operator and a
  stray `"` doesn't break the query), and token-exactness asserted as a
  feature.
- **Migration engine**: versioned via `user_version`, transactional per
  step, rollback behavior pinned by a genuinely clever sabotage test
  (`schema.rs:246-282`), and newer-schema refusal instead of silent
  misbehavior.
- **Participant identity scoped per conversation** — a correctness-first
  call with the trade-off documented in three places that agree with each
  other (code doc, KNOWN_ISSUES, README).
- **`Page` fields are `u32`** specifically so a negative limit dies at
  deserialization instead of becoming SQL's "no limit" — with tests.

---

## 12. Prioritized recommendations

1. **Fix re-import** (C1 + C2): recompute `message_count` from `messages`
   inside `load_conversation`, and switch `populate_fts` to FTS5's
   `'rebuild'` command. Add the T1 invariants test. This closes the known
   "re-imports are broken" issue end to end.
2. ~~**Close the NULL-key upsert hole** (C3) and correct KNOWN_ISSUES #5 and
   #2 (D1) while in there.~~ **Done** — see C3 and D1.
3. ~~**Add ordering tiebreakers** (A1) — one line, prevents user-visible
   pagination glitches the moment the UI exists.~~ **Done** — see A1.
4. ~~**Merge participant find/create/link into one API** (A2)~~ **Done** —
   see A2; the per-conversation cache (P2) is still open.
5. **Persist reactions** (C4) with a dedup story that tolerates reactions
   added to old messages.
6. **Set up ~~workspace~~ (done) + CI** (I2), run `cargo fmt` (I3), switch
   hot-path queries to `prepare_cached` (P1).
7. Sweep the small items (~~A3~~, ~~A4~~, ~~A5~~, ~~A6~~, ~~T3~~, ~~T4~~
   (all done), A7, D2, D3) opportunistically.
8. Before the UI renders a single snippet, settle the escaping story (A8).
   It's the one item here that turns into a vulnerability rather than a bug.
