#!/usr/bin/env bash
# Shared bounded backoff for CI apt commands. Source before calling retry.
tries="${APT_RETRY_TRIES:-4}"
pause="${APT_RETRY_PAUSE:-5}"
retry() {
  local attempt=1 status=0
  while :; do
    "$@" && return 0
    status=$?
    [ "$attempt" -lt "$tries" ] || { echo "$1 failed after $attempt tries (exit $status)" >&2; return "$status"; }
    echo "$1 failed (exit $status), try $attempt of $tries; retrying in $((pause * attempt))s" >&2
    sleep "$((pause * attempt))"
    attempt=$((attempt + 1))
  done
}
