#!/usr/bin/env bash
# Usage: tools/ci/build_folder.sh DIR
# A Blacksmith job's Cargo build folder: DIR is a sticky disk (useblacksmith/stickydisk, mounted just before) that
# keeps what the last job of the same kind built, as our checks runners keep their CARGO_TARGET_DIR. Cargo builds there
# from now on. A checkout dates every file now, so source_mtimes.py gives each file whose Git object the folder's last
# job saw that job's date back: Cargo then rebuilds only what changed. A disk over MAX_TARGET_GB is emptied first
# (old toolchains' and dependencies' builds pile up, as on the checks runners).
set -euo pipefail
dir=${1:?usage: tools/ci/build_folder.sh DIR}
max=${MAX_TARGET_GB:-50}
if mountpoint -q "$dir"; then
  used=$(df --block-size=1G --output=used "$dir" | tail -n 1 | tr -d ' ')
  echo "Build folder: ${used} GB used"
  if [ "$used" -gt "$max" ]; then
    echo "Over $max GB: emptied, this job builds from scratch"
    find "$dir" -mindepth 1 -maxdepth 1 -exec rm -rf {} +
  fi
else
  echo "No sticky disk at $dir: this job builds from scratch"
  mkdir -p "$dir"
fi
echo "CARGO_TARGET_DIR=$dir" >> "$GITHUB_ENV"
python3 tools/ci/source_mtimes.py "$dir/source-mtimes-v1.json"
