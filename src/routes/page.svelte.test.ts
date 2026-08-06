/**
 * The launch screen: import something new, or open something imported before.
 *
 * Tauri commands are stubbed with `mockIPC`, which also catches the dialog
 * plugin — it reaches Rust over the same bridge, as `plugin:dialog|open`.
 */
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, describe, expect, it, vi } from 'vitest';
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
		await expect.element(page.getByRole('button', { name: /^Work chats/ })).toBeInTheDocument();

		// A full-height screen whose padding is added *outside* its height
		// overflows by exactly that padding, and the window grows a scrollbar
		// with nothing to scroll. Content that genuinely overflows still may.
		const root = document.documentElement;
		expect(root.scrollHeight).toBeLessThanOrEqual(root.clientHeight);
	});

	it('lists previous imports with what each one holds', async () => {
		stubCommands({ list_imports: sampleImports });

		render(Page);

		await expect.element(page.getByRole('button', { name: /^Work chats/ })).toBeInTheDocument();
		await expect
			.element(page.getByRole('button', { name: /^2026-05-28 09:15 PDT/ }))
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

	it('hides Delete Import behind the ... button', async () => {
		stubCommands({ list_imports: sampleImports });

		render(Page);
		const more = page.getByRole('button', { name: 'More options for Work chats' });
		await expect.element(more).toBeInTheDocument();

		// Nothing destructive is one stray click away.
		expect(page.getByRole('menuitem').elements()).toHaveLength(0);
		await more.click();

		await expect.element(page.getByRole('menuitem', { name: 'Delete Import' })).toBeInTheDocument();
	});

	it('closes the menu when the ... button is clicked again', async () => {
		stubCommands({ list_imports: sampleImports });

		render(Page);
		const more = page.getByRole('button', { name: 'More options for Work chats' });
		await expect.element(more).toBeInTheDocument();

		await more.click();
		await expect.element(page.getByRole('menuitem', { name: 'Delete Import' })).toBeInTheDocument();
		await more.click();

		await vi.waitFor(() => expect(page.getByRole('menuitem').elements()).toHaveLength(0));
	});

	it('opening the menu does not open the import', async () => {
		const calls: string[] = [];
		mockIPC((command) => {
			calls.push(command);
			if (command === 'list_imports') return sampleImports;
			throw new Error(`unexpected command: ${command}`);
		});

		render(Page);
		const more = page.getByRole('button', { name: 'More options for Work chats' });
		await expect.element(more).toBeInTheDocument();
		await more.click();

		await expect.element(page.getByRole('menuitem', { name: 'Delete Import' })).toBeInTheDocument();
		expect(calls).not.toContain('open_import');
	});

	it('confirms before deleting, and does nothing if the user declines', async () => {
		const calls: string[] = [];
		mockIPC((command) => {
			calls.push(command);
			if (command === 'list_imports') return sampleImports;
			if (command === 'plugin:dialog|message') return 'Cancel';
			throw new Error(`unexpected command: ${command}`);
		});

		render(Page);
		const more = page.getByRole('button', { name: 'More options for Work chats' });
		await expect.element(more).toBeInTheDocument();
		await more.click();
		await page.getByRole('menuitem', { name: 'Delete Import' }).click();

		await vi.waitFor(() => expect(calls).toContain('plugin:dialog|message'));
		expect(calls).not.toContain('delete_import');
	});

	it('deletes the import the menu was opened on once confirmed', async () => {
		const deleted: string[] = [];
		mockIPC((command, args) => {
			// The plugin sends its labels as `{ OkCancelCustom: [ok, cancel] }`
			// and decides the answer by comparing the reply to the ok label —
			// so echoing that label back is what "confirmed" looks like here.
			if (command === 'plugin:dialog|message') {
				return (args as { buttons: { OkCancelCustom: [string, string] } }).buttons
					.OkCancelCustom[0];
			}
			if (command === 'delete_import') {
				deleted.push((args as { id: string }).id);
				return null;
			}
			if (command === 'list_imports') {
				return sampleImports.filter((entry) => !deleted.includes(entry.id));
			}
			throw new Error(`unexpected command: ${command}`);
		});

		render(Page);
		const more = page.getByRole('button', { name: 'More options for Work chats' });
		await expect.element(more).toBeInTheDocument();
		await more.click();
		await page.getByRole('menuitem', { name: 'Delete Import' }).click();

		await vi.waitFor(() => expect(deleted).toEqual([sampleImports[0].id]));
		// The list re-reads afterwards, so the tile goes without a reload.
		await expect.element(page.getByRole('button', { name: /^Work chats/ })).not.toBeInTheDocument();
		await expect
			.element(page.getByRole('button', { name: /^2026-05-28 09:15 PDT/ }))
			.toBeInTheDocument();
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
		await page.getByRole('button', { name: /^Work chats/ }).click();

		await expect.element(page.getByRole('alert')).toHaveTextContent(/Could not find export/);
	});
});
