# grepm

**A local, offline browser and full-text search engine for your Facebook
Messenger data export.**

Facebook lets you download an archive of your Messenger history, but what
you get is a directory tree of raw JSON files — one folder per
conversation, split across `message_1.json`, `message_2.json`, … with no
way to actually *search* across it. `grepm` ingests that export into a
local SQLite database and gives you fast, accent-insensitive full-text
search over every message, with filtering by conversation, sender, and
date range.

Everything runs locally. Your messages never leave your machine.

> **Status:** the core engine (`grepm_core`) — ingestion, storage, and
> search — is built and tested (94 tests). The desktop UI (Tauri +
> SvelteKit) is still the starter scaffold and is not yet wired to the
> engine. See [`CODE_REVIEW.md`](CODE_REVIEW.md) (finding D1) and
> [`core/KNOWN_ISSUES.md`](core/KNOWN_ISSUES.md) for the current state.

---

## What it does

Given the root of an unzipped Facebook data export, `grepm`:

1. **Scans** `messages/inbox/` for conversation folders and their
   `message_N.json` files.
2. **Parses** each file, repairing Facebook's mangled text encoding along
   the way (see [Mojibake repair](#mojibake-repair)).
3. **Loads** conversations, participants, and messages into SQLite,
   de-duplicating as it goes so re-runs and overlapping files don't create
   double rows.
4. **Indexes** every message body into an FTS5 full-text index.
5. **Searches** that index with literal-phrase queries, optional filters,
   pagination, and three sort orders (relevance / newest / oldest).

## Architecture

The project is a Cargo + Tauri workspace:

```
core/          grepm_core — the engine (pure Rust library, no UI deps)
  src/
    db/        SQLite schema, migrations, models, and insert/upsert queries
    ingest/    scan → parse → load pipeline for a Facebook export
    search/    the SearchIndex trait + its FTS5 implementation
  tests/       on-disk end-to-end tests (ingestion.rs, search.rs)
src-tauri/     the Tauri (Rust) desktop shell  — currently scaffold
src/           the SvelteKit (TypeScript) frontend — currently scaffold
samples/       a small synthetic export for manual testing
```

### The ingest pipeline (`core/src/ingest/`)

- **`scan.rs`** locates the `messages/inbox` directory (its depth in the
  archive isn't fixed, so it's *searched for* rather than assumed), then
  lazily yields one `ConversationDir` per conversation folder with its
  `message_N.json` files collected and **sorted numerically** — so
  `message_2.json` comes before `message_10.json`, which a lexicographic
  sort would get wrong.
- **`parse.rs`** deserializes each file into `RawConversationFile` via
  `serde`, tolerating missing/optional fields, and repairs mojibake.
- **`loader.rs`** upserts the conversation, inserts and links its
  participants, resolves each message's sender to a participant id, and
  inserts the messages — all inside a single transaction per conversation.

### Storage (`core/src/db/`)

A small SQLite schema: `conversations`, `participants`,
`conversation_participants` (join table), `messages`, `reactions`, and a
`messages_fts` virtual table. Migrations are versioned via SQLite's
`user_version` pragma so the schema can evolve. The connection is
configured for WAL journaling, `synchronous=NORMAL`, and enforced foreign
keys.

### Search (`core/src/search/`)

Search is defined by a trait:

```rust
pub trait SearchIndex {
    fn search(&self, query: &SearchQuery, page: Page)
        -> rusqlite::Result<SearchResults>;
}
```

The rest of the application depends only on this trait — it never touches
FTS5 SQL directly. `FtsIndex` (`fts.rs`) is the concrete SQLite/FTS5
implementation behind it. A `SearchQuery` carries the search `text`, a set
of `UiFilters` (conversation, participant, date range — any subset), and a
`SortOrder`; results come back as a page of `SearchHit`s plus a total
`count` for the UI's pagination controls.

---

## Notable design choices

A few decisions that were less obvious than they look, and why they went
the way they did.

### External-content FTS5, not a duplicate copy

`messages_fts` is created with `content='messages', content_rowid='id'` —
an [external-content FTS5 table](https://www.sqlite.org/fts5.html#external_content_tables).
The full-text index lives alongside the `messages` table but **doesn't
store a second copy** of every message body; it indexes the text in place
and reads through to `messages` for the original content when producing
snippets. Same searchability, roughly half the storage.

### Diacritic-insensitive search

The index tokenizes with `unicode61 remove_diacritics 2`, so searching
`cafe` matches a message containing `café`. This is verified end-to-end:
`core/tests/search.rs` stores accented content and asserts the un-accented
query finds it — which can only pass if search really goes through the FTS
tokenizer and not a raw substring scan.

### Queries are literal phrases, not FTS5 syntax

FTS5's `MATCH` grammar treats `-`, `*`, `AND`/`OR`, `"`, and more as query
operators. Users typing into a search box don't expect that. So the query
text is wrapped as a quote-escaped FTS5 *phrase*
(`"user text with \"\" doubled"`), which makes the whole thing match
literally — `coffee OR tea` searches for that exact phrase, not "coffee or
tea as a boolean." There are tests asserting exactly this (that `OR` is
*not* interpreted as an operator, and that an embedded `"` doesn't blow up
the query).

### Token-exact matching is a feature, not a bug

Because search goes through FTS5's tokenizer, `coffee` matches the word
"coffee" but **not** "coffeehouse" — they're distinct tokens. A naive
`LIKE '%coffee%'` scan would match both. A test deliberately seeds
"coffeehouse" and asserts it's excluded, which doubles as proof that search
routes through the index rather than scanning raw message text.

### Participant identity is scoped per-conversation

Facebook's export gives no stable per-person id — only a display name — and
two unrelated people can share a name ("John Smith"), while a *deleted*
account shows up as the literal placeholder "Facebook User" in every
conversation it touched. Rather than dedupe participants globally by name
(which would wrongly merge distinct people), `grepm` scopes a participant
to a single conversation: the same display name in two different
conversations becomes two rows. This trades cross-conversation "all
messages from Bob" convenience for **not silently merging different
people** — a correctness-first call. See `core/KNOWN_ISSUES.md` #3 for the
full trade-off.

### Mojibake repair

Messenger's export has a long-standing encoding bug: non-ASCII text is
UTF-8 encoded and then mis-decoded as Latin-1, so an em dash arrives as
`â` and `café` as `cafÃ©`. `grepm` reverses this by
reinterpreting each character as a single Latin-1 byte and decoding the
resulting byte sequence as UTF-8. It's applied defensively — any character
outside the Latin-1 range, or bytes that don't form valid UTF-8, leaves the
original text untouched.

### Errors during a scan are surfaced, not swallowed

`scan()` yields `Result` items, so an I/O error hit partway through the
walk comes back as an `Err` the caller can see, rather than being silently
dropped and mistaken for "this conversation just has no messages."

### Search returns a count *and* a page

Each search runs two queries — one `count(*)` for the total number of
matches and one `LIMIT/OFFSET` query for the current page — so the UI can
show "showing 1–32 of 900" and drive pagination without loading every hit.

---

## Building & testing

The engine has no UI dependencies and can be built and tested on its own:

```sh
cd core
cargo test          # runs unit + on-disk integration tests
cargo clippy --all-targets
```

The full desktop app (once the UI is wired up) uses the standard Tauri
workflow from the repository root:

```sh
npm install
npm run tauri dev
```

### Recommended IDE setup

[VS Code](https://code.visualstudio.com/) +
[Svelte](https://marketplace.visualstudio.com/items?itemName=svelte.svelte-vscode) +
[Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) +
[rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer).

## Further reading

- [`CODE_REVIEW.md`](CODE_REVIEW.md) — a full review of the current code,
  with findings and suggestions.
- [`core/KNOWN_ISSUES.md`](core/KNOWN_ISSUES.md) — tracked correctness
  trade-offs and future work, with most of the early ingestion issues now
  resolved.
