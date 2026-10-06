# Neighboring model-face visibility

`horizon-world::block_render::FaceVisibility` decides whether to emit a model
quad with a culling direction beside a neighboring block. It prepares immutable
face geometry from the pinned raw occlusion shapes, shares faces across states,
and caches shape-pair subtraction with bounded memory. The development client
now uses it for resident chunk model meshing, including cross-chunk neighbors.

The decision preserves the native order: a canonical full neighbor face hides
the quad, block-specific suppression can hide it, canonical empty faces bypass
the shape comparison, and partial faces use `ONLY_FIRST` subtraction. Full
occlusion is computed from geometry, independently of collision or support.

Block-specific suppression covers every method owner in the pinned registry:
ordinary blocks, same-block glass/ice and powder snow, bars/panes, leaves,
liquids, and mangrove roots. Bars respect the loaded bars tag, facing connections,
and the neighbor's boolean property availability. Leaf suppression is controlled
by the cutout-leaves setting. Liquids compare fluid families, including contained
water in waterlogged neighbors. Mangrove roots suppress vertical internal faces.

The [native comparison](../benchmarks/rust-face-visibility-2026-10-06/scoped-parity.json)
calls actual `BlockState.skipRendering` and `Block.shouldRenderFace` with pinned
tags bound in both reference JARs. It covers all 35,723 states and 1,286 block
definitions: 1,441,816 ordered state pairs, six directions and two leaf settings
(17,301,792 direction/setting cases). Every overriding family has exhaustive
same-block state pairs; bars and panes also have exhaustive cross-material
connection pairs. Representative and seeded random neighbors exercise partial
occlusion across the rest of the registry. Both references match 2,883,632 output
lines, each containing suppression and visibility masks for all six directions.

```sh
python3 tools/test_face_visibility_parity.py
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The private imports contain raw shape grids, definition flags, class-rule
assignments and tag membership. Rust computes the behavior being compared.
Unknown state IDs produce errors. Native fixture files, reference JARs and traces
stay under ignored `.local/`; only hashed reports are committed. Lighting,
translucent sorting, fluid surfaces, GPU pixels and gameplay remain outside this
comparison. Native macOS and Windows runs are still required.
