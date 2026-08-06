<script lang="ts">
	import type { ImportProgress } from '$lib/ipc';

	interface Props {
		name: string;
		progress: ImportProgress | null;
	}

	let { name, progress }: Props = $props();

	const numbers = new Intl.NumberFormat();
</script>

<div class="importing">
	<h2>Importing {name}</h2>

	{#if progress && progress.total > 0}
		<progress value={progress.done} max={progress.total}></progress>
		<p>
			{numbers.format(progress.done)} of {numbers.format(progress.total)} conversations
		</p>
	{:else}
		<!-- No value: an indeterminate bar, until the first report arrives with
		     the total. -->
		<progress></progress>
		<p>Looking through the export…</p>
	{/if}
</div>

<style>
	.importing {
		display: flex;
		width: 100%;
		max-width: 26rem;
		flex-direction: column;
		gap: 0.75rem;
	}

	h2 {
		overflow-wrap: anywhere;
		margin: 0;
		font-size: 1.1rem;
	}

	progress {
		width: 100%;
		height: 0.5rem;
		border: none;
		border-radius: 999px;
		background-color: var(--surface-hover);
		-webkit-appearance: none;
		appearance: none;
	}

	progress::-webkit-progress-bar {
		border-radius: 999px;
		background-color: var(--surface-hover);
	}

	progress::-webkit-progress-value {
		border-radius: 999px;
		background-color: var(--accent);
	}

	progress::-moz-progress-bar {
		border-radius: 999px;
		background-color: var(--accent);
	}

	p {
		margin: 0;
		color: var(--text-muted);
		font-size: 0.85rem;
	}
</style>
