<script lang="ts">
	import { defaultImportName } from '$lib/importName';

	interface Props {
		sourcePath: string;
		takenNames: string[];
		onconfirm: (name: string) => void;
		oncancel: () => void;
	}

	let { sourcePath, takenNames, onconfirm, oncancel }: Props = $props();

	let name = $state(defaultImportName());

	let trimmed = $derived(name.trim());
	// Case-insensitive, matching the check `start_import` will run again for
	// real. This one only exists to warn before the user waits on an import.
	let taken = $derived(
		takenNames.some((existing) => existing.toLowerCase() === trimmed.toLowerCase())
	);
	let canConfirm = $derived(trimmed.length > 0 && !taken);

	function onsubmit(event: Event) {
		event.preventDefault();
		if (canConfirm) {
			onconfirm(trimmed);
		}
	}
</script>

<form {onsubmit}>
	<h2>Name this import</h2>
	<p class="source">{sourcePath}</p>

	<label for="import-name">Name</label>
	<input id="import-name" bind:value={name} autocomplete="off" />

	{#if taken}
		<p class="warning" role="alert">An import named “{trimmed}” already exists.</p>
	{/if}

	<div class="actions">
		<button type="button" onclick={oncancel}>Cancel</button>
		<button type="submit" disabled={!canConfirm}>Import</button>
	</div>
</form>

<style>
	form {
		display: flex;
		width: 100%;
		max-width: 26rem;
		flex-direction: column;
		gap: 0.5rem;
	}

	h2 {
		margin: 0;
		font-size: 1.1rem;
	}

	.source {
		overflow-wrap: anywhere;
		margin: 0;
		color: var(--text-muted);
		font-size: 0.85rem;
	}

	label {
		margin-top: 0.5rem;
		color: var(--text-muted);
		font-size: 0.85rem;
	}

	input {
		border: 1px solid var(--divider);
		border-radius: 8px;
		padding: 0.6rem 0.75rem;
		font: inherit;
		color: var(--text);
		background-color: var(--surface-raised);
	}

	input:focus-visible {
		border-color: var(--accent);
		outline: none;
	}

	.warning {
		margin: 0;
		color: var(--danger);
		font-size: 0.85rem;
	}

	.actions {
		display: flex;
		justify-content: flex-end;
		gap: 0.5rem;
		margin-top: 0.5rem;
	}

	button {
		border: none;
		border-radius: 8px;
		padding: 0.6rem 1.25rem;
		font: inherit;
		font-weight: 600;
		color: var(--text);
		background-color: var(--surface-hover);
		cursor: pointer;
	}

	button[type='submit'] {
		color: var(--on-accent);
		background-color: var(--accent);
	}

	button[type='submit']:disabled {
		cursor: not-allowed;
		opacity: 0.5;
	}

	button:focus-visible {
		outline: 2px solid var(--accent-hover);
		outline-offset: 2px;
	}
</style>
