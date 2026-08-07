/**
 * The conversation sidebar is windowed: only about three screens of rows are
 * mounted at a time, with spacers standing in for the rest.
 *
 * Driven through the whole reader page rather than the list component alone,
 * because the thing being tested is a layout property — the list only windows
 * if something above it bounds its height, and that bound comes from the
 * reader's flex column.
 */
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';

import { manyConversations, sampleImports } from '$lib/fixtures';

import '../../app.css';

import Page from './+page.svelte';

const TOTAL = 1000;

function stubOpenImport() {
	const conversations = manyConversations(TOTAL);
	mockIPC((command) => {
		if (command === 'active_import') {
			return {
				entry: sampleImports[0],
				stats: { message_count: 99999, conversation_count: TOTAL }
			};
		}
		if (command === 'list_conversations') return conversations;
		throw new Error(`unexpected command: ${command}`);
	});
}

const rows = () => Array.from(document.querySelectorAll('li.row'));
const scroller = () => document.querySelector('ul') as HTMLUListElement;

/** Row pitch as the component computes it: box height plus its bottom margin. */
function rowPitch(): number {
	const row = rows()[0];
	return row.getBoundingClientRect().height + parseFloat(getComputedStyle(row).marginBottom);
}

/** Renders, then waits for the measure-and-rewindow pass to settle. */
async function renderSettled() {
	stubOpenImport();
	render(Page);
	await expect.element(page.getByText('Conversation 1', { exact: true })).toBeInTheDocument();
	await vi.waitFor(() => expect(rows().length).toBeLessThan(TOTAL / 2));
	return scroller();
}

async function scrollTo(list: HTMLUListElement, top: number) {
	list.scrollTop = top;
	list.dispatchEvent(new Event('scroll'));
}

describe('the windowed conversation list', () => {
	afterEach(clearMocks);

	it('mounts a few screens of rows, not the whole list', async () => {
		const list = await renderSettled();

		const perScreen = Math.ceil(list.clientHeight / rowPitch());
		// Bounded both ways on purpose. The upper bound is the point of
		// windowing; the lower bound is what makes this a test of *measurement*
		// — a hardcoded row count would satisfy one or the other, but only a
		// window sized from the real viewport satisfies both on any screen.
		expect(rows().length).toBeLessThanOrEqual(perScreen * 3 + 2);
		expect(rows().length).toBeGreaterThanOrEqual(perScreen * 3 - 2);
		expect(rows().length).toBeLessThan(TOTAL);
	});

	it('fills a taller pane with more rows', async () => {
		const list = await renderSettled();
		const before = rows().length;

		// Standing in for a taller window: the list is a flex child, so its own
		// height is what changes when the window grows, and a ResizeObserver on
		// it is what notices.
		list.style.flex = 'none';
		list.style.height = `${list.clientHeight * 2}px`;

		await vi.waitFor(() => expect(rows().length).toBeGreaterThan(before * 1.5));
	});

	it('keeps the scrollbar the length of the whole list', async () => {
		const list = await renderSettled();

		// Spacers stand in for every un-mounted row, so the scroll range covers
		// all thousand — not just the couple of dozen in the DOM.
		const expected = TOTAL * rowPitch() + 2 * parseFloat(getComputedStyle(list).paddingTop);
		expect(Math.abs(list.scrollHeight - expected)).toBeLessThan(2);
		expect(list.scrollHeight).toBeGreaterThan(list.clientHeight);
	});

	it('swaps in later rows as it scrolls down, and drops the earlier ones', async () => {
		const list = await renderSettled();

		await scrollTo(list, 400 * rowPitch());

		await vi.waitFor(() =>
			expect(page.getByText('Conversation 401', { exact: true }).elements()).toHaveLength(1)
		);
		expect(page.getByText('Conversation 1', { exact: true }).elements()).toHaveLength(0);
	});

	it('brings the earlier rows back when it scrolls up again', async () => {
		const list = await renderSettled();

		await scrollTo(list, 400 * rowPitch());
		await vi.waitFor(() =>
			expect(page.getByText('Conversation 401', { exact: true }).elements()).toHaveLength(1)
		);
		await scrollTo(list, 0);

		await vi.waitFor(() =>
			expect(page.getByText('Conversation 1', { exact: true }).elements()).toHaveLength(1)
		);
		expect(page.getByText('Conversation 401', { exact: true }).elements()).toHaveLength(0);
	});

	it('scrolls to the very end without running out of rows', async () => {
		const list = await renderSettled();

		await scrollTo(list, list.scrollHeight);

		await vi.waitFor(() =>
			expect(page.getByText(`Conversation ${TOTAL}`, { exact: true }).elements()).toHaveLength(1)
		);
	});

	it('gives every row the same height, N/A rows included', async () => {
		await renderSettled();

		// Every tenth fixture conversation has no messages, so the mounted
		// window contains both kinds. If they differed, every spacer below
		// would be wrong by the difference.
		await expect.element(page.getByText('Last message: N/A').first()).toBeInTheDocument();
		const heights = new Set(rows().map((row) => row.getBoundingClientRect().height));
		expect(heights.size).toBe(1);
	});

	it('reports the whole list to assistive tech, not the mounted slice', async () => {
		await renderSettled();

		const first = rows()[0];
		expect(first.getAttribute('aria-setsize')).toBe(String(TOTAL));
		expect(first.getAttribute('aria-posinset')).toBe('1');
	});

	it('still keeps the window itself from scrolling', async () => {
		await renderSettled();

		const root = document.documentElement;
		expect(root.scrollHeight).toBeLessThanOrEqual(root.clientHeight);
	});
});
