<script lang="ts">
	import { resolve } from '$app/paths';
	import { describeError } from '$lib/errors';
	import { activeImport, type ActiveImportInfo } from '$lib/ipc';

	// Placeholder. The counts come from the *live* connection rather than from
	// the library index, so seeing them is proof the database really opened —
	// which is all this screen is for until search lands here.
	let info = $state.raw<ActiveImportInfo | null>(null);
	let error = $state<string | null>(null);
	let loaded = $state(false);

	void load();

	async function load() {
		try {
			info = await activeImport();
		} catch (err) {
			error = describeError(err);
		} finally {
			loaded = true;
		}
	}

	const numbers = new Intl.NumberFormat();
</script>

<main>
	{#if error}
		<p class="error" role="alert">{error}</p>
	{:else if info}
		<h1>{info.entry.name}</h1>
		<p class="counts">
			{numbers.format(info.stats.message_count)} messages across {numbers.format(
				info.stats.conversation_count
			)} conversations
		</p>
		<p class="note">Read from the open database. Search lands here next.</p>
	{:else if loaded}
		<p class="note">No import is open.</p>
	{/if}

	<a href={resolve('/')}>Back to imports</a>
</main>

<style>
	main {
		display: flex;
		min-height: 100dvh;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 0.5rem;
		padding: 1.5rem;
		text-align: center;
	}

	h1 {
		overflow-wrap: anywhere;
		margin: 0;
		font-size: 1.5rem;
	}

	.counts {
		margin: 0;
		color: var(--text);
	}

	.note {
		margin: 0;
		color: var(--text-muted);
		font-size: 0.85rem;
	}

	.error {
		margin: 0;
		color: var(--danger);
	}

	a {
		margin-top: 1rem;
		color: var(--accent);
		font-size: 0.9rem;
	}
</style>
