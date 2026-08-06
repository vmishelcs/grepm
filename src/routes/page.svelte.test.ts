/**
 * The launch screen: import something new, or open something imported before.
 *
 * Tauri commands are stubbed with `mockIPC`, which also catches the dialog
 * plugin — it reaches Rust over the same bridge, as `plugin:dialog|open`.
 */
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';

import { sampleImports } from '$lib/fixtures';

// Normally pulled in by `+layout.svelte`, which these tests don't render. The
// layout assertion below depends on its `box-sizing` reset, so without it the
// test would measure a page the app never actually shows.
import '../app.css';

import Page from './+page.svelte';

/** Answers the commands named, and fails loudly on anything else. */
function stubCommands(responses: Record<string, unknown>) {
	mockIPC((command) => {
		if (command in responses) {
			return responses[command];
		}
		throw new Error(`unexpected command: ${command}`);
	});
}

describe('the launch screen', () => {
	afterEach(clearMocks);

	it('offers the import and says the library is empty', async () => {
		stubCommands({ list_imports: [] });

		render(Page);

		await expect.element(page.getByRole('button', { name: 'Import Data' })).toBeInTheDocument();
		await expect.element(page.getByText('No imports yet.')).toBeInTheDocument();
	});

	it('fills the window without overflowing it', async () => {
		stubCommands({ list_imports: sampleImports });

		render(Page);
		await expect.element(page.getByRole('button', { name: /Work chats/ })).toBeInTheDocument();

		// A full-height screen whose padding is added *outside* its height
		// overflows by exactly that padding, and the window grows a scrollbar
		// with nothing to scroll. Content that genuinely overflows still may.
		const root = document.documentElement;
		expect(root.scrollHeight).toBeLessThanOrEqual(root.clientHeight);
	});

	it('lists previous imports with what each one holds', async () => {
		stubCommands({ list_imports: sampleImports });

		render(Page);

		await expect.element(page.getByRole('button', { name: /Work chats/ })).toBeInTheDocument();
		await expect
			.element(page.getByRole('button', { name: /2026-05-28 09:15 PDT/ }))
			.toBeInTheDocument();
		await expect
			.element(page.getByRole('button', { name: /47 conversations/ }))
			.toBeInTheDocument();
	});

	it('pre-fills a name once a folder is chosen', async () => {
		stubCommands({ list_imports: [], 'plugin:dialog|open': '/home/vm/Downloads/export' });

		render(Page);
		await page.getByRole('button', { name: 'Import Data' }).click();

		await expect.element(page.getByText('/home/vm/Downloads/export')).toBeInTheDocument();
		// The default is a timestamp, so pin its shape rather than its value.
		await expect
			.element(page.getByLabelText('Name'))
			.toHaveValue(expect.stringMatching(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2} .+$/));
	});

	it('refuses a name already in use, ignoring case', async () => {
		stubCommands({
			list_imports: sampleImports,
			'plugin:dialog|open': '/home/vm/Downloads/export'
		});

		render(Page);
		await page.getByRole('button', { name: 'Import Data' }).click();
		await page.getByLabelText('Name').fill('  work CHATS  ');

		await expect.element(page.getByRole('alert')).toBeInTheDocument();
		await expect.element(page.getByRole('button', { name: 'Import' })).toBeDisabled();
	});

	it('accepts a name nothing else is using', async () => {
		stubCommands({
			list_imports: sampleImports,
			'plugin:dialog|open': '/home/vm/Downloads/export'
		});

		render(Page);
		await page.getByRole('button', { name: 'Import Data' }).click();
		await page.getByLabelText('Name').fill('Family');

		await expect.element(page.getByRole('button', { name: 'Import' })).toBeEnabled();
	});

	it('reports an import whose file has gone missing', async () => {
		stubCommands({ list_imports: sampleImports });
		// `open_import` has to reject rather than return, so it needs its own
		// stub — the shared helper only resolves.
		mockIPC((command) => {
			if (command === 'list_imports') return sampleImports;
			if (command === 'open_import') {
				throw { kind: 'import_file_missing', name: 'Work chats' };
			}
			throw new Error(`unexpected command: ${command}`);
		});

		render(Page);
		await page.getByRole('button', { name: /Work chats/ }).click();

		await expect.element(page.getByRole('alert')).toHaveTextContent(/Could not find export/);
	});
});
