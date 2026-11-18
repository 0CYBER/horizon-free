#!/bin/sh
# Opt-in release build for the CPU this compiler is running on.
# Profile release, same as the release workflow: thin LTO, one codegen unit.
# The default aarch64-apple-darwin CPU is apple-m1. target-cpu=native lets LLVM
# use the schedule and the scalar/NEON features rustc already names for this
# machine (apple-m5 on an M5, and the matching name on any other Mac).
#
# Do not add +sme or +sme2. rustc accepts them only as unstable target-features
# that are being turned into a hard error, and ordinary Rust is not rewritten
# into SME matrix operations. A kernel that wants the matrix unit has to be its
# own compilation unit, with the streaming entry written by hand, and must not
# be the default for every player.
set -eu
cd "$(dirname "$0")/.."
unset CARGO_ENCODED_RUSTFLAGS
export RUSTFLAGS="-C target-cpu=native"
echo "$(rustc --version)"
echo "runtime: profile release, thin LTO, codegen-units 1, target-cpu=native"
cargo build --locked --release -p horizon-game
echo "built binary: horizon-client"
