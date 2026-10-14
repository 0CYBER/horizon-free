#!/usr/bin/env bash
# The vanilla data for this checkout on the self-hosted runner. Horizon never downloads the game: the runner
# holds the maintainer's own copy, imported and decompiled once (HORIZON_SHARED_SOURCE, default
# ~/code/horizon-parity/shared-local). It is cloned once into the runner's work folder ($RUNNER_WORKSPACE/
# shared-local; APFS clones, so no second copy of the bytes), and .local/ links to it as in a parity worktree.
# .local/horizon-client, the resources horizon-prepare makes, is prepared by this checkout's own preparer
# once per preparer version (its sources, the exporters and research-lock.json), so a change to the preparer
# is checked with what it prepares; the four newest preparations are kept.
set -euo pipefail
shared="${RUNNER_WORKSPACE:?}/shared-local"
source="${HORIZON_SHARED_SOURCE:-$HOME/code/horizon-parity/shared-local}"
version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' research-lock.json | head -1)
if [ ! -f "$shared/minecraft/$version/prepared.json" ]; then
  [ -f "$source/minecraft/$version/prepared.json" ] || { echo "::error::no imported Minecraft $version in $source"; exit 1; }
  mkdir -p "$shared"
  if [ "$(uname)" = Darwin ]; then
    for entry in "$source"/*; do
      [ "$(basename "$entry")" = horizon-client ] || cp -cR "$entry" "$shared/"
    done
  else
    for entry in "$source"/*; do
      [ "$(basename "$entry")" = horizon-client ] || cp -R --reflink=auto "$entry" "$shared/"
    done
  fi
fi
mkdir -p .local "$shared/prepared"
for entry in "$shared"/*; do
  name=$(basename "$entry")
  case "$name" in prepared|horizon-client|*.log) continue ;; esac
  if [ -L ".local/$name" ] || [ ! -e ".local/$name" ]; then ln -sfn "$entry" ".local/$name"; fi
done

key=$(git ls-files -s crates/horizon-prepare mods/horizon/src/test/java research-lock.json | shasum -a 256 | cut -c1-16)
prepared="$shared/prepared/$key"
ln -sfn "$prepared" .local/horizon-client
if [ ! -f "$prepared/.done" ]; then
  echo "Preparing resources with this checkout's horizon-prepare ($key)"
  rm -rf "$prepared" && mkdir -p "$prepared"
  started=$SECONDS
  cargo build --release --locked -p horizon-prepare
  if ! target/release/horizon-prepare --root "$PWD" > "$RUNNER_TEMP/prepare.log" 2>&1; then
    tail -40 "$RUNNER_TEMP/prepare.log"
    rm -rf "$prepared"
    exit 1
  fi
  touch "$prepared/.done"
  echo "Prepared in $((SECONDS - started)) s"
fi
touch "$prepared"
# The four newest preparations stay.
ls -1dt "$shared/prepared"/*/ 2>/dev/null | tail -n +5 | while read -r old; do rm -rf "$old"; done
