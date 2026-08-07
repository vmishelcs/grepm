#!/usr/bin/env bash
#
# Enforces the second half of the project's line-length convention: "comments
# wrap at 80". rustfmt can't — see the note in `rustfmt.toml`.
#
# With no arguments, checks every tracked `.rs` file. Given paths, checks only
# those, which is what the PostToolUse hook does with the file just edited.
#
# Two decisions worth knowing about:
#
#   * Only whole-line comments are checked. A trailing comment shares its line
#     with code, and rustfmt already holds that line to 100; flagging it would
#     mean demanding the code move rather than the comment.
#
#   * Length is counted in *characters*, not bytes. This matters more than it
#     sounds: these comments are full of em dashes and accented names, and a
#     byte count reports a compliant 79-column line as an 81-column violation.
#     `awk '{ if (length > 80) ... }'` — the obvious one-liner, and the one
#     this script replaces — gets that wrong under mawk, which has no
#     multibyte support. Hence perl with `-CSD`, which decodes as UTF-8
#     regardless of the caller's locale.
#
set -euo pipefail

files=("$@")
if [ ${#files[@]} -eq 0 ]; then
	# Only in this branch: `git ls-files` prints repo-relative paths, so they
	# need the repo root as cwd. Paths passed as arguments are the caller's,
	# and moving out from under them would break every relative one.
	cd "$(dirname "${BASH_SOURCE[0]}")/.."
	# Read in a loop rather than with `mapfile`, which macOS's bash 3.2
	# doesn't have.
	while IFS= read -r tracked; do
		files+=("$tracked")
	done < <(git ls-files '*.rs')
	if [ ${#files[@]} -eq 0 ]; then
		exit 0
	fi
fi

# perl's `-n` only warns on a file it can't open and still exits 0, so a
# mistyped path would otherwise read as "no violations found".
unreadable=0
for file in "${files[@]}"; do
	if [ ! -f "$file" ] || [ ! -r "$file" ]; then
		echo "check-comment-width: cannot read $file" >&2
		unreadable=1
	fi
done
if [ "$unreadable" -ne 0 ]; then
	exit 1
fi

if perl -CSD -ne '
	# Captured before the close below, which resets both.
	my ($line, $file, $lineno) = ($_, $ARGV, $.);
	# Without this `$.` keeps counting across files instead of restarting at
	# 1 in each. It has to run before any `next`, or the one line that would
	# reset the counter is the one most likely to be skipped.
	close ARGV if eof;
	next unless $line =~ m{^\s*//};
	# A bare URL is one token: there is no wrap point, so the choice is
	# between an over-long line and a citation the reader cannot follow.
	next if $line =~ m{https?://};
	chomp $line;
	next if length($line) <= 80;
	printf "%s:%d: comment is %d columns (max 80)\n", $file, $lineno, length($line);
	$found = 1;
	END { exit($found ? 1 : 0) }
' "${files[@]}"; then
	exit 0
fi

echo >&2
echo "Comments wrap at 80 columns (code wraps at 100). See CLAUDE.md." >&2
exit 1
