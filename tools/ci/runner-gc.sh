#!/usr/bin/env bash
# Keeps the Mac mini's Horizon runners from filling its disk. launchd runs a copy taken from main
# (~/.actions-runners/runner-gc.sh, never a pull request's checkout) every 6 hours; see docs/release-flow.md.
#   - A runner in the middle of a job is left alone.
#   - A checkout whose cargo build folders (target/ and the harness probes' targets under .local/, the
#     directories holding a CACHEDIR.TAG) pass MAX_TARGET_GB loses them: sccache (tools/ci/runner-env.sh)
#     makes the next build cheap. Symlinks into the shared vanilla data are never followed.
#   - Harness work folders under .local/ (vanilla exports, probe outputs: real directories, not the links into
#     the shared vanilla data) that nothing touched for STALE_DAYS go; a harness that runs again rebuilds its own.
#     .local/parity-runner (run state, oracle cache) stays.
#   - The shared prepared resources keep their KEEP_PREPARED newest preparations, only while no runner works.
set -euo pipefail
root=${RUNNERS:-$HOME/.actions-runners}
max_kb=$(( ${MAX_TARGET_GB:-12} * 1024 * 1024 ))
keep=${KEEP_PREPARED:-2}
stale_min=$(( ${STALE_DAYS:-3} * 24 * 60 ))
busy() { pgrep -f "$1/bin/Runner.Worker" > /dev/null; }

any_busy=0
for runner in "$root"/horizon "$root"/horizon-[0-9]*; do
  [ -d "$runner/_work" ] || continue
  if busy "$runner"; then echo "$(basename "$runner"): in a job, skipped"; any_busy=1; continue; fi
  rm -rf "${runner:?}/_work/_temp/"* 2>/dev/null || true
  checkout="$runner/_work/horizon/horizon"
  [ -d "$checkout" ] || continue
  stale=0
  for dir in "$checkout"/.local/*/; do
    dir=${dir%/}
    [ -L "$dir" ] || [ "$(basename "$dir")" = parity-runner ] && continue
    if [ -z "$(find "$dir" -mmin -"$stale_min" -print -quit 2>/dev/null)" ]; then rm -rf "$dir"; stale=$((stale + 1)); fi
  done
  [ "$stale" = 0 ] || echo "$(basename "$runner"): removed $stale harness folders untouched for ${STALE_DAYS:-3} days"
  targets=()
  [ -f "$checkout/target/CACHEDIR.TAG" ] && targets+=("$checkout/target")
  while IFS= read -r tag; do targets+=("$(dirname "$tag")"); done < <(
    find "$checkout/.local" -mindepth 2 -maxdepth 2 -name CACHEDIR.TAG -type f 2>/dev/null)
  [ ${#targets[@]} -gt 0 ] || continue
  size=$(du -sk "${targets[@]}" | awk '{s += $1} END {print s}')
  if [ "$size" -gt "$max_kb" ]; then
    rm -rf "${targets[@]}"
    echo "$(basename "$runner"): removed ${#targets[@]} build folders ($(( size / 1024 / 1024 )) GB)"
  fi
done

prepared="$root/horizon/_work/horizon/shared-local/prepared"
if [ "$any_busy" = 0 ] && [ -d "$prepared" ] && mkdir "$prepared/.lock" 2>/dev/null; then
  trap 'rmdir "$prepared/.lock" 2>/dev/null || true' EXIT
  ls -1t "$prepared" | tail -n +"$(( keep + 1 ))" | while read -r old; do
    rm -rf "${prepared:?}/$old"
    echo "prepared resources $old removed"
  done
fi
df -h "$root" | tail -1
