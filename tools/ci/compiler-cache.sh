#!/usr/bin/env bash
# Route rustc through sccache, falling back to an uncached compile on a cache outage.
# Keep clippy-driver direct for hand-written workspace linting.
# Cargo nests RUSTC_WORKSPACE_WRAPPER (clippy-driver) inside RUSTC_WRAPPER.
set -euo pipefail
case "${1##*/}" in
  clippy-driver | clippy-driver.exe)
    crate_name= previous=
    for arg in "$@"; do
      if [ "$previous" = --crate-name ]; then
        crate_name=$arg
        break
      fi
      previous=$arg
    done
    if [ "$crate_name" != mod_points ]; then
      exec "$@"
    fi
    # The points API is generated: its 29 MB expansion takes seconds to type-check
    # but many minutes in Clippy's lint passes. Still type-check it, denying rustc
    # warnings, and leave every other workspace crate's Clippy invocation intact.
    shift # clippy-driver; the next argument is the real rustc
    args=()
    while [ "$#" -gt 0 ]; do
      case "$1" in
        -C)
          if [[ "${2:-}" == incremental=* ]]; then
            shift 2
            continue
          fi
          ;;
        -Cincremental=*) shift; continue ;;
      esac
      args+=("$1")
      shift
    done
    # Incremental units cannot be cached by sccache. Only this generated crate
    # drops its incremental flag; hand-written workspace crates keep theirs.
    set -- "${args[@]}" -D warnings
    ;;
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
