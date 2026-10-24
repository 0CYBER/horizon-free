# Retired Java performance packs

Before the Rust rewrite, Horizon shipped a Fabric optimization mod, performance pack profiles
and a Prism Launcher Mac starter. That product is retired. Its patches, profiles, builders,
starter, benchmark viewer and product-specific checks have been removed; use Git history
for the former implementation. Horizon's Rust game does not use those Java patches.

The Java parity oracles in `mods/horizon/src/test/java/dev/horizonlab/parity/` remain part of
[the Rust parity workflow](parity-workflow.md). The Fabric and NeoForge benchmark source sets
also remain: [comparison benchmarks](benchmark.md) still run real Java Minecraft against Rust.
Their pinned third-party dependencies live in `tools/benchmark-deps/`; they do not include the
retired Horizon optimization mod. [JFR analysis](profiling.md) remains available for Java comparisons.

For the current game, see [build from source](../README.md#build-from-source).
For server hosting, use [horizon-server](hosting.md).
