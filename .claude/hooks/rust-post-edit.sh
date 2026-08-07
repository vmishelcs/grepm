#!/usr/bin/env bash
#
# PostToolUse hook: after an edit to one of this project's Rust files, format
# it and hold its comments to 80 columns.
#
# The Rust counterpart to what `svelte-autofixer` does for components —
# feedback at the moment of the edit instead of at the end of the task. It can
# only do half of that job on its own: rustfmt fixes layout, but rewrapping
# prose is a judgement call, so comment-width violations come back as text to
# act on. Exit code 2 is what routes stderr into the conversation; every other
# path exits 0 and stays silent.
#
# Deliberately not a substitute for `scripts/verify-rust.sh`. Nothing here
# compiles anything, so it says nothing about whether the code builds, passes
# clippy, or passes its tests.
#
set -uo pipefail

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

command -v jq >/dev/null 2>&1 || exit 0
command -v rustfmt >/dev/null 2>&1 || exit 0

file="$(jq -r '.tool_input.file_path // empty')"
[ -n "$file" ] || exit 0

# This project's Rust only. A `.rs` file in a scratchpad isn't held to these
# conventions, and reformatting it would be an edit nobody asked for.
case "$file" in
"$project_dir"/*.rs) ;;
*) exit 0 ;;
esac
[ -f "$file" ] || exit 0

# rustfmt reads its config from the current directory upward, and finding no
# rustfmt.toml it parses as edition 2015 — under which `async fn` is a syntax
# error rather than a function. Hence the cd, and hence `edition` being
# pinned in rustfmt.toml at all.
cd "$project_dir" || exit 0

formatted="$(mktemp)"
errors="$(mktemp)"
trap 'rm -f "$formatted" "$errors"' EXIT

# Relative to the repo, since that is how the project refers to its own
# files — and cwd is the repo root by the time anything uses it.
relative="${file#"$project_dir"/}"

problems=""

# Formatted through stdin rather than as `rustfmt <path>`, which follows the
# file's `mod` declarations and rewrites the children too. For a file like
# `core/src/lib.rs` that would silently reformat the whole crate — files this
# edit never touched, and whose contents the assistant is still holding a now
# stale copy of. Stdin has no path, so there are no children to follow.
if rustfmt --emit stdout --quiet <"$file" >"$formatted" 2>"$errors"; then
	if ! cmp -s "$formatted" "$file"; then
		# Copied rather than moved: `mv` from a tempdir can cross a
		# filesystem and take the file's mode and ownership with it.
		#
		# Reformatting is not reported. Claude Code already emits its own
		# "a hook modified this file" notice, and saying it twice would make
		# every stray space an interruption. Only what needs a decision —
		# a parse failure, or a comment to rewrap — is worth speaking up for.
		cat "$formatted" >"$file"
	fi
else
	problems+="rustfmt could not parse $relative; the edit may have left invalid Rust:"$'\n'
	problems+="$(cat "$errors")"$'\n'
fi

if ! width_report="$(scripts/check-comment-width.sh "$relative" 2>&1)"; then
	problems+="$width_report"$'\n'
fi

if [ -n "$problems" ]; then
	printf '%s' "$problems" >&2
	exit 2
fi
