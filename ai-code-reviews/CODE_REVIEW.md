# Code Review — grepm

**Date:** 2026-08-05
**Scope:** `core/` (grepm_core), `src-tauri/`, project layout. The SvelteKit
frontend and Tauri shell are still the starter scaffold and are reviewed only
as such.

**Acknowledged up front (per the author):** reactions are not persisted yet,
and re-imports / further imports into the same database are known-broken.
Those areas are still covered below, because the review verified *specific
mechanisms* of the breakage that are worth having on record — but they are
marked as acknowledged.

Every finding below is **open**. Resolved findings, with the resolution
blocks describing what changed, have moved to `CODE_REVIEW-addressed.md`;
their IDs are not reused.

---

## 1. Method

Everything labeled **[verified]** was demonstrated by running code, not by
reading it. Against the tree as reviewed:

| Check | Result |
|---|---|
| `cargo test --workspace` | **158 passed** (129 unit + 12 ingestion + 17 search), 0 failed |
| `cargo clippy --workspace --all-targets` | clean, 0 warnings |
| `cargo fmt --all --check` | clean |

Temporary probe tests (written, run, then deleted) exercised the re-import
failure modes against the real pipeline and against raw SQLite (bundled
version **3.53.2**). Probe results are cited inline as evidence.

---

## 2. Summary

`grepm_core` is a genuinely well-built crate. The error type is a proper
`thiserror` enum with `Send + Sync` pinned by a test, there are no
`unwrap`/`expect` calls outside test code, public functions are documented
with *why*-comments rather than *what*-comments, and the test suite is large,
behavior-named, and unusually sophisticated — it knows about FTS5 shadow
tables, read-through semantics of external-content tables, and
permission-denied edge cases. The hard domain problems — mojibake repair,
per-conversation participant identity, numeric file ordering, message dedup
keys — show careful, documented reasoning.

The problems cluster in one place: **the import pipeline is only correct for
a single import into a fresh database.** Running it twice, or importing a
newer export over an old database, inflates `conversations.message_count`
[verified] and leaves the full-text index detectably corrupt [verified].
Everything else is doc drift, API hardening, and performance headroom for
large exports.

**Finding index:**

| ID | Severity | Area | Title |
|---|---|---|---|
| C1 | High (acknowledged) | ingest/db | `message_count` inflates on every re-import |
| C2 | High (acknowledged) | ingest/fts | A second import corrupts the FTS index |
| C4 | Medium (acknowledged) | ingest | Reactions parsed but dropped; note on the future dedup story |
| C5 | Medium | ingest | Import is all-or-nothing per run, but partial per conversation |
| A8 | Medium (when the UI lands) | ui/security | Rendering a snippet as HTML is an injection vector |
| A7 | Low | search | Participant filter is case-sensitive; empty query silently returns 0 |
| D2 | Low | docs | README drift: trait signature, test count, "(re)builds" wording |
| D3 | Low | code | `db/models.rs` is currently dead code |
| K10 | Low (accepted) | ingest | `repair_mojibake` assumes exports never contain correct non-ASCII text |
| P1–P4 | — | perf | Statement caching, participant lookup caching, count-query duplication, parse-inside-transaction |
| T1–T2 | — | tests | Missing re-import assertions, import-failure recovery |

Twenty-seven further findings are resolved — see `CODE_REVIEW-addressed.md`.

**Provenance of the `K` findings.** `core/KNOWN_ISSUES.md` was folded into
these two documents and removed. Its entries keep their original numbers under
a `K` prefix, so an older citation of "KNOWN_ISSUES #3" resolves to K3. Three
of them were already covered here and were not duplicated:

| Was | Now |
|---|---|
| #1, #4, #5, #7 | K1, K4, K5, K7 — resolved, in `CODE_REVIEW-addressed.md` |
| #3 | K3 (the trade-off) — resolved; A2 covers the API shape that enforces it |
| #2 | retired when the entry was deleted (D1). The number is not reused |
| #6 — reactions dropped | **C4**, which also settles the dedup and actor-resolution questions #6 left open |
| #8 — migration rollback | **A9**, resolved 2026-07-24. The KNOWN_ISSUES entry had gone stale and still described the pre-fix behaviour |
| #9 — re-import | **C1 + C2**, which answer what #9 left open: FTS5 does not reject the repeated rowid, and the index *is* corrupt today — verified |
| #10 — mojibake residual risk | K10 below, open and accepted |

---

## 3. Correctness findings

### C1 — `message_count` inflates on every re-import — **High** (acknowledged)

`upsert_conversation` (`core/src/db/queries.rs`) accumulates unconditionally
on conflict:

```sql
ON CONFLICT (title, thread_path) DO UPDATE SET
    ...
    message_count = message_count + excluded.message_count
```

This is the right accumulation rule *within* one import — a conversation
split across `message_1.json`, `message_2.json`, … — but it has no
idempotence story. On a second import of the same export, every file adds its
full count again while the actual `messages` rows are deduped away by
`idx_messages_dedup`.

**[verified]** Importing a 2-message conversation twice yields
`conversations.message_count = 4` with `count(*) FROM messages = 2`.

The existing reload tests (`load_conversation_is_idempotent_on_reload`,
`re_importing_a_conversation_does_not_duplicate_its_attachments`) assert
`count(*)` on `messages`/`attachments` but never re-check the stored
`message_count` column — which is exactly the value that goes wrong. See T1.

**Recommendation.** Stop trying to maintain the running total incrementally.
Two clean options:

1. Derive it: after `load_conversation` finishes its loop, still inside the
   transaction, issue one
   `UPDATE conversations SET message_count = (SELECT count(*) FROM messages WHERE conversation_id = ?1)`.
   This is self-healing for every past and future dedup interaction.
2. Or count what was *actually inserted*: `insert_message` already returns
   `Option<i64>`; sum the `Some`s in `load_messages` and add only that. This
   preserves incrementality but stays coupled to the dedup behavior.

Option 1 is simpler and immune to drift; the per-conversation `count(*)` is
served by the `idx_messages_dedup` prefix on `conversation_id`, so it's
cheap. Either way, add a test asserting `message_count` — not just row counts
— after a double `import_export`.

### C2 — A second import corrupts the FTS index — **High** (acknowledged)

`populate_fts` (`core/src/db/schema.rs`) runs a plain append after *every*
import (`core/src/ingest/mod.rs`):

```sql
INSERT INTO messages_fts(rowid, content)
SELECT id, content FROM messages WHERE content IS NOT NULL AND content != ''
```

It selects **all** messages, not just newly inserted ones, so a second import
re-inserts every already-indexed rowid into the external-content table. FTS5
does not deduplicate those.

**[verified]** on the bundled SQLite 3.53.2, after a double import through
the real pipeline:

```
INSERT INTO messages_fts(messages_fts) VALUES('integrity-check')          => PASS
INSERT INTO messages_fts(messages_fts, rank) VALUES('integrity-check', 1) => FAIL
                            (DatabaseCorrupt 267, "database disk image is malformed")
```

The distinction matters and is easy to get wrong. The one-argument form only
checks the index's *internal* consistency, which survives a duplicate-rowid
append. The `rank = 1` form additionally cross-checks the index against its
content table — and that is what breaks. **The index is malformed today**, on
the current schema, after one repeat import.

What makes this dangerous is that everything else still looks healthy:
`MATCH` counts, phrase queries, `fts5vocab` term statistics, and
`messages_fts_docsize` row counts all remain correct after the double import,
because identical postings collapse at query time. A user — or a test that
only asserts hit counts — sees a working search over a corrupt index.

A raw probe on an external-content table also shows the failure mode a future
feature would hit: insert rowid 1 as `'apple'`, then re-insert the same rowid
as `'zebra'`, and afterwards **both** `MATCH 'apple'` and `MATCH 'zebra'`
return the row — the stale posting survives. Within the current schema this
mismatched-content case can't arise (`messages.id` is `AUTOINCREMENT`, so
rowids are never reused, and a row's content is never updated), but the
moment any feature deletes or edits a message without FTS-sync triggers, it
does.

**Recommendation.** Replace the append with the canonical external-content
rebuild command:

```sql
INSERT INTO messages_fts(messages_fts) VALUES('rebuild');
```

It drops and rebuilds the whole index from the `messages` content table, is
idempotent by construction, fixes re-import, and even repairs an index left
stale by a previously failed import — today a partial import commits
conversations but never indexes them (see C5). For a messages-scale table the
rebuild cost is a full scan, the same order of work `populate_fts` already
does. If incremental indexing is ever needed, the supported routes are FTS5's
delete commands or `AFTER INSERT/UPDATE/DELETE` triggers on `messages`, not
repeated INSERTs.

`import_export`'s doc comment already claims it "(re)builds" the index, so
this fix makes the comment true for free (see D2).

### C4 — Reactions parsed but dropped — **Medium** (acknowledged)

`RawReaction` is fully parsed and even mojibake-repaired
(`core/src/ingest/parse.rs`), the `reactions` table exists
(`core/src/db/schema.rs`), and `Reaction`/`from_row` exist in `models.rs` —
but no insert query exists and `load_messages` ignores `message.reactions`.
The table is always empty and all of that work is thrown away per import.

Two design notes for when this is implemented, both cheaper to decide now:

1. **Dedup:** `reactions` has no unique constraint. If reactions follow the
   attachments pattern ("insert only for a newly inserted message"),
   re-import safety inherits from the message dedup — but unlike attachments,
   reactions genuinely *change over time* on old messages (someone reacts to
   a year-old message between two exports). The attachments pattern will
   silently miss those. A delete-and-reinsert per message, or a
   `UNIQUE (message_id, actor_id, reaction)` index with `INSERT OR IGNORE`,
   handles both.
2. **Actor resolution:** the reaction `actor` is a display name; it should go
   through the same `find_or_create_participant` path a sender does (see A2),
   or a reactor who never sent a message will either fail the
   `actor_id NOT NULL` FK or force a dangling participant.

The alternative is equally legitimate: delete the table and the
parsing/repair work until the feature exists, rather than keeping dead weight
that misleadingly suggests reactions are captured.

### C5 — Import is all-or-nothing per run, but partial per conversation — **Medium**

One malformed file anywhere aborts `import_export`, yet each conversation
commits its own transaction. A failure mid-run therefore leaves the database
partially populated **and with no FTS index at all**, since `populate_fts`
only runs at the very end. The integration test
`a_malformed_conversation_file_fails_the_whole_import_but_leaves_earlier_conversations_committed`
documents this as intended, but from a product standpoint a single bad file —
and real exports do contain oddities — discarding the other 99% of an import
with no way to resume, skip, or report progress is worth revisiting.

**Recommendation.** Collect per-conversation errors and continue, returning a
summary: `ImportReport { loaded, skipped: Vec<(PathBuf, Error)> }`. This
pairs naturally with the structured error type (A10) — the crate can already
name the offending file. If fail-fast stays, at least run `populate_fts` on
the error path so already-committed conversations remain searchable; with the
C2 `rebuild` fix that becomes safe to do unconditionally.

### K10 — `repair_mojibake` assumes exports never contain legitimately-correct non-ASCII text — **Low (accepted)**

*Folded in from `core/KNOWN_ISSUES.md` #10, which recorded this as an accepted
risk rather than a bug to fix.*

`repair_mojibake` (`core/src/ingest/parse.rs`) is applied unconditionally to
every non-ASCII string, on the stated assumption that real Messenger exports
are always mojibake-corrupted and never already-correct UTF-8. Its fallback —
bail out on an out-of-Latin-1 `char`, or on bytes that aren't valid UTF-8 once
reinterpreted — catches most accidental misfires. But a string that is already
correct, entirely within the Latin-1 range, and happens to reinterpret as
valid UTF-8 is silently mis-repaired. Low practical risk given the source
data; worth remembering if this logic is ever reused against a different or
cleaner data source.

Half of the concern is now a checked invariant rather than an argument (T4).
`repair_mojibake_undoes_the_corruption_for_any_text` asserts that corrupting
*any* string and repairing it returns the original, so the "it always undoes
the bug" half holds for every input, not just the hand-picked examples. The
residual risk above is unchanged, and
`repair_mojibake_peels_exactly_one_layer_of_corruption` pins a concrete
instance of it: `"Ã©"` is already-correct Latin-1 text that this function will
happily "repair" into `"é"`, because nothing distinguishes it from a corrupted
`"é"`. That also means the function is deliberately not idempotent.

---

## 4. API and robustness findings

### A7 — Small search-behavior notes — **Low**

- **Empty query:** `run("")`, or all-whitespace after the trim, becomes the
  FTS phrase `""`, which **[verified]** returns `Ok` with 0 hits. Reasonable,
  but it's an accident of FTS5 phrase semantics rather than a chosen
  behavior, and it's untested; consider short-circuiting empty text
  explicitly.
- **Trim location:** the trim lives in `search::run`'s `build_query`, not in
  `FtsIndex::search`. Any caller constructing a `SearchQuery` directly — as
  every test does — silently gets phrase-with-whitespace semantics, so one
  trait has two behaviors depending on entry point. Trimming inside `search`,
  or in a `SearchQuery` constructor, puts the invariant where it's enforced.
- **Case sensitivity:** the participant filter is `p.name = ?3` — exact and
  case-sensitive. Fine for a filter fed by a picker; wrong for one fed by a
  text box. Worth a `COLLATE NOCASE` decision when the UI lands.

### A8 — Rendering a snippet as HTML is an injection vector — **Medium (when the UI lands)**

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
scoped to "when the UI lands" — but the bug is easy to write and easy to miss
in review, because the marker-to-markup replacement *looks* like the whole
job.

**The rule:** escape first, then substitute. HTML-escape the entire snippet,
and only afterwards replace the (now unescaped-by-construction) sentinels
with `<mark>`. Equivalently, and better: split the snippet on the sentinels
and let the framework render the pieces as text nodes, so no raw-HTML
insertion happens at all —

```svelte
{#each splitOnMarkers(hit.snippet) as part}{#if part.matched}<mark>{part.text}</mark>{:else}{part.text}{/if}{/each}
```

This is why the sentinels are non-ASCII format characters rather than
markup-adjacent punctuation (A6): the split is unambiguous even when the
message contains angle brackets, quotes, or entities.

Note the ordering matters in one direction only. Escaping first is safe
because escaping can't produce a sentinel; substituting first is not, because
the escape pass would then mangle the `<mark>` tags it just inserted, and a
careless "escape everything except tags" pass reopens the hole.

Worth deciding at the same time: whether `conversation_title` and
`sender_name` get the same treatment. Both are also export-derived,
sender-influenced strings, and both will be rendered next to the snippet.

---

## 5. Documentation drift

### D2 — README drift — **Low**

- The `SearchIndex` snippet in the README's "Search" section shows
  `-> rusqlite::Result<SearchResults>`; the real trait returns the crate's own
  `Result` — a meaningful difference, since not leaking backend error types is
  one of the crate's stated design points (see A10).
- The hard-coded test count will keep rotting; consider dropping it or
  wording it loosely.
- `import_export`'s doc comment says it "(re)builds the full-text search
  index"; it appends (see C2). If the C2 `rebuild` fix lands, the comment
  becomes true for free.

### D3 — `db/models.rs` is dead code — **Low**

Nothing outside `models.rs` references any of its types — no production
caller, no test outside its own module. These are clearly the intended
Tauri-boundary DTOs, so keeping them is fine, but until a consumer exists
they're untested-against-need API surface: nothing guarantees `SearchHit` vs
`Message` serialization stays coherent for the UI, and hand-written `from_row`
mappers with no callers rot silently as the schema evolves.

`Message` has a partial guard —
`message_from_row_maps_a_row_with_null_content_and_null_sender` runs
`Message::from_row` against a row from the *real migrated schema*, so that
one mapper breaks loudly if the struct and schema diverge. The other four
have no equivalent.

Either wire them up with the first Tauri command, or mark the module with a
short comment stating its intended consumer so it isn't mistaken for
abandoned code.

---

## 6. Performance (unmeasured — headroom, not observed bottlenecks)

Real exports reach hundreds of thousands of messages; none of these findings
were benchmarked, but all sit directly on the per-message hot path.

- **P1 — Statement re-preparation.** Every insert in `queries.rs` goes
  through `conn.execute`/`conn.query_row` with a SQL string, so SQLite
  re-parses each statement per message (`insert_message`, the three statements
  in `find_or_create_participant`, `insert_attachments`). rusqlite's
  `prepare_cached` is a drop-in change and typically a large constant-factor
  win on bulk ingest. Same for the two statements in `FtsIndex::search`,
  though that path is per-keystroke, not per-million-rows.
- **P2 — Per-message participant lookup.** `load_messages` runs the
  find-participant SELECT for *every* message. A `HashMap<String, i64>` per
  conversation in the loader — participants per conversation are few — removes
  roughly one query per message, and `find_or_create_participant` (A2) is now
  the single place to add it.
- **P3 — Double MATCH per search.** The count query and the page query each
  run the FTS match. A single query with `count(*) OVER ()` alongside the page
  columns halves the FTS work, removes the need for the snapshot transaction
  (A14), and removes the hand-duplicated JOIN/WHERE logic between the two.
  Optional — the current two-query-in-one-transaction design is correct and
  clearer; do this only if search latency on a large index warrants it.
- **P4 — Parse inside the write transaction.** `load_conversation` opens the
  transaction before parsing any file, so JSON parse time — potentially tens
  of MB per conversation — is spent holding the write lock. Irrelevant while
  the app is single-connection; becomes real contention the moment a UI reads
  while an import runs. Parsing each file before `tx` starts, or all files
  before the loop, is a small reorder.

Genuinely fine as-is: WAL + `synchronous=NORMAL` for ingest; one transaction
per conversation (good batching granularity with documented partial-import
semantics and a test pinning it); the dedup-index prefix doubling as the
`conversation_id` lookup index; bundled SQLite for reproducibility.

---

## 7. Tests

The suite is a strength: behavior-revealing names throughout, error paths
exercised (missing file, malformed JSON, permission-denied with a
restore-on-drop guard and a root-user escape hatch), integration tests on a
real on-disk DB, and FTS assertions that correctly route around
external-content read-through semantics (`messages_fts_docsize` counting — a
detail many teams get wrong). Tests document *why* fixtures are shaped as
they are.

Gaps, in priority order:

- **T1 — Re-import invariants** (would have caught C1): after two
  `import_export` runs, assert `conversations.message_count` equals
  `count(*) FROM messages` per conversation, and — post-C2 — that the FTS
  index passes `integrity-check` with `messages_fts_docsize` matching the
  indexed-message count. **Use the `rank = 1` form of the check.** The
  one-argument form passes against today's corrupt index (see C2), so a test
  written with it would assert nothing.
- **T2 — Import-failure recovery** (pairs with C5): after a run that fails
  partway, assert what the database is left holding — committed conversations
  present, and once C2 lands, still searchable.

---

## 8. Style and idiom

Measured against the Apollo Rust handbook conventions the project uses:

- **Conforms well:** no production `unwrap`/`expect`; `thiserror` with
  `#[from]` and contextual variants (`ReadFile`/`Parse` carrying the path);
  `?` propagation throughout; `let`/`else` and `.optional()` where
  appropriate; comments explain *why* (the `unchecked_transaction`
  justification in `fts.rs` and the FTS5 read-through comments are model
  examples); imports grouped std / external / crate; `Copy` derived on the
  small `MessageType`/`SortOrder` enums and not on the String-bearing structs.
- **SQL hygiene:** every user-supplied value is parameterized; the only
  `format!`-interpolated SQL fragments (`FILTER_SQL`, `order_by`) are
  compile-time constants or enum-matched literals, so there is no injection
  surface.
- **Error context nit:** `Error::Io(#[from] io::Error)` is transparent, so a
  walk error surfaced from `scan_inbox` loses the path that `walkdir` knew —
  `io::Error::from(err)` discards it. The crate already has the right pattern
  in `ReadFile { path, source }`; a `Scan { path, source }` variant would make
  mid-walk failures as diagnosable as parse failures.
- **`UiFilters` names the UI layer inside the core search type.**
  `SearchQuery.filters: UiFilters` puts a `Ui`-prefixed type at the center of
  the backend-agnostic `SearchIndex` contract, which reads oddly for something
  every search backend must accept. `SearchFilters` would keep the vocabulary
  layer-neutral. Purely a naming call, no behavior implication.

---

## 9. Tauri / frontend (scaffold)

`src-tauri/src/lib.rs` is the untouched template (`greet` command);
`src/routes/+page.svelte` likewise. Nothing to review yet beyond flagging,
for the wiring step:

- Keep the app layer behind the `SearchIndex` trait / `search::run`
  composition root — the boundary is already designed for exactly this; the
  Tauri command should never see FTS5 SQL.
- `rusqlite::Connection` is `!Sync`; under Tauri it will need to live in
  managed state behind a `Mutex`, or a dedicated DB thread/channel.
  `import_export` is long-running and must not run on the main thread — use
  an async command with `spawn_blocking`, and `scan::count_inbox` +
  `find_messages_root` already exist to drive a progress UI (mind A5).
- `grepm_core::Error` will need a serializable mapping for command results,
  since Tauri requires `Serialize` errors; the structured error enum makes
  that straightforward — resist flattening to `String` at the boundary.
- Snippet rendering is the one step in the wiring with a security
  consequence: escape before substituting the match markers, or skip raw-HTML
  insertion entirely. See A8.

---

## 10. Infrastructure

All three infrastructure findings (I1 — no Cargo workspace, I2 — no CI,
I3 — fmt drift) are resolved. See `CODE_REVIEW-addressed.md` §10.

---

## 11. What's done well

Called out explicitly, because most of this review's length is complaints
about a codebase that is above average:

- **Mojibake repair** is the correct inverse transform, applied with two
  defensive escape hatches, an ASCII fast path, a pre-sized buffer, and honest
  documentation of its one residual false-positive risk (K10). Now backed by
  property tests (T4).
- **The message dedup key** shows real iteration: NULL-safe via `COALESCE`,
  extended by `type` and `attachment_count` for same-millisecond attachment
  batches, each extension pinned by a test explaining the collision it
  prevents.
- **External-content FTS5** with the diacritic-folding tokenizer, literal
  -phrase query escaping (with tests proving `OR` is not an operator and a
  stray `"` doesn't break the query), and token-exactness asserted as a
  feature.
- **Migration engine**: versioned via `user_version`, transactional per step,
  rollback behavior pinned by a genuinely clever sabotage test, and
  newer-schema refusal instead of silent misbehavior.
- **Participant identity scoped per conversation** — a correctness-first call
  with the trade-off documented in three places that agree with each other
  (code doc, K3, README).
- **`Page` fields are `u32`** specifically so a negative limit dies at
  deserialization instead of becoming SQL's "no limit" — with tests.
- **Test discipline** — descriptive names, one behavior per test, unit tests
  co-located with modules alongside genuinely end-to-end integration tests on
  real files and databases, and tests that encode *why* (relevance-versus
  -chronological ordering deliberately de-confounded; the FTS5-versus-`LIKE`
  distinguishing scenarios).

---

## 12. Prioritized recommendations

1. **Fix re-import** (C1 + C2): recompute `message_count` from `messages`
   inside `load_conversation`, and switch `populate_fts` to FTS5's `'rebuild'`
   command. Add the T1 invariants test, with the `rank = 1` integrity check.
   This closes the known "re-imports are broken" issue end to end, and the
   index is corrupt *today* — not latently.
2. **Persist reactions** (C4) with a dedup story that tolerates reactions
   added to old messages, and **settle the partial-import story** (C5).
   Both are Tauri-layer prerequisites.
3. Before the UI renders a single snippet, settle the escaping story (A8).
   It's the one item here that turns into a vulnerability rather than a bug.
4. **Performance headroom** (§6), both called out when their neighbouring
   findings were closed: `prepare_cached` on the hot-path queries (P1), and
   the per-conversation participant cache (P2) — `find_or_create_participant`
   is now the single place to add it.
5. Sweep the remaining small items (A7, D2, D3, and the `Scan { path, source }`
   error-context nit) opportunistically.
