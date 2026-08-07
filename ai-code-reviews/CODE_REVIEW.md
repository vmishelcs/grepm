# Code Review — grepm

**Date:** 2026-08-05; app-layer pass 2026-08-06
**Scope:** `core/` (grepm_core), `src-tauri/`, project layout. At the first
pass the SvelteKit frontend and Tauri shell were starter scaffold and were
reviewed only as such. The 2026-08-06 pass reviewed them as real code — the
library (`src-tauri/src/library.rs`), the command surface and error boundary,
and every component, route and test under `src/` — plus the queries and
progress plumbing added to `core` for them.

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

**2026-08-06 pass**, against `ui-dev` at 4cdf311:

| Check | Result |
|---|---|
| `cargo test --workspace` | **191 passed** (25 shell + 137 core unit + 12 ingestion + 17 search), 0 failed |
| `cargo clippy --workspace --all-targets` | clean, 0 warnings |
| `cargo fmt --all --check` + comment width | clean |
| `npm run check` (svelte-check) | 0 errors, 0 warnings |
| `npx eslint .` | clean |
| `npm run test` (vitest, real Chromium) | **38 passed**, 0 failed |
| `npm run verify` | **failed** before its test stage — I4, fixed 2026-08-06 |

A temporary probe (written, run, deleted) timed the app-facing read queries
against a seeded 500,000-message database; its numbers are cited in A17. The
claim about which thread runs a command was checked against the vendored
Tauri sources rather than by running a window, and is labeled accordingly.

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

**2026-08-06.** The app layer this review was waiting on has landed, and it
is built the way §9's wiring notes asked: no SQL above the boundary, a
structured serializable error type, the import off the main thread with
progress events, and the snippet-escaping rule (A8) written into the front
end's conventions before any snippet is rendered. The re-import corruption
above is also now *contained* — the library gives every import a fresh
database file, so no UI path can reach C1/C2 — though the engine still does
not enforce that discipline itself. The new findings: a main-thread hazard
that can freeze the window (A17), disabled-CSP hardening (A18), a temp-file
leak on crash (A19), and smaller UI, copy, CI and tooling items (A20–A22,
plus T5 and I4, both fixed 2026-08-06), plus dated status notes inside
C1/C2, A8, D2 and D3.

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
| A17 | Medium | tauri | Sync commands run on the webview's main thread |
| A18 | Medium | tauri/security | CSP disabled; unused opener plugin widens the bridge |
| A19 | Low | library | A crashed import leaks its temp database forever |
| A20 | Low | ui/errors | A newer build's index is reported as "damaged" |
| A21 | Low | ui | The windowed list measures once; load order saves it |
| A22 | Low | ui | Small front-end notes |

Twenty-nine further findings are resolved — see `CODE_REVIEW-addressed.md`.

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

**Status 2026-08-06.** C1 and C2 both stay open, but the app layer now
contains the blast radius by construction: the library gives every import a
fresh database file and offers no path that imports into an existing one
(`src-tauri/src/library.rs`; the root `CLAUDE.md` cites C1/C2 as a reason
for that design). The corruption is no longer reachable from the UI.
Nothing in `grepm_core` enforces the one-import-per-database discipline,
though — the moment a "refresh this import" feature lands, it lands exactly
here, so the engine-level fixes and the T1 test are still worth their cost.

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

**Status 2026-08-06.** Still nothing renders snippets, so still open — but
the groundwork is ahead of need: the rule is codified as `src/CLAUDE.md`
rule 1, the shared fixtures carry a live `<img onerror>` payload in a title,
a sender name *and* a snippet, and the sidebar already renders
export-derived titles and participant names as text nodes with a browser
test pinning that (`renders a title containing markup as text`). The answer
to the last paragraph's question turned out to be yes.

### A17 — Sync commands run on the webview's main thread — **Medium**

Every command except `start_import` is a plain `fn`, and Tauri executes
those inline on the main thread: wry delivers the IPC request on the UI
thread, and a non-`async` command runs to completion inside that handler
(`ExecutionContext::Blocking` in `tauri-macros`' `command/wrapper.rs`; the
Tauri docs state it directly — "Commands without the *async* keyword are
executed on the main thread"). Verified against the vendored sources, not by
running a window. Two consequences, one latent and one live:

- **The freeze.** `start_import` holds the `library` mutex for its whole
  run — deliberately, and the comment argues nothing else needs the lock
  meanwhile because the launch screen is replaced by the progress view. That
  is true of this window's own navigation and of nothing else. A webview
  reload during an import (trivial in a dev build, platform-dependent in
  release) lands back on the launch screen, whose `list_imports` then blocks
  **the event loop** on a mutex that stays held for minutes: no repaint, no
  close, no progress events — the window is dead until the import finishes.
  The same holds for `open_import` and `delete_import` the moment any future
  UI can reach them while an import runs.
- **The jank.** `active_import` and `list_conversations` run full-scan
  aggregates over the open database. **[verified]** against a seeded
  database of 2,000 conversations / 500,000 messages / four participants
  each: `db::queries::conversations` takes **~65 ms** in release and
  **~380 ms** in debug; `stats` 6–8 ms. That work runs on the thread that
  paints the window, on every `/opened` load, and it grows linearly with the
  export — plus the IPC serialization of every summary on top.

**Recommendation.** Mark the read commands `#[tauri::command(async)]` — on a
sync `fn` that alone moves execution onto the async runtime — and make the
three `library`-lock takers genuinely async with the lock-holding section in
`spawn_blocking`, the pattern `start_import` already uses. A command that
arrives mid-import then *pends*, as a promise the front end already awaits,
instead of freezing the event loop. Worth deciding before the search UI
lands: search will put a per-keystroke query behind a command, which is the
worst place to discover this.

### A18 — CSP is disabled, and an unused plugin widens the bridge — **Medium (hardening)**

`tauri.conf.json` still carries the scaffold's `"security": { "csp": null }`.
A8 explains why this app's threat model is unusual — the corpus is text
strangers wrote, rendered in a webview that reaches the IPC bridge. The
escaping rule is the primary defense, but it is one `{@html}` slip away from
mattering, and with a CSP configured Tauri injects nonces/hashes for its
bundled assets so an injected inline script stops short of executing. With
`null` there is no second line. The app is offline by design — nothing
legitimate loads from a remote origin — so a restrictive policy costs
nothing; start from Tauri's documented default (`default-src 'self'` plus
the `ipc:`/`http://ipc.localhost` connect-src the bridge needs) and verify
the app still runs.

Separately, `tauri_plugin_opener` is initialized and granted
`opener:default`, and nothing in `src/` imports `@tauri-apps/plugin-opener`
— scaffold residue. An injected script's reach is exactly the set of
commands exposed on the bridge, and today that includes "open a URL or path
with the system handler" in exchange for no feature. Drop the plugin
(`Cargo.toml`, `lib.rs`, `capabilities/default.json`, `package.json`) until
something needs it, then re-add it scoped to what that feature opens.

### A19 — A crashed import leaks its temp database forever — **Low**

The failure *path* cleans up: `import_into_library` deletes
`.tmp-<id>.sqlite3` when the ingest returns an error. A failure that never
returns — a crash, a kill, a power cut, minutes into a multi-GB import —
leaves the temp file in the imports folder with nothing pointing at it and
nothing that will ever delete it: `allocate_id` steps around it, no listing
shows it, and the module doc's "Nothing is left behind on failure" quietly
becomes untrue. Two smaller residue paths sit in the same function: a failed
`fs::rename` into place returns without the cleanup the ingest-failure arm
has, and a failed `write_index` *after* the rename orphans a fully-built,
real-named database that no index entry will ever list.

**Recommendation.** Sweep `.tmp-*.sqlite3` from the imports folder at
launch, before the first command can start an import — at that moment any
temp file is stale by definition. Extend the cleanup arm over the rename
failure, and consider removing the renamed file when `write_index` fails, so
the all-or-nothing claim is true on every path. Worth a line in the module
doc while there: all of this assumes one process. Two running instances
share the folder with no cross-process lock; the rename keeps each index
write atomic, but two writers can silently drop each other's entries.

### A20 — A newer build's index is reported as "damaged" — **Low**

`read_index` folds two different situations into `CorruptIndex`: JSON that
does not parse, and a well-formed index whose `version` is newer than the
build understands. The schema layer keeps those apart
(`UnsupportedSchemaVersion` vs `InvalidSchemaVersion`) precisely so the UI
can say "update grepm" instead of "your data is damaged" — but for the
index, `$lib/errors` renders both as "The library index is damaged", which
is the wrong message with the wrong emotional weight for a user who merely
launched an older build after a newer one. A `NewerIndex { found,
supported }` variant, mirrored in `types.ts` with its own copy in
`errors.ts`, aligns the two version checks.

### A21 — The windowed list measures once, and only load order saves it — **Low**

`ConversationList`'s `trackSize` attachment measures the row pitch from the
first `li.row` in the DOM. If no row exists yet it returns without setting
anything, and nothing retries: the `ResizeObserver` watches the `ul`, whose
size never changes when rows are added inside its fixed flex height. Mounted
empty, the list stays unmeasured forever — `rowHeight` 0, spacers 0px, and
rendering silently capped at the 40 `UNMEASURED_ROWS` with the rest of the
list unreachable.

It works today because `/opened` gates the reader behind `{#if info}` and
assigns `info` and `conversations` from one `Promise.all`, so the list never
mounts before its rows exist. That invariant is held two files away from the
code that depends on it, and the windowing suite cannot catch a regression —
it drives the page, so it inherits the same load order. A small guard makes
the component correct on its own: re-run the measurement when rows exist but
`measured` is still false (an `$effect` keyed on the first non-empty
`visible`), or measure from a prop-driven callback rather than only from the
observer.

### A22 — Small front-end notes — **Low**

- **Menu focus is dropped on close.** Opening the `...` menu moves focus
  onto the menu item; Escape or an outside click closes it without restoring
  focus, which lands back on `<body>` and costs a keyboard user their place.
  Return focus to the triggering button on close. (Arrow-key navigation can
  wait — the menu has one item.)
- **One test stubs the bridge twice.** `page.svelte.test.ts`'s "reports an
  import whose file has gone missing" calls `stubCommands` and then
  immediately replaces the stub with its own `mockIPC`; the first call is
  dead and reads as though both were needed.
- **`shoot.mjs` answers a `search` command that doesn't exist.** Harmless
  today, but the file's own comment promises entries are added *when* a
  command is added, and a pre-registered stub will mask the "no fixture"
  error precisely when the real search command lands with a different name
  or shape.

---

## 5. Documentation drift

### D2 — README drift — **Low**

- The `SearchIndex` snippet in the README's "Search" section shows
  `-> rusqlite::Result<SearchResults>`; the real trait returns the crate's own
  `Result` — a meaningful difference, since not leaking backend error types is
  one of the crate's stated design points (see A10).
- The hard-coded test count will keep rotting; consider dropping it or
  wording it loosely. (It has: the README still says 158, and the workspace
  now runs 191 — the drift this bullet predicted.)
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

**Status 2026-08-06.** The first Tauri commands landed and did *not* use
these types: the boundary grew its own instead (`ConversationSummary` and
`Stats` in `queries.rs`, `ImportEntry` in the shell), and
`src/lib/ipc/types.ts` explicitly declines to mirror `models.rs` "because
none of it crosses the boundary yet". The module is now dead code with a
working counter-example beside it, which strengthens the delete option:
whatever the search UI ends up needing, the pattern so far is to define it
where it is queried, with tests, not to reach for these.

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

## 9. Tauri / frontend (reviewed 2026-08-06)

At the first pass this section could only leave wiring advice for a layer
that didn't exist. The layer now exists, and every piece of that advice was
followed, verifiably: no SQL crosses the boundary (the two queries the UI
needed were added to `queries.rs`, with tests); the `Connection` lives in
managed state behind a `Mutex` and the import runs in `spawn_blocking` with
progress driven by `find_messages_root` + `count_inbox`, the `total`
upper-bound caveat documented on both sides of the wire; `AppError` is a
structured, internally-tagged `Serialize` enum rather than a flattened
string, mirrored by hand in `types.ts` with narrowing and user copy kept in
`$lib/errors`; and the snippet rule is codified ahead of need (see the A8
status note).

Beyond the advice, the layer has real design in it — the atomic-rename
discipline on every state change, the index that may deliberately drift from
the folder, delete ordered so a partial failure is retryable, and a windowed
sidebar whose load-bearing invariants are documented and tested. Nothing in
it rises to the engine review's C severity: there is no data-corruption path
in the shell. What this pass found is filed above — A17 (main-thread
commands, the one that can freeze the window), A18 (CSP), A19 (temp-file
residue), A20 (version copy), A21 (windowing measurement), A22 (small
notes) — plus the two gate gaps it found closed the same day, T5 (CI never
ran this crate's tests) and I4 (`npm run verify` failing on a machine-local
file), now in `CODE_REVIEW-addressed.md`.

---

## 10. Infrastructure

All four findings (I1 — no Cargo workspace, I2 — no CI, I3 — fmt drift,
I4 — `npm run verify` failing on a machine-local file) are resolved. See
`CODE_REVIEW-addressed.md` §10. I2's resolution block keeps one open note
that is not a code change: CI reports after a push to `main` rather than
gating it, and making it block needs branch protection in repo settings.

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

From the 2026-08-06 pass, in the same spirit:

- **The library's failure ordering** — build under a temp name → rename →
  list; delete files → delist — is reasoned about explicitly at each step,
  with the recoverable state after every possible interruption named in a
  comment, and the delete tests build a throwaway export so a bug that
  reached for `source_path` couldn't destroy the fixture proving it exists.
- **`AppError` as a UI contract** — a variant per case the UI branches on,
  data intact, wire shape and user-facing copy in separate files, and the
  hand-mirroring rule written identically at both ends of the wire.
- **The windowed sidebar** documents its two load-bearing invariants
  (uniform row pitch, externally bounded height), is tested for measurement
  rather than for hardcoded row counts, and reports `aria-setsize`/
  `aria-posinset` for the whole list rather than the mounted slice.
- **Front-end test discipline matches the engine's** — `mockIPC` stubs that
  throw on unexpected commands so a screen can't quietly gain an unanswered
  call, fixtures shared between the vitest suite and the screenshot script,
  the injection payload kept permanently in those fixtures, and
  sample-export tests asserting shapes rather than totals so the fixture can
  grow.
- **Progress reporting done honestly** — `total` documented as an upper
  bound at the type, the wrapper, the TS mirror *and* the UI (which treats
  the promise resolving, not `done == total`, as completion), with the
  no-going-backwards property pinned by a test.

---

## 12. Prioritized recommendations

The first pass's recommendation 1 — the two one-line fixes to the definitions
of done (T5 + I4) — was done on 2026-08-06, and both gates now mean what they
say. Its resolution blocks are in `CODE_REVIEW-addressed.md`. What remains,
in order:

1. **Decide the command-threading pattern now** (A17): `async` commands with
   `spawn_blocking` around the lock-holders. The freeze is an edge case
   today; the search UI will put a per-keystroke query behind a command,
   which is where the main-thread habit gets expensive.
2. **Fix re-import in the engine** (C1 + C2): recompute `message_count`
   inside `load_conversation`, switch `populate_fts` to FTS5's `'rebuild'`,
   add the T1 invariants test with the `rank = 1` integrity check. The
   library's one-file-per-import design means no user can trigger it today,
   which lowers the urgency — but the index corruption is real, and the next
   import-shaped feature inherits it.
3. Before the UI renders a single snippet, settle the escaping story (A8) —
   the groundwork is already in place — and pair it with the CSP and plugin
   trim (A18), which is what stands behind the escaping if it ever slips.
4. **Persist reactions** (C4) with a dedup story that tolerates reactions
   added to old messages, and **settle the partial-import story** (C5).
5. **Performance headroom** (§6): `prepare_cached` on the hot-path queries
   (P1) and the per-conversation participant cache (P2) —
   `find_or_create_participant` is the single place to add it.
6. Sweep the remaining small items opportunistically: A7, A19–A22, D2, D3,
   and the `Scan { path, source }` error-context nit.
