# Known issues / future work

Notes on things spotted while building out `core`, in roughly the order
they'd bite someone. Nothing here is blocking, but each is worth a
deliberate decision before this data is relied on.

## Correctness

### 1. `message_count` isn't accumulated across a conversation's files

`upsert_conversation` (`src/db/queries.rs:5-33`) is designed to add each
call's message count onto the running total on conflict
(`message_count = message_count + excluded.message_count`), specifically
to handle a conversation split across multiple `message_N.json` files.

But `load_conversation` (`src/ingest/loader.rs:12-37`) only calls
`upsert_conversation` once, for `message_files[0]` (line 20). Every file
after the first only goes through `load_messages` (line 29-32), which
inserts message rows but never touches `conversations.message_count`. So
today `message_count` always equals just the first file's message count,
not the conversation's real total.

### 2. The messages dedup constraint doesn't catch `NULL == NULL`

`messages` has `UNIQUE (conversation_id, sender_id, timestamp_ms, content)`
(`src/db/schema.rs:36`), and both `sender_id` and `content` are nullable.
SQLite (like standard SQL) treats `NULL` as distinct from `NULL` in a
UNIQUE index, so two messages with the same `conversation_id`/
`timestamp_ms` where both also have `sender_id IS NULL` and/or
`content IS NULL` won't be recognized as duplicates and will both be
inserted. This mainly matters on re-import (e.g. re-running the importer
over the same export) — a message with no text (e.g. a reaction-only or
attachment-only message with `content: null`) or an unresolved sender
could get duplicated.

A fix would need an expression index using `COALESCE` to give NULLs a
comparable sentinel value, e.g.:

```sql
CREATE UNIQUE INDEX idx_messages_dedup ON messages (
    conversation_id,
    COALESCE(sender_id, -1),
    timestamp_ms,
    COALESCE(content, '')
);
```

### 3. Participants are deduped globally by exact name match

`participants.name` is `UNIQUE` (`src/db/schema.rs:17`), and
`insert_participant` (`src/db/queries.rs:42-49`) upserts on that name.
Facebook's export format gives no stable per-person ID, only a display
name, so this is the only signal available — but it means two different
real people who happen to share a display name (not unusual: "John
Smith" across unrelated threads) will silently be merged into a single
`participants` row. There's no way to detect or undo this from the export
data alone; worth deciding whether that's an acceptable tradeoff for a
personal search tool or whether participants need to be scoped per
conversation instead of deduped globally.

### 4. Participants that only appear in a later file never get linked

In `load_conversation`, only `first_file.participants` are inserted and
linked via `link_conversation_participant` (`src/ingest/loader.rs:22-25`).
Files after the first are only passed to `load_messages`, which inserts a
participant row for a message's *sender* but never calls
`link_conversation_participant`. In the (uncommon but possible) case where
a later page's `participants` list includes someone who isn't in the first
file and who never sends a message in that file either, they'd never be
linked to the conversation via `conversation_participants` at all.

### 5. Conversation metadata is unconditionally overwritten on conflict

`upsert_conversation`'s `DO UPDATE` unconditionally sets
`title`/`is_still_participant`/`thread_path` to whatever the incoming
file says (`src/db/queries.rs:19-22`), with no "prefer non-null" or
"prefer latest" merge logic. This is dormant today since only the first
file's data ever reaches `upsert_conversation` (see #1), but if that's
fixed by calling it per-file, a later file with an absent/null title
could blank out a title a previous file had set.

## Gaps (parsed but not persisted)

### 6. Reactions are never written to the database

`RawReaction` (`src/ingest/parse.rs:94-97`) is fully parsed off of each
message, and the `reactions` table exists in the schema
(`src/db/schema.rs:39-46`), but there's no `insert_reaction` query and
`load_messages` never does anything with `message.reactions`. Reaction
data is silently dropped during import.

### 7. `messages_fts` is never populated during import

`populate_messages_fts` (`src/db/schema.rs:84-90`) exists and is tested,
but nothing in `ingest::loader` calls it. As it stands, after a fresh
import the `messages_fts` table stays empty and full-text search over
message content will return nothing until something remembers to call
`populate_messages_fts` manually.

## Robustness

### 8. `migrate` doesn't roll back a failed migration

`migrate` (`src/db/schema.rs:71-82`) runs `BEGIN;`, then the migration
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
