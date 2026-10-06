# Vertical anchors

Horizon implements the pinned Java Edition **26.3** vertical anchors: absolute,
above bottom, below top and relative to sea level. Runtime resolution preserves
signed integer wrapping. Codec parsing and encoding enforce the vanilla field
range, preserve decimal truncation and integer wrapping, and follow the nested
exclusive alternatives even when multiple fields appear in one object.

`WorldGenerationContext` carries the minimum, height and sea level. Its generator
adapter chooses the larger minimum and smaller height independently, matching
vanilla; this differs from intersecting the vertical intervals.

Run `python3 tools/test_anchor_parity.py`. The
[local report](../benchmarks/rust-anchor-2026-10-06/scoped-parity.json) records
**181,632 matching output lines per reference JAR**, across **192 fixtures**,
**74,112 codec inputs** and **107,328 runtime anchors**. The original Java
transport calls the actual codecs, helper constructors and generation context,
including the native noise-based chunk generator for bounds comparisons.

Sources and fixtures are hash-checked before publishing results. Raw reference
output stays under `.local`. These tests cover anchor behavior and the three
context values, not complete material rules, level access or JSON diagnostic text.
