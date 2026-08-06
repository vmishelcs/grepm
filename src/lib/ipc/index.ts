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

export * from './types';

/** Wraps the `greet` command. Scaffold — replace when real commands land. */
export function greet(name: string): Promise<string> {
	return invoke('greet', { name });
}
