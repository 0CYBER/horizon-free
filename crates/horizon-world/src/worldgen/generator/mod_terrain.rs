//! A level whose terrain a mod's chunk generator writes, with the vanilla stages it asks for
//! ([`VanillaStages`]): the noise generator's pipeline over the level's noise settings and biome
//! source, with the mod's terrain in place of `NoiseBasedChunkGenerator.doFill`. The material
//! rule and the carvers run after it as `buildTerrain` runs them after its fill, and the
//! structures and features as the base `ChunkGenerator` runs them.
use super::*;
use crate::worldgen::mod_generators::{ChunkAccess, ModChunkGenerator, VanillaStages};
use crate::worldgen::terrain::TerrainError;

/// A mod's terrain and the stages its level runs. `stages.structures` is off when the world's
/// structures are.
pub(super) struct ModTerrain {
    /// The registered generator's id, for the refusals.
    id: String,
    level: Box<str>,
    generator: Arc<dyn ModChunkGenerator>,
    pub(super) stages: VanillaStages,
}

impl ModTerrain {
    /// The fill of `buildTerrain`: the mod's blocks in `chunk`, which is air, within the noise's
    /// `min_y` and `height` as the noise fill writes, and the worldgen heightmaps the noise fill
    /// keeps (`ChunkStatus.NOISE`'s), which the material rule and the carvers read.
    pub(super) fn fill(
        &self,
        chunk: &mut TerrainChunk<'_, GenBlockStates>,
        registry: &'static BlockRegistry,
        seed: i64,
        [min_y, height]: [i32; 2],
    ) -> Result<(), TerrainError> {
        self.generator.build_terrain(&mut Filling {
            chunk,
            registry,
            level: &self.level,
            seed,
            min_y,
            height,
        });
        chunk.prime_heightmaps(&[HeightmapType::OceanFloorWg, HeightmapType::WorldSurfaceWg])
    }
}

/// The proto chunk as a mod's generator writes it: its noise range only.
struct Filling<'a, 'r> {
    chunk: &'a mut TerrainChunk<'r, GenBlockStates>,
    registry: &'static BlockRegistry,
    level: &'a str,
    seed: i64,
    min_y: i32,
    height: i32,
}

impl ChunkAccess for Filling<'_, '_> {
    fn registry(&self) -> &'static BlockRegistry {
        self.registry
    }
    fn pos(&self) -> ChunkPos {
        self.chunk.position()
    }
    fn level(&self) -> &str {
        self.level
    }
    fn seed(&self) -> i64 {
        self.seed
    }
    fn min_y(&self) -> i32 {
        self.min_y
    }
    fn height(&self) -> i32 {
        self.height
    }
    fn state(&self, name: &str) -> BlockStateId {
        super::super::structure::legacy::block_state(self.registry, name, &[])
    }
    fn state_with(&self, name: &str, properties: &[(&str, &str)]) -> BlockStateId {
        super::super::structure::legacy::block_state(self.registry, name, properties)
    }
    fn set_block(&mut self, [x, y, z]: [i32; 3], state: BlockStateId) -> bool {
        let pos = self.chunk.position();
        x >> 4 == pos.x
            && z >> 4 == pos.z
            && (self.min_y..self.min_y + self.height).contains(&y)
            && self.chunk.set_block_raw(x, y, z, state).is_ok()
    }
}

impl<R: RandomSource + 'static> Pipeline<R>
where
    R::Positional: Clone + Send + Sync,
{
    /// The pipeline of a mod's level: `terrain` in place of the noise, and decoration with the
    /// biomes' placed features only when it asks for them, and none when it asks for neither
    /// structures nor features. A stage the world's data cannot supply (no structure set, no
    /// placed features, a placed feature the game cannot place) refuses the level, naming the
    /// generator.
    pub(super) fn with_mod_terrain(
        mut self,
        terrain: Option<ModTerrain>,
    ) -> Result<Self, GeneratorError> {
        let Some(terrain) = terrain else {
            return Ok(self);
        };
        let (id, stages) = (terrain.id.as_str(), terrain.stages);
        if stages.structures && self.structures.is_none() {
            return Err(resource(format!(
                "chunk generator {id} asks for structures, but the world has no structure sets"
            )));
        }
        if !stages.features {
            // The structure pieces still place in FEATURES, so the decoration stays without them.
            self.decoration = if stages.structures {
                super::super::generator_decoration::Decoration::new_with(
                    &self.data,
                    &self.source,
                    Some(super::super::generator_decoration::FlatDecoration::NONE),
                )?
                .map(Arc::new)
            } else {
                None
            };
        }
        let asked = if stages.features {
            "features"
        } else {
            "structures"
        };
        if (stages.structures || stages.features) && self.decoration.is_none() {
            return Err(resource(format!(
                "chunk generator {id} asks for {asked}, but the world has no placed features"
            )));
        }
        if let Some(decoration) = self.decoration.as_ref().filter(|_| stages.features) {
            let unsupported = decoration.unsupported();
            if !unsupported.is_empty() {
                let features: Vec<String> = unsupported
                    .iter()
                    .map(|(name, error)| format!("{name} ({error})"))
                    .collect();
                return Err(resource(format!(
                    "chunk generator {id} asks for features the game cannot place: {}",
                    features.join(", ")
                )));
            }
        }
        self.mod_terrain = Some(terrain);
        Ok(self)
    }
}

impl OverworldGenerator {
    /// A level whose stem names `generator`, a mod's that asks for vanilla stages: its terrain,
    /// then those stages, over the stem's noise settings and biome source. `structures` is the
    /// world's `generate_structures`. A level whose stem names no noise settings, or ones the
    /// world lacks, is refused naming the generator and the settings.
    pub fn for_mod(
        data: Arc<GeneratorData>,
        seed: i64,
        level: &str,
        stem: &Value,
        generator: Arc<dyn ModChunkGenerator>,
        structures: bool,
    ) -> Result<Self, GeneratorError> {
        let id = stem["generator"]["type"].as_str().unwrap_or_default();
        match &stem["generator"]["settings"] {
            Value::String(name) if !data.settings.contains_key(name) => {
                return Err(resource(format!(
                    "chunk generator {id} runs vanilla stages over the unknown noise settings {name}"
                )));
            }
            Value::String(_) | Value::Object(_) => {}
            _ => {
                return Err(resource(format!(
                    "chunk generator {id} runs vanilla stages, but its level names no noise settings"
                )));
            }
        }
        let mut stages = generator.vanilla_stages();
        stages.structures &= structures;
        let terrain = ModTerrain {
            id: id.to_owned(),
            level: level.into(),
            generator,
            stages,
        };
        Self::from_stem(data, seed, stem, stages.structures, None, Some(terrain))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stone from the level's floor up to y 70 in every column of the chunk.
    struct Slab(VanillaStages);
    impl ModChunkGenerator for Slab {
        fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
            let (pos, stone) = (chunk.pos(), chunk.state("stone"));
            for y in chunk.min_y()..=70 {
                for x in pos.x * 16..pos.x * 16 + 16 {
                    for z in pos.z * 16..pos.z * 16 + 16 {
                        chunk.set_block([x, y, z], stone);
                    }
                }
            }
        }
        fn vanilla_stages(&self) -> VanillaStages {
            self.0
        }
    }

    /// The worldgen data of the prepared resources, loaded once for these tests.
    fn data() -> Arc<GeneratorData> {
        static DATA: OnceLock<Arc<GeneratorData>> = OnceLock::new();
        DATA.get_or_init(|| {
            let root = std::path::PathBuf::from(
                std::env::var_os("HORIZON_TEST_RESOURCES").expect("resources"),
            );
            let registry = Box::leak(Box::new(crate::level::data::load_registry(&root).unwrap()));
            GeneratorData::load(&root.join("worldgen"), registry).unwrap()
        })
        .clone()
    }

    /// An Overworld-shaped level of plains whose terrain is the slab, over the Overworld's noise
    /// settings, at seed 12345.
    fn level(stages: VanillaStages) -> OverworldGenerator {
        let stem = stem(Value::from("minecraft:overworld"));
        OverworldGenerator::for_mod(
            data(),
            12345,
            "minecraft:overworld",
            &stem,
            Arc::new(Slab(stages)),
            true,
        )
        .unwrap()
    }

    fn stem(settings: Value) -> Value {
        serde_json::json!({
            "type": "minecraft:overworld",
            "generator": {
                "type": "test:slab",
                "settings": settings,
                "biome_source": {"type": "minecraft:fixed", "biome": "minecraft:plains"},
            },
        })
    }

    /// The material rule runs over the mod's stone after its terrain: plains grass and dirt on
    /// top, deepslate below y 8 and the bedrock floor.
    #[test]
    #[ignore = "daily"]
    fn the_surface_covers_the_mods_terrain() {
        let level = level(VanillaStages {
            surface: true,
            ..Default::default()
        });
        let chunk = level.worker().generate_proto(ChunkPos::new(0, 0)).unwrap();
        for (y, block) in [
            (71, "minecraft:air"),
            (70, "minecraft:grass_block"),
            (69, "minecraft:dirt"),
            (67, "minecraft:dirt"),
            (66, "minecraft:stone"),
            (0, "minecraft:deepslate"),
            (-60, "minecraft:deepslate"),
            (-64, "minecraft:bedrock"),
        ] {
            assert_eq!(chunk.block_name(0, y, 0), block, "y {y}");
        }
    }

    /// A carver cuts the mod's stone at seed 12345, the aquifer of the Overworld's noise filling
    /// it below sea level; the top stays the mod's stone without the surface.
    #[test]
    #[ignore = "daily"]
    fn the_carvers_cut_the_mods_terrain() {
        let level = level(VanillaStages {
            carvers: true,
            ..Default::default()
        });
        let chunk = level.worker().generate_proto(ChunkPos::new(0, 1)).unwrap();
        assert_eq!(chunk.block_name(0, 40, 25), "minecraft:water");
        assert_eq!(chunk.block_name(0, 70, 16), "minecraft:stone");
    }

    /// At seed 12345 a ruined portal starts in chunk 1 0 and its pieces place there in
    /// FEATURES (not without the structures), and the plains' grass grows on the surface in
    /// chunk 0 0.
    #[test]
    #[ignore = "daily"]
    fn structures_and_features_place_after_the_terrain() {
        let all = VanillaStages {
            surface: true,
            carvers: true,
            structures: true,
            features: true,
        };
        // The decorated chunk `x` 0 of a level with `stages`.
        let decorated = |stages, x: i32| {
            let mut worker = level(stages).worker();
            let mut region: [ProtoChunk; 9] = std::array::from_fn(|i| {
                let i = i as i32;
                worker
                    .generate_proto(ChunkPos::new(x + i % 3 - 1, i / 3 - 1))
                    .unwrap()
            });
            worker.decorate(&mut region).unwrap();
            let [.., center, _, _, _, _] = region;
            (worker, center)
        };
        let (mut worker, chunk) = decorated(all, 0);
        assert_eq!(chunk.block_name(15, 71, 0), "minecraft:short_grass");
        assert_eq!(chunk.block_name(15, 70, 0), "minecraft:grass_block");
        let starts = worker.structure_starts(ChunkPos::new(1, 0));
        let portal = &starts[0];
        assert_eq!(portal.name, "minecraft:ruined_portal");
        assert_eq!(
            (portal.bounding_box.min, portal.bounding_box.max),
            ([15, 12, 1], [21, 18, 5])
        );
        let (_, with) = decorated(all, 1);
        let (_, without) = decorated(
            VanillaStages {
                structures: false,
                ..all
            },
            1,
        );
        for ([x, y, z], piece) in [
            ([16, 12, 1], "minecraft:netherrack"),
            ([16, 12, 3], "minecraft:magma_block"),
            ([16, 13, 1], "minecraft:air"),
        ] {
            assert_eq!(with.block_name(x, y, z), piece, "{x} {y} {z}");
            assert_ne!(without.block_name(x, y, z), piece, "{x} {y} {z}");
        }
    }

    /// A level whose stem names no noise settings, or ones the world lacks, is refused naming
    /// the generator and the settings.
    #[test]
    #[ignore = "daily"]
    fn a_level_without_its_noise_settings_is_refused_by_name() {
        let refusal = |settings| {
            let slab = Arc::new(Slab(VanillaStages {
                surface: true,
                ..Default::default()
            }));
            match OverworldGenerator::for_mod(
                data(),
                0,
                "minecraft:overworld",
                &stem(settings),
                slab,
                true,
            ) {
                Ok(_) => panic!("opened"),
                Err(e) => e.to_string(),
            }
        };
        assert_eq!(
            refusal(Value::Null),
            "invalid generator resources: chunk generator test:slab runs vanilla stages, but its \
             level names no noise settings"
        );
        assert_eq!(
            refusal(Value::from("test:gone")),
            "invalid generator resources: chunk generator test:slab runs vanilla stages over the \
             unknown noise settings test:gone"
        );
    }

    /// The mod writes within the noise range, as the noise fill does: an Overworld-shaped level
    /// (y -64 to 319) over the Nether's noise settings (y 0 to 127) gets the mod's stone there
    /// and nowhere else.
    #[test]
    #[ignore = "daily"]
    fn the_mods_terrain_fills_the_noise_range_not_the_levels() {
        struct Column;
        impl ModChunkGenerator for Column {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                assert_eq!((chunk.min_y(), chunk.height()), (0, 128));
                let (pos, stone) = (chunk.pos(), chunk.state("stone"));
                let written = (-64..320)
                    .filter(|&y| chunk.set_block([pos.x * 16, y, pos.z * 16], stone))
                    .count();
                assert_eq!(written, 128);
            }
            fn vanilla_stages(&self) -> VanillaStages {
                VanillaStages {
                    structures: true,
                    ..Default::default()
                }
            }
        }
        let stem = stem(Value::from("minecraft:nether"));
        let level = OverworldGenerator::for_mod(
            data(),
            0,
            "minecraft:overworld",
            &stem,
            Arc::new(Column),
            true,
        )
        .unwrap();
        let chunk = level.worker().generate_proto(ChunkPos::new(0, 0)).unwrap();
        assert_eq!(chunk.terrain.min_y(), -64);
        for (y, block) in [
            (-64, "minecraft:air"),
            (-1, "minecraft:air"),
            (0, "minecraft:stone"),
            (127, "minecraft:stone"),
            (128, "minecraft:air"),
        ] {
            assert_eq!(chunk.block_name(0, y, 0), block, "y {y}");
        }
    }

    #[test]
    #[ignore = "daily"]
    fn staged_mod_terrain_receives_its_level_id() {
        struct Pocket;
        impl ModChunkGenerator for Pocket {
            fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
                if chunk.level() == "test:pocket" {
                    let pos = chunk.pos();
                    let stone = chunk.state("stone");
                    assert!(chunk.set_block([pos.x * 16, 64, pos.z * 16], stone));
                }
            }
        }
        let stem = stem(Value::from("minecraft:overworld"));
        for (id, expected) in [
            ("test:pocket", "minecraft:stone"),
            ("test:other", "minecraft:air"),
        ] {
            let level =
                OverworldGenerator::for_mod(data(), 12345, id, &stem, Arc::new(Pocket), true)
                    .unwrap();
            let chunk = level.worker().generate_proto(ChunkPos::new(0, 0)).unwrap();
            assert_eq!(chunk.block_name(0, 64, 0), expected, "level {id}");
        }
    }

    /// In a world without placed features or structure sets, a generator that asks only for the
    /// surface and the carvers opens, and its FEATURES status places nothing; one that asks for
    /// features or structures is refused, naming the generator and what is missing.
    #[test]
    #[ignore = "daily"]
    fn without_placed_features_only_the_terrain_stages_open() {
        let root = std::path::PathBuf::from(
            std::env::var_os("HORIZON_TEST_RESOURCES").expect("resources"),
        );
        let registry = Box::leak(Box::new(crate::level::data::load_registry(&root).unwrap()));
        let mut bare = GeneratorData::load(&root.join("worldgen"), registry).unwrap();
        let fields = Arc::get_mut(&mut bare).expect("a fresh load");
        fields.feature_resources = None;
        fields.structures = None;
        fields.structure_resources = None;
        let open = |stages| {
            let stem = stem(Value::from("minecraft:overworld"));
            OverworldGenerator::for_mod(
                bare.clone(),
                12345,
                "minecraft:overworld",
                &stem,
                Arc::new(Slab(stages)),
                true,
            )
        };
        let surface = VanillaStages {
            surface: true,
            ..Default::default()
        };
        let carvers = VanillaStages {
            carvers: true,
            ..Default::default()
        };
        for stages in [surface, carvers] {
            let mut worker = open(stages).unwrap().worker();
            let mut region: [ProtoChunk; 9] = std::array::from_fn(|i| {
                let i = i as i32;
                worker
                    .generate_proto(ChunkPos::new(i % 3 - 1, i / 3 - 1))
                    .unwrap()
            });
            let undecorated = blocks(&region[4]);
            worker.decorate(&mut region).unwrap();
            assert!(region[4].is_decorated(), "{stages:?}");
            assert!(blocks(&region[4]) == undecorated, "{stages:?}");
            if stages == surface {
                assert_eq!(region[4].block_name(0, 70, 0), "minecraft:grass_block");
            }
        }
        let refusal = |stages| match open(stages) {
            Ok(_) => panic!("opened"),
            Err(e) => e.to_string(),
        };
        assert_eq!(
            refusal(VanillaStages {
                features: true,
                ..Default::default()
            }),
            "invalid generator resources: chunk generator test:slab asks for features, but the \
             world has no placed features"
        );
        assert_eq!(
            refusal(VanillaStages {
                structures: true,
                ..Default::default()
            }),
            "invalid generator resources: chunk generator test:slab asks for structures, but the \
             world has no structure sets"
        );
    }

    /// Stone up to a height that changes from column to column and with the seed, with every
    /// stage after it.
    struct Hills;
    impl ModChunkGenerator for Hills {
        fn build_terrain(&self, chunk: &mut dyn ChunkAccess) {
            let (pos, stone, seed) = (chunk.pos(), chunk.state("stone"), chunk.seed() as i32);
            for x in pos.x * 16..pos.x * 16 + 16 {
                for z in pos.z * 16..pos.z * 16 + 16 {
                    let top = 60 + ((x.wrapping_mul(31) ^ z.wrapping_mul(17) ^ seed) & 15);
                    for y in chunk.min_y()..=top {
                        chunk.set_block([x, y, z], stone);
                    }
                }
            }
        }
        fn vanilla_stages(&self) -> VanillaStages {
            VanillaStages {
                surface: true,
                carvers: true,
                structures: true,
                features: true,
            }
        }
    }

    /// Every block of `chunk`, y up, then z, then x.
    fn blocks(chunk: &ProtoChunk) -> Vec<BlockStateId> {
        let ChunkPos { x, z } = chunk.position();
        (-64..320)
            .flat_map(|y| (0..16).flat_map(move |dz| (0..16).map(move |dx| [dx, y, dz])))
            .map(|[dx, y, dz]| chunk.block(x * 16 + dx, y, z * 16 + dz))
            .collect()
    }

    /// A structure start as a chunk holds it or refers to it: its structure, start chunk,
    /// bounding box and piece count.
    type Start = (String, [i32; 2], [i32; 3], [i32; 3], usize);

    /// What a proto chunk holds: its blocks, the biome of every quart cell, the structure
    /// starts it owns and those it refers to (sorted, so their order does not matter), its
    /// status and whether it is decorated.
    #[derive(PartialEq)]
    struct Snapshot {
        blocks: Vec<BlockStateId>,
        biomes: Vec<BiomeId>,
        own_starts: Vec<Start>,
        references: Vec<Start>,
        stage: PreLightChunkStage,
        decorated: bool,
    }

    fn snapshot(chunk: &ProtoChunk) -> Snapshot {
        let ChunkPos { x, z } = chunk.position();
        let biomes = (-64..320)
            .step_by(4)
            .flat_map(|y| {
                (0..16)
                    .step_by(4)
                    .flat_map(move |dz| (0..16).step_by(4).map(move |dx| [dx, y, dz]))
            })
            .map(|[dx, y, dz]| chunk.biome_id_at(x * 16 + dx, y, z * 16 + dz))
            .collect();
        let start = |s: &super::super::super::structure::start::StructureStart| {
            let (min, max) = (s.bounding_box.min, s.bounding_box.max);
            (
                s.name.clone(),
                [s.chunk.x, s.chunk.z],
                min,
                max,
                s.pieces.len(),
            )
        };
        let mut own_starts: Vec<Start> = chunk.own_starts().iter().map(|s| start(s)).collect();
        let mut references: Vec<Start> = chunk.structure_starts.iter().map(|s| start(s)).collect();
        own_starts.sort();
        references.sort();
        Snapshot {
            blocks: blocks(chunk),
            biomes,
            own_starts,
            references,
            stage: chunk.terrain.generation_stage(),
            decorated: chunk.is_decorated(),
        }
    }

    /// The region around `center`, its chunks generated in the order of `order` (row-major
    /// indices), as snapshots before and after its FEATURES status.
    fn complete(
        worker: &mut GeneratorWorker,
        center: ChunkPos,
        order: impl Iterator<Item = usize>,
    ) -> [[Snapshot; 9]; 2] {
        let mut made: Vec<(usize, ProtoChunk)> = order
            .map(|i| {
                let (dx, dz) = (i as i32 % 3 - 1, i as i32 / 3 - 1);
                let chunk = ChunkPos::new(center.x + dx, center.z + dz);
                (i, worker.generate_proto(chunk).unwrap())
            })
            .collect();
        made.sort_by_key(|(i, _)| *i);
        let mut region: [ProtoChunk; 9] = made
            .into_iter()
            .map(|(_, chunk)| chunk)
            .collect::<Vec<_>>()
            .try_into()
            .unwrap_or_else(|_| panic!("nine chunks"));
        let before = std::array::from_fn(|i| snapshot(&region[i]));
        worker.decorate(&mut region).unwrap();
        [before, std::array::from_fn(|i| snapshot(&region[i]))]
    }

    /// Complete chunks of a staged hilly level over the Overworld's biomes (blocks, biomes,
    /// structure starts and references, status) are the same whichever order they are generated
    /// in and whichever workers generate them: one worker in order, against two workers at once
    /// on a second generator, each going backwards. The regions hold more than one biome, a
    /// structure reference, a cave crossing a chunk border and features written into a
    /// neighbour chunk.
    #[test]
    #[ignore = "daily"]
    fn complete_chunks_do_not_depend_on_order_or_workers() {
        let mut stem = stem(Value::from("minecraft:overworld"));
        stem["generator"]["biome_source"] =
            serde_json::json!({"type": "minecraft:multi_noise", "preset": "minecraft:overworld"});
        let open = || {
            OverworldGenerator::for_mod(
                data(),
                12345,
                "minecraft:overworld",
                &stem,
                Arc::new(Hills),
                true,
            )
        };
        let centers = [(0, 0), (1, 0), (0, 1), (1, 1)].map(|(x, z)| ChunkPos::new(x, z));
        let first = open().unwrap();
        let mut worker = first.worker();
        let ordered: Vec<_> = centers
            .iter()
            .map(|&center| complete(&mut worker, center, 0..9))
            .collect();
        let second = open().unwrap();
        let mut shuffled: Vec<_> = std::thread::scope(|scope| {
            let halves = [[3, 0], [2, 1]].map(|half| {
                let second = &second;
                scope.spawn(move || {
                    let mut worker = second.worker();
                    half.map(|c| (c, complete(&mut worker, centers[c], (0..9).rev())))
                })
            });
            halves
                .into_iter()
                .flat_map(|half| half.join().unwrap())
                .collect()
        });
        shuffled.sort_by_key(|(c, _)| *c);
        for (c, (_, chunks)) in shuffled.iter().enumerate() {
            assert!(chunks == &ordered[c], "the region around {:?}", centers[c]);
        }
        let snapshots = || ordered.iter().flatten().flatten();
        assert!(ordered.iter().all(|[_, after]| {
            after[4].decorated && after[4].stage == PreLightChunkStage::Features
        }));
        let biomes: BTreeSet<BiomeId> =
            snapshots().flat_map(|c| c.biomes.iter().copied()).collect();
        assert!(biomes.len() > 1, "{biomes:?}");
        assert!(
            snapshots().any(|c| !c.references.is_empty()),
            "a structure reference"
        );
        let name = |id: BlockStateId| first.data().registry.state(id).unwrap().name.as_str();
        let carved = |id| {
            matches!(
                name(id),
                "minecraft:air" | "minecraft:cave_air" | "minecraft:water" | "minecraft:lava"
            )
        };
        // Below y 56 the hills are solid until a carver opens them; a column's index is
        // (y + 64) * 256 + z * 16 + x.
        let at = |x: usize, y: i32, z: usize| (y + 64) as usize * 256 + z * 16 + x;
        let crosses = ordered.iter().any(|[before, _]| {
            (0..9).filter(|i| i % 3 < 2).any(|i| {
                (-60..56).any(|y| {
                    (0..16).any(|z| {
                        carved(before[i].blocks[at(15, y, z)])
                            && carved(before[i + 1].blocks[at(0, y, z)])
                    })
                })
            })
        });
        assert!(crosses, "a cave crossing a chunk border");
        let spills = ordered
            .iter()
            .any(|[before, after]| (0..9).any(|i| i != 4 && before[i].blocks != after[i].blocks));
        assert!(spills, "a feature written into a neighbour chunk");
    }
}
