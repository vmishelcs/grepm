/**
 * The reader: a sidebar of conversations, and a main pane that so far only
 * says nothing is selected.
 */
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';

import { sampleConversations, sampleImports } from '$lib/fixtures';

import '../../app.css';

import Page from './+page.svelte';

const activeInfo = {
	entry: sampleImports[0],
	stats: { message_count: 12431, conversation_count: 47 }
};

function stubOpenImport(conversations = sampleConversations) {
	mockIPC((command) => {
		if (command === 'active_import') return activeInfo;
		if (command === 'list_conversations') return conversations;
		throw new Error(`unexpected command: ${command}`);
	});
}

describe('the reader', () => {
	afterEach(clearMocks);

	it('prompts for a selection when nothing is selected', async () => {
		stubOpenImport();

		render(Page);

		await expect.element(page.getByText('Select a conversation to read')).toBeInTheDocument();
		await expect.element(page.getByText('Nothing is selected')).toBeInTheDocument();
	});

	it('lists each conversation with its participants and message count', async () => {
		stubOpenImport();

		render(Page);

		await expect.element(page.getByText('Weekend plans')).toBeInTheDocument();
		await expect
			.element(page.getByText('2 Participants: Ada Lovelace, Grace Hopper'))
			.toBeInTheDocument();
		await expect.element(page.getByText('1,842 messages')).toBeInTheDocument();
	});

	it('shows when the last message was sent, in local time', async () => {
		stubOpenImport();

		render(Page);

		// The fixture's newest message is 2021-05-12 09:34 UTC. Asserting the
		// browser's own rendering of that instant rather than a fixed string,
		// since the test runner's zone is not the fixture author's.
		const expected = new Intl.DateTimeFormat(undefined, {
			year: 'numeric',
			month: 'short',
			day: 'numeric',
			hour: '2-digit',
			minute: '2-digit',
			hourCycle: 'h23',
			timeZoneName: 'short'
		}).format(new Date(sampleConversations[0].last_message_ms!));

		await expect.element(page.getByText(`Last message: ${expected}`)).toBeInTheDocument();
	});

	it('says N/A rather than dropping the line when there are no messages', async () => {
		stubOpenImport([{ ...sampleConversations[0], message_count: 0, last_message_ms: null }]);

		render(Page);

		await expect.element(page.getByText('0 messages')).toBeInTheDocument();
		// Kept rather than omitted so every row is the same height — the
		// windowed list sizes its spacers from a single measured row.
		await expect.element(page.getByText('Last message: N/A')).toBeInTheDocument();
	});

	it('says Participant, singular, for a conversation with one', async () => {
		stubOpenImport();

		render(Page);

		await expect.element(page.getByText('1 Participant: Margaret Hamilton')).toBeInTheDocument();
	});

	it('renders a title containing markup as text', async () => {
		stubOpenImport();

		render(Page);

		// `<b>Book club</b>` is a title a stranger wrote. Finding it verbatim
		// proves it became a text node rather than an element.
		const title = page.getByText('<b>Book club</b>');
		await expect.element(title).toBeInTheDocument();
		expect((await title.element()).querySelector('b')).toBeNull();
	});

	it('keeps a long participant list on one line', async () => {
		stubOpenImport();

		render(Page);

		const label = page.getByText(/^7 Participants:/);
		await expect.element(label).toBeInTheDocument();

		// Clipped rather than wrapped, or one busy group chat would set the
		// width of the whole sidebar.
		const element = await label.element();
		const style = getComputedStyle(element);
		expect(style.whiteSpace).toBe('nowrap');
		expect(style.textOverflow).toBe('ellipsis');
		expect(element.scrollWidth).toBeGreaterThan(element.clientWidth);
	});

	it('holds the conversation list inside the window', async () => {
		stubOpenImport();

		render(Page);
		await expect.element(page.getByText('Weekend plans')).toBeInTheDocument();

		const root = document.documentElement;
		expect(root.scrollHeight).toBeLessThanOrEqual(root.clientHeight);
	});

	it('says so when no import is open', async () => {
		mockIPC((command) => {
			if (command === 'active_import') return null;
			if (command === 'list_conversations') return [];
			throw new Error(`unexpected command: ${command}`);
		});

		render(Page);

		await expect.element(page.getByText('No import is open.')).toBeInTheDocument();
	});
});
