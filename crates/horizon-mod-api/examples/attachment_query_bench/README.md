# Native attachment query benchmark

Run from the repository root:

```sh
cargo build -p horizon-mod-api --profile fast --example attachment_query_bench
taskset -c 2 target/fast/examples/attachment_query_bench
```

`taskset` is optional and Linux-specific. It pins the measurement, not the build.
The fixture implements the ordinary Host area methods over real runtime attachment
maps, their mutation observer, and their index. It calls the real typed facade
through an opaque `dyn Host`. The baseline acquires and traverses that same index,
with the same checked typed reads, holder identities and checksum. It is not a
contiguous `Vec<u32>` baseline or a substitute query implementation.

Warm results are medians of 9 samples after one warm-up sample, 1,000 traversals
per sample. Acquisition baseline/API ordering alternates. Both acquisition plus
traversal and traversal-only overhead must remain within approximately 20%.
Timing assertions run only when this example is explicitly executed, not in CI.

## Measured result

Intel Core i9-13900H, Rust 1.99.1, `fast` profile (optimized, no LTO), pinned to
logical CPU 2. Times are microseconds for 10,000 holders unless stated otherwise.

| Measurement | Raw / ordinary | Typed query / pinned snapshot |
| --- | ---: | ---: |
| Warm acquisition + traversal | 126.006 | 126.272 (+0.21%) |
| Warm traversal alone | 138.505 | 143.270 (+3.44%) |
| First write | 8.742 | 347.852 |

Warm acquisition plus traversal made **zero allocations**. A previous pinned run
of the corrected harness measured -16.08% acquisition-plus-traversal and +14.87%
traversal-only. Treat that variability as noise, not a speedup claim. The first
unfixed harness measured +23.67% while imposing a dynamic `take(limit)` on only
the typed warm path; removing that unequal work corrected the comparison. No
production query implementation was changed to obtain these results.

| Cold consumption | Lazy time | Eager predecode time | Lazy / eager allocations |
| --- | ---: | ---: | ---: |
| First 1,000 of 10,000 rows | 220.658 | 2,062.632 | 4,000 / 40,000 |
| All 10,000 rows | 1,983.808 | 2,336.349 | 40,000 / 40,000 |

Eager means predecoding the whole captured batch through the same production read
path before traversal, not a hypothetical redesigned contiguous storage engine.
**Keep lazy decoding**: partial consumers avoid 90% of decode allocations, and
predecoding did not improve a complete consumer in this measurement.

Requested live memory (not resident memory or allocator bookkeeping):

- Maps, index, decoded u32 values and their serialized payloads: 19,460,360 bytes.
- Increment attributable to binding/indexing the same maps: 4,700,192 bytes.
- First write with a pinned snapshot: 16 allocations, 640,908 requested bytes.
- Ordinary unpinned write: 14 allocations, 868 requested bytes.

The no-query runtime control makes zero allocations and retains exactly the same
bucket. This is **not a full Game idle-tick measurement**; that is separate proof.
