#!/usr/bin/env bash
# Exercise the built artifact with private resources, never in a player's game folder.
set -euo pipefail
home=$(mktemp -d "$RUNNER_TEMP/horizon-smoke.XXXXXX")
version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' research-lock.json | head -1)
client="$(cd "$BIN" && pwd)/horizon-client"
resources="$PWD/.local/horizon-client/$version"
tools=$(cd "$(dirname "$0")" && pwd)
display=()
if [[ "$RUNNER_OS" == Linux ]]; then
  display=(python3 "$tools/with_xvfb.py")
fi
(cd "$home" && HORIZON_HOME="$home" HORIZON_NO_AUDIO=1 \
  HORIZON_LAUNCHER_HOME="$home/launcher" XDG_DATA_HOME="$home/data" XDG_CONFIG_HOME="$home/config" \
  HORIZON_LAUNCHER_NO_SELF_UPDATE=1 HORIZON_LAUNCHER_TRASH="$home/trash" \
  HORIZON_LAUNCHER_CLIPBOARD="$home/clipboard" HORIZON_LAUNCHER_DESKTOP="$home/desktop" \
  "${display[@]}" "$client" --resources "$resources" --offscreen \
  --world smoke --seed 1 --render-distance 4 --world-frames 1500 --timeout-seconds 180 \
  --smoke-report "$home/smoke.json" > "$home/client.log" 2>&1) || { tail -40 "$home/client.log"; exit 1; }
python3 - "$home/smoke.json" <<'PY'
import json
import sys
report = json.load(open(sys.argv[1]))
print(json.dumps(report))
if report.get("error") is not None or report.get("frames", 0) < 1500 or not report.get("chunks") \
        or not report.get("meshed_sections") or not report.get("player"):
    sys.exit("the smoke run did not draw a world")
PY
cp "$home/smoke.json" "$RUNNER_TEMP/smoke-test.json"
