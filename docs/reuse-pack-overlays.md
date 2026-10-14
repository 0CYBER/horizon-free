# Reusing resource-pack overlays

`packs::build_overlay` reuses an overlay after validating a versioned sibling
JSON marker. The marker covers pack source paths/order, active overlay roots,
compiled filters, resource topology, merged inputs and the materialized output.
SHA-256 checks content rather than relying on file size or timestamps.

On Unix, ordinary directory-pack resources and untouched vanilla resources are
live symlinks. Their bytes stay current on edits/replacements, so validation
checks topology and link targets without reading those bytes. Merged language,
sound, font and atlas JSON inputs are hashed. Vanilla directories that were
materialized by the builder are checked for additions/removals/type changes;
untouched directory symlinks stay live without a recursive vanilla scan.

ZIP archives and extracted output files are fully hashed. On platforms where
`link` copies files, directory-pack contents and copied vanilla files are also
hashed. Existing merge order and filter behavior are unchanged.

A persistent sibling `.overlay-cache.lock` uses an advisory OS file lock to
serialize validation, rebuilding and publication for the same output directory.
The lock inode is never removed by the builder. The old marker is removed before
rebuilding; its replacement is renamed into place only after a successful build
and stable input checks. Missing/corrupt markers or output trigger rebuilding.
Directory packs with absolute/parent-traversal overlay paths or symlink cycles
build through the original path without publishing a reusable marker. External
overlay paths retain their previous behavior.

## Reproduce the benchmark

```sh
CARGO_TARGET_DIR=/home/bob/Documents/horizon/target cargo test -p horizon-client --lib --profile fast packs::cache::tests::benchmark -- --ignored --nocapture --test-threads=1
```

The ignored tests compare the original builder body (dependency recording
disabled) with the production cache path on identical inputs. They warm both
paths, collect 11 repetitions and compare output signatures after each pair.
The unchanged timing includes locking and all input/output validation. Cold
means the marker is absent but filesystem caches are warm; it is not a
cold-disk measurement.

Fixtures are synthetic because prepared game assets were unavailable in this
workspace: 8,192 vanilla texture files (128 MiB), 2,048 overrides (32 MiB), and
32 merged language files with 2,000 vanilla/1,000 pack keys each. Texture bytes
are deterministic incompressible payloads approximating already-compressed PNG
data; this benchmark does not decode images. The directory and deflated ZIP
cases contain equivalent resources. A separate four-file/32 MiB directory case
checks that live content does not impose a large hashing cost.

Measured on 2026-10-14: Linux 7.2.8-zen1-2-zen x86_64, ext4/NVMe,
AMD Ryzen 5 5600X (6 cores/12 threads), Rust 1.99.0, `fast` profile.
The deflated ZIP is 34,030,189 bytes (32.45 MiB).

| Fixture | Original rebuild | Cached cold/rebuild | Unchanged validated hit | Hit speedup |
| --- | ---: | ---: | ---: | ---: |
| Directory pack | 107.129 ms | 184.868 ms | 50.484 ms | 2.12× |
| Deflated ZIP pack | 152.032 ms | 271.798 ms | 88.416 ms | 1.72× |
| Directory, four 8 MiB files | 0.112 ms | 0.316 ms | 0.122 ms | ~10 µs overhead |

The two larger cold paths are 73–79% slower because they also establish and
verify cache dependencies/output. The large-file/few-file case adds about
0.2 ms cold and 0.01 ms on an unchanged call; reading all of its live texture
bytes would instead cost tens of milliseconds, so those bytes are excluded.

The cache adds a small marker and persistent empty lock file per overlay. Hashes
stream through a 64 KiB buffer rather than retaining archive/resource contents
on cache hits. Initial/rebuilt overlays do extra validation and marker work;
tiny overlays can have a small unchanged-call overhead. Measurements cover
overlay construction only, not total startup time or frame rate. Non-Unix
copy behavior is covered in the implementation but was not runtime-tested here.
