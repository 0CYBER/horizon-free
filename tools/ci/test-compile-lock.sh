#!/usr/bin/env bash
# Serialize the two large test compiles, preserving the configured compiler cache wrapper.
set -euo pipefail
# Cargo retries E2BIG with newline-delimited response files; inspect without rewriting argv.
classify_args=()
for arg in "$@"; do
  if [[ "$arg" == @* && -f "${arg#@}" ]]; then
    while IFS= read -r line || [ -n "$line" ]; do
      classify_args+=("$line")
    done < "${arg#@}"
  else
    classify_args+=("$arg")
  fi
done
crate_name= previous= test_unit=false
for arg in "${classify_args[@]}"; do
  if [ "$previous" = --crate-name ]; then crate_name=$arg; fi
  case "$arg" in
    --crate-name=*) crate_name=${arg#--crate-name=} ;;
    --test) test_unit=true ;;
  esac
  previous=$arg
done
command=("$@")
if [ -n "${HORIZON_TEST_RUSTC_WRAPPER:-}" ]; then
  command=("$HORIZON_TEST_RUSTC_WRAPPER" "$@")
fi
if $test_unit && { [ "$crate_name" = horizon_client ] || [ "$crate_name" = horizon_client_lib ]; }; then
  # Keep the lock in flock's parent, not in compiler/cache daemon descendants.
  exec flock -x -o "${RUNNER_TEMP:?}/horizon-lib-test-compile.lock" "${command[@]}"
fi
exec "${command[@]}"
