#!/usr/bin/env bash
# Installs the pinned Rust (tools/ci/rust-version) with the given `rustup toolchain install` options and makes it
# this job's toolchain: the rustup default, and RUSTUP_TOOLCHAIN for the job's later steps (a step's own env
# would override it, so workflows never set it). CI, the launcher's native-mod toolchain and cargo-horizon-mod's
# wasm toolchain all read that one file. Bumping it: docs/ci-runners.md, "Rust version".
# rustup gets a bounded retry: a reset connection to static.rust-lang.org is transient and hits boat and GitHub's
# runners alike. Four attempts, 10, 20 and 40 s apart; the last failure fails the step.
set -u
pin=$(tr -d '[:space:]' < "$(dirname "$0")/rust-version")
install() {
  for delay in 10 20 40 0; do
    rustup "$@" && return 0
    status=$?
    [ "$delay" -eq 0 ] && return "$status"
    echo "rustup $1 failed ($status); retrying in ${delay}s" >&2
    sleep "$delay"
  done
}
install toolchain install "$pin" "$@" || exit
rustup default "$pin" || exit
[ -z "${GITHUB_ENV:-}" ] || echo "RUSTUP_TOOLCHAIN=$pin" >> "$GITHUB_ENV"
# Wasm mods build with this toolchain too (cargo-horizon-mod reads the same pin), and run the mod API generator
# and with it rustfmt inside the workspace's tests. A job that installed the minimal profile gets rustfmt and
# clippy here; a no-op without network when they are present.
installed=$(rustup component list --installed --toolchain "$pin" 2>/dev/null)
missing=$(for c in rustfmt clippy; do echo "$installed" | grep -q "^$c-" || echo "$c"; done)
# shellcheck disable=SC2086 # one word per missing component
[ -z "$missing" ] || install component add --toolchain "$pin" $missing || exit
