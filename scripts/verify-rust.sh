#!/usr/bin/env bash
#
# The Rust definition of done — the counterpart to `npm run verify` on the
# front end. Run this before calling a Rust change finished.
#
# The gates are the ones CI runs, in the order that fails cheapest first: a
# misformatted line shouldn't cost a full clippy build to discover. `--locked`
# matches CI too, so a Cargo.lock that would need updating fails here rather
# than three minutes into a pipeline.
#
# CI splits clippy across two jobs (`engine` and `app`) because the app crate
# pulls in the whole Tauri tree and its system libraries; there's no reason to
# split it locally, where those are already installed and cached. If the app
# crate fails to *build* here with missing `libwebkit2gtk`/`libsoup` headers,
# that's the environment, not the change — the install list is in
# `.github/workflows/ci.yml`.
#
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."

step() {
	printf '\n\033[1m==> %s\033[0m\n' "$1"
}

step "cargo fmt --all --check"
cargo fmt --all --check

step "scripts/check-comment-width.sh"
scripts/check-comment-width.sh

step "cargo clippy --workspace --all-targets -- -D warnings"
cargo clippy --locked --workspace --all-targets -- -D warnings

step "cargo test --workspace"
cargo test --locked --workspace

printf '\n\033[1;32m==> Rust verify passed\033[0m\n'
