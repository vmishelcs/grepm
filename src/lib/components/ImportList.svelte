<script lang="ts">
	import type { ImportEntry } from '$lib/ipc';

	interface Props {
		imports: ImportEntry[];
		onopen: (entry: ImportEntry) => void;
	}

	let { imports, onopen }: Props = $props();

	const numbers = new Intl.NumberFormat();
</script>

{#if imports.length === 0}
	<p class="empty">No imports yet.</p>
{:else}
	<ul class="grid">
		{#each imports as entry (entry.id)}
			<li>
				<button type="button" onclick={() => onopen(entry)}>
					<span class="title">{entry.name}</span>
					<span class="stats">
						<span>{numbers.format(entry.conversation_count)} conversations</span>
						<span>{numbers.format(entry.message_count)} messages</span>
					</span>
				</button>
			</li>
		{/each}
	</ul>
{/if}

<style>
	.empty {
		margin: 0;
		color: var(--text-muted);
		font-size: 0.9rem;
	}

	/* Scrolls within itself once the tiles outgrow the panel, so the window
	   never grows a scrollbar for a list that has somewhere to go. */
	.grid {
		display: grid;
		overflow-y: auto;
		margin: 0;
		padding: 0;
		gap: 0.75rem;
		grid-template-columns: repeat(auto-fill, minmax(13rem, 1fr));
		list-style: none;
	}

	button {
		display: flex;
		width: 100%;
		height: 100%;
		min-height: 5.5rem;
		flex-direction: column;
		justify-content: space-between;
		gap: 0.75rem;
		border: 1px solid var(--divider);
		border-radius: 8px;
		padding: 0.85rem 1rem;
		font: inherit;
		color: var(--text);
		background-color: var(--surface-raised);
		cursor: pointer;
		text-align: left;
		transition:
			background-color 0.15s ease,
			border-color 0.15s ease;
	}

	button:hover {
		border-color: var(--accent);
		background-color: var(--surface-hover);
	}

	button:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}

	.title {
		overflow-wrap: anywhere;
		font-weight: 600;
	}

	.stats {
		display: flex;
		flex-direction: column;
		color: var(--text-muted);
		font-size: 0.8rem;
		line-height: 1.35;
	}
</style>
