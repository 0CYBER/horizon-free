# Block-state JSON codecs

`BlockStateJsonCodec` implements compact and full Java Edition **26.3** block-state
JSON forms over the imported `BlockRegistry`, with integer property kinds supplied
alongside the registry. Compact encoding uses a block name for the default state;
full encoding uses an `id` object and properties for blocks with multiple states.

Missing or invalid properties retain their defaults. Unknown property names are
ignored. Integer properties accept Java decimal strings, including signs, leading
zeroes and JDK 25 BMP decimal digits; enum and boolean properties use exact names.
This codec is separate from `BlockRegistry::resolve`, whose descriptor validation
continues to reject invalid properties.

Run `python3 tools/test_block_state_codec_parity.py`. The
[local report](../benchmarks/rust-block-state-codec-2026-10-06/scoped-parity.json)
records **221,261 matching output lines per reference JAR**, covering all
**35,723 states**, **1,286 blocks**, **146 integer properties** and **185,538 decoding
inputs**. Tests exercise default/full encoding, namespaces, invalid types,
property fallback, integer overflow and duplicate fields.

The two authenticated JARs must export identical state metadata. Exports and raw
comparison traces remain private under `.local`. The public report contains
metadata and hashes. This validates codec behavior over imported identities;
block gameplay, registry lifecycle, warning messages and arbitrary malformed
JSON/unpaired-surrogate text remain outside the tested scope.
