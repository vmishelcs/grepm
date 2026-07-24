# Known issues / future work

Notes on things spotted while building out `core`, in roughly the order
they'd bite someone. Nothing here is blocking, but each is worth a
deliberate decision before this data is relied on.

## Correctness

### 1. ~~`message_count` isn't accumulated across a conversation's files~~ (fixed)

`load_conversation` (`src/ingest/loader.rs:12-34`) now loops over every
file in `conversation_dir.message_files` and calls `upsert_conversation`
(`src/db/queries.rs:10-30`) once per file, so each file's message count
gets added onto the running total via
`message_count = message_count + excluded.message_count`. Files are
matched to the same conversation row via the `(title, thread_path)`
unique key (see #5) rather than the removed `raw_name` folder name.

### 2. ~~The messages dedup constraint doesn't catch `NULL == NULL`~~ (fixed)

`content` is now `TEXT NOT NULL DEFAULT ''` (`src/db/schema.rs:28-35`) —
`insert_message` stores a missing message body as `''` instead of `NULL`
(`src/db/queries.rs:82`), so `content` can no longer take part in a
`NULL == NULL` mismatch.

`sender_id` is still nullable (an unresolved sender is a real "no id"
case, not something to default away), so the dedup constraint moved out
of the `messages` table definition into a standalone
`CREATE UNIQUE INDEX idx_messages_dedup` (`src/db/schema.rs:37-42`) that
compares `COALESCE(sender_id, -1)` instead of `sender_id` directly.
SQLite doesn't allow expressions in an inline table-level `UNIQUE`
constraint (only in a `CREATE INDEX`), which is why this one constraint
couldn't stay inline like the rest of the schema.

### 3. Participants are deduped globally by exact name match

`participants.name` is `UNIQUE` (`src/db/schema.rs:17`), and
`insert_participant` (`src/db/queries.rs:39-47`) upserts on that name.
Facebook's export format gives no stable per-person ID, only a display
name, so this is the only signal available — but it means two different
real people who happen to share a display name (not unusual: "John
Smith" across unrelated threads) will silently be merged into a single
`participants` row. There's no way to detect or undo this from the export
data alone; worth deciding whether that's an acceptable tradeoff for a
personal search tool or whether participants need to be scoped per
conversation instead of deduped globally.

### 4. ~~Participants that only appear in a later file never get linked~~ (fixed)

`load_conversation` now inserts and links every file's `participants`
list (`src/ingest/loader.rs:23-26`), not just the first file's, so
someone who only appears in a later page's `participants` list (and
never sends a message) still gets linked via
`conversation_participants`.

### 5. ~~Conversation metadata is unconditionally overwritten on conflict~~ (fixed)

`conversations` no longer has a `raw_name` column; a conversation is now
identified by `UNIQUE (title, thread_path)` (`src/db/schema.rs:6-12`), and
`upsert_conversation` conflicts on that pair (`src/db/queries.rs:18`).
Since `title`/`thread_path` are the conflict key rather than fields the
`DO UPDATE` touches, they're implicitly always equal to what's already
stored on conflict — there's no overwrite to blank them out. Only
`is_still_participant` and `message_count` are still updated on conflict.

Note this does mean two files for the same conversation must agree on
`title` *and* `thread_path` exactly (including both being present or
both absent) to be recognized as the same conversation — if a page ever
reports a `title` of `None` while another reports `Some("...")` for the
same thread, they'd be treated as two different conversations instead of
one, and (now that #1 is fixed and every file goes through
`upsert_conversation`) that would show up as a duplicate `conversations`
row with a split `message_count`. Not observed in practice, but worth
knowing if that ever comes up.

## Gaps (parsed but not persisted)

### 6. Reactions are never written to the database

`RawReaction` (`src/ingest/parse.rs:94-97`) is fully parsed off of each
message, and the `reactions` table exists in the schema
(`src/db/schema.rs:45-52`), but there's no `insert_reaction` query and
`load_messages` never does anything with `message.reactions`. Reaction
data is silently dropped during import.

### 7. `messages_fts` is never populated during import

`populate_messages_fts` (`src/db/schema.rs:90-96`) exists and is tested,
but nothing in `ingest::loader` calls it. As it stands, after a fresh
import the `messages_fts` table stays empty and full-text search over
message content will return nothing until something remembers to call
`populate_messages_fts` manually.

## Robustness

### 8. `migrate` doesn't roll back a failed migration

`migrate` (`src/db/schema.rs:77-88`) runs `BEGIN;`, then the migration
SQL, then bumps `user_version`, then `COMMIT;`, all as separate
`execute_batch`/`pragma_update` calls. If the migration SQL batch fails
partway through, the `?` returns immediately — `COMMIT;` (and any
rollback) never runs, leaving the connection sitting inside an open
transaction. Any subsequent write on that connection (including a retry
of `migrate`) would then fail. Only matters if a future migration is
buggy or partially applies, but there's currently no recovery path if it
happens.

### 9. `repair_mojibake` assumes exports never contain legitimately-correct non-ASCII text

`repair_mojibake` (`src/ingest/parse.rs:15-42`) is applied unconditionally
to every non-ASCII string on the stated assumption that real Messenger
exports are always mojibake-corrupted, never already-correct UTF-8. Its
fallback (bail out on an out-of-Latin-1 `char` or invalid UTF-8 once
reinterpreted) catches most accidental misfires, but a string that is
already correct, entirely within the Latin-1 range, and happens to
reinterpret as valid UTF-8 would be silently mis-repaired. Low practical
risk given the source data, but worth remembering if this logic is ever
reused against a different/cleaner data source.
