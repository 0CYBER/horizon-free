# Reusing resource-pack overlays

`packs::build_overlay` reuses a completed overlay through a versioned sibling JSON marker and persistent OS lock. Pack sources/order, overlay roots, filters, source topology and materialized vanilla inputs determine validity. The marker is removed before rebuilding and published only after a successful build and stable input checks.

On Unix, ZIPs and materialized inputs use length, modification time, device/inode and change-time stamps. Cache hits do not read archive bytes or traverse generated output. Directory packs still validate their topology; ordinary linked files and untouched vanilla subtrees remain live. Materialized vanilla directory stamps detect membership changes without listing them again. Only marker identity resolves paths; the builder retains the caller's original symlink targets and root aliases.

On non-Unix platforms, materialized files retain content hashing because portable metadata lacks a change timestamp. That path was not runtime-tested. External folder overlays and symlink cycles retain uncached behavior. Missing/corrupt markers and a missing output root rebuild. Generated output is owned cache data: external edits to individual output files are deliberately not detected; delete the sibling marker or overlay directory to rebuild it.

## Real pack measurements

The ignored test measures the actual production `resource_root` entry point against its original selection/key/build sequence. Two warmups and 11 alternating samples use identical prepared Minecraft 26.3 resources (manifest schema 7). Complete materialized output bytes and symlink identities are compared outside timing. Cold means an absent marker with warm filesystem caches, rather than cold storage.

Linux x86_64/ext4/NVMe, Ryzen 5 5600X, Rust 1.99.0, Cargo `fast`, measured 2026-10-15:

| ZIP pack | Original rebuild | Cached cold | Unchanged hit | Saved per hit |
| --- | ---: | ---: | ---: | ---: |
| Faithful 32x, 26.3 September 2026 | 159.827 ms | 162.711 ms | 0.707 ms | 159.120 ms |
| Default HD 128x demo, 1.8.2.7 | 111.501 ms | 114.386 ms | 0.310 ms | 111.191 ms |
| FPBR, 1.21.9 R1.3 | 347.639 ms | 348.012 ms | 0.448 ms | 347.191 ms |

Unchanged speedups are approximately 226x, 359x and 775x. Cold overhead on these ZIPs is 0.1–2.6%, replacing the original cache implementation's 73–79% synthetic overhead. This measures overlay assembly and cache validation, excluding scan, texture decoding, total startup and FPS.

Mac mini (M4 Pro, APFS, Rust stable, Cargo `fast`), same harness and packs, measured 2026-10-15 during review while the machine ran other builds:

| ZIP pack | Original rebuild | Cached cold | Unchanged hit | Saved per hit |
| --- | ---: | ---: | ---: | ---: |
| Faithful 32x, 26.3 September 2026 | 1,184.400 ms | 1,295.069 ms | 0.571 ms | 1,183.829 ms |
| FPBR, 1.21.9 R1.3 | 5,292.149 ms | 5,503.965 ms | 0.391 ms | 5,291.758 ms |

Creating thousands of files and links costs far more on macOS than on Linux, so the saving there is seconds per startup with a pack enabled.

Pack downloads were verified against publisher SHA-512 metadata. Exact versions, URLs, sizes, asset counts, hashes and prepared-manifest identity are recorded in `benchmarks/overlay-real-packs-2026-10-15.json`. Sources: [Faithful](https://modrinth.com/resourcepack/faithful-32x/version/lDYpMiqk), [Default HD](https://modrinth.com/resourcepack/default-hd-128x/version/rErHu3BE), [FPBR](https://modrinth.com/resourcepack/fpbr/version/dynf82xk). FPBR targets older 1.21.9 snapshots; it is measured as a large real 128x archive over prepared 26.3 resources, without claiming visual/version compatibility. Default HD is the publisher's free demo. Downloaded pack contents are not committed.

## Reproduce and validate

```sh
export CARGO_INCREMENTAL=0
export HORIZON_OVERLAY_BENCH_VANILLA=/path/to/prepared/26.3
export HORIZON_OVERLAY_BENCH_PACKS='["/path/to/Faithful.zip","/path/to/DefaultHD.zip","/path/to/FPBR.zip"]'
cargo test -p horizon-client --lib --profile fast packs::cache::tests::benchmark_real_resource_root -- --ignored --nocapture --test-threads=1
cargo test -p horizon-client --lib --profile fast packs:: -- --nocapture
cargo clippy -p horizon-client --lib --tests --profile fast -- -D warnings
cargo fmt --all -- --check
```

Thirteen focused tests pass, covering source edits/replacements, Unix preserved-mtime edits, pack order/removal, overlays/filters, vanilla membership, marker/output-root recovery, failed ZIP rebuilds, symlink aliases/cycles, contention and unchanged relative link targets. The opt-in real-pack benchmark passes; strict Clippy, formatting and whitespace checks pass. Each overlay retains a small marker and empty lock file; cache hits retain no archive byte buffer.
