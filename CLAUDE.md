# grepm

A local, offline full-text search engine over a Facebook Messenger data
export. Tauri desktop app: a Rust engine, a Rust shell, a SvelteKit front
end. Nothing leaves the machine.

## Layout

```
core/        grepm_core — the engine. Pure Rust library, no UI deps.
             db/ (SQLite schema, migrations, queries)
             ingest/ (scan → parse → load)
             search/ (the SearchIndex trait + its FTS5 implementation)
src-tauri/   the Tauri shell.
             library.rs (the managed imports folder + its index)
             commands.rs (the IPC surface), error.rs (serializable errors)
src/         the SvelteKit front end. See src/CLAUDE.md before editing.
samples/     a synthetic export — 104 conversations, ~1,200 messages — used
             by manual testing and by `src-tauri/src/library.rs`'s import
             tests. Big enough to exercise the windowed sidebar, and it
             includes two conversations with no messages and one group of
             nine. Tests over it assert shapes, not exact totals, so it can
             grow again without breaking them.
```

The engine is finished and heavily tested. The app layer now imports an export
with live progress, lists and reopens past imports, deletes one, and shows an
opened import's conversations in a sidebar. There is no search UI yet — that is
the next slice, and `src/routes/opened/` is where it lands.

## The library

An import is one SQLite file in a folder grepm owns
(`<app_data_dir>/imports/`), named by an `index.json` beside it. The index is
the source of truth for the launch screen's list, so it can drift from the
folder: a database deleted by hand stays listed until someone opens it, which
is the `ImportFileMissing` error. That's deliberate — the alternative is
opening every file at launch just to read a name.

Ids are opaque, so a user's chosen name never becomes a filename and never has
to satisfy a filesystem's rules. One import per file, which also sidesteps
`KNOWN_ISSUES.md` §9 (`populate_fts` isn't idempotent, so re-importing into an
existing database is unsafe).

Deleting an import removes its database, the `-wal`/`-shm` sidecars WAL mode
leaves beside it, and its index entry — and closes the connection first if that
import was the open one. It never touches `source_path`: the Facebook export is
the user's own data, which grepm only ever read.

## The command surface

Everything in `src-tauri/src/commands.rs`, registered in `lib.rs`:

| command              | does                                                           |
| -------------------- | -------------------------------------------------------------- |
| `list_imports`       | reads `index.json`; `[]` on first launch                       |
| `start_import`       | async; validates, ingests on a blocking thread, emits progress  |
| `open_import`        | opens the database into managed state                          |
| `active_import`      | the open import plus row counts read live                      |
| `list_conversations` | conversations in the open import                               |
| `delete_import`      | removes an import's data                                       |

Managed state holds the imports folder and the open `Connection`, each behind a
`Mutex`. **Take `library` before `active`** — every command touching both does,
and the reverse order is a deadlock waiting for two of them to land together.

The long-running work is `import_into_library`, a plain function over a
directory and a progress callback. Keeping it out of the command itself is what
makes the whole import path testable without a running window; `start_import`
is only the Tauri wrapper around it.

## Two counts that look interchangeable and aren't

- `conversations.message_count` is the total the export **claimed**, summed
  before duplicate messages were dropped. `db::queries::conversations` reports
  the rows **actually stored**, which is what a search over that conversation
  could return. Prefer the stored count anywhere a user sees a number.
- `ingest::Progress.total` is an **upper bound**: it counts folders under
  `messages/inbox` without opening them, while the import skips any holding no
  message files. A finished import can end at `done < total`, so treat the call
  returning — not `done == total` — as completion.

## Don't page `db::queries::conversations`

It looks like the obvious fix for a large library, and it isn't: the
`ORDER BY max(m.timestamp_ms)` over a `GROUP BY` makes SQLite aggregate and
sort every message before it can apply an `OFFSET`, so each page would repeat a
full scan and scrolling would get *slower* as the export grew. The sidebar
fetches every summary once and windows the DOM instead — see `src/CLAUDE.md`
§9. Paging would first need `last_message_ms` and a deduplicated
`message_count` stored on the `conversations` row at import time, so the order
comes from an index rather than an aggregate.

## Commands

```sh
cargo test --workspace                       # engine tests
cargo clippy --workspace --all-targets       # gated at -D warnings in CI
cargo fmt --all --check

npm run verify                               # check + lint + test
npm run shoot                                # screenshot the UI (see below)
npm run tauri dev                            # the real desktop app
```

`npm run verify` is the front-end definition of done. `npm run shoot` renders
routes in headless Chromium against a stubbed Tauri bridge and writes PNGs to
`.screenshots/` — read them to see what the UI actually looks like, since a
Tauri window can't be observed any other way.

## Conventions

- **Rust wraps at 100 columns; comments wrap at 80.**
- **The app layer goes through the `SearchIndex` trait** (`core/src/search/`)
  or `search::run`. Tauri commands and Svelte code never see FTS5 SQL.
- Comments explain _why_, not _what_. Match the density of the surrounding
  file — the engine is commented heavily where a decision was non-obvious and
  not at all where it wasn't.
- Tests are expected with behaviour changes. The established habit is to
  sabotage a fix and confirm the new test fails before keeping it.

## Further reading

- `core/KNOWN_ISSUES.md` — tracked correctness trade-offs and future work.
- `ai-code-reviews/CODE_REVIEW.md` — findings, open and closed, by ID.
  Findings are cited by ID in commits and comments; keep the IDs stable.
- `AGENTS.md` — the Svelte MCP server's tools (documentation lookup and a
  compiler-backed autofixer). Loaded automatically via `.claude/CLAUDE.md`.
