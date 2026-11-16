# Design: worldgen features and placers

Provisional answer to [arismoko/horizon#815](https://github.com/arismoko/horizon/issues/815): register feature implementations and placement modifiers once, decode their configured instances with the shared typed codec foundation, and execute both through one scoped target-access boundary. This is a proposal, not an implemented API or authority to close the issue. Structure-type registration remains separate.

## Vanilla and NeoForge evidence

Horizon mirrors configured features, placed features and ordered placement modifiers. At the inspected base `0ef42f9e54`, `crates/horizon-world/src/worldgen/feature_graph.rs:245` creates one execution budget and passes the caller's random into modifiers and the selected feature; line 264 carries that shared run into nested placement. This is evidence about Horizon, not an independently verified vanilla contract.

Vanilla 26.3 was queried exclusively through `cd /workspace/horizon && python3 tools/rea_vanilla.py method net.minecraft.world.level.levelgen.feature.ConfiguredFeature place`: exact class unavailable. The subsequent `classes ConfiguredFeature` query returned zero matches with complete coverage. Consequently vanilla caller coverage, modifier laziness, random draw order, target clipping and capacity are **unverified** here. Resolve the pinned jar's actual names and obtain oracle fixtures before implementing; do not replace missing evidence with another project's source.

The issue names NeoForge 26.3 `RegisterEvent` as the counterpart. Requests for the [public feature guide](https://docs.neoforged.net/docs/worldgen/features/) and [26.3 PlacedFeature Javadoc](https://aldak.netlify.app/javadoc/26.3.x/net/minecraft/world/level/levelgen/placement/PlacedFeature.html) were inaccessible. Exact registration signatures, worker guarantees, codec limits and reload behavior are **unverified**. Require public documentation before adopting counterpart names or claiming full NeoForge equivalence. No NeoForge source, patches or other project's source were consulted.

## Proposed registration and codec contract

Use one content-phase registration moment with the appropriate Host and scoped registrar, dependency ordered for both targets. Illustrative traits are `FeatureType` and `PlacementModifierType`; these are proposed names. A placer transforms one input position into an ordered stream, rather than replacing the feature runner. Freeze implementation declarations after mod loading. Reject duplicate ids, foreign namespace ownership, unresolved implementation references and declarations requiring unavailable target capabilities at registration. In particular, refuse native-only worker functions, arbitrary engine pointers, Game access, retained mutable state and guest callbacks without a worker execution adapter. Do not silently admit a declaration that will fail only when a chunk reaches it.

Reuse arismoko/horizon#2332's declaring-mod-owned typed schema, game-owned candidate decoding, complete candidate reference lookup and atomic publication. Use explicit `#[derive(Wire)]` and the shared generated layout, never a second encoding mechanism implied by the illustrative `*Wire` names. Define API-owned `FeatureConfigWire`, `PlacementConfigWire`, `FeatureRefWire`, `PositionWire`, `BlockStateWire` and `WorldgenErrorWire` through the shared generated codec foundation. These describe bounded immutable values, checked ids and structured errors, never Rust layout, opaque JSON transport, native pointers or guest addresses. Configuration codecs have no world writes, random, I/O or scheduling. Native and wasm decode the same schema into immutable worker configuration. Resolve nested configured and placed references, forward references and cycles before admission; unsupported recursion refuses the candidate.

Reuse the existing world lifecycle holder as snapshot identity and retirement authority. Do not add a second worldgen generation counter. Jobs retain the holder and immutable decoded graph; reload prepares everything before publishing through that holder. Failed decoding publishes nothing. Old jobs may finish only under their retained holder and must be rejected at installation if that holder is no longer current; never mix old configs with new block ids. Mod unload waits for dependent worlds/jobs to close. Data-only reload cannot replace callable implementations. Persist names, not resolved slots or worker handles.

## Workers, target access and random

Decoration runs feature and modifier code on worldgen workers. Live growth and commands run the same adapters against their existing authoritative level target. Give callbacks a scoped worker-safe context following the structure context principles, not Game or a general ServerHost capable of gameplay services. The registration hook receives Host; worker execution needs a generated restricted Host/context contract reviewed alongside the structure boundary. A worker instance must be independently owned or reentrant with immutable shared configuration; never serialize every chunk behind one mutable mod instance. Wasm needs worker-local execution stores with the same callable and dependency semantics; inability to provide these refuses registration.

One target service owns block reads/writes, height and biome queries, metadata, nested placement and supported queued effects. It advertises actual readable region, writable region and vertical bounds. Outside reads return an explicit unavailable result; outside writes return false without loading chunks. No unbounded neighbor loads, direct live-world mutation, scheduling or retained target handle. Validate block identities against the held snapshot. Require read-your-writes and route block entities, ticks, neighbor effects and entities through the target's existing staging machinery; capabilities unavailable on either target must be refused explicitly. Structure piece chunk-box clipping is not automatically the feature contract.

In live lifted passes, callback writes, nested placement and apply share the root transaction. An inner success does not retire handles or emit journals: defer retirement and journal emission to root commit. Rollback discards staged writes and effects. An open action savepoint refuses saves, rather than persisting provisional state. Worker output stays private until accepted by the owning job; failed or stale output cannot install. Random is not rewound on an ordinary false result; failure policy must pin whether an aborted job restarts from its original seed.

Preserve decoration step/global feature-index seeding for each independently seeded root; inserting a mod must neither incidentally renumber vanilla roots nor consume their draws. The shared stream below is scoped to one root, not shared across independently seeded decoration roots. Borrow the caller's exact RNG. Modifier and feature draws can interleave: Horizon's current runner can execute the feature for the first position before a later modifier processes the second position (`crates/horizon-world/src/worldgen/placement/chain.rs:215-260`). Preserve duplicates and output order; the exact modifier/feature traversal and draw order remain subject to the pinned vanilla oracle. Nested features share the same stream; no callback reseeding, per-mod streams, eager random precomputation or parallel evaluation of one placement chain. No-listener execution preserves the built-in path. Native and wasm expose the same Java-compatible primitive draws and checked bounds; reject zero/negative next-int bounds before drawing.

## Inspected route owners and required integration

These are verified source entry points at the base above, not proof of complete vanilla caller coverage. Every route must converge on the shared adapter; completion of the command, growth and worker caller trace is an implementation gate.

| Route | Verified owner and treatment |
| --- | --- |
| Worker biome decoration | `crates/horizon-world/src/worldgen/generator_decoration.rs:388` owns decorate; use the job's held target and seeded random. |
| Placed graph and nested features | `crates/horizon-world/src/worldgen/feature_graph.rs:245`, `:264`, `:811` own placed entry, nested run and feature entry; preserve shared budgets and random. |
| Modifier execution | `crates/horizon-world/src/worldgen/placement.rs:313`, `:322` own modify/modify_in; stream custom results through the same ordered runner. |
| Named configured, inline, explicit definition and placed execution | `crates/horizon-world/src/worldgen/live.rs:280`, `:310`, `:318`, `:338`; share decoding and adapter dispatch, including nested references. |
| Command placement | `crates/horizon-world/src/level/bonemeal.rs:131` owns place_feature; use the authoritative live target and its existing RNG. |
| Growth configured and placed routes | `crates/horizon-world/src/level/growth.rs:291`, `:339` document the two placement paths; lift effects into the existing action transaction. |
| Catalog decoding | `crates/horizon-world/src/worldgen/feature_catalog.rs:111` owns placed_feature; resolve custom implementations here through the shared candidate decoder. |
| Shared target | `crates/horizon-world/src/worldgen/feature.rs:30` owns FeatureWorld; bridge this boundary rather than introducing a second block store. |
| Structure contexts | `crates/horizon-mod-vocab/src/world/structure_type.rs` owns GenerationContext and PieceContext; reuse principles, retain separate registration and behavior. |

Initial world load, datapack reload, remote world authority, worker cancellation and chunk installation must be traced through their owning lifecycle coordinator before coding. Remote clients never independently execute authoritative mod generation. This note does not assert uninspected file:line owners for those routes.

## Untrusted input, failure and cost

Treat pack configuration, callback positions, states, reference graphs and wasm memory as untrusted. Validate lengths before allocation, checked coordinate arithmetic, finite scalar values, ids, depth and work budgets at the adapter boundary. Preserve vanilla's own capacity: never apply a smaller arbitrary custom cap to built-in placement, world height or legal neighbor reach. The current placed entry starts with 65,536 work units; that is a Horizon observation, not a verified vanilla maximum. Propose a separate configurable custom-work budget initially 65,536 units and depth 64, with structured refusal rather than truncation, subject to oracle capacity review. Stream positions so output cannot force an unbounded vector; charge nested calls and writes to the same custom budget.

Native unwind and wasm trap abort the current root action/job and discard staged effects; report mod, implementation and origin, and disable the failing callable under runtime policy. Propagate nested callback failures to the owning root, keeping nested successes provisional; subsequent calls to a disabled callable return an explicit structured failure rather than silently omitting terrain. Never continue with partially installed terrain. Native aborts cannot be caught, and native code cannot be forcibly fuel-limited; executable admission remains trusted. Wasm additionally uses existing memory/fuel limits. An ordinary feature false is successful execution with no success claim, not a panic or automatic rollback of vanilla-permitted writes.

Resolve ids once at load. The unused path adds at most an indexed variant dispatch, no mod scans, JSON parsing or per-block allocations. Custom execution costs scale with explicit queries, writes and emitted positions; wasm batching may reduce crossings only if it preserves lazy RNG and read-your-writes semantics. Measure allocations and boundary calls on empty, built-in and custom paths before claiming parity.

## Acceptance matrix

Proposed expectations, not passing implementation tests. Run every applicable fixture on native and wasm, worker and live targets.

| Fixture | Concrete expected result |
| --- | --- |
| Test placer emits origin and origin+[1,0,0]; feature writes stone at each | Origin [0,64,0] produces exactly [0,64,0] and [1,64,0], in that order, two writes. |
| Placer emits duplicate origin twice | Two feature calls, two ordered attempts; no deduplication. |
| Callback draws nextInt(1), then built-in step within the same root | Callback gets 0; oracle pins the following built-in draw and coordinate independently for seed 0. Same-seed equality alone is insufficient. |
| Multiple positions with random draws in a later modifier and in the feature | Record the ordered modifier/feature draw trace, resulting coordinates and following RNG value against independent pinned vanilla oracle values; cover a feature call before the later modifier processes the second position. |
| Empty registration | Zero custom calls, decode allocations and mod scans; identical built-in coordinates and following RNG value to baseline. |
| Duplicate example:test or unsupported live-write capability | Registration refused before any world job; zero installed custom declarations for failed admission. |
| Reload valid config from count 2 to 3 | New jobs use 3; old jobs retain 2 and stale results install zero blocks. A subsequent failed reload retains the currently published count 3; new jobs still use 3. |
| Write at writable maximum and one beyond | Maximum accepted if legal; beyond returns false, zero chunk loads. Test negative and extreme i32 coordinates without overflow. |
| NaN scalar, invalid state, 65,537 custom work units, depth 65 | Structured failure, zero committed output; next valid job completes. Built-in capacity remains unchanged. |
| Native unwind / guest trap after one staged write | Zero committed writes, journals and retirement events. |
| Nested success then outer failure; save during open action | Zero committed effects; save refused. Root success emits each journal and retirement once. |
| Inline/named/nested/growth/command/decoration with same test config | Each resolves the same implementation; target-specific authority and RNG remain with the caller. |

## Stage 1 evidence: the pinned 26.3 bytecode

Queried with `tools/rea_vanilla.py` against the pinned 26.3 client jar (checksum equal to `research-lock.json`'s `client_sha1`), after the earlier attempt could not resolve the classes. The classes exist; the earlier query used a name this version does not have (`ConfiguredFeature`; configured features are folded into `Feature`). These are vanilla's shapes, read from the bytecode; what NeoForge 26.3 adds on top is still unread, because its public Javadoc was unreachable (HTTP 503) and its docs have no feature pages.

**The API shapes in 26.3 differ from the 1.21-era ones the illustrative names assumed:**

- `Feature` is not generic over a configuration and has no `FeaturePlaceContext`. Its configuration is part of the feature itself (the JSON has the settings inline in the root object next to `type`). It has `place(WorldGenLevel, ChunkGenerator, RandomSource, BlockPos): boolean` and `codec(): MapCodec<? extends Feature>`.
- `PlacedFeature` is `(Holder<Feature>, List<PlacementModifier>)`, with `place(WorldGenLevel, ChunkGenerator, RandomSource, BlockPos): boolean`. It makes a `FeaturePlacer` and calls it.
- `PlacementModifier` has no `getPositions` stream and there is no `PlacementModifierType` class. A modifier is `modify(PlacementContext, RandomSource, BlockPos, Consumer<BlockPos>): void`: it pushes each output position to the consumer, and has `codec(): MapCodec<? extends PlacementModifier>`.
- `PlacementContext` offers `getLevel`, `generator`, `getBlockState(BlockPos)`, `getHeight(Heightmap.Types, int, int)`, `getMinY` and `topFeature(): Optional<PlacedFeature>`.

**Traversal and draw order (`FeaturePlacer.place`, overload with `biomeCheck`):**

1. With no modifiers, the feature places once at the origin.
2. Otherwise a stack of `(position, modifier index)` starts with the origin at index 0. Each step pops the last pair, calls that modifier's `modify` with the shared `RandomSource`, and collects its outputs in order.
3. If another modifier follows, the outputs are pushed in reverse order at the next index, so the first output is popped next: depth first.
4. If it was the last modifier, the feature places at each output now, in output order, before the next stack pop. `placedAny` is the OR of the results.
5. Every modifier and feature draws from the same `RandomSource`, in this order. Nothing is deduplicated, and a draw is never undone for a false result.

Horizon's `crates/horizon-world/src/worldgen/placement/chain.rs` (`place_control`) follows exactly this: a pending stack popped from the end, outputs extended in reverse, features run on the last level's outputs in order. So the interleaving the acceptance matrix asks about (a feature call for the first position before a later modifier processes the second) is vanilla's behaviour and is already Horizon's.

**Callers.** `FeaturePlacer.placeWithBiomeCheck` is called only from `ChunkGenerator.applyBiomeDecoration` (decoration, with the biome check). `PlacedFeature.place` is called from `GrassBlock.placeBonemealEffect`, `CoralClawFeature`, `CoralTreeFeature`, `RandomBooleanSelectorFeature`, `RandomSelectorFeature`, `SimpleRandomSelectorFeature`, `WeightedPlacedFeature`, `WeightedRandomSelectorFeature`, `RootSystemFeature` (`placeDirtAndTree`), `SingleBlockPillarFeature`, `VegetationPatchFeature` (`placeVegetation`) and `FeaturePoolElement` (structure pools): the nested and growth routes the acceptance matrix lists. The command route is its own query: `PlaceCommand.placeFeature(source, Holder<Feature>, pos)` calls `Feature.place(level, level.getChunkSource().getGenerator(), level.getRandom(), pos)` directly, with the level's own random, no placement modifiers and no biome check, and fails the command when `place` returns false. So the command route runs a feature on its own and a placed feature (modifiers and all) is not a command target; the custom feature API must therefore expose the feature call separately from the placement chain.

**Numeric draws, run against the jar.** A Java program run against the pinned `client.jar` printed these vanilla values, and `crates/horizon-core/src/random/worldgen/oracle_tests.rs` asserts them for Horizon's `WorldgenRandom`: from seed 0 `nextInt(1)` is 0 and still consumes one draw, so the next two `nextInt(16)` are 13 and 3 and the following `nextLong` is -7261648964369397258; `setDecorationSeed(0, 0, 0)` is 0 and `setFeatureSeed(0, 0, 0)` gives `nextInt(16)` 11 then 13; `setDecorationSeed(0, 16, -32)` is -31007559772603472 and `setFeatureSeed(.., 3, 2)` gives 10 then 4. Those pin the callback draw and the global feature-index and step seeding. The Xoroshiro root is pinned the same way (run with Mojang's DataFixerUpper, fastutil, Guava and SLF4J on the class path): `XoroshiroRandomSource(0)` gives `nextInt(1)` 0, then `nextInt(16)` 2 and 2; the `WorldgenRandom` over it gives 0, then 12 and 4; `setDecorationSeed(0, 16, -32)` is -7422581477381670832 and `setFeatureSeed(.., 3, 2)` gives 4 then 9. The draws of a whole placement chain remain to be pinned by a run of a real feature. The stream signature can now be fixed as the push-based `modify` above instead of a position stream, but this API shape alone does not eliminate output-list allocations: vanilla FeaturePlacer and Horizon chain.rs still collect modifier outputs. Allocation changes require separate measurement.

**Stage 1 fixtures.** These run the jar the player prepared and compare Horizon against it at test time, so no vanilla values or assets are stored. `tools/test_feature_placement_parity.py` runs the real `FeaturePlacer` over decoded modifier chains (duplicates, depth-first traversal, partial failures, both random roots) with a feature sink that draws `nextLong`, `nextInt(1)` or `nextInt(16)`, and compares each draw, position and the following `nextLong`. `tools/test_decoration_parity.py` runs the real `ChunkGenerator.applyBiomeDecoration` over the bundled biome lists, also with one extra feature inserted into a step: that shows how the global feature indices, and so the seeds of the vanilla features after it, move. Horizon's own capacity limits (work units, depth) have no vanilla counterpart and stay with stage 5.

## Proposed stages

No Rust work is authorized by this note. These stages are implementation gates, not demonstrated behavior:

1. (Callers and shapes resolved above; the numeric fixtures remain.) Build independent oracle fixtures for lazy modifier traversal, duplicates, primitive draws and capacity before freezing a stream signature. Pin a draw following `nextInt(1)` within the same root, and separately verify decoration step/global feature-index seeding and unchanged vanilla roots after mod insertion.
2. Settle the shared Wire and lifecycle contracts with arismoko/horizon#811/#2332. Specify object-safe registrar and execution signatures, API-owned IDs, errors and positions, and callback-scoped target access that cannot escape. Require explicit `#[derive(Wire)]`, the shared layout, and named load-time refusal of unsupported types and capabilities on both targets; registrar names remain illustrative pending public 26.3 evidence.
3. Generate registration and execution adapters together for native and wasm. Keep phase-limited registration authority separate from a restricted generated worker Host over captured registries and the scoped target, with no ServerHost or Game authority. Review its method set against existing worker-safe colour and structure paths. Prove worker-local wasm stores preserve native dependency linkage and callable semantics before admission.
4. Integrate target staging and every live/worker route under one root owner. Prove nested successes remain provisional, stale worker output cannot install, and disabled callables fail explicitly on subsequent calls. Distinguish ordinary false with vanilla-permitted writes from panic/trap rollback.
5. Run the acceptance matrix with independent expected values, adversarial fixtures and hot-path measurements on both targets. Obtain public 26.3 registration evidence and retain the explicit limits on equivalence claims until verified.

## Open questions

- Exact NeoForge contract and limits? Recommendation: require public 26.3 documentation before adopting names or limits; retain unverified status where silent.
- Exact vanilla modifier traversal and RNG values? Recommendation: oracle the actual pinned classes and all callers before freezing the stream interface.
- Which effects and neighbor access must both targets support? Recommendation: publish a capability table and refuse unsupported registration; preserve verified vanilla region capacity.
- Worker Host integration and wasm concurrency? Recommendation: review one restricted generated context with structure workers, then prove worker-local stores and dependency linkage before admission.
- Reload and installation owner? Recommendation: trace the existing lifecycle holder through world entry, cancellation and installation; use its identity and root commit, never another generation allocator.
- Custom budget policy? Recommendation: keep budgets explicit and configurable, measure real packs and verify vanilla capacity before freezing defaults.
- Native callback failure recovery? Recommendation: fail the owning root action/job atomically, disable the callable and require explicit recovery; document unrecoverable process aborts.
