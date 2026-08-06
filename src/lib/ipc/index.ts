/**
 * The one place the front end talks to Rust.
 *
 * Components import from `$lib/ipc`, never from `@tauri-apps/api/core`
 * directly. This mirrors the `SearchIndex` trait on the engine side: one
 * boundary, so the wire format is described in exactly one file and tests can
 * stub it in exactly one place.
 *
 * Only commands that actually exist in `src-tauri/src/lib.rs` get a wrapper
 * here. Wrappers for commands that haven't been written yet are dead code.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { confirm, open } from '@tauri-apps/plugin-dialog';

import type { ActiveImportInfo, ImportEntry, ImportProgress } from './types';

export * from './types';

/** Every import in the library, newest last. Empty on a fresh install. */
export function listImports(): Promise<ImportEntry[]> {
	return invoke('list_imports');
}

/** The open import and its live row counts, or `null` if nothing is open. */
export function activeImport(): Promise<ActiveImportInfo | null> {
	return invoke('active_import');
}

/**
 * Opens an import's database and makes it the active one.
 *
 * Rejects with `import_file_missing` when the index still lists an import
 * whose file has been deleted behind the app's back.
 */
export function openImport(id: string): Promise<ImportEntry> {
	return invoke('open_import', { id });
}

/**
 * Imports a Messenger export into a new database and adds it to the library.
 *
 * Long-running — minutes, for a large export. Subscribe with
 * {@link onImportProgress} before awaiting this, or the early events are missed.
 */
export function startImport(sourcePath: string, name: string): Promise<ImportEntry> {
	return invoke('start_import', { sourcePath, name });
}

/**
 * Removes an import from the app: its database and its entry in the library.
 *
 * The Facebook export it was built from is not touched — that folder is the
 * user's own data and grepm only ever read it.
 */
export function deleteImport(id: string): Promise<void> {
	return invoke('delete_import', { id });
}

/**
 * A native modal confirmation for something irreversible. Resolves `true` only
 * if the user accepts.
 *
 * The wording is the caller's, not this layer's — this only owns the fact that
 * a confirmation is a round trip to Rust like everything else. `mockIPC` sees
 * it as `plugin:dialog|message`, which is what the plugin builds `confirm` on.
 */
export function confirmDestructive(message: string, okLabel: string): Promise<boolean> {
	return confirm(message, { title: 'grepm', kind: 'warning', okLabel });
}

/**
 * The native folder picker. Resolves `null` if the user cancels.
 *
 * Wrapped here rather than called from a component so the rule holds without
 * exception: components import from `$lib/ipc` and nowhere else. The dialog
 * plugin goes over the same IPC bridge, so `mockIPC` stubs it too — as the
 * command `plugin:dialog|open`.
 */
export async function pickExportFolder(): Promise<string | null> {
	const picked = await open({
		directory: true,
		multiple: false,
		title: 'Choose a Messenger export folder'
	});
	return typeof picked === 'string' ? picked : null;
}

/**
 * Subscribes to import progress. Resolves to the unsubscribe function — call
 * it once the import settles, or the next one reports to two listeners.
 */
export function onImportProgress(handler: (progress: ImportProgress) => void): Promise<UnlistenFn> {
	return listen<ImportProgress>('import:progress', (event) => handler(event.payload));
}
