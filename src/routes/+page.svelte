<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import ImportList from '$lib/components/ImportList.svelte';
	import ImportProgressView from '$lib/components/ImportProgress.svelte';
	import NameImport from '$lib/components/NameImport.svelte';
	import { describeError } from '$lib/errors';
	import {
		confirmDestructive,
		deleteImport,
		listImports,
		onImportProgress,
		openImport,
		pickExportFolder,
		startImport,
		type ImportEntry,
		type ImportProgress
	} from '$lib/ipc';

	type Phase =
		| { step: 'browsing' }
		| { step: 'naming'; sourcePath: string }
		| { step: 'importing'; name: string };

	let imports = $state.raw<ImportEntry[]>([]);
	let phase = $state.raw<Phase>({ step: 'browsing' });
	let progress = $state.raw<ImportProgress | null>(null);
	let error = $state<string | null>(null);

	let takenNames = $derived(imports.map((entry) => entry.name));

	// The library lives in the Rust process, so this is a one-off fetch, not
	// state derived from anything. Started at init rather than from an
	// `$effect`, which would imply a reactive trigger that doesn't exist.
	void refresh();

	async function refresh() {
		try {
			imports = await listImports();
		} catch (err) {
			error = describeError(err);
		}
	}

	async function chooseFolder() {
		error = null;
		try {
			const sourcePath = await pickExportFolder();
			if (sourcePath !== null) {
				phase = { step: 'naming', sourcePath };
			}
		} catch (err) {
			error = describeError(err);
		}
	}

	async function runImport(sourcePath: string, name: string) {
		error = null;
		progress = null;
		phase = { step: 'importing', name };

		// Subscribed before awaiting the command, or the leading report — the
		// one carrying the total — arrives before anyone is listening.
		const unlisten = await onImportProgress((next) => (progress = next));
		try {
			const entry = await startImport(sourcePath, name);
			await openImport(entry.id);
			await goto(resolve('/opened'));
		} catch (err) {
			error = describeError(err);
			phase = { step: 'browsing' };
			await refresh();
		} finally {
			unlisten();
		}
	}

	async function deleteEntry(entry: ImportEntry) {
		error = null;

		// Irreversible, and a right-click plus one more click is a short path to
		// losing an import that took minutes to build — so it asks first. The
		// wording says what is and isn't destroyed, because "delete" next to a
		// folder the user chose is otherwise an alarming thing to click.
		const confirmed = await confirmDestructive(
			`Delete “${entry.name}”?\n\nThis removes the imported data from the app. ` +
				`The Facebook export it was made from will not be deleted.`,
			'Delete'
		);
		if (!confirmed) {
			return;
		}

		try {
			await deleteImport(entry.id);
		} catch (err) {
			error = describeError(err);
		}
		// Either way: on success the row is gone, and on failure the list may
		// have moved on anyway.
		await refresh();
	}

	async function openEntry(entry: ImportEntry) {
		error = null;
		try {
			await openImport(entry.id);
			await goto(resolve('/opened'));
		} catch (err) {
			error = describeError(err);
			// The index can outlive the file it names, so re-read: a vanished
			// import shouldn't sit in the list waiting to fail again.
			await refresh();
		}
	}
</script>

<main>
	{#if phase.step === 'importing'}
		<div class="middle">
			<ImportProgressView name={phase.name} {progress} />
		</div>
	{:else if phase.step === 'naming'}
		{@const sourcePath = phase.sourcePath}
		<div class="middle">
			<NameImport
				{sourcePath}
				{takenNames}
				onconfirm={(name) => runImport(sourcePath, name)}
				oncancel={() => (phase = { step: 'browsing' })}
			/>
		</div>
	{:else}
		<div class="middle">
			<button class="primary" type="button" onclick={chooseFolder}>Import Data</button>
			{#if error}
				<p class="error" role="alert">{error}</p>
			{/if}
		</div>

		<section class="imports">
			<h2>Imports</h2>
			<ImportList {imports} onopen={openEntry} ondelete={deleteEntry} />
		</section>
	{/if}
</main>

<style>
	/* `min-height`, not `height`: the window should still scroll if it is ever
	   shorter than the content — just not when there is nothing to scroll. */
	main {
		display: flex;
		min-height: 100dvh;
		flex-direction: column;
	}

	/* Takes whatever the imports panel doesn't, so the button sits in the
	   middle of the space left over rather than the middle of the window. */
	.middle {
		display: flex;
		flex: 1;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 1rem;
		padding: 1.5rem;
	}

	.primary {
		border: none;
		border-radius: 8px;
		padding: 0.875rem 2rem;
		font: inherit;
		font-weight: 600;
		color: var(--on-accent);
		background-color: var(--accent);
		cursor: pointer;
		transition: background-color 0.15s ease;
	}

	.primary:hover {
		background-color: var(--accent-hover);
	}

	.primary:active {
		background-color: var(--accent-active);
	}

	.primary:focus-visible {
		outline: 2px solid var(--accent-hover);
		outline-offset: 2px;
	}

	.imports {
		display: flex;
		max-height: 55dvh;
		min-height: 0;
		flex-direction: column;
		gap: 0.75rem;
		border-top: 1px solid var(--divider);
		padding: 1.25rem 1.5rem;
	}

	h2 {
		margin: 0;
		color: var(--text-muted);
		font-size: 0.75rem;
		font-weight: 600;
		letter-spacing: 0.08em;
		text-transform: uppercase;
	}

	.error {
		max-width: 26rem;
		margin: 0;
		color: var(--danger);
		font-size: 0.9rem;
		text-align: center;
	}
</style>
