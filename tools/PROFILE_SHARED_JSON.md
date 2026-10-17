# Reproducing PR #607 block-facts startup profiles

These are loader-only measurements. They include `Assets::load`, `BlockFacts`, and
`Behaviours` and exclude content root preparation, graphics, input setup, first
frame, and the rest of the client startup sequence. No full-client startup gain
has been measured. The prepared `26.3` resource root must match the revision.

## Historical release builds

From the PR branch checkout, use an absolute prepared resource-root path and a
single, explicit Cargo release profile for each detached historical worktree:

```sh
PR_ROOT="$(pwd)"
BENCH_ROOT="$(mktemp -d)"
mkdir -p "$BENCH_ROOT/bin"
CARGO_TARGET_DIR="$BENCH_ROOT/target"
export CARGO_TARGET_DIR

for entry in baseline:e422bd13 a01:a01d47f2 cf:cf837118 cd:cd61e11b; do
  name="${entry%%:*}"
  revision="${entry#*:}"
  git worktree add --detach "$BENCH_ROOT/$name" "$revision"
done

# e422bd13 predates the profiler; add the same baseline example source used
# for the benchmark, without changing any production implementation.
cp "$PR_ROOT/tools/baseline_loader_profile.rs" \
   "$BENCH_ROOT/baseline/crates/horizon-client/examples/shared_json_startup_profile.rs"

for name in baseline a01 cf cd; do
  (cd "$BENCH_ROOT/$name" && \
    cargo build --offline --release -p horizon-client --example shared_json_startup_profile)
  cp "$CARGO_TARGET_DIR/release/examples/shared_json_startup_profile" \
     "$BENCH_ROOT/bin/$name"
done

cargo build --offline --release -p horizon-client --example shared_json_startup_profile
cp "$CARGO_TARGET_DIR/release/examples/shared_json_startup_profile" "$BENCH_ROOT/bin/current"

python3 tools/profile_client_startup.py \
  --resources /absolute/path/to/.local/horizon-client/26.3 \
  --baseline-binary "$BENCH_ROOT/bin/baseline" \
  --a01-binary "$BENCH_ROOT/bin/a01" \
  --cf-binary "$BENCH_ROOT/bin/cf" \
  --cd-binary "$BENCH_ROOT/bin/cd" \
  --binary "$BENCH_ROOT/bin/current" \
  --repeats 10 --output "$BENCH_ROOT/five-way.json"
```

The source tree of each historical revision is exact. The baseline profiler is
an added standalone measurement example, and therefore does not alter its
historical loader logic. Comparisons use alternating forward/reverse process
order, separate processes, Linux `wait4` CPU time and peak RSS, and saved raw
samples with 25th/75th percentiles, extrema, build hashes and system load.
Keep concurrent compilation and other CPU-intensive jobs idle if possible.
`ru_maxrss` records process peak residency and cannot establish the real game's
memory usage after rendering begins. If system load rises, preserve rather than
discard outliers and report the uncertainty.

## Decoding passes and buffer lifetime

```sh
cargo build --offline --release -p horizon-client --example gameplay_decode_profile
python3 tools/profile_gameplay_decode.py \
  --resources /absolute/path/to/.local/horizon-client/26.3 \
  --repeats 10 --output "$BENCH_ROOT/decode.json"

python3 tools/profile_client_startup.py \
  --resources /absolute/path/to/.local/horizon-client/26.3 \
  --variant released="$BENCH_ROOT/bin/current"@deferred \
  --variant retained="$BENCH_ROOT/bin/current"@deferred-held \
  --repeats 10 --output "$BENCH_ROOT/lifetime.json"
```

The decoder profile measures the `Validate` pass and typed `Gameplay` pass
separately, including the JSON read in `total_ms`. The primary facts loader
continues both passes to preserve JSON validation, error precedence and useful
diagnostics, including overwritten fields and raw geometry. The lifetime
comparison measures current process `VmRSS` after materialisation, plus peak
RSS across the process. An explicit `drop` releases owned Rust values, but the
allocator may retain physical pages, so immediate RSS savings are not implied.

When finished, `git worktree remove` each temporary historical worktree. Do
not `git clean` or reset the active PR working tree.
