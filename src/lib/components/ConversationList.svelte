<script lang="ts">
	import { formatTimestamp } from '$lib/datetime';
	import type { ConversationSummary } from '$lib/ipc';

	interface Props {
		conversations: ConversationSummary[];
	}

	let { conversations }: Props = $props();

	/** Screens of rows kept mounted: the one in view, plus one either side. */
	const PARTITIONS = 3;

	/**
	 * Rendered before anything has been measured, purely so there is a row to
	 * measure. One screenful on any plausible display; the real count replaces
	 * it on the next tick.
	 */
	const UNMEASURED_ROWS = 40;

	const numbers = new Intl.NumberFormat();

	let rowHeight = $state(0);
	let viewportHeight = $state(0);
	let scrollTop = $state(0);
	let frame = 0;

	let total = $derived(conversations.length);
	let measured = $derived(rowHeight > 0 && viewportHeight > 0);
	let rowsPerScreen = $derived(measured ? Math.max(1, Math.ceil(viewportHeight / rowHeight)) : 0);

	/**
	 * The window's first row. One partition is one screen; the partition in
	 * view sits in the middle of the three whenever there is room either side.
	 */
	let start = $derived.by(() => {
		if (!measured) return 0;
		const inView = Math.floor(scrollTop / (rowsPerScreen * rowHeight));
		const lastPartition = Math.max(0, Math.ceil(total / rowsPerScreen) - 1);
		const first = Math.min(Math.max(inView - 1, 0), Math.max(lastPartition - (PARTITIONS - 1), 0));
		return first * rowsPerScreen;
	});

	let end = $derived(
		measured
			? Math.min(start + rowsPerScreen * PARTITIONS, total)
			: Math.min(UNMEASURED_ROWS, total)
	);

	let visible = $derived(conversations.slice(start, end));

	/**
	 * Layout only exists once the browser has done it, so the row pitch and the
	 * viewport have to be read from the DOM rather than derived. Both change
	 * when the window resizes, hence the observer.
	 */
	function trackSize(node: HTMLUListElement) {
		const measure = () => {
			const row = node.querySelector('li.row');
			if (!row) return;
			// The bottom margin is part of the pitch a row occupies, so it is
			// part of what a spacer has to stand in for.
			const margin = parseFloat(getComputedStyle(row).marginBottom);
			rowHeight = row.getBoundingClientRect().height + margin;
			viewportHeight = node.clientHeight;
		};

		measure();
		const observer = new ResizeObserver(measure);
		observer.observe(node);

		return () => {
			observer.disconnect();
			cancelAnimationFrame(frame);
		};
	}

	function onscroll(event: Event & { currentTarget: HTMLUListElement }) {
		// Scroll fires far faster than paint and only the latest position
		// matters, so coalesce to one read per frame.
		const node = event.currentTarget;
		if (frame !== 0) return;
		frame = requestAnimationFrame(() => {
			frame = 0;
			scrollTop = node.scrollTop;
		});
	}

	/** `3 Participants: Ada Lovelace, Alan Turing, Grace Hopper` */
	function participantLabel(names: string[]): string {
		const noun = names.length === 1 ? 'Participant' : 'Participants';
		return `${names.length} ${noun}: ${names.join(', ')}`;
	}

	function lastMessageLabel(conversation: ConversationSummary): string {
		return conversation.last_message_ms === null
			? 'N/A'
			: formatTimestamp(conversation.last_message_ms);
	}
</script>

<ul {onscroll} {@attach trackSize}>
	<li class="spacer" style:height="{start * rowHeight}px" aria-hidden="true"></li>

	{#each visible as conversation, index (conversation.id)}
		<!-- `aria-setsize`/`aria-posinset` report the whole list, not the slice
		     that happens to be mounted — otherwise a screen reader would
		     announce "1 of 60" in a list of five thousand. -->
		<li class="row" aria-setsize={total} aria-posinset={start + index + 1}>
			<!-- Titles and participant names are export-derived text a stranger
			     wrote. Rendered as text nodes, never `{@html}`. See rule 1 in
			     src/CLAUDE.md. -->
			<span class="title">{conversation.title}</span>
			<span class="meta">{participantLabel(conversation.participants)}</span>
			<span class="meta">{numbers.format(conversation.message_count)} messages</span>
			<span class="meta">Last message: {lastMessageLabel(conversation)}</span>
		</li>
	{/each}

	<li class="spacer" style:height="{(total - end) * rowHeight}px" aria-hidden="true"></li>
</ul>

<style>
	/* `min-height: 0` because a flex item defaults to `min-height: auto` and
	   would grow to fit every row rather than scrolling — which with a few
	   conversations looks fine and with a few thousand defeats the point. */
	ul {
		overflow-y: auto;
		flex: 1;
		min-height: 0;
		margin: 0;
		padding: 0.5rem;
		list-style: none;
	}

	/* Stands in for the rows either side of the window, so discarding them
	   shifts nothing and the scrollbar keeps the length of the whole list. */
	.spacer {
		margin: 0;
		padding: 0;
	}

	.row {
		display: flex;
		flex-direction: column;
		gap: 0.1rem;
		border-radius: 8px;
		/* Uniform, not `li + li`: that would skip the first row, and every
		   spacer height would drift by one margin per partition. */
		margin-bottom: 0.15rem;
		padding: 0.6rem 0.75rem;
	}

	.title {
		overflow: hidden;
		font-weight: 600;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	/* One line each, clipped with an ellipsis — a long participant list must
	   not set the sidebar's width, and a row that wrapped would break the fixed
	   pitch the window maths depends on. */
	.meta {
		overflow: hidden;
		color: var(--text-muted);
		font-size: 0.78rem;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
</style>
