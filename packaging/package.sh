#!/usr/bin/env bash
# packaging/package.sh BINARY OUTDIR: a playable Horizon folder with the game
# binary, horizon-ctl and horizon-prepare (built beside it: cargo build --release
# -p horizon-client --bins -p horizon-prepare), the exporters horizon-prepare compiles,
# its sources (the launcher compares them to its own), and PLAY.txt.
# It holds none of Minecraft's files: players prepare those from their own copy.
set -euo pipefail
bin=$1
out=$2
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
rm -rf "$out"
mkdir -p "$out"
git -C "$repo" archive HEAD tools mods/horizon/src/test/java crates/horizon-prepare research-lock.json LICENSE \
    | tar -x -C "$out"
cp "$bin" "$out/"
# horizon-ctl, the command line and MCP server that play the game, and horizon-prepare, which
# prepares the resources from the player's own Minecraft, sit beside the client.
for tool in horizon-ctl horizon-prepare; do
    path="$(dirname "$bin")/$tool"
    case "$bin" in *.exe) path="$path.exe" ;; esac
    if [ ! -f "$path" ]; then
        echo "package.sh: $path is missing; build it too: cargo build --release -p horizon-client --bins -p horizon-prepare" >&2
        exit 1
    fi
    cp "$path" "$out/"
done
cp "$repo/packaging/PLAY.txt" "$out/PLAY.txt"
