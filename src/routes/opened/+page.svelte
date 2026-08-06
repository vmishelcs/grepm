<script lang="ts">
	import { resolve } from '$app/paths';
	import { describeError } from '$lib/errors';
	import {
		activeImport,
		listConversations,
		type ActiveImportInfo,
		type ConversationSummary
	} from '$lib/ipc';

	// Rows are inert for now: no click handler, and not buttons. A button that
	// does nothing is a promise to a keyboard or screen-reader user that this
	// screen can't keep yet — they become interactive when selection lands.
	let info = $state.raw<ActiveImportInfo | null>(null);
	let conversations = $state.raw<ConversationSummary[]>([]);
	let error = $state<string | null>(null);
	let loaded = $state(false);

	void load();

	async function load() {
		try {
			// Both come off the open connection, so the sidebar rendering at all
			// is proof the database really opened.
			[info, conversations] = await Promise.all([activeImport(), listConversations()]);
		} catch (err) {
			error = describeError(err);
		} finally {
			loaded = true;
		}
	}

	const numbers = new Intl.NumberFormat();

	/** `3 Participants: Ada Lovelace, Alan Turing, Grace Hopper` */
	function participantLabel(names: string[]): string {
		const noun = names.length === 1 ? 'Participant' : 'Participants';
		return `${names.length} ${noun}: ${names.join(', ')}`;
	}
</script>

{#if error}
	<main class="message">
		<p class="error" role="alert">{error}</p>
		<a href={resolve('/')}>Back to imports</a>
	</main>
{:else if info}
	<main class="reader">
		<aside>
			<header>
				<!-- Still a link, not a <button>: it navigates, so it should keep
				     a real href — middle-click, keyboard, and the back stack all
				     come free. Only its appearance is button-shaped. -->
				<a class="back" href={resolve('/')}>Back to imports</a>
				<div class="identity">
					<h1>{info.entry.name}</h1>
					<p class="summary">
						{numbers.format(info.stats.conversation_count)} conversations · {numbers.format(
							info.stats.message_count
						)} messages
					</p>
				</div>
			</header>

			<ul>
				{#each conversations as conversation (conversation.id)}
					<li>
						<!-- Titles and participant names are export-derived text a
						     stranger wrote. Rendered as text nodes, never `{@html}`.
						     See rule 1 in src/CLAUDE.md. -->
						<span class="title">{conversation.title}</span>
						<span class="meta">{participantLabel(conversation.participants)}</span>
						<span class="meta">{numbers.format(conversation.message_count)} messages</span>
					</li>
				{/each}
			</ul>
		</aside>

		<section>
			<!-- Drawn rather than shipped as a bitmap: it stays sharp at any
			     size or display density, and the app bundles no image assets.
			     Decorative, so hidden from assistive tech — the text below
			     already says everything it says. -->
			<svg class="bubble" viewBox="0 0 96 96" aria-hidden="true" focusable="false">
				<path
					d="M24 10H72A18 18 0 0 1 90 28V52A18 18 0 0 1 72 70H38L16 88L24 70A18 18 0 0 1 6 52V28A18 18 0 0 1 24 10Z"
				/>
				<circle cx="32" cy="40" r="6" />
				<circle cx="48" cy="40" r="6" />
				<circle cx="64" cy="40" r="6" />
			</svg>

			<p class="prompt">Select a conversation to read</p>
			<p class="hint">Nothing is selected</p>
		</section>
	</main>
{:else if loaded}
	<main class="message">
		<p class="hint">No import is open.</p>
		<a href={resolve('/')}>Back to imports</a>
	</main>
{/if}

<style>
	/* Fixed to the window with the list scrolling inside it, the way a mail
	   client behaves — the page itself never scrolls. */
	.reader {
		display: flex;
		height: 100dvh;
		overflow: hidden;
	}

	aside {
		display: flex;
		width: 20rem;
		flex-direction: column;
		flex-shrink: 0;
		border-right: 1px solid var(--divider);
	}

	/* Children stretch by default, which is what lets the title truncate to
	   the sidebar's width; only the back button opts out to hug its text. */
	header {
		display: flex;
		flex-direction: column;
		gap: 0.75rem;
		border-bottom: 1px solid var(--divider);
		padding: 1rem 1.25rem;
	}

	/* `line-height: normal`, not the 1.5 inherited from `:root`, and not 1
	   either. Measured: the label sits 1.19px high at 1.5 and 0.8px high at 1,
	   because half-leading is split evenly around a glyph box that isn't
	   itself symmetric. At `normal` the line box *is* the glyph box, leaving
	   nothing to split, and the padding then centres it exactly. */
	.back {
		display: inline-flex;
		align-self: flex-start;
		align-items: center;
		justify-content: center;
		line-height: normal;
		border: 1px solid var(--divider);
		border-radius: 8px;
		padding: 0.35rem 0.75rem;
		color: var(--text);
		background-color: var(--surface-raised);
		font-size: 0.8rem;
		text-decoration: none;
		transition:
			background-color 0.15s ease,
			border-color 0.15s ease;
	}

	.back:hover {
		border-color: var(--accent);
		background-color: var(--surface-hover);
	}

	.back:focus-visible {
		outline: 2px solid var(--accent);
		outline-offset: 2px;
	}

	.identity {
		min-width: 0;
	}

	/* `font-weight: 800` alone does nothing here: the system font stack ships
	   only regular and bold, so 600 through 900 all snap to the same face, and
	   `font-synthesis: none` in app.css rules out a faux-bold fallback. The
	   weight is still declared for a machine whose font has more faces; the
	   stroke is what actually thickens the glyphs on one that doesn't. */
	h1 {
		overflow: hidden;
		margin: 0;
		font-size: 1.25rem;
		font-weight: 800;
		text-overflow: ellipsis;
		white-space: nowrap;
		-webkit-text-stroke: 0.45px currentcolor;
	}

	.summary {
		margin: 0.15rem 0 0;
		color: var(--text-muted);
		font-size: 0.75rem;
	}

	ul {
		overflow-y: auto;
		margin: 0;
		padding: 0.5rem;
		list-style: none;
	}

	li {
		display: flex;
		flex-direction: column;
		gap: 0.1rem;
		border-radius: 8px;
		padding: 0.6rem 0.75rem;
	}

	li + li {
		margin-top: 0.15rem;
	}

	.title {
		overflow: hidden;
		font-weight: 600;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	/* One line each, clipped with an ellipsis — a long participant list must
	   not be allowed to set the sidebar's width. */
	.meta {
		overflow: hidden;
		color: var(--text-muted);
		font-size: 0.78rem;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	section {
		display: flex;
		flex: 1;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 0.35rem;
		padding: 1.5rem;
	}

	.bubble {
		width: 5.5rem;
		height: 5.5rem;
		margin-bottom: 0.75rem;
	}

	.bubble path {
		fill: var(--accent);
	}

	.bubble circle {
		fill: var(--on-accent);
	}

	.prompt {
		margin: 0;
		font-size: 1.05rem;
	}

	.hint {
		margin: 0;
		color: var(--text-muted);
		font-size: 0.9rem;
	}

	.message {
		display: flex;
		min-height: 100dvh;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 1rem;
		padding: 1.5rem;
		text-align: center;
	}

	.message a {
		color: var(--accent);
		font-size: 0.9rem;
	}

	.error {
		margin: 0;
		color: var(--danger);
	}
</style>
