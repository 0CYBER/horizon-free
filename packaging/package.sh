#!/usr/bin/env bash
# packaging/package.sh BINARY OUTDIR: a playable Horizon folder with the game
# binary, the resource preparation tool and what it compiles, and PLAY.txt.
# It holds none of Minecraft's files: players prepare those from their own copy.
set -euo pipefail
bin=$1
out=$2
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
rm -rf "$out"
mkdir -p "$out"
git -C "$repo" archive HEAD tools mods/horizon/src/test/java research-lock.json LICENSE | tar -x -C "$out"
cp "$bin" "$out/"
cp "$repo/packaging/PLAY.txt" "$out/PLAY.txt"
