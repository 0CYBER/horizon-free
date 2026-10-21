# Java performance mods: which of their techniques apply to Horizon

A survey of the techniques used by eight Java Edition performance mods. Asked for in #991, from famescare on Discord. For each technique it gives the vanilla cost it targets, whether Horizon already avoids that cost, and whether it is worth measuring here.

**Rules.** Techniques only, taken from each mod's public description, documentation and changelogs (linked below). No mod code was read or copied. Several of these mods are LGPL-3.0, and Horizon's code stays its own. Anything Horizon adopts is reimplemented against vanilla 26.3's behaviour. It keeps parity, so the harnesses stay unchanged, and it needs before/after numbers like any other perf PR (`docs/benchmark.md`, `docs/profiling.md`).

**Why many don't apply.** A large share of these techniques work around costs of the JVM, OpenGL or GLFW rather than costs of the game's rules. Examples are object headers and duplicate objects, boxing, lambdas and Streams, garbage collection, JNI upcalls, immediate-mode GL draws and GL error checks. Horizon is Rust on wgpu and winit, so it never pays those costs. The techniques that do carry over remove work the game's rules don't need: sleeping block entities, skipped searches, and cheaper geometry tests.

Verdicts:
- **has**: Horizon already avoids the cost, by its design or with a merged change.
- **n/a**: the cost comes from the JVM, OpenGL or GLFW, and Horizon doesn't have it.
- **later**: missing, and worth measuring. These are listed in "Worth measuring" below and in the LATER list.
- **no**: missing, but it would change vanilla's behaviour or look, or the gain here is too small to pay for itself.

## Lithium (server and shared game logic)

Source: [Lithium's mixin configuration](https://github.com/CaffeineMC/lithium/blob/develop/lithium-fabric-mixin-config.md). Lithium keeps vanilla's behaviour, which is the same rule Horizon follows.

| Technique | Vanilla cost | Horizon | Verdict |
| --- | --- | --- | --- |
| Hopper caching, inventory change listeners and hopper sleeping | Every hopper looks up its source and target containers and tries a transfer every tick, even when nothing can move | `game/machines.rs` ticks every hopper on each tick. It checks the block's name and cooldown, then runs the transfer attempt. Nothing is cached or put to sleep | **later** |
| Block entity sleeping (furnaces, brewing stands, campfires, crafters, shulker boxes, sculk) | Idle block entities still tick | `game/block_entity_ticks.rs` ticks every ticker each tick, as vanilla does | **later** |
| Entity collisions: faster block access, chunk caching, the supporting block first | Collision scans read blocks through generic level lookups | Collision scans already run over Horizon's own section arrays (`horizon-world` `block_collision_scan`). A counting cursor that does fewer lookups is open as #659 | **has** (more in #659) |
| Specialised cuboid and empty `VoxelShape`s, precomputed shape arrays | Generic voxel shape intersection and merging | `horizon-core::geometry` has its own immutable shapes and box lists (`docs/collision-geometry.md`) | **has** |
| POI search and portal search | POI lookups go through vanilla's stream-heavy search | Section-indexed POIs (`poi_sections.rs`), searched in vanilla's order | **has** |
| Pathfinding: cached node types per block | Node evaluation reads the same blocks over and over | Not measured. Mob pathfinding costs haven't been profiled against vanilla yet | **later** (profile first) |
| Explosions: one access per block on a ray, skip air, skip exposure work | Each ray step reads the block and the fluid again | `game/explosion.rs` follows vanilla's loop. It hasn't been profiled, and TNT chains are the case to measure | **later** (profile first) |
| Random ticks: roll the chance early, fast block search | Sections with nothing that ticks randomly are still walked | Sections with nothing random-tickable are skipped (`docs/random-tick-eligibility-benchmark.md`) | **has** |
| Tick scheduler: faster collections, packed scheduled ticks | Boxed `ScheduledTick` objects in hash sets | Horizon's own tick heap and index (`horizon-core::ticks`, `docs/scheduled-ticks.md`) | **has** |
| Collections and allocation (fastutil maps, cached `values()`, fewer `BlockPos`, no Streams) | JVM allocation, boxing, GC | Rust value types, no GC | **n/a** |
| Math inlining, a smaller sine table | JIT inlining limits | Compiled and inlined by rustc. Horizon keeps vanilla's `Mth.sin` table where results depend on it | **n/a** |
| Chunk access without debug locks or bounds validation | `PalettedContainer`'s thread-check lock | Horizon doesn't have that lock (`docs/performance/paletted-container-access-results.md`) | **n/a** |
| Palette replacement and compaction | Hash palette resizing on the JVM | Horizon's own paletted containers (`horizon-core::paletted`) | **has** |
| Game event dispatchers made only for sections with a listener | Every section holds a dispatcher | Not checked. This is small next to the items above | **no** (until a profile shows it) |
| Item entity merging grouped by item type (experimental) | Each item entity checks every nearby item to merge with | Not present. It matters only with thousands of dropped items (see #996: fresh worlds drop about twice as many items as vanilla) | **later**, after #996 |
| World border cache, redstone wire double reads, fluid flow reads, framed-map scans | Small repeated reads | Each is minor. Fix them where a profile points at them | **no** |

## FerriteCore (memory)

Source: [FerriteCore's summary of its changes](https://github.com/malte0811/FerriteCore/blob/main/summary.md).

| Technique | Vanilla cost | Horizon | Verdict |
| --- | --- | --- | --- |
| One `FastMap` of neighbour states per block instead of a table per state | About 600 MB of per-state neighbour tables in large packs | Block states are numeric ids with flat tables, with no object per state | **n/a** |
| Compact property storage per state | A per-state `ImmutableMap` | Same as above | **n/a** |
| Cached multipart predicates, shared multipart models | Hundreds of thousands of model and predicate objects | Models are baked once into a shared library (`docs/shared-model-library-benchmark.md`) | **has** |
| Deduplicated baked-quad vertex arrays | Identical `int[]` per quad | Section meshes are built per section from the shared baked models, and no per-quad arrays are kept | **has** |
| Deduplicated shape caches | Duplicate cached `VoxelShape`s per state | One table of distinct shapes, indexed per state (`horizon-world` `CachedCollisionRegistry`) | **has** |
| String deduplication in model locations | Duplicate `String`s from the constructor's round-trips | No `ModelResourceLocation` objects, so the round-trip doesn't exist | **n/a** |
| Partly loaded chunk NBT kept between loading steps | The whole tag is kept | Chunks are decoded into Horizon's own structures | **n/a** |

Horizon's process memory is tracked against vanilla in `docs/process-memory.md`.

## ImmediatelyFast (client draw submission)

Source: [ImmediatelyFast on Modrinth](https://modrinth.com/mod/immediatelyfast). Its gains show only when the CPU is the bottleneck.

| Technique | Vanilla cost | Horizon | Verdict |
| --- | --- | --- | --- |
| A batching buffer for immediate-mode draws (entities, block entities, particles, text, GUI) | Many small draw calls and uploads | The HUD is one atlas and one batch of quads (`hud.rs`). Entities are grouped by material (`gfx/entity_batch.rs`). Terrain uses indirect multi-draws (`gfx.rs`, `gfx/flat.rs`) | **has** |
| HUD batching, fewer GUI draw calls | One draw per GUI element | Same batch (`hud.rs`) | **has** |
| Fast text lookup, a bigger font atlas | Per-character lookups, atlas thrash with large fonts | Glyphs live in maps in `font.rs`, drawn into the same HUD atlas | **has** |
| One texture for all map item textures | One texture per visible map | Not measured. It matters only with walls of maps in item frames | **later** (with a map-wall scene) |
| Cached closed-chest geometry | Re-tessellating chests each frame | Merged in #728 (Online5277) | **has** |
| Buffered sign text instead of redrawing it each frame | Text redrawn every frame | Sign text is recorded as text calls in the batch. Not measured with sign-heavy scenes | **later** (with a sign-heavy scene) |
| Fewer framebuffer switches | GL state changes between render types | wgpu render passes are fixed by Horizon's pipeline | **n/a** |
| GL error checking off | `glGetError` overhead | No GL. Release builds run without wgpu's backend validation and, outside DirectX 12, without its indirect-draw validation (`gfx.rs` `instance_descriptor`) | **n/a** |

## Gnetum (HUD rendering)

Source: [Gnetum on Modrinth](https://modrinth.com/mod/gnetum).

| Technique | Vanilla cost | Horizon | Verdict |
| --- | --- | --- | --- |
| Draw the HUD into an offscreen buffer in several passes, one pass per frame, and show the last complete buffer | The HUD is redrawn every frame | Horizon redraws its one HUD batch every frame | **no**. The HUD would then update at a lower rate than vanilla's, which changes what players see. Horizon's HUD is already one batch. |
| Adaptive distribution and a separate HUD FPS cap | Same | Same | **no**, for the same reason |

## ScalableLux (lighting, from Starlight)

Source: [ScalableLux on Modrinth](https://modrinth.com/mod/scalablelux).

| Technique | Vanilla cost | Horizon | Verdict |
| --- | --- | --- | --- |
| Starlight's light engine: per-section light arrays and direct propagation queues | Vanilla's light engine is a bottleneck in fast chunk generation | Horizon has its own engine with vanilla's rules (`light.rs`). New chunks are lit on the streaming worker pool, and edits are relit incrementally (`relight_block`). See `docs/performance/lighting-worker-results.md` | **has** |
| Parallel light updates | One light thread | Chunk lighting already runs on several workers during streaming | **has**. Measure the light share of fresh-chunk generation before doing more. |

## Structure Layout Optimizer (jigsaw structures)

Source: [Structure Layout Optimizer on Modrinth](https://modrinth.com/mod/structure-layout-optimizer).

| Technique | Vanilla cost | Horizon | Verdict |
| --- | --- | --- | --- |
| An octree of piece boxes instead of a `VoxelShape` for free space | Voxel shape boolean ops slow down as pieces add up | Free space is a container box minus a list of placed boxes (`worldgen/structure/jigsaw.rs` `Free`). That avoids the voxel shapes, but the scan is linear per test | **has**. An octree pays only for structures with hundreds of pieces. |
| Skip jigsaw blocks that face the structure boundary or another piece | Every parent jigsaw is matched against every child | Not present | **later** (time trial chambers and ancient cities first) |
| Skip pool elements already found not to fit at this spot | Weighted pools repeat an element once per weight point, and it is re-tested each time | Not present. Only the variant that keeps the result identical applies (no deduplication of the list, which changes layouts) | **later** |
| Cull positions outside the chunk before processors run | Whole templates are processed and then mostly discarded | Not checked | **later** (same measurement) |
| Palette-based template storage | Templates keep every block and NBT object | Templates are Horizon's own structures. Memory isn't measured | **no** (until memory shows it) |

## AsyncParticles (particles)

Source: [AsyncParticles on Modrinth](https://modrinth.com/mod/asyncparticles).

| Technique | Vanilla cost | Horizon | Verdict |
| --- | --- | --- | --- |
| Particle culling | Off-screen particles are built anyway | Vanilla already skips quad particles whose position is outside the frustum (`QuadParticleGroup.extractRenderState`, `Frustum.pointInFrustum`). Horizon builds every particle's quad (`particles.rs` `render`). This is a parity gap as well as a cost | **later** (vanilla's frustum point test) |
| Vertex transformation on the GPU | Every particle's quad is built on the CPU and uploaded every frame | Same in Horizon (`particles.rs` `render`). Instancing would upload one record per particle | **later** (measure with 10k+ particles) |
| Particle ticks on another thread | Particle ticks on the client thread | Horizon ticks particles on the game thread | **no**. Vanilla's particle ticks read the level, and racing them against block changes risks parity. Revisit only if a profile shows particle ticks are big. |
| Asynchronous light cache for particles | A light lookup per particle per frame | `particles.rs` looks light up per particle | **later**, with the item above |
| Texture animation one frame late | Animated textures tick on the client tick | Not adopted | **no**, because it changes timing |

## Ixeris (input and the render thread)

Source: [Ixeris on Modrinth](https://modrinth.com/project/p8RJPJIC).

| Technique | Vanilla cost | Horizon | Verdict |
| --- | --- | --- | --- |
| Event polling on the main thread, rendering on its own thread | GLFW event polling and JNI upcalls take time from rendering | winit, without JNI. Input and rendering share the event loop (`game.rs`, `ApplicationHandler`) | **n/a** for the JNI part |
| Buffered raw input (`GetRawInputBuffer`) | One `GetRawInputData` call per event. An 8000 Hz mouse costs a lot on Windows | winit reads `WM_INPUT` one message at a time | **later**. Measure on Windows with a high-polling-rate mouse before changing it. |
| Cached GLFW state | Round trips to the main thread | No GLFW | **n/a** |
| A frame limiter that sleeps, then spins | Imprecise sleeps | Horizon has its own limiter | **no** (no reported problem) |

## Worth measuring

Each of these needs a profile or a benchmark first. They are ordered by the expected gain. They are in the LATER list in `~/code/horizon-parity/FEATURES.md`, not filed as issues:

1. Hopper and block-entity sleeping, with vanilla-identical results (Lithium). Benchmark: a hopper and furnace array.
2. Particles: vanilla's frustum point test (a parity fix too), then GPU-built quads (AsyncParticles). Benchmark: 1,000+ campfires.
3. Jigsaw placement: skip blocked connectors and already-failed elements without changing the layout (Structure Layout Optimizer). Benchmark: trial chambers and ancient cities in fresh worlds, `docs/generation-scan-benchmark.md`.
4. Windows raw input in batches for high-polling-rate mice (Ixeris). Benchmark: an 8000 Hz mouse on Windows.
5. Profile before choosing a technique: mob pathfinding, TNT explosions, item merging after #996, map walls and sign-heavy scenes.
