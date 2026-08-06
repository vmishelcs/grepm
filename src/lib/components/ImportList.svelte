<script lang="ts">
	import type { ImportEntry } from '$lib/ipc';

	interface Props {
		imports: ImportEntry[];
		onopen: (entry: ImportEntry) => void;
		ondelete: (entry: ImportEntry) => void;
	}

	let { imports, onopen, ondelete }: Props = $props();

	const numbers = new Intl.NumberFormat();

	/** Must match the menu's size in the stylesheet below, for clamping. */
	const MENU_WIDTH = 180;
	const MENU_HEIGHT = 44;
	const MARGIN = 8;

	let menu = $state.raw<{ entry: ImportEntry; x: number; y: number } | null>(null);

	function toggleMenu(event: MouseEvent, entry: ImportEntry) {
		// Without this the click carries on to the window handler below, which
		// would close the menu in the same tick it was opened.
		event.stopPropagation();

		if (menu?.entry.id === entry.id) {
			menu = null;
			return;
		}

		// Anchored to the button's right edge and clamped inside the window, so
		// a tile near an edge doesn't push the page wider.
		const button = (event.currentTarget as HTMLElement).getBoundingClientRect();
		menu = {
			entry,
			x: Math.max(
				MARGIN,
				Math.min(button.right - MENU_WIDTH, window.innerWidth - MENU_WIDTH - MARGIN)
			),
			y: Math.min(button.bottom + 4, window.innerHeight - MENU_HEIGHT - MARGIN)
		};
	}

	function chooseDelete(entry: ImportEntry) {
		menu = null;
		ondelete(entry);
	}
</script>

<svelte:window
	onclick={() => (menu = null)}
	onresize={() => (menu = null)}
	onkeydown={(event) => {
		if (event.key === 'Escape') menu = null;
	}}
/>

{#if imports.length === 0}
	<p class="empty">No imports yet.</p>
{:else}
	<ul class="grid" onscrollcapture={() => (menu = null)}>
		{#each imports as entry (entry.id)}
			<li>
				<button class="open" type="button" onclick={() => onopen(entry)}>
					<span class="title">{entry.name}</span>
					<span class="stats">
						<span>{numbers.format(entry.conversation_count)} conversations</span>
						<span>{numbers.format(entry.message_count)} messages</span>
					</span>
				</button>

				<button
					class="more"
					type="button"
					aria-label="More options for {entry.name}"
					aria-haspopup="menu"
					aria-expanded={menu?.entry.id === entry.id}
					onclick={(event) => toggleMenu(event, entry)}
				>
					<!-- Drawn rather than typed: text dots sit on the baseline, so
					     they can only be centred by fudging padding, and their
					     weight is whatever the system font decides. -->
					<svg viewBox="0 0 18 4" width="18" height="4" aria-hidden="true" focusable="false">
						<circle cx="2" cy="2" r="2" />
						<circle cx="9" cy="2" r="2" />
						<circle cx="16" cy="2" r="2" />
					</svg>
				</button>
			</li>
		{/each}
	</ul>
{/if}

{#if menu}
	{@const current = menu}
	<div class="menu" role="menu" tabindex="-1" style:left="{current.x}px" style:top="{current.y}px">
		<button
			type="button"
			role="menuitem"
			onclick={() => chooseDelete(current.entry)}
			{@attach (node) => node.focus()}
		>
			Delete Import
		</button>
	</div>
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

	li {
		position: relative;
	}

	.open {
		display: flex;
		width: 100%;
		height: 100%;
		min-height: 5.5rem;
		flex-direction: column;
		justify-content: space-between;
		gap: 0.75rem;
		border: 1px solid var(--divider);
		border-radius: 8px;
		/* Right padding leaves the corner clear for the "..." button. */
		padding: 0.85rem 2.75rem 0.85rem 1rem;
		font: inherit;
		color: var(--text);
		background-color: var(--surface-raised);
		cursor: pointer;
		text-align: left;
		transition:
			background-color 0.15s ease,
			border-color 0.15s ease;
	}

	.open:hover {
		border-color: var(--accent);
		background-color: var(--surface-hover);
	}

	.open:focus-visible {
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

	.more {
		position: absolute;
		top: 0.5rem;
		right: 0.5rem;
		display: flex;
		width: 1.75rem;
		height: 1.75rem;
		align-items: center;
		justify-content: center;
		border: none;
		border-radius: 6px;
		padding: 0;
		color: var(--text-muted);
		background: none;
		cursor: pointer;
	}

	/* A block-level box with no baseline of its own, so the flex centring on
	   the button lands it exactly in the middle. */
	.more svg {
		display: block;
		fill: currentColor;
	}

	.more:hover {
		color: var(--text);
		background-color: var(--divider);
	}

	.more:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 1px;
	}

	/* Fixed, so it escapes the grid's own scroll container rather than being
	   clipped by it. */
	.menu {
		position: fixed;
		z-index: 10;
		width: 180px;
		overflow: hidden;
		border: 1px solid var(--divider);
		border-radius: 8px;
		background-color: var(--surface-raised);
		box-shadow: 0 8px 24px rgb(0 0 0 / 45%);
	}

	.menu button {
		display: block;
		width: 100%;
		border: none;
		padding: 0.7rem 0.9rem;
		font: inherit;
		color: var(--text);
		background: none;
		cursor: pointer;
		text-align: left;
	}

	.menu button:hover,
	.menu button:focus-visible {
		background-color: var(--surface-hover);
		outline: none;
	}
</style>
