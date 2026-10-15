# Entity model document loading

The three startup batches (boats/chest boats, minecarts, armor stands/player armor)
now share one parsed document. For this fixture, that replaces 43 full document
reads/parses (boat discovery plus 42 selected layers) with one. Selected models
still produce independent `Part` trees; the document is dropped after the batches
are built. Other entity loaders retain their existing behavior.

## Reproduce

From the repository root:

```sh
python3 benchmarks/entity-model-document-2026-10-14/generate_fixture.py /tmp/horizon-entity-model-document-fixture.json
ENTITY_MODEL_BENCH_DOCUMENT=/tmp/horizon-entity-model-document-fixture.json \
  CARGO_TARGET_DIR=/home/bob/Documents/horizon/target \
  cargo test -p horizon-client --profile fast --bin horizon-client \
  entity_model_document_benchmark -- --ignored --nocapture
```

`CARGO_TARGET_DIR` is optional and can be changed for another machine. Supply a
prepared `entity-models.json` through `ENTITY_MODEL_BENCH_DOCUMENT` to benchmark
that document instead; the harness copies only that file to a temporary root.
`ENTITY_MODEL_BENCH_REPETITIONS` controls repetitions (default 15).

The opt-in Rust test calls the actual three optimized production loaders and
compares every selected part name, pose, polygon normal/vertex and child against
the previous algorithm using the compatible per-model `load_model` API. Timings
include reading, parsing, constructing, normalizing into a common result map, and
dropping the result. Each variant gets three warmups and 15 measured calls, with
order alternating. `black_box` consumes inputs/results. No other builds or
optimization benchmarks ran during the measured calls.

## Input and limitations

Prepared resources were unavailable in the repository and local Horizon asset
directories. This is a **synthetic** 2,473,695-byte document with 458 model layers,
each containing 12 posed parts with six exported quads and normals. It selects 22
boat/chest-boat layers, six minecart layers and 14 armor/player layers (42 total).
One wood name is synthetic (`test_wood`); boat discovery accepts arbitrary wood
names. The extra layers model the cost of parsing a full production-size document
for each selected model.

File data is warm in the OS page cache. Equipment files are absent, so equipment
texture loading is excluded. This measures the entity-model batch only and does
not establish overall startup, frame time or FPS improvement. Shapes/layer counts
in real prepared documents can produce different absolute timings.

Machine: AMD Ryzen 5 5600X (6 cores / 12 threads), Linux x86_64,
Rust 1.99.0 (b940084d7, LLVM 23.1.1), Cargo `fast` profile (opt-level 3,
16 codegen units, no LTO). Baseline algorithm corresponds to main
`73a019967e9665c6b96332988c13cb66679d8477`.

## Results

| Variant | Median batch time | Full document reads/parses |
| --- | ---: | ---: |
| Previous per-model algorithm | 1,434.974 ms | 43 |
| Shared document | 33.420 ms | 1 |

The measured batch is **42.94× faster** on this synthetic input. All 15 samples
for both variants, warmup count, document size and selected model count are in
[`results.json`](results.json).

## Validation

```sh
cargo fmt --all -- --check
CARGO_TARGET_DIR=/home/bob/Documents/horizon/target \
  cargo test -p horizon-client --profile fast --bin horizon-client \
  entity_model_loading_tests -- --nocapture
CARGO_TARGET_DIR=/home/bob/Documents/horizon/target \
  cargo clippy -p horizon-client --profile fast --lib --bin horizon-client --tests -- -D warnings
```

The regular tests cover full/sparse documents, unrelated layers, a missing file,
invalid JSON, non-object JSON, missing model keys, independent model trees, and
equipment loading when the document is unavailable.
