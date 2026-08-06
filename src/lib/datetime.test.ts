import { describe, expect, it } from 'vitest';

import { formatTimestamp } from './datetime';

/** 2021-05-12 16:34 UTC — an afternoon in London, a morning in California. */
const INSTANT = Date.UTC(2021, 4, 12, 16, 34);

describe('formatTimestamp', () => {
	it('renders the instant in the given zone', () => {
		expect(formatTimestamp(INSTANT, { locale: 'en-GB', timeZone: 'UTC' })).toBe(
			'12 May 2021, 16:34 UTC'
		);
	});

	it('moves the clock with the zone, not just the label', () => {
		const california = formatTimestamp(INSTANT, {
			locale: 'en-GB',
			timeZone: 'America/Los_Angeles'
		});

		// Same moment, a different wall clock — the whole point of formatting
		// locally rather than showing the raw UTC the engine stores.
		expect(california).toBe('12 May 2021, 09:34 GMT-7');
	});

	it('names the zone the way the reader’s locale does', () => {
		// `GMT-7` above, `PDT` here, for the same zone and instant: the
		// abbreviation comes from the locale's data, not the timezone. Both are
		// correct for the reader who gets them, which is why the app passes no
		// locale at all and takes the machine's.
		expect(formatTimestamp(INSTANT, { locale: 'en-US', timeZone: 'America/Los_Angeles' })).toBe(
			'May 12, 2021, 09:34 PDT'
		);
	});

	it('renders midnight as 00, not 24', () => {
		const midnight = Date.UTC(2021, 0, 1, 0, 0);

		expect(formatTimestamp(midnight, { locale: 'en-GB', timeZone: 'UTC' })).toContain('00:00');
	});
});
