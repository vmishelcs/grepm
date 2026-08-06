#!/usr/bin/env node
/**
 * Screenshot the app so a human — or an agent — can actually see it.
 *
 * The front end normally only renders inside a Tauri window, which nothing can
 * look at programmatically. This drives the plain `vite dev` server in Chromium
 * instead, standing in a fake `__TAURI_INTERNALS__.invoke` for the Rust
 * backend, and writes PNGs to `.screenshots/` (git-ignored).
 *
 * Usage: `npm run shoot`. If a dev server is already listening on 1420 it is
 * reused; otherwise one is started and shut down afterwards.
 */
import { spawn } from 'node:child_process';
import { mkdir, rm } from 'node:fs/promises';
import { chromium } from 'playwright';

const ORIGIN = 'http://localhost:1420';
const OUT_DIR = '.screenshots';

/** Routes to capture. Add new ones here as the app grows. */
const ROUTES = ['/'];

const VIEWPORTS = [
	{ name: 'desktop', width: 1280, height: 800 },
	{ name: 'narrow', width: 720, height: 900 }
];

/**
 * Replaces the Tauri IPC bridge. Runs in the browser before any app code, so
 * `invoke()` resolves against fixtures instead of throwing.
 *
 * Fixtures are imported lazily from the dev server rather than inlined here:
 * `invoke` is async anyway, and it keeps `src/lib/fixtures.ts` the one source
 * of sample data shared with the test suite. The table is keyed by Tauri
 * command name — add an entry when you add a command to `src-tauri/src/lib.rs`.
 */
function installIpcStub() {
	window.__TAURI_INTERNALS__ = window.__TAURI_INTERNALS__ ?? {};
	window.__TAURI_INTERNALS__.invoke = async (command, args) => {
		const fixtures = await import('/src/lib/fixtures.ts');
		switch (command) {
			case 'greet':
				return `Hello, ${args.name}! You've been greeted from Rust!`;
			case 'search':
				return fixtures.sampleResults;
			default:
				throw new Error(`scripts/shoot.mjs has no fixture for the "${command}" command`);
		}
	};
}

async function serverIsUp() {
	try {
		await fetch(ORIGIN, { signal: AbortSignal.timeout(1000) });
		return true;
	} catch {
		return false;
	}
}

/** Starts `vite dev` and waits for it to answer. Returns the child process. */
async function startServer() {
	const child = spawn('npx', ['vite', 'dev'], { stdio: 'ignore' });
	for (let attempt = 0; attempt < 60; attempt += 1) {
		if (await serverIsUp()) return child;
		await new Promise((resolve) => setTimeout(resolve, 500));
	}
	child.kill();
	throw new Error(`vite dev never answered on ${ORIGIN}`);
}

/** `/` becomes `root`, `/search/results` becomes `search-results`. */
function slugify(route) {
	return route.replace(/^\/|\/$/g, '').replace(/\//g, '-') || 'root';
}

const startedServer = (await serverIsUp()) ? null : await startServer();
await rm(OUT_DIR, { recursive: true, force: true });
await mkdir(OUT_DIR, { recursive: true });

const browser = await chromium.launch();
try {
	for (const viewport of VIEWPORTS) {
		const context = await browser.newContext({
			viewport: { width: viewport.width, height: viewport.height }
		});
		await context.addInitScript(installIpcStub);

		for (const route of ROUTES) {
			const page = await context.newPage();
			await page.goto(`${ORIGIN}${route}`, { waitUntil: 'networkidle' });
			const path = `${OUT_DIR}/${slugify(route)}-${viewport.name}.png`;
			await page.screenshot({ path, fullPage: true });
			console.log(path);
			await page.close();
		}

		await context.close();
	}
} finally {
	await browser.close();
	startedServer?.kill();
}
