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

That covers `@tauri-apps/api/event` and the dialog plugin too — both reach Rust
over the same bridge, so both are wrapped here rather than called from a
component. A useful consequence: `mockIPC` stubs the dialogs as well. The
folder picker is `plugin:dialog|open`; `confirm()` is **`plugin:dialog|message`**,
and it decides the answer by comparing the reply to the ok label it sent as
`{ buttons: { OkCancelCustom: [ok, cancel] } }` — so a stub that "confirms"
echoes that first label back.

## 5. The type mirrors are hand-maintained

`src/lib/ipc/types.ts` mirrors the `Serialize` types in
`core/src/search/mod.rs` and in `src-tauri/src/{library,commands,error}.rs`.
There is no codegen — **change one, change the other in the same commit.**
Field names are snake_case (serde's default) and Rust's `Option<T>` arrives as
`T | null`, not as a missing key.

`AppError` is serialized internally tagged, so a rejected `invoke` rejects with
`{ kind, ...fields }` rather than an `Error`. Narrow it with `$lib/errors`,
which is also where the user-facing wording lives — `types.ts` describes the
wire, not the copy.

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
`mockIPC` from `@tauri-apps/api/mocks`, which installs itself on `window` — so
a test that stubs a command has to be a `*.svelte.test.ts` even when it renders
no component:

```ts
afterEach(clearMocks);

mockIPC((command) => {
	if (command !== 'list_imports') throw new Error(`unexpected command: ${command}`);
	return sampleImports;
});
```

Throwing on unexpected commands is the point: a screen that quietly gained a
call would otherwise pass on a stub that never answers it.

Shared sample data lives in `src/lib/fixtures.ts`, and is the same data the
screenshot script uses.

For anything visual, run `npm run shoot` and look at the PNGs it writes to
`.screenshots/`. Don't guess at layout. Its `SHOTS` list takes an optional
`prepare` hook and an `imports` override, so states that aren't reachable by
navigation — the naming step, an empty library — get captured too. Add an entry
when you add a screen.

## 8. Colours come from `src/app.css`

The palette approximates Messenger's dark mode and lives in one place as custom
properties (`--surface`, `--accent`, `--text-muted`, `--danger`, …), imported
once by `+layout.svelte`. Use the tokens; don't hard-code hex in a component.

## 9. The screens

- `/` — the launch screen. `Import Data` centred above an `IMPORTS` panel of
  tiles, the shape MySQL Workbench uses. Each tile has a `...` button opening a
  menu with `Delete Import`. Three phases live here (browsing, naming,
  importing) as a `Phase` union rather than a pile of booleans.
- `/opened` — the reader. A sidebar of conversations beside a pane that so far
  only says nothing is selected. **Its rows are deliberately not buttons**: a
  `<button>` that does nothing promises a keyboard or screen-reader user
  something the screen can't keep. Make them interactive when selection lands.

Shared pieces are in `$lib/components`. Anything drawn — the `...` glyph, the
empty-pane bubble — is inline SVG, not a bitmap and not a font glyph: sharp at
any density, no asset to bundle, and it takes `fill` from the palette tokens.

## 10. Layout facts that cost time to rediscover

All three were measured in a real browser, not eyeballed. Re-measure rather
than reason about them from scratch.

- **The window must not scroll when there's nothing to scroll.** A full-height
  element with padding overflows by exactly that padding unless `box-sizing`
  is `border-box` — which `app.css` sets globally for this reason. Use
  `min-height: 100dvh` so genuinely tall content can still scroll, and let long
  lists scroll inside their own container. `page.svelte.test.ts` asserts this.
- **`font-weight` above 700 usually does nothing.** The system font stack ships
  regular and bold only, so 600–900 all render identically (measured: same
  string, same 89.7px at every weight), and `font-synthesis: none` rules out a
  faux-bold fallback. To thicken text, use `-webkit-text-stroke` — supported by
  every webview Tauri targets.
- **Centring a label needs `line-height: normal`.** With the `1.5` inherited
  from `:root` a button label sat 1.19px high, and at `1` it sat 0.8px high:
  half-leading is split evenly around a glyph box that isn't itself symmetric.
  At `normal` the line box *is* the glyph box, so padding centres it exactly —
  and that follows the font's metrics rather than a magic pixel nudge.
