# Shared startup release response

Startup previously fetched `/releases?per_page=100` on two worker threads: one
for game releases and one for updates. It now decodes that response once and
parses game releases once, publishing them before older-launcher fallback
lookups. Manual and four-hour update checks still request fresh data. Release
page retries remain independent. Existing self-update guards still apply.

Run the committed benchmark against the actual backend implementation:

```sh
CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 \
  cargo test --manifest-path launcher/Cargo.toml --locked --lib \
  backend::release_loading_tests::benchmark_shared_startup_response \
  -- --ignored --nocapture
```

Measured on 2026-10-14, Linux x86_64, AMD Ryzen 5 5600X, Rust 1.99.0,
unoptimized Cargo test profile with debug information disabled. A shared
`CARGO_TARGET_DIR` was used for local build reuse. Each case warms up 20 times
and takes the median of 201 samples; outputs are validated for equivalence
and passed to `black_box`.

| Scenario | Before median | After median | Ratio | Fetches per startup |
| --- | ---: | ---: | ---: | ---: |
| Serial parsing/decoding work, zero transport delay | 14.701 ms | 7.339 ms | 2.003x | 2 → 1 |
| Concurrent startup workers, 5 ms per mock request | 14.778 ms | 13.642 ms | 1.083x | 2 → 1 |

The measured 201 samples make 402 mock release-list fetches before and 201
after. The concurrent case reduces elapsed time by 7.7%; the two old requests
can overlap, so halving requests does not halve startup latency.

The synthetic 453,197-byte JSON contains 99 game releases (including a draft)
with roughly 4 KiB of notes each and one launcher release with a platform asset.
The mock fetch decodes identical bytes for each request; it does not exercise
HTTP, TLS, GitHub rate limits, filesystem installation scans, GUI rendering or
self-update downloads. The comparison covers backend orchestration, JSON
decoding, sorting, filtering and update selection, not end-to-end launcher
startup or release-build timing. Fallback timing is omitted from the benchmark
and its ordering/error behavior is tested separately. No persistent cache or
extra copy of the game-release vector is introduced.

Validation: 26 launcher library tests passed (benchmark ignored by default);
the benchmark passed explicitly; full launcher all-target check and Clippy
with `-D warnings` passed; root and launcher format checks and `git diff --check`
passed. The GUI compiled but was not launched interactively.
