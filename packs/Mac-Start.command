#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")"
PACK="$PWD/horizon-m2-balanced-@PACK_VERSION@.mrpack"
if [[ ! -f "$PACK" ]]; then
  printf 'Keep this script beside the .mrpack files from the extracted starter ZIP.\n'
  exit 1
fi
APP=""
for candidate in "/Applications/Prism Launcher.app" "$HOME/Applications/Prism Launcher.app"; do
  if [[ -d "$candidate" ]]; then APP="$candidate"; break; fi
done
if [[ -z "$APP" ]]; then
  printf 'Installing official Prism Launcher 11.1.1 into ~/Applications...\n'
  TEMP_DIR=$(mktemp -d)
  trap 'rm -rf "$TEMP_DIR"' EXIT
  curl --fail --location --retry 3 --output "$TEMP_DIR/prism.zip" \
    'https://github.com/PrismLauncher/PrismLauncher/releases/download/11.1.1/PrismLauncher-macOS-11.1.1.zip'
  printf '%s  %s\n' '15126a7df57acb4f37fe581dad68cb03c7bb60b70eb82345f8bb3246235600c6' "$TEMP_DIR/prism.zip" | shasum -a 256 -c -
  ditto -x -k "$TEMP_DIR/prism.zip" "$TEMP_DIR/unpacked"
  mkdir -p "$HOME/Applications"
  APP="$HOME/Applications/Prism Launcher.app"
  if [[ ! -d "$TEMP_DIR/unpacked/Prism Launcher.app" ]]; then
    printf 'Unexpected official launcher archive layout. Install Prism from https://prismlauncher.org/download/macOS/\n'
    exit 1
  fi
  mv "$TEMP_DIR/unpacked/Prism Launcher.app" "$APP"
fi
printf '\nIn Prism: sign into Microsoft yourself, allow automatic Java 25 download, import the pack, and launch.\n'
printf 'For this 16 GB Mac set maximum memory to 6144 MiB; start at 1280x800 or 1920x1200.\n'
printf 'Read START-HERE.txt for the baseline comparison and graphics profiles.\n\n'
open -a "$APP" --args --import "$PACK"
