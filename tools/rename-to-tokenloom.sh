#!/bin/sh
# tools/rename-to-tokenloom.sh
#
# One-shot repo-wide rename that was applied when this project became
# tokenloom: <old-name> -> tokenloom (all case variants), in file contents
# and in file/directory names. Idempotent — safe to re-run.
#
# Excluded: build output (target/), the local cargo cache (.cargo-home/),
# VCS internals (.git/), and this script itself.
#
# NOTE: the old name is constructed at runtime from fragments, so this file
# never literally contains it and can never rewrite itself.

set -eu

REPO_ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$REPO_ROOT"

# "search" + "ng" — assembled at runtime; see NOTE above.
OLD="search"'ng'
NEW=tokenloom

# ── 1. file contents ─────────────────────────────────────────────────────────
# Text files of every type the repo ships, minus build/cache/VCS trees.
# NOTE: the prune group must be parenthesised — -a/-o precedence would
# otherwise let the build trees leak into the scan.
CHANGED=0
SCANNED=0
SELF=$(basename "$0")
for f in $(find . \( -path ./target -o -path ./.cargo-home -o -path ./.git \) -prune -o -type f \( \
		-name '*.rs' -o -name '*.toml' -o -name '*.md' -o -name '*.yml' \
		-o -name '*.yaml' -o -name '*.py' -o -name '*.sh' -o -name '*.js' \
		-o -name '*.json' -o -name '*.lock' -o -name '*.html' -o -name '*.txt' \
	\) -not -name "$SELF" -print); do
	SCANNED=$((SCANNED + 1))
	if grep -qIi "$OLD" "$f" 2>/dev/null; then
		sed -i '' \
			-e "s/$(echo "$OLD" | tr '[:lower:]' '[:upper:]')/$(echo "$NEW" | tr '[:lower:]' '[:upper:]')/g" \
			-e "s/$OLD/$NEW/g" \
			"$f"
		echo "rewrote: $f"
		CHANGED=$((CHANGED + 1))
	fi
done
echo "contents: scanned $SCANNED files, rewrote $CHANGED"

# ── 2. file and directory names ──────────────────────────────────────────────
# -depth must NOT be combined with -prune (find disables pruning under -depth).
# Two passes instead: files, then dirs deepest-first so child paths are
# handled before their parent moves. mv(1) same-path no-ops are tolerated.
PRUNE='( -path ./target -o -path ./.cargo-home -o -path ./.git )'
for p in $(find . $PRUNE -prune -o -type f -name "*$OLD*" -print); do
	new=$(echo "$p" | sed "s/$OLD/$NEW/g")
	[ "$p" = "$new" ] && continue
	mkdir -p "$(dirname "$new")"
	mv "$p" "$new"
	echo "renamed: $p -> $new"
done
for p in $(find . $PRUNE -prune -o -type d -name "*$OLD*" -print | sort -r); do
	new=$(echo "$p" | sed "s/$OLD/$NEW/g")
	[ "$p" = "$new" ] && continue
	mv "$p" "$new"
	echo "renamed: $p -> $new"
done

# ── 3. verification ──────────────────────────────────────────────────────────
LEFT=$(find . $PRUNE -prune -o -type f -not -name "$SELF" -print \
	| xargs grep -li "$OLD" 2>/dev/null || true)
if [ -n "$LEFT" ]; then
	echo "WARNING — remaining occurrences in:"
	echo "$LEFT"
	exit 1
fi
echo "clean: no occurrences of the old name remain in the repo"
