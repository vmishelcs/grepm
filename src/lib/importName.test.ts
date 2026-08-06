import { describe, expect, it } from 'vitest';

import { defaultImportName } from './importName';

describe('defaultImportName', () => {
	it('renders the local date, time and zone', () => {
		const instant = new Date(Date.UTC(2026, 7, 6, 18, 42));

		expect(defaultImportName(instant, 'America/Los_Angeles')).toBe('2026-08-06 11:42 PDT');
	});

	it('follows the zone rather than the instant', () => {
		const instant = new Date(Date.UTC(2026, 7, 6, 18, 42));

		// Same moment, named differently depending on where you're standing —
		// which is why the zone is part of the name and not left implicit.
		expect(defaultImportName(instant, 'UTC')).toBe('2026-08-06 18:42 UTC');
	});

	it('renders midnight as 00, not 24', () => {
		const midnight = new Date(Date.UTC(2026, 0, 1, 0, 0));

		expect(defaultImportName(midnight, 'UTC')).toBe('2026-01-01 00:00 UTC');
	});
});
