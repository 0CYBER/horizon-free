#!/usr/bin/env bash
# Route rustc through sccache, falling back to an uncached compile on a cache outage.
# Keep clippy-driver direct for hand-written workspace linting.
# Cargo nests RUSTC_WORKSPACE_WRAPPER (clippy-driver) inside RUSTC_WRAPPER.
set -euo pipefail
case "${1##*/}" in
  clippy-driver | clippy-driver.exe) exec "$@" ;;
esac
# Cargo has already selected incremental compilation in rustc's arguments.
# sccache refuses CARGO_INCREMENTAL=1 even for non-incremental dependencies.
unset CARGO_INCREMENTAL
# sccache 0.18.0 src/lib.rs: an internal error prints this prefix and exits 2.
# commands.rs::handle_compile_finished forwards compiler stderr and its exit code.
# The exit code alone cannot distinguish the two (rustc can also exit 2).
if ! command -v sccache > /dev/null; then
  echo '::warning::sccache is unavailable; compiling uncached.' >&2
  exec "$@"
fi
errors=$(mktemp) || { echo '::warning::Cannot capture sccache diagnostics; compiling uncached.' >&2; exec "$@"; }
trap 'rm -f "$errors"' EXIT
for attempt in 1 2; do
  status=0
  sccache "$@" 2> "$errors" || status=$?
  cat "$errors" >&2
  if [ "$status" -ne 2 ] || ! grep -q '^sccache: error:' "$errors"; then
    exit "$status"
  fi
done
echo '::warning::sccache failed twice; compiling uncached.' >&2
rm -f "$errors"
trap - EXIT
exec "$@"
