/**
 * The name an import gets when the user doesn't type one.
 *
 * Built in the browser rather than in Rust on purpose: the webview already
 * knows the user's timezone and how to abbreviate it, while Rust would need a
 * date crate and a bundled tz database to say the same thing. The name is a
 * frozen label from the moment it's chosen — it is never reformatted later, so
 * it stays truthful even if the machine moves timezone.
 */

/**
 * `2026-08-06 11:42 PDT` — sortable date first, then local time and its zone.
 *
 * `timeZone` exists so tests can pin a zone rather than depending on wherever
 * they happen to run.
 */
export function defaultImportName(now: Date = new Date(), timeZone?: string): string {
	const parts = new Intl.DateTimeFormat('en-CA', {
		year: 'numeric',
		month: '2-digit',
		day: '2-digit',
		hour: '2-digit',
		minute: '2-digit',
		// `hour12: false` renders midnight as 24 in some implementations.
		hourCycle: 'h23',
		timeZoneName: 'short',
		timeZone
	}).formatToParts(now);

	const value = (type: Intl.DateTimeFormatPartTypes) =>
		parts.find((part) => part.type === type)?.value ?? '';

	const date = `${value('year')}-${value('month')}-${value('day')}`;
	return `${date} ${value('hour')}:${value('minute')} ${value('timeZoneName')}`;
}
