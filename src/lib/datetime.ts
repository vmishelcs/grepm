/**
 * Rendering the engine's `i64` millisecond timestamps for a reader.
 *
 * Everything crossing the IPC boundary is epoch milliseconds — the engine has
 * no date library and no notion of a timezone. Turning that into a local time
 * is the front end's job, and the webview is the right place for it: it
 * already knows the machine's zone and the user's locale, where Rust would
 * need a date crate and a bundled tz database to say the same thing.
 */

/**
 * `12 May 2021, 09:34 PDT` — the instant as it read where the user is sitting.
 *
 * The zone abbreviation is shown rather than implied: a Messenger export spans
 * years, and a reader who has moved timezones deserves to know which one a
 * timestamp is being expressed in.
 *
 * `locale` and `timeZone` exist so tests can pin both rather than depending on
 * wherever they happen to run; leaving them out uses the machine's own.
 */
export function formatTimestamp(
	ms: number,
	options: { locale?: string; timeZone?: string } = {}
): string {
	return new Intl.DateTimeFormat(options.locale, {
		year: 'numeric',
		month: 'short',
		day: 'numeric',
		hour: '2-digit',
		minute: '2-digit',
		// `hour12: false` renders midnight as 24 in some implementations.
		hourCycle: 'h23',
		timeZoneName: 'short',
		timeZone: options.timeZone
	}).format(new Date(ms));
}
