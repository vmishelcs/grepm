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
src-tauri/   the Tauri shell. Currently the `greet` scaffold.
src/         the SvelteKit front end. See src/CLAUDE.md before editing.
samples/     a small synthetic export for manual testing.
```

The engine is finished and heavily tested; the app layer is not. Most work
happens in `core/` or `src/`.

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
