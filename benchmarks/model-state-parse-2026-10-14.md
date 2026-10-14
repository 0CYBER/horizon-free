# Reuse parsed model states

`models::prepare` parsed `model-states.json` to select blocks, then
`StateModels::load` reopened and parsed the same document for binding and baking.
`StateModels::from_facts` now borrows the first parsed map. The public `load`
entry point still reads, parses and validates its own document. Both paths retain
the `fs::read` + `serde_json::from_slice` strategy merged in PR #205.

The shared map uses `BTreeMap`, preserving the original catalog loader's JSON
validation, including rejection of top-level `null`. The old selection map was
`serde_json::Map`, also backed by a BTreeMap with the project's current features.
Catalog binding, model selection and baking are unchanged. Preparation no longer
holds a second complete JSON tree while constructing the catalog; no persistent
cache or extra clones were added.

## Measurement

The committed `model_states_bench` example runs main's two-parse sequence after PR #205
(first a serde JSON map, then `StateModels::load`) and the new one-parse sequence
(first a BTreeMap, then `StateModels::from_facts`). Both select blocks from the
same generated document with slice parsing and call the actual catalog binding/baking code. Timing
includes file reads, parsing, filtering, binding, baking and result destruction.
Fixture generation and equality checks run outside the timed region.

Prepared Minecraft resources were unavailable locally, so this is a synthetic
2,836,696-byte document containing 843 block schemas and 53,952 states. Schemas
combine integer and named properties. Blockstate resources use weighted and
conditional multipart choices with rotations and UV locking, sharing one model
with a baked face. Its absent texture uses the normal missing-sprite fallback.
Both paths read identical files with a warm OS cache. The workload does not
represent the full diversity of production model and texture assets.

On AMD Ryzen 5 5600X, Linux 7.2.8-zen1-2-zen x86_64, rustc 1.99.0
(b940084d7 2026-09-28), Cargo `fast` profile, serial execution:

| Path | Median (15 samples) |
| --- | ---: |
| Before: parse twice, with #205 slice readers | 96.080 ms |
| After: reuse first parse, with #205 slice readers | 73.585 ms |

**1.306x speedup; 23.41% less elapsed time** for this loading segment after PR #205. Each path
has two warmups, sample order alternates, and inputs/results use `black_box`.
This excludes gallery placements, GPU work and overall application startup.

These measurements supersede the original unbuffered-reader timings
(2732.004 ms → 1386.060 ms, 1.971x) after rebasing onto main at
`e96e1571`. The reviewer measured approximately 1.05x with real prepared resources;
the generated fixture's balance of parsing and binding differs from production.
The change still removes one complete document read, parse and JSON tree.

Equality validation compares every state's selected variants and baked geometry
at three positions, plus sprite layers and opacity/translucency flags. The
focused test also checks matching schema/registry errors, malformed/missing
source files, rejection of `null`, and successful shared-facts loading after the
source file has been deleted.

## Reproduce

```sh
export CARGO_TARGET_DIR=/home/bob/Documents/horizon/target
export CARGO_INCREMENTAL=0
cargo run -p horizon-client --profile fast --example model_states_bench -- 15
cargo test -p horizon-client --profile fast --example model_states_bench
cargo clippy -p horizon-client --all-targets --profile fast -- -D warnings
cargo fmt --all -- --check
```

The target directory can be changed for another machine. The example creates and
removes its fixtures in the system temporary directory.
