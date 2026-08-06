/**
 * Fake data, shared by the vitest suite and `scripts/shoot.mjs`.
 *
 * One source of sample data, so a screenshot and a test are looking at the
 * same thing. Deliberately includes the two cases that break naive rendering:
 * a message containing markup, and a message containing the match sentinels.
 */
import {
	MATCH_END,
	MATCH_START,
	type ImportEntry,
	type SearchHit,
	type SearchResults
} from './ipc/types';

/** A library with something in it, for the launch screen's list. */
export const sampleImports: ImportEntry[] = [
	{
		id: '1786041256217',
		name: 'Work chats',
		created_at_ms: Date.UTC(2026, 7, 6, 18, 42),
		source_path: '/home/vm/Downloads/facebook-export',
		message_count: 12431,
		conversation_count: 47
	},
	{
		// Left with the default name, to show what one looks like in the list.
		id: '1780329300000',
		name: '2026-05-28 09:15 PDT',
		created_at_ms: Date.UTC(2026, 4, 28, 16, 15),
		source_path: '/media/backup/fb-archive-2019',
		message_count: 512,
		conversation_count: 6
	}
];

/** Marks `text` the way `snippet()` does on the Rust side. */
function matched(text: string): string {
	return `${MATCH_START}${text}${MATCH_END}`;
}

export const sampleHits: SearchHit[] = [
	{
		message_id: 1,
		conversation_id: 10,
		conversation_title: 'Weekend plans',
		sender_name: 'Ada Lovelace',
		timestamp_ms: Date.UTC(2019, 4, 12, 9, 30),
		snippet: `...grab a ${matched('coffee')} before the train?`
	},
	{
		// Accented content: proof the diacritic-insensitive index round-trips.
		message_id: 2,
		conversation_id: 10,
		conversation_title: 'Weekend plans',
		sender_name: 'Ada Lovelace',
		timestamp_ms: Date.UTC(2019, 4, 12, 9, 34),
		snippet: `the ${matched('café')} on the corner...`
	},
	{
		// The injection case. Rendering this through `{@html}` executes it.
		message_id: 3,
		conversation_id: 11,
		conversation_title: '<b>Book club</b>',
		sender_name: '<img src=x onerror=alert(1)>',
		timestamp_ms: Date.UTC(2020, 10, 3, 18, 5),
		snippet: `check out <img src=x onerror=alert(1)> this ${matched('link')}...`
	},
	{
		// A deleted account shows up under Facebook's literal placeholder.
		message_id: 4,
		conversation_id: 12,
		conversation_title: null,
		sender_name: 'Facebook User',
		timestamp_ms: Date.UTC(2021, 1, 28, 22, 47),
		snippet: `...see you at the ${matched('coffee')} place`
	}
];

/** A full first page, with a `count` larger than `hits` so paging is live. */
export const sampleResults: SearchResults = {
	hits: sampleHits,
	count: 137,
	page: { limit: 32, offset: 0 }
};
