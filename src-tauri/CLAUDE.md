# The desktop shell

The Tauri layer: managed state, the IPC surface, and the folder of imported
databases. Read this before editing anything under `src-tauri/`.

The root `CLAUDE.md` describes *what* the library and the command surface are.
This is about how to change them without breaking something quiet. The front
end's half of the same contract is in `src/CLAUDE.md`.

## 1. Commands are wrappers; the work goes in `library.rs`

`start_import` is nine lines of Tauri around `import_into_library`, a plain
function over a directory, a source path and a progress callback. That split
is the whole reason the import path has tests at all — none of them launch a
window.

Keep it that way. A command should validate, take its locks, call into
`library` or `grepm_core`, and map the error. Anything with logic worth
testing belongs one level down.

## 2. Take `library` before `active`

Every command touching both locks does, and the reverse order is a deadlock
waiting for two of them to land at the same moment. This is repeated in the
root `CLAUDE.md` because it is the one rule here that fails intermittently and
only under load.

`lock()` recovers from poisoning rather than propagating it, deliberately: a
panic while either lock is held leaves a perfectly usable value behind — a
path, and a connection that can be reopened — so poisoning every later command
in the process would be the larger bug.

The `library` lock is an `Arc<Mutex<PathBuf>>` and not just a `PathBuf` beside
the mutex, so an import can carry it onto a blocking thread and hold it for
its whole run. Nothing else needs it meanwhile: the launch screen is the only
caller of `list_imports`, and it is replaced by the progress view for the
duration.

## 3. Every state change lands by rename

There are three, and each is ordered so that an interruption leaves something
recoverable rather than something wrong:

- **The index** is written to `.index.json.tmp` and renamed over
  `index.json`. A crash mid-write leaves the previous index intact. Losing the
  newest import from a list is recoverable; a truncated index is not.
- **An import** builds into `.tmp-<id>.sqlite3`, is renamed into place, and
  only then is added to the index — so the index never names a file that isn't
  there yet. A failure deletes the temp file, because a half-imported database
  sitting unreferenced is indistinguishable from a good one.
- **A delete** removes the files *before* the index entry. The reverse would
  report success with the data still on disk, which is the one promise a
  delete can't break. Failing partway leaves an entry that the existing
  `ImportFileMissing` path already handles, and retrying finishes the job.

Deleting also removes the `-wal`/`-shm` sidecars, and closes the connection
first if that import was the open one — a live connection holds the file open,
which is undeletable on Windows.

It never touches `source_path`. The export is the user's own data and grepm
only ever read it.

## 4. The index is allowed to drift from the folder

`index.json` is the source of truth for the launch screen, so a database
deleted by hand stays listed until someone opens it. That is the deliberate
trade for not opening every file at launch just to read a name, and
`ImportFileMissing` is what the drift looks like when it surfaces.

Which is why `open_import` checks `path.exists()` before calling
`schema::open` — that function would happily *create* a database there, and an
import deleted behind the app's back would silently come back as an empty one
instead of reporting itself missing.

## 5. `AppError` is a UI contract, not a log line

Rendering everything to a string at the boundary is the tempting shortcut, and
it forces the front end to parse prose to tell "that name is taken"
(recoverable, show it by the field) from "the disk is full" (not). So:

**Every case the UI branches on gets its own variant with its data intact.**
Only genuinely unexpected errors carry a rendered `message`.

It serializes internally tagged (`{ "kind": "name_in_use", "name": "…" }`),
and `src/lib/ipc/types.ts` mirrors it **by hand**. There is no codegen —
change one, change the other in the same commit. The user-facing wording lives
in `src/lib/errors`, not here; these `#[error]` strings are the developer's
view.

## 6. Adding a command touches four places

1. `commands.rs` — the `#[tauri::command]` function.
2. `lib.rs` — the `generate_handler!` list. Missing this compiles fine and
   fails at runtime.
3. `src/lib/ipc/index.ts` — the only file allowed to call `invoke`.
4. `src/lib/ipc/types.ts` — if it returns or takes a new shape.

Field names cross as snake_case (serde's default) and `Option<T>` arrives as
`T | null`, not as a missing key.

## 7. Say nothing twice

`list_conversations` returns an empty vec when no import is open, rather than
an error. The screen asking has already checked with `active_import` and shows
its own message; a second way to say the same thing is only a second thing to
handle.

Progress events are advisory for the same reason — `app.emit` failing means
the window is gone, and the import should still run to completion rather than
die on a dead channel.

## 8. No SQL here

Commands go through `grepm_core`'s public API — `db::schema`, `db::queries`,
`ingest`, and `SearchIndex`/`search::run` when search lands. If you find
yourself wanting a query this layer can't express, add it to
`core/src/db/queries.rs` where it can be tested against a real database.

`active_import` reads its counts live from the open connection rather than
from the index, so a screen showing them has proven the file really is open.
Prefer the stored counts over `conversations.message_count` anywhere a user
sees a number — the root `CLAUDE.md` explains why those two disagree.

## 9. Tests

`./scripts/verify-rust.sh` from the repo root is the definition of done. CI
splits Rust across two jobs — the engine job and this crate's — but both lint
and test their own crate, so a green CI covers these tests too.

The tests are in `library.rs` under `#[cfg(test)]` and run against the repo's
`samples/` export. Two habits worth keeping:

- Assert **shapes, not exact totals**. `samples/` has grown before and will
  again; a test pinning "104 conversations" turns that into a chore.
- The delete tests build their own throwaway export with `write_export`
  instead of pointing at `samples/`. A bug that reached for `source_path`
  would otherwise destroy the real fixture while proving it exists.

Building this crate needs the Tauri system libraries; the install list is in
`.github/workflows/ci.yml`.
