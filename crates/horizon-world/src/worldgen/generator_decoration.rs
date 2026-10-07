//! FEATURES status over a 3×3 neighborhood of generated chunks.
//!
//! Placed features are resolved by name from the bundled feature registries.
//! Roots whose algorithm or bindings are not implemented are left out of
//! execution (but keep their place in vanilla's per-step feature ordering, so
//! decoration seeds of the remaining features are unchanged).
use super::{
    anchor::WorldGenerationContext,
    biome::BiomeId,
    biome_chunk::BiomeRegion,
    biome_predicate::BiomePredicateBindings,
    biome_source::ResolvedBiomeSource,
    biome_temperature::BiomeTemperature,
    biome_weather::BiomeWeatherBindings,
    block_feature::FeatureBlockBindings,
    decoration::{DecorationWorld, FeatureDecorationPlan},
    feature::{FeatureBlocks, FeatureWorld},
    feature_catalog::FeatureCatalog,
    feature_graph::PlacedFeatureDefinition,
    feature_region::{RegionError, TerrainRegion},
    feature_sort::FeatureIdentity,
    generator::{
        DecorateStats, GenBlockStates, GeneratorData, GeneratorError, ProtoChunk, WorkerState,
    },
    placement::{LayerBlock, PlacementContext, PlacementQueryError},
    tags::TagDefinitions,
    terrain::PreLightChunkStage,
    write_zone::GenerationWriteZone,
};
use super::{
    generator_blocks::BlockBehavior,
    state_provider::{StateProviderBindings, StateProviderCatalog},
};
use crate::{
    block_collision::{DynamicCollisionWorld, ShulkerCollisionState},
    block_placement::BlockPlacementBindings,
    block_support::SupportType,
    block_survival::SurvivalWorld,
    dynamic_support::DynamicSupportWorld,
    fluid::{FluidKind, FluidState},
    heightmap::{HeightmapState, HeightmapType},
    registry::BlockStateId,
};
use horizon_core::{
    biome::obfuscate_seed,
    geometry::{Axis, Direction, VoxelShape},
    position::{BlockPos, ChunkPos},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

pub(super) struct Decoration {
    plan: FeatureDecorationPlan,
    names: Vec<String>,
    /// Executable roots by identity, or why the root cannot be executed.
    features: Vec<Result<PlacedFeatureDefinition, String>>,
    /// Placed features of each biome's generation settings, by BiomeId.
    biome_features: Vec<BTreeSet<FeatureIdentity>>,
    bindings: FeatureBlockBindings<'static>,
    weather: Option<BiomeWeatherBindings>,
    blocks: FeatureBlocks,
}
fn resource(message: impl Into<String>) -> GeneratorError {
    GeneratorError::Resource(message.into())
}
/// Biomes in vanilla's possible-biome encounter order for this source.
fn possible_biomes(source: &ResolvedBiomeSource) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let ordered: Vec<String> = match source {
        ResolvedBiomeSource::MultiNoise(source) => source
            .parameters()
            .entries()
            .iter()
            .map(|(_, name)| name.clone())
            .collect(),
        ResolvedBiomeSource::End(source) => [
            &source.center,
            &source.highlands,
            &source.midlands,
            &source.islands,
            &source.barrens,
        ]
        .into_iter()
        .cloned()
        .collect(),
        other => other
            .possible_biomes()
            .into_iter()
            .map(str::to_owned)
            .collect(),
    };
    ordered
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect()
}
impl Decoration {
    pub(super) fn new(
        data: &Arc<GeneratorData>,
        source: &ResolvedBiomeSource,
    ) -> Result<Option<Self>, GeneratorError> {
        let Some(resources) = &data.feature_resources else {
            return Ok(None);
        };
        let registry = data.registry();
        let mut feature_tags = TagDefinitions::default();
        for (name, json) in &resources.feature_tags {
            feature_tags.apply_json(name, "vanilla", json)?;
        }
        let catalog =
            FeatureCatalog::from_resources(&resources.features, &resources.placed, &feature_tags)?;
        let biomes = Arc::new(BiomePredicateBindings::new(
            &data
                .biome_names()
                .iter()
                .enumerate()
                .map(|(i, name)| (name.to_string(), BiomeId(i as u32)))
                .collect(),
            &data.biome_tags,
        )?);
        let mut bindings =
            FeatureBlockBindings::with_tags(registry, data.integers().clone(), &data.block_tags)?
                .with_fluid_tags(&resources.fluid_tags)?
                .with_biomes(biomes.clone());
        if let Some(blocks) = &resources.blocks {
            bindings = bindings
                .with_block_properties(&blocks.properties)?
                .with_state_providers(
                    StateProviderBindings::new(
                        registry,
                        data.integers().clone(),
                        &data.block_tags,
                    )?
                    .with_block_properties(&blocks.properties)?
                    .with_fluid_tags(&resources.fluid_tags)?
                    .with_biomes(biomes),
                    StateProviderCatalog::from_resources(&resources.state_providers)?,
                    BlockPlacementBindings::new(
                        registry,
                        &blocks.properties,
                        &blocks.double_plants,
                        &blocks.mossy_carpets,
                    )?,
                )?;
        }
        let tags = feature_tags
            .build(|name, _| resources.placed.contains_key(name).then(|| name.to_owned()))
            .map_err(|e| resource(format!("feature tags: {e:?}")))?
            .tags;
        let mut ids = BTreeMap::<String, FeatureIdentity>::new();
        let mut names = Vec::new();
        let mut features = Vec::new();
        let mut identity = |name: &str| -> FeatureIdentity {
            *ids.entry(name.to_owned()).or_insert_with(|| {
                let id = FeatureIdentity(names.len() as u32);
                names.push(name.to_owned());
                features.push(
                    catalog
                        .placed_feature_with_blocks(&Value::from(name).to_string(), &bindings)
                        .map_err(|e| e.to_string()),
                );
                id
            })
        };
        let mut biome_features = vec![BTreeSet::new(); data.biome_names().len()];
        let mut plan = Vec::new();
        for name in possible_biomes(source) {
            let id = data
                .biome_id(&name)
                .ok_or_else(|| resource(format!("unknown biome {name}")))?;
            let json: Value = serde_json::from_str(data.biome_json(id))?;
            let mut steps = Vec::new();
            for step in json["features"].as_array().into_iter().flatten() {
                let mut list = Vec::new();
                let entries = match step {
                    Value::String(tag) if tag.starts_with('#') => tags
                        .get(&super::tags::identifier(&tag[1..])?)
                        .cloned()
                        .ok_or_else(|| resource(format!("unknown feature tag {tag}")))?,
                    Value::String(name) => vec![name.clone()],
                    Value::Array(values) => values
                        .iter()
                        .map(|v| {
                            v.as_str()
                                .map(str::to_owned)
                                .ok_or_else(|| resource("inline placed feature in biome"))
                        })
                        .collect::<Result<_, _>>()?,
                    _ => return Err(resource("biome feature step")),
                };
                for entry in entries {
                    let feature = identity(&super::tags::identifier(&entry)?);
                    list.push(feature);
                    biome_features[id.0 as usize].insert(feature);
                }
                steps.push(list);
            }
            plan.push((id, steps));
        }
        let plan = FeatureDecorationPlan::new(plan).map_err(|e| resource(e.to_string()))?;
        let weather = BiomeWeatherBindings::new(&bindings).ok();
        Ok(Some(Self {
            plan,
            names,
            features,
            biome_features,
            weather,
            blocks: FeatureBlocks::from_registry(registry)?,
            bindings,
        }))
    }
    /// Placed-feature roots that cannot run, with their decode errors.
    pub(super) fn unsupported(&self) -> Vec<(String, String)> {
        self.names
            .iter()
            .zip(&self.features)
            .filter_map(|(name, feature)| feature.as_ref().err().map(|e| (name.clone(), e.clone())))
            .collect()
    }
    pub(super) fn decorate(
        &self,
        data: &Arc<GeneratorData>,
        seed: i64,
        generation: WorldGenerationContext,
        worker: &mut WorkerState,
        region: &mut [ProtoChunk; 9],
        next_boolean: impl FnMut() -> bool,
    ) -> Result<DecorateStats, GeneratorError> {
        let center = region[4].position();
        for (i, chunk) in region.iter().enumerate() {
            let expected = ChunkPos::new(
                center.x.wrapping_add(i as i32 % 3 - 1),
                center.z.wrapping_add(i as i32 / 3 - 1),
            );
            if chunk.position() != expected {
                return Err(resource("decoration region is not a 3×3 neighborhood"));
            }
            if (chunk.terrain.generation_stage() as u8) < PreLightChunkStage::Terrain as u8 {
                return Err(resource("decoration requires terrain-stage chunks"));
            }
        }
        if region[4].is_decorated() {
            return Ok(DecorateStats::default());
        }
        let mut chunks = Vec::with_capacity(9);
        let mut biomes = Vec::with_capacity(9);
        let (min_y, height) = (region[4].terrain.min_y(), region[4].terrain.height());
        for chunk in region.iter_mut() {
            let empty = super::terrain::TerrainChunk::new(
                chunk.position(),
                min_y,
                height,
                data.registry(),
                data.block_states().clone(),
            )
            .map_err(|e| resource(e.to_string()))?;
            chunks.push(std::mem::replace(&mut chunk.terrain, empty));
            biomes.push(chunk.biomes().clone());
        }
        let terrain = TerrainRegion::new(chunks, generation)
            .and_then(|region| region.with_block_bindings(&self.bindings))
            .map(|region| region.with_world_seed(seed));
        let terrain = match terrain {
            Ok(terrain) => terrain,
            Err(e) => return Err(resource(e.to_string())),
        };
        let mut next_boolean = next_boolean;
        let mut world = World {
            next_boolean: &mut next_boolean,
            blocks: data
                .feature_resources
                .as_ref()
                .and_then(|r| r.blocks.as_ref()),
            region: terrain,
            biomes: BiomeRegion::new(biomes, obfuscate_seed(seed))
                .map_err(|e| resource(e.to_string()))?,
            zone: GenerationWriteZone::new(center, 1),
            decoration: self,
            temperatures: &mut worker.temperatures,
        };
        let mut stats = DecorateStats::default();
        let blocks = self.blocks;
        let result = self.plan.decorate_features(
            &mut world,
            center,
            seed,
            |world, placer, random, id, origin| {
                let Ok(feature) = &self.features[id.0 as usize] else {
                    stats.unsupported += 1;
                    stats.skipped.insert(self.names[id.0 as usize].clone());
                    return Ok(false);
                };
                stats.placed += 1;
                let placed = placer.place_with_biome_check(
                    &feature.placement,
                    world,
                    random,
                    origin,
                    id,
                    |world, random, p| feature.feature.place(world, random, p, blocks),
                );
                if placed.is_err() {
                    stats.failed += 1;
                    stats.skipped.insert(self.names[id.0 as usize].clone());
                }
                Ok(placed.unwrap_or(false))
            },
        );
        for terrain in world.region.into_chunks() {
            let p = terrain.position();
            let index = (p.z.wrapping_sub(center.z) + 1) * 3 + p.x.wrapping_sub(center.x) + 1;
            region[index as usize].terrain = terrain;
        }
        result.map_err(|e| GeneratorError::Stage("features", e.to_string()))?;
        region[4]
            .terrain
            .set_generation_stage(PreLightChunkStage::Features);
        region[4].set_decorated();
        Ok(stats)
    }
}

/// WorldGenRegion-style view: reads anywhere in the 3×3 region, writes within
/// the FEATURES step's write radius, biome lookups through the biome zoom.
struct World<'a> {
    next_boolean: &'a mut dyn FnMut() -> bool,
    blocks: Option<&'a BlockBehavior>,
    region: TerrainRegion<'static, GenBlockStates>,
    biomes: BiomeRegion<BiomeId>,
    zone: GenerationWriteZone,
    decoration: &'a Decoration,
    temperatures: &'a mut [BiomeTemperature],
}
impl DecorationWorld for World<'_> {
    type Error = RegionError;
    fn min_section_y(&mut self) -> Result<i32, RegionError> {
        Ok(self
            .region
            .min_y()
            .map_err(|_| RegionError::InvalidChunks)?
            >> 4)
    }
    fn chunk_biomes(&mut self, chunk: ChunkPos) -> Result<Vec<BiomeId>, RegionError> {
        self.biomes
            .present_in_chunk(chunk)
            .map_err(RegionError::Biome)
    }
    fn set_currently_generating(&mut self, _: Option<FeatureIdentity>) -> Result<(), RegionError> {
        Ok(())
    }
}
impl FeatureWorld for World<'_> {
    type Error = RegionError;
    fn block(&mut self, position: [i32; 3]) -> Result<BlockStateId, RegionError> {
        self.region.block(position)
    }
    fn set_block(
        &mut self,
        position: [i32; 3],
        state: BlockStateId,
        _flags: i32,
    ) -> Result<bool, RegionError> {
        if !self
            .zone
            .contains_block(BlockPos::new(position[0], position[1], position[2]))
        {
            return Ok(false);
        }
        self.region.set_block_worldgen(position, state)?;
        Ok(true)
    }
}
fn query(e: impl std::fmt::Display) -> PlacementQueryError {
    PlacementQueryError::Failed(e.to_string())
}
impl<'a> World<'a> {
    fn blocks(&self) -> Result<&'a BlockBehavior, PlacementQueryError> {
        self.blocks.ok_or(PlacementQueryError::Unavailable)
    }
}
/// Proto chunks hold no block entities, so dynamic shapes resolve to absent.
impl DynamicSupportWorld for World<'_> {
    type Error = PlacementQueryError;
    fn shulker_is_closed(&mut self, _: [i32; 3]) -> Result<Option<bool>, PlacementQueryError> {
        Ok(None)
    }
    fn moving_piston_shape(
        &mut self,
        _: [i32; 3],
    ) -> Result<Option<VoxelShape>, PlacementQueryError> {
        Ok(None)
    }
}
impl DynamicCollisionWorld for World<'_> {
    fn shulker_collision_state(
        &mut self,
        _: [i32; 3],
    ) -> Result<Option<ShulkerCollisionState>, PlacementQueryError> {
        Ok(None)
    }
}
impl SurvivalWorld for World<'_> {
    fn fluid_state(&mut self, position: [i32; 3]) -> Result<FluidState, PlacementQueryError> {
        self.region.fluid_state(position)
    }
    /// Unlit generation chunks report no sky or block light.
    fn raw_brightness(&mut self, _: [i32; 3], _: i32) -> Result<i32, PlacementQueryError> {
        Ok(0)
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.region.block_state(position)
    }
}
impl PlacementContext for World<'_> {
    fn state_collision_empty(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let collision = self.blocks()?.collision.clone();
        collision
            .collision_shape(self, state, position)
            .map(|shape| shape.is_empty())
            .map_err(query)
    }
    /// Block entities are not stored in proto chunks; loot tables are no-ops.
    fn set_block_entity_loot_table(
        &mut self,
        _: [i32; 3],
        _: &str,
        _: &mut impl horizon_core::random::RandomSource,
    ) -> Result<(), PlacementQueryError> {
        Ok(())
    }
    fn attachment_at(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let blocks = self.blocks()?;
        blocks
            .survival
            .can_attach_to(self, position, direction)
            .map_err(query)
    }
    fn next_world_boolean(&mut self) -> Result<bool, PlacementQueryError> {
        Ok((self.next_boolean)())
    }
    fn block_survives(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let blocks = self.blocks()?;
        blocks
            .survival
            .can_survive(self, state, position)
            .map_err(query)
    }
    fn face_sturdy(
        &mut self,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let state = self.region.block_state(position)?;
        self.state_face_sturdy(state, position, direction)
    }
    fn state_face_sturdy(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
        direction: Direction,
    ) -> Result<bool, PlacementQueryError> {
        let support = self.blocks()?.support.clone();
        support
            .is_face_sturdy(self, state, position, direction, SupportType::Full)
            .map_err(query)
    }
    fn collision_full_block(
        &mut self,
        state: BlockStateId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let collision = self.blocks()?.collision.clone();
        let shape = collision
            .collision_shape(self, state, position)
            .map_err(query)?;
        let [nx, ny, nz] = shape.sizes();
        Ok(!shape.is_empty()
            && [Axis::X, Axis::Y, Axis::Z]
                .into_iter()
                .all(|axis| shape.min(axis) == 0.0 && shape.max(axis) == 1.0)
            && (0..nx).all(|x| {
                (0..ny).all(|y| (0..nz).all(|z| shape.is_full(x as i32, y as i32, z as i32)))
            }))
    }
    fn world_seed(&mut self) -> Result<i64, PlacementQueryError> {
        self.region.world_seed()
    }
    fn ensure_can_write(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Ok(self
            .zone
            .contains_block(BlockPos::new(position[0], position[1], position[2])))
    }
    /// BulkSectionAccess write: section storage only, no heightmap updates.
    fn set_section_block(
        &mut self,
        [x, y, z]: [i32; 3],
        state: BlockStateId,
    ) -> Result<(), PlacementQueryError> {
        self.region
            .chunk_mut(x >> 4, z >> 4)?
            .set_block_raw(x, y, z, state)
            .map_err(|e| query(format!("{e:?}")))?;
        Ok(())
    }
    /// Generation regions contain no entities.
    fn unobstructed_block(&mut self, _: [i32; 3]) -> Result<bool, PlacementQueryError> {
        Ok(true)
    }
    fn destroy_block_without_entity(
        &mut self,
        position: [i32; 3],
        _drop_resources: bool,
    ) -> Result<bool, PlacementQueryError> {
        let chunk = self.region.chunk(position[0] >> 4, position[2] >> 4)?;
        let state = chunk.block(position[0], position[1], position[2]);
        if chunk.block_properties(state).is_air() {
            return Ok(false);
        }
        let air = self
            .decoration
            .bindings
            .registry()
            .default_state("minecraft:air")
            .ok_or(PlacementQueryError::Unavailable)?;
        self.set_block(position, air, 3).map_err(query)
    }
    fn biome_should_freeze(
        &mut self,
        biome: BiomeId,
        position: [i32; 3],
        require_edge: bool,
    ) -> Result<bool, PlacementQueryError> {
        let weather = self
            .decoration
            .weather
            .clone()
            .ok_or(PlacementQueryError::Unavailable)?;
        let settings = self.temperatures[biome.0 as usize].settings();
        let mut temperature = std::mem::replace(
            &mut self.temperatures[biome.0 as usize],
            BiomeTemperature::new(settings),
        );
        let result = weather.should_freeze(self, &mut temperature, position, require_edge);
        self.temperatures[biome.0 as usize] = temperature;
        result
    }
    fn biome_should_snow(
        &mut self,
        biome: BiomeId,
        position: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let weather = self
            .decoration
            .weather
            .clone()
            .ok_or(PlacementQueryError::Unavailable)?;
        let settings = self.temperatures[biome.0 as usize].settings();
        let mut temperature = std::mem::replace(
            &mut self.temperatures[biome.0 as usize],
            BiomeTemperature::new(settings),
        );
        let result = weather.should_snow(self, &mut temperature, position);
        self.temperatures[biome.0 as usize] = temperature;
        result
    }
    /// Lighting has not run during FEATURES; block light reads zero.
    fn block_brightness(&mut self, _: [i32; 3]) -> Result<i32, PlacementQueryError> {
        Ok(0)
    }
    fn fluid_state(&mut self, position: [i32; 3]) -> Result<FluidState, PlacementQueryError> {
        self.region.fluid_state(position)
    }
    fn sea_level(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.sea_level()
    }
    fn schedule_fluid_tick(
        &mut self,
        position: [i32; 3],
        fluid: FluidKind,
        delay: i32,
    ) -> Result<(), PlacementQueryError> {
        self.region.schedule_fluid_tick(position, fluid, delay)
    }
    fn mark_postprocessing(&mut self, position: [i32; 3]) -> Result<(), PlacementQueryError> {
        self.region.mark_postprocessing(position)
    }
    fn water_at(&mut self, position: [i32; 3]) -> Result<bool, PlacementQueryError> {
        self.region.water_at(position)
    }
    fn schedule_block_tick(
        &mut self,
        position: [i32; 3],
        block: BlockStateId,
        delay: i32,
    ) -> Result<(), PlacementQueryError> {
        self.region.schedule_block_tick(position, block, delay)
    }
    fn biome_identity(&mut self, position: [i32; 3]) -> Result<BiomeId, PlacementQueryError> {
        self.region
            .biome_at(
                &self.biomes,
                BlockPos::new(position[0], position[1], position[2]),
            )
            .copied()
            .map_err(Into::into)
    }
    fn biome_allows_feature(
        &mut self,
        position: [i32; 3],
        feature: FeatureIdentity,
    ) -> Result<bool, PlacementQueryError> {
        let biome = self.biome_identity(position)?;
        Ok(self.decoration.biome_features[biome.0 as usize].contains(&feature))
    }
    fn block_state(&mut self, position: [i32; 3]) -> Result<BlockStateId, PlacementQueryError> {
        self.region.block_state(position)
    }
    fn block_matches_tag(
        &mut self,
        position: [i32; 3],
        tag: &str,
    ) -> Result<bool, PlacementQueryError> {
        self.region.block_matches_tag(position, tag)
    }
    fn max_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.max_y()
    }
    fn layer_block(&mut self, position: [i32; 3]) -> Result<LayerBlock, PlacementQueryError> {
        self.region.layer_block(position)
    }
    fn generation(&self) -> WorldGenerationContext {
        self.region.generation()
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        self.region.min_y()
    }
    fn height(&mut self, kind: HeightmapType, x: i32, z: i32) -> Result<i32, PlacementQueryError> {
        self.region.height(kind, x, z)
    }
}
