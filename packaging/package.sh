#!/usr/bin/env bash
# packaging/package.sh BINARY OUTDIR: a playable Horizon folder with the game
# binary, horizon-ctl (built beside it: cargo build --release -p horizon-client --bins),
# the resource preparation tool and what it compiles, and PLAY.txt.
# It holds none of Minecraft's files: players prepare those from their own copy.
set -euo pipefail
bin=$1
out=$2
repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
rm -rf "$out"
mkdir -p "$out"
git -C "$repo" archive HEAD tools mods/horizon/src/test/java research-lock.json LICENSE | tar -x -C "$out"
cp "$bin" "$out/"
# horizon-ctl, the command line and MCP server that play the game, sits beside the client.
ctl="$(dirname "$bin")/horizon-ctl"
case "$bin" in *.exe) ctl="$ctl.exe" ;; esac
if [ ! -f "$ctl" ]; then
    echo "package.sh: $ctl is missing; build it too: cargo build --release -p horizon-client --bins" >&2
    exit 1
fi
cp "$ctl" "$out/"
cp "$repo/packaging/PLAY.txt" "$out/PLAY.txt"
