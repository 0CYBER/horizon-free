//! Apply ordered carving-mask intervals to pre-lighting terrain.
use super::{
    material_condition::{
        CompiledSurfaceRule, MaterialConditions, MaterialDensitySources, MaterialRule,
        MaterialSurfaceQueries,
    },
    material_stage::{SurfaceError, TopMaterialInputs, top_material_cell},
    terrain::{TerrainAquifer, TerrainBlockStates, TerrainChunk, TerrainError},
};
use crate::{
    heightmap::HeightmapState,
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{
    carving::CarvingMask, density::SamplerContext, noise::DensityVolume, position::BlockPos,
    random::RandomSource,
};
use std::collections::BTreeSet;

/// Per-state predicates resolved once from the world's block registry and
/// `minecraft:uncarvable` tag. Keep these bound to the same registry as the chunk.
pub struct CarvingBlocks {
    flags: Vec<u8>,
}
impl CarvingBlocks {
    pub fn from_registry(
        registry: &BlockRegistry,
        uncarvable: &BTreeSet<String>,
    ) -> Result<Self, TerrainError> {
        for name in uncarvable.iter().map(String::as_str).chain([
            "minecraft:grass_block",
            "minecraft:mycelium",
            "minecraft:dirt",
        ]) {
            if registry.default_state(name).is_none() {
                return Err(TerrainError::InvalidState);
            }
        }
        let flags = (0..registry.len())
            .map(|index| {
                let name = &registry.state(BlockStateId(index as u32)).unwrap().name;
                u8::from(uncarvable.contains(name))
                    | (u8::from(matches!(
                        name.as_str(),
                        "minecraft:grass_block" | "minecraft:mycelium"
                    )) << 1)
                    | (u8::from(name == "minecraft:dirt") << 2)
            })
            .collect();
        Ok(Self { flags })
    }
    fn flags(&self, state: BlockStateId) -> Result<u8, TerrainError> {
        self.flags
            .get(state.0 as usize)
            .copied()
            .ok_or(TerrainError::InvalidState)
    }
}

/// Visit mask runs in packed order and each interval top to bottom. The filter
/// receives local X/Z and absolute Y before any block or aquifer lookup. A run
/// remembers grass/mycelium even if its aquifer returns no replacement. Exposed
/// dirt below subsequent carved cells is restored through `top_material`.
/// Earlier writes, aquifer state and fluid postprocessing survive later errors.
pub fn apply_carving_mask<R: RandomSource, S: MaterialSurfaceQueries, B: TerrainBlockStates>(
    chunk: &mut TerrainChunk<'_, B>,
    mask: &CarvingMask,
    rule: MaterialRule,
    inputs: TopMaterialInputs<'_, R, S>,
    aquifer: &mut (impl TerrainAquifer + ?Sized),
    blocks: &CarvingBlocks,
    filter: impl FnMut([i32; 3]) -> bool,
) -> Result<(), SurfaceError>
where
    R::Positional: Clone,
{
    apply_carving_mask_shared(
        chunk, mask, &rule, &mut None, inputs, aquifer, blocks, filter,
    )
}
/// `apply_carving_mask` with the rule compiled once for the chunks `shared`
/// serves (`MaterialConditions::add_shared_rule`).
pub fn apply_carving_mask_shared<
    R: RandomSource,
    S: MaterialSurfaceQueries,
    B: TerrainBlockStates,
>(
    chunk: &mut TerrainChunk<'_, B>,
    mask: &CarvingMask,
    rule: &MaterialRule,
    shared: &mut Option<CompiledSurfaceRule<R>>,
    inputs: TopMaterialInputs<'_, R, S>,
    aquifer: &mut (impl TerrainAquifer + ?Sized),
    blocks: &CarvingBlocks,
    mut filter: impl FnMut([i32; 3]) -> bool,
) -> Result<(), SurfaceError>
where
    R::Positional: Clone,
{
    if mask.is_empty() {
        return Ok(());
    }
    // Most carved columns never expose dirt below grass/mycelium. When `shared`
    // is already compiled and has no eager density bindings, defer constructing
    // the per-cell top-material worker until an exposed dirt block actually
    // needs repair.
    //
    // Deferring in that case cannot hide a rule-compile, noise-binding, or
    // density-sampling error: `CompiledSurfaceRule` is only populated by
    // `MaterialConditions::add_shared_rule` after `CompiledRule::compile`
    // validates the rule and resolves every noise, and once populated
    // `add_shared_rule` reuses the already-bound rule and noise handles
    // directly without re-compiling `rule` or calling `bind_noise`. When
    // `shared` is `None` or has eager density bindings, initialize the worker
    // up front at `[0, 0, 0]` before any terrain write so invalid rules and
    // eager origin sampling errors still fail before mutating the chunk.
    let TopMaterialInputs {
        generation,
        root,
        surface,
        preliminary_surface,
        density_context,
    } = inputs;
    let defer_worker = shared
        .as_ref()
        .is_some_and(|compiled| !compiled.has_eager_density_bindings());
    let mut top_worker = None;
    let mut top_inputs = if defer_worker {
        Some((generation, root, surface, preliminary_surface))
    } else {
        let volume =
            DensityVolume::new([1; 3], [0; 3], [1; 3]).map_err(|_| SurfaceError::InvalidVolume)?;
        let sources = MaterialDensitySources::new(
            surface,
            volume,
            preliminary_surface,
            SamplerContext::default(),
        )?;
        let mut worker =
            MaterialConditions::<R, _>::with_possible_biomes(generation, root, sources, None);
        let rule_id = worker.add_shared_rule(rule, shared)?;
        top_worker = Some((worker, rule_id));
        None
    };
    let cx = chunk.position().x.wrapping_shl(4);
    let cz = chunk.position().z.wrapping_shl(4);
    for column in mask.columns() {
        let mut found_surface = false;
        let x = cx.wrapping_add(column.x);
        let z = cz.wrapping_add(column.z);
        for y in (column.min_y..=column.max_y).rev() {
            if !filter([column.x, y, column.z]) {
                continue;
            }
            let flags = blocks.flags(chunk.block(x, y, z))?;
            if flags & 1 != 0 {
                continue;
            }
            found_surface |= flags & 2 != 0;
            let Some(state) = aquifer
                .compute_substance(density_context, [x, y, z], 0.)
                .map_err(TerrainError::from)?
            else {
                continue;
            };
            chunk.set_block_worldgen(x, y, z, state)?;
            if aquifer.should_schedule_fluid_update() && chunk.block_properties(state).has_fluid() {
                chunk.mark_postprocessing(x, y, z);
            }
            let below = y.wrapping_sub(1);
            if found_surface && blocks.flags(chunk.block(x, below, z))? & 4 != 0 {
                let submerged = chunk.block_properties(state).has_fluid();
                let (worker, rule_id) = match &mut top_worker {
                    Some((worker, rule_id)) => {
                        worker.retarget_surface_densities([x, below, z])?;
                        (worker, *rule_id)
                    }
                    None => {
                        let (generation, root, surface, preliminary_surface) =
                            top_inputs.take().expect("top material inputs");
                        let volume = DensityVolume::new([1; 3], [x, below, z], [1; 3])
                            .map_err(|_| SurfaceError::InvalidVolume)?;
                        let sources = MaterialDensitySources::new(
                            surface,
                            volume,
                            preliminary_surface,
                            SamplerContext::default(),
                        )?;
                        let mut worker = MaterialConditions::<R, _>::with_possible_biomes(
                            generation, root, sources, None,
                        );
                        let rule_id = worker.add_shared_rule(rule, shared)?;
                        let (worker, rule_id) = top_worker.insert((worker, rule_id));
                        (worker, *rule_id)
                    }
                };
                let replacement = top_material_cell(
                    worker,
                    chunk,
                    rule_id,
                    BlockPos::new(x, below, z),
                    submerged,
                )?;
                if let Some(replacement) = replacement {
                    chunk.set_block_worldgen(x, below, z, replacement)?;
                    if chunk.block_properties(replacement).has_fluid() {
                        chunk.mark_postprocessing(x, below, z);
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        DATA_VERSION,
        worldgen::{
            DecodeError,
            anchor::WorldGenerationContext,
            aquifer::{DisabledAquifer, GlobalFluidPicker},
            material_condition::{MaterialCondition, MaterialNoiseSources},
            material_stage::top_material,
            terrain::TerrainChunk,
        },
    };
    use horizon_core::{
        density::{DensitySampler, InterpolatedSampler, SampleError},
        nbt::Tag,
        position::ChunkPos,
        random::LegacyRandom,
    };
    use std::{collections::BTreeMap, sync::Arc};

    #[derive(Clone, Copy)]
    struct SimpleState {
        air: bool,
        fluid: bool,
    }
    impl HeightmapState for SimpleState {
        fn is_air(self) -> bool {
            self.air
        }
        fn is_air_block(self) -> bool {
            self.air
        }
        fn has_fluid(self) -> bool {
            self.fluid
        }
        fn blocks_motion_in_heightmap(self) -> bool {
            !self.air && !self.fluid
        }
        fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
            !self.air && !self.fluid
        }
    }
    struct SimpleStates {
        air: BlockStateId,
        water: BlockStateId,
        lava: BlockStateId,
    }
    impl TerrainBlockStates for SimpleStates {
        type State = SimpleState;
        fn heightmap_state(&self, id: BlockStateId) -> SimpleState {
            SimpleState {
                air: id == self.air,
                fluid: id == self.water || id == self.lava,
            }
        }
    }
    #[derive(Debug)]
    struct ZeroSampler;
    impl DensitySampler for ZeroSampler {
        fn sample_value(&self, _: &mut SamplerContext, _: [i32; 3]) -> Result<f32, SampleError> {
            Ok(0.0)
        }
    }
    struct EmptySurface;
    impl MaterialSurfaceQueries for EmptySurface {
        fn surface_depth(&mut self, _: i32, _: i32) -> i32 {
            0
        }
        fn surface_secondary(&mut self, _: i32, _: i32) -> f64 {
            0.0
        }
    }

    fn test_registry() -> BlockRegistry {
        let names = [
            "minecraft:air",
            "minecraft:void_air",
            "minecraft:cave_air",
            "minecraft:water",
            "minecraft:lava",
            "minecraft:stone",
            "minecraft:grass_block",
            "minecraft:mycelium",
            "minecraft:dirt",
        ];
        let defaults: Vec<_> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (*n, Tag::Int(i as i32)))
            .collect();
        let states: Vec<_> = names
            .iter()
            .map(|n| Tag::compound([("id", Tag::String((*n).into()))]))
            .collect();
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(DATA_VERSION)),
            ("states", Tag::List(states)),
            ("defaults", Tag::compound(defaults)),
        ]))
        .unwrap()
    }

    fn assert_chunks_equal(
        a: &TerrainChunk<'_, SimpleStates>,
        b: &TerrainChunk<'_, SimpleStates>,
        min_y: i32,
        max_y_excl: i32,
        label: &str,
    ) {
        let ox = a.position().x * 16;
        let oz = a.position().z * 16;
        assert_eq!(a.position(), b.position(), "{label}: position mismatch");
        for y in min_y..max_y_excl {
            for lx in 0..16 {
                for lz in 0..16 {
                    let wx = ox + lx;
                    let wz = oz + lz;
                    assert_eq!(
                        a.block(wx, y, wz),
                        b.block(wx, y, wz),
                        "{label}: block mismatch at ({wx}, {y}, {wz})"
                    );
                }
            }
        }
        assert_eq!(
            a.postprocessing(),
            b.postprocessing(),
            "{label}: postprocessing mismatch"
        );
    }

    #[test]
    fn invalid_noise_threshold_fails_before_carving_writes_any_block() {
        let registry = test_registry();
        let air = registry.default_state("minecraft:air").unwrap();
        let water = registry.default_state("minecraft:water").unwrap();
        let lava = registry.default_state("minecraft:lava").unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        let blocks = CarvingBlocks::from_registry(&registry, &BTreeSet::new()).unwrap();
        let mut mask = CarvingMask::new(0, 31).unwrap();
        mask.carve(2, 10, 3).unwrap();
        let invalid_rule = MaterialRule::Condition {
            if_true: MaterialCondition::NoiseThreshold {
                noise: "minecraft:no_such_noise".to_owned(),
                min: 0.0,
                max: 1.0,
                is_3d: false,
            },
            then_run: Box::new(MaterialRule::Block(stone)),
        };
        let root = LegacyRandom::new(1).fork_positional();

        // Direct unshared call fails before writing any block.
        let mut chunk = TerrainChunk::new(
            ChunkPos::new(0, 0),
            0,
            32,
            &registry,
            SimpleStates { air, water, lava },
        )
        .unwrap();
        chunk.set_block_worldgen(2, 10, 3, stone).unwrap();
        let mut aquifer = DisabledAquifer::new(GlobalFluidPicker::new(0, water, lava, air), air);
        let mut density_context = SamplerContext::default();
        let err = apply_carving_mask::<LegacyRandom, _, _>(
            &mut chunk,
            &mask,
            invalid_rule.clone(),
            TopMaterialInputs {
                generation: WorldGenerationContext::new(0, 32, 0),
                root,
                surface: EmptySurface,
                preliminary_surface: Arc::new(ZeroSampler),
                density_context: &mut density_context,
            },
            &mut aquifer,
            &blocks,
            |_| true,
        );
        assert!(matches!(err, Err(SurfaceError::Decode(_))));
        assert_eq!(chunk.block(2, 10, 3), stone);

        // Shared call with `shared = None` also fails before writing and keeps `shared = None`,
        // so a second call with the same cache slot still fails before carving any block.
        let mut shared = None;
        for attempt in 0..2 {
            let mut shared_chunk = TerrainChunk::new(
                ChunkPos::new(0, 0),
                0,
                32,
                &registry,
                SimpleStates { air, water, lava },
            )
            .unwrap();
            shared_chunk.set_block_worldgen(2, 10, 3, stone).unwrap();
            let mut aq = DisabledAquifer::new(GlobalFluidPicker::new(0, water, lava, air), air);
            let mut ctx = SamplerContext::default();
            let err = apply_carving_mask_shared::<LegacyRandom, _, _>(
                &mut shared_chunk,
                &mask,
                &invalid_rule,
                &mut shared,
                TopMaterialInputs {
                    generation: WorldGenerationContext::new(0, 32, 0),
                    root,
                    surface: EmptySurface,
                    preliminary_surface: Arc::new(ZeroSampler),
                    density_context: &mut ctx,
                },
                &mut aq,
                &blocks,
                |_| true,
            );
            assert!(
                matches!(err, Err(SurfaceError::Decode(_))),
                "attempt {attempt} should fail"
            );
            assert!(shared.is_none(), "failed compile must not populate shared");
            assert_eq!(shared_chunk.block(2, 10, 3), stone);
        }
    }

    #[test]
    fn shared_compiled_rule_without_exposed_dirt_matches_eager_and_never_defers_bind_error() {
        let registry = test_registry();
        let air = registry.default_state("minecraft:air").unwrap();
        let water = registry.default_state("minecraft:water").unwrap();
        let lava = registry.default_state("minecraft:lava").unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        let grass = registry.default_state("minecraft:grass_block").unwrap();
        let mycelium = registry.default_state("minecraft:mycelium").unwrap();
        let dirt = registry.default_state("minecraft:dirt").unwrap();
        let blocks = CarvingBlocks::from_registry(&registry, &BTreeSet::new()).unwrap();

        let generation = WorldGenerationContext::new(0, 32, 12);
        let root = LegacyRandom::new(77).fork_positional();
        let prelim: Arc<dyn DensitySampler> = Arc::new(ZeroSampler);
        let noise_defs = BTreeMap::from([(
            "minecraft:test_noise".to_owned(),
            r#"{"base_octave":-4}"#.to_owned(),
        )]);
        let valid_noise_rule = MaterialRule::Sequence(vec![
            MaterialRule::Condition {
                if_true: MaterialCondition::NoiseThreshold {
                    noise: "minecraft:test_noise".to_owned(),
                    min: -0.1,
                    max: 0.1,
                    is_3d: false,
                },
                then_run: Box::new(MaterialRule::Block(mycelium)),
            },
            MaterialRule::Block(grass),
        ]);
        let invalid_noise_rule = MaterialRule::Condition {
            if_true: MaterialCondition::NoiseThreshold {
                noise: "minecraft:no_such_noise".to_owned(),
                min: 0.0,
                max: 1.0,
                is_3d: false,
            },
            then_run: Box::new(MaterialRule::Block(stone)),
        };

        // 1. Warm `shared` with the valid NoiseThreshold rule via MaterialNoiseSources.
        let mut shared = None;
        {
            let mut warmup = TerrainChunk::new(
                ChunkPos::new(0, 0),
                0,
                32,
                &registry,
                SimpleStates { air, water, lava },
            )
            .unwrap();
            let mut warmup_mask = CarvingMask::new(0, 31).unwrap();
            warmup_mask.carve(0, 10, 0).unwrap();
            let mut aq = DisabledAquifer::new(GlobalFluidPicker::new(12, water, lava, air), air);
            let mut ctx = SamplerContext::default();
            let noise_surface =
                MaterialNoiseSources::from_json(root, EmptySurface, &noise_defs).unwrap();
            apply_carving_mask_shared::<LegacyRandom, _, _>(
                &mut warmup,
                &warmup_mask,
                &valid_noise_rule,
                &mut shared,
                TopMaterialInputs {
                    generation,
                    root,
                    surface: noise_surface,
                    preliminary_surface: Arc::clone(&prelim),
                    density_context: &mut ctx,
                },
                &mut aq,
                &blocks,
                |_| false,
            )
            .unwrap();
        }
        assert!(
            shared
                .as_ref()
                .is_some_and(|compiled| !compiled.has_eager_density_bindings())
        );

        let build_chunk = |expose_dirt: bool| {
            let mut chunk = TerrainChunk::new(
                ChunkPos::new(2, 3),
                0,
                32,
                &registry,
                SimpleStates { air, water, lava },
            )
            .unwrap();
            let mut mask = CarvingMask::new(0, 31).unwrap();
            for (lx, lz, top_y) in [(1, 2, 18), (6, 9, 20), (12, 4, 22)] {
                let wx = 32 + lx;
                let wz = 48 + lz;
                chunk
                    .set_block_worldgen(wx, top_y, wz, if expose_dirt { grass } else { stone })
                    .unwrap();
                chunk.set_block_worldgen(wx, top_y - 1, wz, dirt).unwrap();
                mask.carve(lx, top_y, lz).unwrap();
            }
            (chunk, mask)
        };

        // 2 & 3. With `shared` already compiled, test both no exposed dirt and exposed dirt.
        // Passing `EmptySurface` (no noise provider) proves `bind_noise` is never re-invoked
        // even when dirt IS exposed, and outputs match eager `apply_carving_mask` with
        // `MaterialNoiseSources` identically in both cases.
        for expose_dirt in [false, true] {
            let (mut eager_chunk, mask) = build_chunk(expose_dirt);
            let (mut lazy_chunk, _) = build_chunk(expose_dirt);
            let (mut mismatched_rule_chunk, _) = build_chunk(expose_dirt);

            let mut aq_eager =
                DisabledAquifer::new(GlobalFluidPicker::new(12, water, lava, air), air);
            let mut ctx_eager = SamplerContext::default();
            let noise_surface =
                MaterialNoiseSources::from_json(root, EmptySurface, &noise_defs).unwrap();
            apply_carving_mask::<LegacyRandom, _, _>(
                &mut eager_chunk,
                &mask,
                valid_noise_rule.clone(),
                TopMaterialInputs {
                    generation,
                    root,
                    surface: noise_surface,
                    preliminary_surface: Arc::clone(&prelim),
                    density_context: &mut ctx_eager,
                },
                &mut aq_eager,
                &blocks,
                |_| true,
            )
            .unwrap();

            let mut aq_lazy =
                DisabledAquifer::new(GlobalFluidPicker::new(12, water, lava, air), air);
            let mut ctx_lazy = SamplerContext::default();
            apply_carving_mask_shared::<LegacyRandom, _, _>(
                &mut lazy_chunk,
                &mask,
                &valid_noise_rule,
                &mut shared,
                TopMaterialInputs {
                    generation,
                    root,
                    surface: EmptySurface,
                    preliminary_surface: Arc::clone(&prelim),
                    density_context: &mut ctx_lazy,
                },
                &mut aq_lazy,
                &blocks,
                |_| true,
            )
            .unwrap();
            assert_chunks_equal(
                &eager_chunk,
                &lazy_chunk,
                0,
                32,
                if expose_dirt {
                    "compiled shared exposed dirt"
                } else {
                    "compiled shared no exposed dirt"
                },
            );
            for (lx, lz, top_y) in [(1, 2, 18), (6, 9, 20), (12, 4, 22)] {
                let wx = 32 + lx;
                let wz = 48 + lz;
                assert_eq!(lazy_chunk.block(wx, top_y, wz), air);
                let below = lazy_chunk.block(wx, top_y - 1, wz);
                if expose_dirt {
                    assert!(
                        below == grass || below == mycelium,
                        "expected exposed dirt at ({wx}, {}, {wz}) to be repaired",
                        top_y - 1
                    );
                } else {
                    assert_eq!(below, dirt);
                }
            }

            // 4. Even if a different invalid rule is passed alongside an already-compiled
            // `shared`, `add_shared_rule` reuses the already-compiled rule in `shared`
            // without re-binding noises, so no-exposed-dirt and exposed-dirt stay identical
            // (never failing only when dirt is exposed).
            let mut aq_mis =
                DisabledAquifer::new(GlobalFluidPicker::new(12, water, lava, air), air);
            let mut ctx_mis = SamplerContext::default();
            apply_carving_mask_shared::<LegacyRandom, _, _>(
                &mut mismatched_rule_chunk,
                &mask,
                &invalid_noise_rule,
                &mut shared,
                TopMaterialInputs {
                    generation,
                    root,
                    surface: EmptySurface,
                    preliminary_surface: Arc::clone(&prelim),
                    density_context: &mut ctx_mis,
                },
                &mut aq_mis,
                &blocks,
                |_| true,
            )
            .unwrap();
            assert_chunks_equal(
                &eager_chunk,
                &mismatched_rule_chunk,
                0,
                32,
                if expose_dirt {
                    "compiled shared mismatched rule exposed dirt"
                } else {
                    "compiled shared mismatched rule no exposed dirt"
                },
            );
        }
    }

    #[derive(Debug)]
    struct WaveSampler;
    impl DensitySampler for WaveSampler {
        fn sample_value(&self, _: &mut SamplerContext, pos: [i32; 3]) -> Result<f32, SampleError> {
            Ok(((pos[0] * 13 + pos[1] * 7 + pos[2] * 5) & 7) as f32 * 0.25 - 0.5)
        }
    }

    #[derive(Debug)]
    struct SurfaceHeightSampler;
    impl DensitySampler for SurfaceHeightSampler {
        fn sample_value(&self, _: &mut SamplerContext, pos: [i32; 3]) -> Result<f32, SampleError> {
            Ok(14.0 + (pos[0] & 3) as f32)
        }
    }

    #[test]
    fn lazy_shared_top_material_worker_matches_eager_and_point_evaluation() {
        let registry = test_registry();
        let air = registry.default_state("minecraft:air").unwrap();
        let water = registry.default_state("minecraft:water").unwrap();
        let lava = registry.default_state("minecraft:lava").unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        let grass = registry.default_state("minecraft:grass_block").unwrap();
        let mycelium = registry.default_state("minecraft:mycelium").unwrap();
        let dirt = registry.default_state("minecraft:dirt").unwrap();
        let blocks = CarvingBlocks::from_registry(&registry, &BTreeSet::new()).unwrap();

        let interp: Arc<dyn DensitySampler> =
            Arc::new(InterpolatedSampler::new(Arc::new(WaveSampler), 4, 8).unwrap());
        let rule = MaterialRule::Sequence(vec![
            MaterialRule::OreVein {
                ore: mycelium,
                raw_ore: stone,
                filler: stone,
                raw_ore_chance: 0.25,
                density: Arc::clone(&interp),
                richness: Arc::clone(&interp),
                filler_gap: Arc::clone(&interp),
                positive_y: [0, 31],
            },
            MaterialRule::Condition {
                if_true: MaterialCondition::AbovePreliminarySurface,
                then_run: Box::new(MaterialRule::Block(grass)),
            },
            MaterialRule::Block(stone),
        ]);

        let generation = WorldGenerationContext::new(0, 32, 12);
        let root = LegacyRandom::new(42).fork_positional();
        let prelim: Arc<dyn DensitySampler> = Arc::new(SurfaceHeightSampler);

        // Warm `shared` on a dummy pass so it is reused below.
        let mut shared = None;
        {
            let mut warmup = TerrainChunk::new(
                ChunkPos::new(0, 0),
                0,
                32,
                &registry,
                SimpleStates { air, water, lava },
            )
            .unwrap();
            let mut warmup_mask = CarvingMask::new(0, 31).unwrap();
            warmup_mask.carve(0, 0, 0).unwrap();
            let mut aquifer =
                DisabledAquifer::new(GlobalFluidPicker::new(12, water, lava, air), air);
            let mut ctx = SamplerContext::default();
            apply_carving_mask_shared::<LegacyRandom, _, _>(
                &mut warmup,
                &warmup_mask,
                &rule,
                &mut shared,
                TopMaterialInputs {
                    generation,
                    root,
                    surface: EmptySurface,
                    preliminary_surface: Arc::clone(&prelim),
                    density_context: &mut ctx,
                },
                &mut aquifer,
                &blocks,
                |_| false,
            )
            .unwrap();
        }
        assert!(
            shared
                .as_ref()
                .is_some_and(CompiledSurfaceRule::has_eager_density_bindings)
        );

        let columns = [(1, 2, 20), (5, 7, 16), (9, 11, 10), (14, 3, 24)];
        let build_chunk = |expose_dirt: bool| {
            let mut chunk = TerrainChunk::new(
                ChunkPos::new(1, -1),
                0,
                32,
                &registry,
                SimpleStates { air, water, lava },
            )
            .unwrap();
            let mut mask = CarvingMask::new(0, 31).unwrap();
            for (lx, lz, top_y) in columns {
                let wx = 16 + lx;
                let wz = -16 + lz;
                chunk
                    .set_block_worldgen(wx, top_y, wz, if expose_dirt { grass } else { stone })
                    .unwrap();
                chunk.set_block_worldgen(wx, top_y - 1, wz, dirt).unwrap();
                chunk.set_block_worldgen(wx, top_y - 2, wz, stone).unwrap();
                mask.carve(lx, top_y, lz).unwrap();
            }
            (chunk, mask)
        };

        for expose_dirt in [false, true] {
            let (mut eager_chunk, mask) = build_chunk(expose_dirt);
            let (mut lazy_chunk, _) = build_chunk(expose_dirt);

            let mut aq_eager =
                DisabledAquifer::new(GlobalFluidPicker::new(12, water, lava, air), air);
            let mut ctx_eager = SamplerContext::default();
            apply_carving_mask::<LegacyRandom, _, _>(
                &mut eager_chunk,
                &mask,
                rule.clone(),
                TopMaterialInputs {
                    generation,
                    root,
                    surface: EmptySurface,
                    preliminary_surface: Arc::clone(&prelim),
                    density_context: &mut ctx_eager,
                },
                &mut aq_eager,
                &blocks,
                |_| true,
            )
            .unwrap();

            let mut aq_lazy =
                DisabledAquifer::new(GlobalFluidPicker::new(12, water, lava, air), air);
            let mut ctx_lazy = SamplerContext::default();
            apply_carving_mask_shared::<LegacyRandom, _, _>(
                &mut lazy_chunk,
                &mask,
                &rule,
                &mut shared,
                TopMaterialInputs {
                    generation,
                    root,
                    surface: EmptySurface,
                    preliminary_surface: Arc::clone(&prelim),
                    density_context: &mut ctx_lazy,
                },
                &mut aq_lazy,
                &blocks,
                |_| true,
            )
            .unwrap();

            assert_chunks_equal(&eager_chunk, &lazy_chunk, 0, 32, "eager vs lazy");
            if expose_dirt {
                let (mut point_chunk, _) = build_chunk(true);
                let mut ctx_point = SamplerContext::default();
                let mut repaired = 0usize;
                for &(lx, lz, top_y) in &columns {
                    let wx = 16 + lx;
                    let wz = -16 + lz;
                    let carved = eager_chunk.block(wx, top_y, wz);
                    point_chunk
                        .set_block_worldgen(wx, top_y, wz, carved)
                        .unwrap();
                    let submerged = point_chunk.block_properties(carved).has_fluid();
                    let replacement = top_material::<LegacyRandom, _, _>(
                        &mut point_chunk,
                        rule.clone(),
                        TopMaterialInputs {
                            generation,
                            root,
                            surface: EmptySurface,
                            preliminary_surface: Arc::clone(&prelim),
                            density_context: &mut ctx_point,
                        },
                        BlockPos::new(wx, top_y - 1, wz),
                        submerged,
                    )
                    .unwrap();
                    if let Some(state) = replacement {
                        point_chunk
                            .set_block_worldgen(wx, top_y - 1, wz, state)
                            .unwrap();
                    }
                    assert_eq!(
                        point_chunk.block(wx, top_y - 1, wz),
                        lazy_chunk.block(wx, top_y - 1, wz),
                        "point vs lazy mismatch at ({wx}, {}, {wz})",
                        top_y - 1
                    );
                    if eager_chunk.block(wx, top_y - 1, wz) != dirt {
                        repaired += 1;
                    }
                }
                assert!(repaired > 0, "expected exposed dirt cells to be repaired");
            }
        }
    }

    #[test]
    fn shared_carving_reuse_across_shuffled_chunks_matches_fresh_workers() {
        let registry = test_registry();
        let air = registry.default_state("minecraft:air").unwrap();
        let water = registry.default_state("minecraft:water").unwrap();
        let lava = registry.default_state("minecraft:lava").unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        let grass = registry.default_state("minecraft:grass_block").unwrap();
        let mycelium = registry.default_state("minecraft:mycelium").unwrap();
        let dirt = registry.default_state("minecraft:dirt").unwrap();
        let blocks = CarvingBlocks::from_registry(&registry, &BTreeSet::new()).unwrap();

        let generation = WorldGenerationContext::new(0, 32, 12);
        let root = LegacyRandom::new(999).fork_positional();
        let prelim: Arc<dyn DensitySampler> = Arc::new(SurfaceHeightSampler);
        let interp: Arc<dyn DensitySampler> =
            Arc::new(InterpolatedSampler::new(Arc::new(WaveSampler), 4, 8).unwrap());
        let noise_defs = BTreeMap::from([(
            "minecraft:shuffle_noise".to_owned(),
            r#"{"base_octave":-5}"#.to_owned(),
        )]);
        let rule = MaterialRule::Sequence(vec![
            MaterialRule::OreVein {
                ore: mycelium,
                raw_ore: stone,
                filler: stone,
                raw_ore_chance: 0.25,
                density: Arc::clone(&interp),
                richness: Arc::clone(&interp),
                filler_gap: Arc::clone(&interp),
                positive_y: [0, 31],
            },
            MaterialRule::Condition {
                if_true: MaterialCondition::NoiseThreshold {
                    noise: "minecraft:shuffle_noise".to_owned(),
                    min: -0.2,
                    max: 0.2,
                    is_3d: false,
                },
                then_run: Box::new(MaterialRule::Block(mycelium)),
            },
            MaterialRule::Condition {
                if_true: MaterialCondition::AbovePreliminarySurface,
                then_run: Box::new(MaterialRule::Block(grass)),
            },
            MaterialRule::Block(stone),
        ]);

        // Mix chunks with exposed dirt (grass & mycelium) and stone-only chunks (no exposed dirt).
        let specs = [
            (ChunkPos::new(-2, -1), true, grass),
            (ChunkPos::new(0, 0), false, stone),
            (ChunkPos::new(3, -2), true, mycelium),
            (ChunkPos::new(-1, 4), false, stone),
            (ChunkPos::new(5, 2), true, grass),
            (ChunkPos::new(-4, 1), true, mycelium),
        ];
        let build_case = |idx: usize| {
            let (pos, expose_dirt, surface_block) = specs[idx];
            let mut chunk =
                TerrainChunk::new(pos, 0, 32, &registry, SimpleStates { air, water, lava })
                    .unwrap();
            let mut mask = CarvingMask::new(0, 31).unwrap();
            let ox = pos.x * 16;
            let oz = pos.z * 16;
            for (lx, lz, top_y) in [(2, 3, 18), (7, 11, 15), (13, 5, 22)] {
                chunk
                    .set_block_worldgen(
                        ox + lx,
                        top_y,
                        oz + lz,
                        if expose_dirt { surface_block } else { stone },
                    )
                    .unwrap();
                chunk
                    .set_block_worldgen(ox + lx, top_y - 1, oz + lz, dirt)
                    .unwrap();
                chunk
                    .set_block_worldgen(ox + lx, top_y - 2, oz + lz, stone)
                    .unwrap();
                mask.carve(lx, top_y, lz).unwrap();
            }
            (chunk, mask)
        };

        // Baseline: fresh unshared worker per chunk.
        let baseline: Vec<_> = (0..specs.len())
            .map(|idx| {
                let (mut chunk, mask) = build_case(idx);
                let mut aq =
                    DisabledAquifer::new(GlobalFluidPicker::new(12, water, lava, air), air);
                let mut ctx = SamplerContext::default();
                let surface =
                    MaterialNoiseSources::from_json(root, EmptySurface, &noise_defs).unwrap();
                apply_carving_mask::<LegacyRandom, _, _>(
                    &mut chunk,
                    &mask,
                    rule.clone(),
                    TopMaterialInputs {
                        generation,
                        root,
                        surface,
                        preliminary_surface: Arc::clone(&prelim),
                        density_context: &mut ctx,
                    },
                    &mut aq,
                    &blocks,
                    |_| true,
                )
                .unwrap();
                chunk
            })
            .collect();

        // Candidate: ONE reused `shared` cache across two different shuffled orders.
        let mut shared = None;
        for order in [[3, 0, 5, 1, 4, 2], [5, 2, 0, 4, 1, 3]] {
            for idx in order {
                let (mut chunk, mask) = build_case(idx);
                let mut aq =
                    DisabledAquifer::new(GlobalFluidPicker::new(12, water, lava, air), air);
                let mut ctx = SamplerContext::default();
                let surface =
                    MaterialNoiseSources::from_json(root, EmptySurface, &noise_defs).unwrap();
                apply_carving_mask_shared::<LegacyRandom, _, _>(
                    &mut chunk,
                    &mask,
                    &rule,
                    &mut shared,
                    TopMaterialInputs {
                        generation,
                        root,
                        surface,
                        preliminary_surface: Arc::clone(&prelim),
                        density_context: &mut ctx,
                    },
                    &mut aq,
                    &blocks,
                    |_| true,
                )
                .unwrap();
                assert_chunks_equal(&baseline[idx], &chunk, 0, 32, "shuffled shared reuse");
            }
        }
        assert!(shared.is_some());
    }

    #[derive(Debug)]
    struct FailAtOriginSampler;
    impl DensitySampler for FailAtOriginSampler {
        fn sample_value(&self, _: &mut SamplerContext, pos: [i32; 3]) -> Result<f32, SampleError> {
            if pos == [0, 0, 0] {
                Err(SampleError::ZeroGradientPeriod)
            } else {
                Ok(0.0)
            }
        }
    }

    #[test]
    fn warm_cache_eager_density_origin_error_leaves_chunk_untouched() {
        let registry = test_registry();
        let air = registry.default_state("minecraft:air").unwrap();
        let water = registry.default_state("minecraft:water").unwrap();
        let lava = registry.default_state("minecraft:lava").unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        let grass = registry.default_state("minecraft:grass_block").unwrap();
        let mycelium = registry.default_state("minecraft:mycelium").unwrap();
        let blocks = CarvingBlocks::from_registry(&registry, &BTreeSet::new()).unwrap();

        let generation = WorldGenerationContext::new(0, 32, 12);
        let root = LegacyRandom::new(123).fork_positional();
        let prelim: Arc<dyn DensitySampler> = Arc::new(ZeroSampler);
        let zero: Arc<dyn DensitySampler> = Arc::new(ZeroSampler);
        let failing: Arc<dyn DensitySampler> = Arc::new(FailAtOriginSampler);

        let pos = ChunkPos::new(1, 1);
        let ox = pos.x * 16;
        let oz = pos.z * 16;
        let columns = [(2, 3, 18), (7, 11, 8), (13, 5, 22)];
        let build_stone_chunk = || {
            let mut chunk =
                TerrainChunk::new(pos, 0, 32, &registry, SimpleStates { air, water, lava })
                    .unwrap();
            for (lx, lz, top_y) in columns {
                chunk
                    .set_block_worldgen(ox + lx, top_y, oz + lz, stone)
                    .unwrap();
                chunk
                    .set_block_worldgen(ox + lx, top_y - 1, oz + lz, stone)
                    .unwrap();
                chunk
                    .set_block_worldgen(ox + lx, top_y - 2, oz + lz, stone)
                    .unwrap();
            }
            chunk
        };
        let mut mask = CarvingMask::new(0, 31).unwrap();
        for (lx, lz, top_y) in columns {
            mask.carve(lx, top_y, lz).unwrap();
        }
        assert!(!mask.is_empty());

        for fail_on_richness in [false, true] {
            let rule = MaterialRule::Sequence(vec![
                MaterialRule::OreVein {
                    ore: mycelium,
                    raw_ore: stone,
                    filler: stone,
                    raw_ore_chance: 0.25,
                    density: if fail_on_richness {
                        Arc::clone(&zero)
                    } else {
                        Arc::clone(&failing)
                    },
                    richness: if fail_on_richness {
                        Arc::clone(&failing)
                    } else {
                        Arc::clone(&zero)
                    },
                    filler_gap: Arc::clone(&zero),
                    positive_y: [0, 31],
                },
                MaterialRule::Block(grass),
            ]);

            // Warm `shared` on chunk (1, 1) (off-origin), matching surface building
            // before carving, so `shared` is populated without sampling `[0, 0, 0]`.
            let mut shared = None;
            {
                let volume = DensityVolume::new([16, 32, 16], [ox, 0, oz], [1; 3]).unwrap();
                let sources = MaterialDensitySources::new(
                    EmptySurface,
                    volume,
                    Arc::clone(&prelim),
                    SamplerContext::default(),
                )
                .unwrap();
                let mut warmup_worker = MaterialConditions::<LegacyRandom, _>::with_possible_biomes(
                    generation, root, sources, None,
                );
                warmup_worker.add_shared_rule(&rule, &mut shared).unwrap();
            }
            assert!(
                shared
                    .as_ref()
                    .is_some_and(CompiledSurfaceRule::has_eager_density_bindings)
            );

            // Nonempty carve with no exposed dirt must still run eager origin
            // sampling, return the sample error, and leave every block untouched.
            let expected_unchanged = build_stone_chunk();
            let mut chunk = build_stone_chunk();
            let mut aquifer =
                DisabledAquifer::new(GlobalFluidPicker::new(12, water, lava, air), air);
            let mut density_context = SamplerContext::default();
            let err = apply_carving_mask_shared::<LegacyRandom, _, _>(
                &mut chunk,
                &mask,
                &rule,
                &mut shared,
                TopMaterialInputs {
                    generation,
                    root,
                    surface: EmptySurface,
                    preliminary_surface: Arc::clone(&prelim),
                    density_context: &mut density_context,
                },
                &mut aquifer,
                &blocks,
                |_| true,
            );
            assert!(
                matches!(
                    err,
                    Err(SurfaceError::Decode(DecodeError::Sample(
                        SampleError::ZeroGradientPeriod
                    )))
                ),
                "expected origin sample error for fail_on_richness={fail_on_richness}, got {err:?}"
            );
            assert_chunks_equal(
                &expected_unchanged,
                &chunk,
                0,
                32,
                "warm cache origin sampling error",
            );
        }
    }
}
