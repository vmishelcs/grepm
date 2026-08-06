/**
 * Smoke test for the browser-mode harness itself: component rendering, real
 * user events, and a stubbed Tauri command in one pass.
 *
 * The page under test is still the Tauri scaffold. This test exists to prove
 * the toolchain works and to show the `mockIPC` pattern — delete it along with
 * the scaffold page when the real UI lands.
 */
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { afterEach, describe, expect, it } from 'vitest';
import { page } from 'vitest/browser';
import { render } from 'vitest-browser-svelte';

import Page from './+page.svelte';

describe('the scaffold page', () => {
	afterEach(clearMocks);

	it('renders what the greet command returns', async () => {
		mockIPC((command, args) => {
			if (command !== 'greet') {
				throw new Error(`unexpected command: ${command}`);
			}
			return `Hello, ${(args as { name: string }).name}!`;
		});

		render(Page);
		await page.getByPlaceholder('Enter a name...').fill('Ada');
		await page.getByRole('button', { name: 'Greet' }).click();

		await expect.element(page.getByText('Hello, Ada!')).toBeInTheDocument();
	});
});
