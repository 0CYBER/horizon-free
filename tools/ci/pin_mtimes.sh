#!/usr/bin/env bash
# Usage: tools/ci/pin_mtimes.sh DIR
# Makes cargo rebuild only what a change touched. DIR/sha names the main commit whose build is in target/. If that
# commit is in this repository, every file and directory outside target/, .git/ and DIR gets the mtime 2001-01-01 UTC
# (older than the build), then each file that differs from that commit, and its parent directories, gets the current
# time. Without DIR/sha or the commit it does nothing. It always exits 0: the worst case is a full rebuild.
set -u

dir="${1:-}"
dir="${dir%/}"
dir="${dir#./}"
sha=
if [ -n "$dir" ] && [ -f "$dir/sha" ]; then
  sha="$(tr -d '[:space:]' < "$dir/sha")"
fi
if [ -z "$sha" ] || ! git cat-file -e "$sha^{commit}" 2>/dev/null; then
  echo "pin_mtimes: no built commit in ${dir:-<no dir>}, nothing pinned"
  exit 0
fi

find . \( -path ./target -o -path ./.git -o -path "./$dir" \) -prune -o \
  -exec touch -h -d '2001-01-01 00:00:00 UTC' {} + 2>/dev/null

changed=0
while IFS= read -r -d '' file; do
  [ -e "$file" ] || [ -L "$file" ] || continue
  touch -h "$file" 2>/dev/null
  changed=$((changed + 1))
  parent="$file"
  while [ "$parent" != "." ] && [ "$parent" != "/" ]; do
    parent="$(dirname "$parent")"
    touch "$parent" 2>/dev/null
  done
done < <(git diff --name-only -z "$sha" HEAD)

echo "pin_mtimes: pinned the tree to 2001-01-01, touched $changed files changed since ${sha:0:12}"
exit 0
