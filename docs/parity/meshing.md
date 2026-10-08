# Section meshing parity

`tools/test_section_mesh_parity.py` compares the vertices Horizon's section
mesher (`crates/horizon-client/src/mesher.rs`, with `mesher/lighting.rs` and
`mesher/fluid.rs`) emits with the vertices the pinned client's own renderers
emit for the same scenes.

## What is compared

- **Vanilla side**: `VanillaSectionMeshOracle` bootstraps the game, binds the
  prepared block tags, parses the jar's blockstate definitions and block
  models, and bakes `BlockStateModel`s through the real `ModelDiscovery`,
  `UnbakedRoot.bake` and `FaceBakery` path. Every texture becomes a real sprite
  (its PNG from the jar) alone in a unit atlas, so UVs stay in sprite-local
  [0,1] space; the five fluid sprites share one strip with gaps so the fluid
  vertex stream still identifies them. A fixed `BlockAndTintGetter` serves the
  scene's block states, per-cell sky/block light, constant biome colours per
  `ColorResolver` and the overworld or Nether `CardinalLighting`. Blocks of one
  section are visited like `SectionCompiler.compile`: fluids through
  `FluidRenderer.tesselate`, then `RenderShape.MODEL` blocks through
  `ModelBlockRenderer.tesselateBlock` with smooth lighting on, culling on,
  cutout leaves, `BlockColors.createDefault()` and `state.getSeed(pos)`.
- **Horizon side**: `crates/horizon-client/examples/section_mesh_fixture.rs`
  loads the prepared resources (`Assets`, `BlockFacts`, `Biomes`,
  `load_visibility`, `MeshTables`), builds the same blocks and light in real
  client `Chunk`s (a 3x3 chunk region), sets every biome's colours to the
  scene constants and runs `Mesher::mesh`.
- **Comparison**: quads are matched as multisets per scene, keyed by sprite,
  render-layer class (translucent vs. solid/cutout) and geometry rounded to
  1/1000 after binary32 normalisation, with a winding-preserving rotation.
  Matched quads compare per vertex: positions and UVs within 1e-4, ARGB colour
  and light coordinates (sky and block, 1/16 steps) exactly.
- **Per-state facts**: a second gate compares, for all 35,723 states, the facts
  the Rust tables derive from prepared resources with the client's accessors:
  `getShadeBrightness`, `emissiveRendering`, `isSolid`, `isLightPermeable`,
  `isSolidRender`, `isCollisionShapeFullBlock`, light emission, the water
  overlay classes (`HalfTransparentBlock`/`LeavesBlock`), tint source counts
  and `getOffset` at five columns.

The corpus (94 sections, ~842k quads, ~206k of them fluid quads) has full
cubes in open air, corners, cavities and overhangs; slabs, stairs, walls,
fences, panes, bars, torches, flowers and grasses with XZ/XYZ offsets, tall
plants, tinted blocks (grass, leaves, stems, redstone, lily pad, petals, leaf
litter, cauldrons), snow layers, carpets, farmland, paths, hoppers, anvils,
chains, lanterns, dripleaf, dripstone; water and lava sources, flowing levels,
falling columns, waterlogged blocks, water against glass/leaves/slabs/ice;
section edges with neighbours in other chunks and sections; a cave with
emissive and light-emitting blocks (magma, glowstone, sea lantern, active
sculk sensor, glow lichen); Nether shading; every block's default state; every
`RenderShape.MODEL` state of the registry on a sparse grid; and dense random
scenes mixing all model states with fluids under propagated or random light.

## Running

```sh
python3 tools/test_section_mesh_parity.py            # full corpus
python3 tools/test_section_mesh_parity.py --quick    # skip the all-states catalogue
python3 tools/test_section_mesh_parity.py --strict   # exit 1 on any difference
python3 tools/test_section_mesh_parity.py --only fluids
```

It needs the pinned jars (`tools/lab.py prepare`) and prepared client resources
(`tools/prepare_native_client.py --prepare`). Outputs, logs and
`report.json` go to `target/section-mesh-parity/`; nothing is written under
`.local/`. The fixture is built with `--profile fast` and reports its
meshing time.

## Results

Before the port (commit `2940b5a`, first 18 scenes): 25,391 of 55,017 vanilla
quads identical (46%); 7,685 fluid quads missing, 9,231 extra quads, and
light/colour differences on most of the rest.

After: all 841,775 quads of the 94 scenes match and are identical, and all
per-state facts match.

What was ported to get there:

- `BlockModelLighter`: quad shape (`faceCubic`/`facePartial`), the corner,
  diagonal and centre samples (diagonals only through light-permeable blocks,
  the centre from the neighbour unless it is solid-render), shade brightness
  overrides, emissive full-bright samples, `smoothBlend` (zero samples take the
  centre, sky-less samples borrow its sky, only when the centre is above 2),
  the partial-face weights with `smoothWeightedBlend`'s int narrowing, and the
  vertex remap. The padded section border grew to two blocks because cubic
  faces sample two blocks out along their normal.
- `ModelBlockRenderer`: flat lighting for light-emitting blocks or when the
  first model part disables AO (culled quads read light from the cull
  neighbour, others from the neighbour only when flush), `ARGB.gray`/
  `scaleRGB`/`multiply` integer arithmetic, cardinal shading including
  `NETHER`, shade-direction overrides, per-element light emission, tint source
  counts (layer indices past the list are untinted; water and bubble columns
  are untinted in the world) and `getOffset` for XZ/XYZ blocks.
- `FluidRenderer`: face culling against the neighbour's occlusion face shape
  for the fluid box height, self occlusion, `isSolid` corner heights, the
  weighted corner averages in vanilla order, the 0.001 insets (also feeding the
  side faces), `FlowingFluid.getFlow` with `Mth.atan2` and the sine table, the
  water overlay beside half-transparent and leaf blocks, back faces,
  `max(self, above)` light, bottom light from below, and cardinal shading.
- `UnbakedCuboidGeometry.bake`: elements flat along an axis draw no faces
  across it (`bake_block_model` in horizon-assets; the raw
  `bake_model_transformed` used by the model-baking gate is unchanged).

Exported facts used: `block-behaviours.json` (class chains, `isSolid`, full
face sturdiness), `gameplay-facts.json` (`offset` flag, emission, shapes),
the `#minecraft:blocks_fluid_flow` tag and the occlusion faces already loaded
for face visibility. Offset types and maximum offsets, shade-brightness and
emissive overrides are keyed by class or block name and checked by the facts
gate.

## What remains different or untested

- Biome tint blending: vanilla blends `BiomeManager`'s zoomed biome at each
  block's own height; Horizon blends its quart biome cells at the section's
  mid-height. The gate uses one constant colour per resolver, so this is not
  compared.
- Model variant seeds: `DoublePlantBlock`, `DoorBlock` and `AbstractBedBlock`
  override `getSeed`; Horizon seeds every block from its own position. No
  vanilla block among those has weighted variants, so output is unaffected.
- Settings: only smooth lighting on and cutout ("fancy") leaves are covered;
  Horizon has no flat-lighting or opaque-leaves modes.
- Layers: vanilla's solid and cutout layers are compared as one class because
  Horizon draws both in its opaque pass.
- Not compared: block entities and their renderers, animated sprite frames,
  translucent sorting, chunk-border light outside loaded chunks and GPU
  pixels.
