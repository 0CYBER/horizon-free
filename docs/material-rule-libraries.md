# Material-rule libraries

`MaterialRuleLibrary` loads the pinned Java Edition **26.3** rule resources against
block-state, condition, noise and density registries. It supports block rules,
ordered sequences, conditional rules, terracotta bands, ore veins and nested named
rule references. Loading is atomic; missing references, cycles and duplicate
canonical names return errors. Limits are 8,192 registered definitions, 65,536
parsed nodes and 256 nested levels.

The library owns a density compiler configured for one world seed and random
mode. `rule(name)` resolves a runtime tree and lazily compiles its density roots.
Binding that tree to `MaterialConditions` performs per-chunk density sampling.
`named_density(name)` shares the compiler with preliminary-surface sampling.
`compile_router` and `compile_aquifer` bind configurations decoded into the same
density registry before it is passed to the library. Their samplers share the
material roots' cache namespace, so noise filling and surface application can
reuse a worker context. Workers need matching world random settings and separate
cached sampler contexts.

Run `python3 tools/test_material_rule_library_parity.py`. The
[local report](../benchmarks/rust-material-rule-library-2026-10-06/scoped-parity.json)
records **678,174 matching output lines per reference JAR**, including **665,712
rule applications** and **142 codec inputs** across 12 fixtures. The test loads all
**42 bundled rules**, **8 conditions**, **55 densities**, **64 noises**, **67 biome
climates** and **86 biome tags** from both authenticated reference JARs. It also
checks synthetic forward references and constant-density ore rules.

Comparisons use actual seeded noise and ore densities, strided chunk caches,
scalar fallback, varying biomes/depths/heights, possible-biome sets and distinct
context/application coordinates. Surface depth and secondary noise are supplied
context inputs; preliminary density is zero to isolate rule evaluation.

The [surface stage](material-surfaces.md#chunk-surface-stage) now applies these
rules to chunks. Rule/density encoding, live reload, registry-owner checks and
playable integration remain pending. The report establishes
this tested scope, not complete vanilla gameplay parity or an FPS improvement.
