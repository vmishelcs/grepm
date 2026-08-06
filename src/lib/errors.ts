/**
 * User-facing wording for the errors commands reject with.
 *
 * Kept out of `$lib/ipc/types`, which is a mechanical mirror of the Rust
 * types: the wire shape is the engine's business, the copy is the UI's. Rust's
 * `Display` strings are for logs, and never reach a screen.
 */
import type { AppError } from '$lib/ipc';

/**
 * Narrows whatever a rejected `invoke` threw. Anything else — a bridge
 * failure, a panic, a plain `Error` — is not an `AppError` and gets the
 * fallback message.
 */
export function isAppError(value: unknown): value is AppError {
	return (
		typeof value === 'object' && value !== null && typeof (value as AppError).kind === 'string'
	);
}

export function describeError(value: unknown): string {
	if (!isAppError(value)) {
		return 'Something went wrong.';
	}

	switch (value.kind) {
		case 'name_in_use':
			return `An import named “${value.name}” already exists.`;
		case 'empty_name':
			return 'Give this import a name.';
		case 'not_an_export':
			return "That folder doesn't look like a Messenger export. Pick the folder that contains “messages”.";
		case 'import_file_missing':
			return 'Could not find export. Its database file is no longer on disk.';
		case 'unknown_import':
			return 'Could not find export.';
		case 'unsupported_schema_version':
			return 'This import was made by a newer version of grepm. Update to open it.';
		case 'invalid_schema_version':
			return "This import's database is damaged and can't be opened.";
		case 'corrupt_index':
			return 'The library index is damaged, so your imports could not be listed.';
		case 'io':
		case 'parse':
		case 'database':
			return value.message;
	}
}
