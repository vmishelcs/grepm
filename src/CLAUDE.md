# Front end

SvelteKit 5 + TypeScript, running as a static SPA inside a Tauri webview.
Read this before editing anything under `src/`.

## 1. Never `{@html}` a string that came from the export

`SearchHit.snippet` is message text a stranger wrote years ago, carrying
`MATCH_START`/`MATCH_END` (U+FFF9/U+FFFB) around each matched term. Turning
those markers into `<mark>` looks like the whole job, and the obvious version
is an exploit:

```svelte
<!-- NO. A message containing <img src=x onerror=...> gets script execution,
     in a context that reaches the Tauri IPC bridge. -->
{@html snippet.replaceAll(MATCH_START, '<mark>').replaceAll(MATCH_END, '</mark>')}
```

Split on the sentinels and let Svelte render the pieces as text nodes, so no
raw HTML is ever inserted:

```svelte
{#each splitOnMarkers(snippet) as part}
	{#if part.matched}<mark>{part.text}</mark>{:else}{part.text}{/if}
{/each}
```

Escaping first and substituting second is also safe; substituting first is
not. This is why the sentinels are non-ASCII format characters rather than
brackets — the split stays unambiguous even when the message contains angle
brackets or quotes.

`conversation_title` and `sender_name` are export-derived too. Same rule.

(Review finding **A8**. Sentinels are defined in `core/src/search/mod.rs` and
mirrored in `$lib/ipc/types.ts`.)

## 2. This is an SPA — there is no server

`+layout.ts` sets `ssr = false` and the adapter is `adapter-static` with
`fallback: "index.html"`. So: no `+page.server.ts`, no `+server.ts`, no form
actions, no `$app/server`, no `fetch` to an API. Data comes from Rust over
Tauri IPC, and only from there.

## 3. Runes only

Svelte 5: `$state`, `$derived`, `$props`. No `export let`, no `$:`, no
`svelte/store` in new code. `$effect` is an escape hatch for syncing with
something outside Svelte — never for deriving a value from other state; use
`$derived`. Large objects that are replaced rather than mutated (search
results, for one) belong in `$state.raw`.

The `svelte-core-bestpractices` skill covers this in depth, and the Svelte MCP
server's `svelte-autofixer` will catch violations in code you have written but
not yet saved. Use it.

## 4. All IPC goes through `$lib/ipc`

Components never import `@tauri-apps/api/core`. `src/lib/ipc/index.ts` is the
only file that calls `invoke`, and it only wraps commands that actually exist
in `src-tauri/src/lib.rs`. This mirrors the `SearchIndex` rule on the engine
side: one boundary, one place to stub in tests.

## 5. The type mirrors are hand-maintained

`src/lib/ipc/types.ts` mirrors the `Serialize` types in
`core/src/search/mod.rs`. There is no codegen — **change one, change the other
in the same commit.** Field names are snake_case (serde's default) and Rust's
`Option<T>` arrives as `T | null`, not as a missing key.

## 6. Results are paged

A search returns `{ hits, count, page }` where `count` is the total number of
matches, not `hits.length`. Drive pagination from `count`; never try to load
every hit. The default page is 32.

## 7. Definition of done

```sh
npm run verify   # svelte-check, then prettier + eslint, then vitest
```

Tests live next to what they test. `*.svelte.test.ts` runs in real Chromium
(browser mode); plain `*.test.ts` runs in node. Tauri commands are stubbed with
`mockIPC` from `@tauri-apps/api/mocks` — see `src/routes/page.svelte.test.ts`.
Shared sample data lives in `src/lib/fixtures.ts`, and is the same data the
screenshot script uses.

For anything visual, run `npm run shoot` and look at the PNGs it writes to
`.screenshots/`. Don't guess at layout.
