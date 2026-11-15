use super::*;
use horizon_world::worldgen::structure::legacy::Result as PieceResult;
use mod_api::Tag;
use mod_api::{FoundStructure, StructureError};

/// A type whose piece is a stone box of a size the config names, from the
/// chunk's corner: what the adapter saves, it loads back.
struct Boxes;
#[derive(Debug)]
struct BoxPiece {
    min: [i32; 3],
    max: [i32; 3],
}
impl StructurePiece for BoxPiece {
    fn bounding_box(&self) -> BoundingBox {
        BoundingBox::new(self.min, self.max)
    }
    fn post_process(&self, _: &mut dyn PieceContext) -> Result<(), StructureError> {
        Ok(())
    }
    fn add_additional_save_data(&self, tag: &mut Compound) {
        tag.insert("Min".into(), Tag::IntArray(self.min.to_vec()));
        tag.insert("Max".into(), Tag::IntArray(self.max.to_vec()));
    }
}
impl StructureType for Boxes {
    fn find_generation_point(&self, context: &mut dyn GenerationContext) -> Option<FoundStructure> {
        let chunk = context.chunk();
        let height = context.base_height(
            chunk.x * 16 + 8,
            chunk.z * 16 + 8,
            HeightmapType::WorldSurfaceWg,
        );
        let min = [chunk.x * 16, height - 1, chunk.z * 16];
        Some(FoundStructure {
            position: min,
            pieces: vec![Arc::new(BoxPiece {
                min,
                max: [min[0] + 8, min[1] + 5, min[2] + 8],
            })],
        })
    }
    fn load_piece(
        &self,
        _: &dyn BlockMetadata,
        tag: &Compound,
    ) -> Result<Arc<dyn StructurePiece>, StructureError> {
        let ints = |key: &str| match tag.get(&key.into()) {
            Some(Tag::IntArray(a)) if a.len() == 3 => Ok([a[0], a[1], a[2]]),
            _ => Err(StructureError(format!("invalid saved {key}"))),
        };
        Ok(Arc::new(BoxPiece {
            min: ints("Min")?,
            max: ints("Max")?,
        }))
    }
}

/// The same seed gives the same pieces: the type's answer depends on
/// nothing but its context, so two worlds of one seed see one structure.
#[test]
fn the_same_context_gives_the_same_pieces() {
    let adapter = Adapter::new("testmod:boxes", Arc::new(Boxes));
    let run = || {
        let mut random = WorldgenRandom::new(LegacyRandom::new(0));
        random.set_large_feature_seed(12345, 3, 7);
        // A context of the queries Boxes asks; the engine's own tests
        // cover the context itself.
        let (position, pieces) = adapter
            .find_generation_point(
                &mut StubContext,
                test_resources(),
                ChunkPos::new(3, 7),
                &Arc::new(Tag::Compound(Compound::new())),
                &mut random,
            )
            .unwrap();
        (position, pieces[0].bounding_box(), pieces.len())
    };
    assert_eq!(run(), run());
}

/// `createTag` + the piece's own data load back into the same piece: a
/// saved tower comes back the same tower, not a rebuilt one.
#[test]
fn a_saved_piece_loads_back_the_same() {
    let resources = test_resources();
    let adapter = Adapter::new("testmod:boxes", Arc::new(Boxes));
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    random.set_large_feature_seed(1, 0, 0);
    let (_, pieces) = adapter
        .find_generation_point(
            &mut StubContext,
            resources,
            ChunkPos::new(0, 0),
            &Arc::new(Tag::Compound(Compound::new())),
            &mut random,
        )
        .unwrap();
    let tag = pieces[0].save().unwrap();
    assert!(matches!(
        tag.get(&"id".into()),
        Some(Tag::String(s)) if s.to_string_lossy() == "testmod:boxes"
    ));
    assert_eq!(tag.get(&"Min".into()), Some(&Tag::IntArray(vec![0, 64, 0])));
    let loaded = adapter.load_piece(resources.registry, &tag).unwrap();
    assert_eq!(loaded.bounding_box(), pieces[0].bounding_box());
    assert_eq!(loaded.type_name(), "testmod:boxes");
    // And what came back is the saved one, not a fresh generation.
    let again = loaded.save().unwrap();
    assert_eq!(
        again.get(&"Min".into()).unwrap(),
        tag.get(&"Min".into()).unwrap()
    );
}

/// The engine's once-per-chunk `afterPlace`, which it runs over the first
/// piece, reaches the type's `after_place` — the piece has none — with
/// every piece's box.
#[test]
fn the_engines_after_place_call_reaches_the_type() {
    use horizon_world::worldgen::structure::legacy::{PieceContext as EngineContext, SolidRender};
    struct Counting {
        calls: std::sync::Arc<std::sync::Mutex<Vec<usize>>>,
    }
    impl StructureType for Counting {
        fn find_generation_point(
            &self,
            context: &mut dyn GenerationContext,
        ) -> Option<FoundStructure> {
            let chunk = context.chunk();
            let height = context.base_height(
                chunk.x * 16 + 8,
                chunk.z * 16 + 8,
                HeightmapType::WorldSurfaceWg,
            );
            let min = [chunk.x * 16, height - 1, chunk.z * 16];
            Some(FoundStructure {
                position: min,
                pieces: vec![Arc::new(BoxPiece {
                    min,
                    max: [min[0] + 8, min[1] + 5, min[2] + 8],
                })],
            })
        }
        fn load_piece(
            &self,
            _: &dyn BlockMetadata,
            _: &Compound,
        ) -> Result<Arc<dyn StructurePiece>, StructureError> {
            Err(StructureError("fixture has no saved pieces".into()))
        }
        fn after_place(
            &self,
            _: &mut dyn PieceContext,
            pieces: &[BoundingBox],
        ) -> Result<(), StructureError> {
            self.calls.lock().unwrap().push(pieces.len());
            Ok(())
        }
    }
    let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let adapter = Adapter::new(
        "testmod:counting",
        Arc::new(Counting {
            calls: calls.clone(),
        }),
    );
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    random.set_large_feature_seed(2, 1, 1);
    let (_, pieces) = adapter
        .find_generation_point(
            &mut StubContext,
            test_resources(),
            ChunkPos::new(1, 1),
            &Arc::new(Tag::Compound(Compound::new())),
            &mut random,
        )
        .unwrap();
    let boxes: Vec<BoundingBox> = pieces.iter().map(|p| p.bounding_box()).collect();
    let resources = test_resources();
    struct NoSolid;
    impl SolidRender for NoSolid {
        fn solid_render(&self, _: horizon_world::registry::BlockStateId) -> bool {
            false
        }
    }
    let air = resources.registry.default_state("minecraft:air").unwrap();
    let mut world = RecordingWorld {
        air,
        writes: Vec::new(),
        states: Vec::new(),
    };
    let mut piece_random = LegacyRandom::new(3);
    let mut engine = EngineContext {
        world: &mut world,
        resources,
        blocks: &NoSolid,
        random: &mut piece_random,
        chunk_box: BoundingBox::new([0, -64, 0], [15, 320, 15]),
        chunk: ChunkPos::new(0, 0),
        reference: [8, 64, 8],
    };
    // What the engine does once per chunk: the first piece's call.
    pieces[0].after_place(&mut engine, &boxes).unwrap();
    assert_eq!(*calls.lock().unwrap(), vec![boxes.len()]);
}

/// The start context the stub answers: heights from a flat 64.
struct StubContext;
impl horizon_world::worldgen::structure::jigsaw::JigsawContext for StubContext {
    fn min_y(&self) -> i32 {
        -64
    }
    fn max_y(&self) -> i32 {
        320
    }
    fn base_height(&mut self, _: i32, _: i32, _: horizon_world::heightmap::HeightmapType) -> i32 {
        65
    }
    fn column_has_valid_biome(&mut self, _: i32, _: i32, _: i32, _: i32) -> bool {
        true
    }
}
impl horizon_world::worldgen::structure::start::StartContext for StubContext {
    fn seed(&self) -> i64 {
        1
    }
    fn sea_level(&self) -> i32 {
        63
    }
    fn base_column(
        &mut self,
        _: i32,
        _: i32,
        _: horizon_world::heightmap::HeightmapType,
    ) -> Vec<bool> {
        Vec::new()
    }
    fn base_columns_by(
        &mut self,
        _: i32,
        _: i32,
        count: usize,
        kinds: &dyn Fn(usize) -> horizon_world::heightmap::HeightmapType,
    ) -> Vec<Vec<bool>> {
        use horizon_world::heightmap::HeightmapType::*;
        let all = [
            WorldSurfaceWg,
            WorldSurface,
            OceanFloorWg,
            OceanFloor,
            MotionBlocking,
            MotionBlockingNoLeaves,
        ];
        (0..count)
            .map(|index| {
                let selected = kinds(index);
                all.iter().map(|kind| *kind == selected).collect()
            })
            .collect()
    }
    fn noise_column_into(
        &mut self,
        _: i32,
        _: i32,
        column: &mut dyn horizon_world::worldgen::structure::start::NoiseColumnTarget,
    ) {
        use horizon_world::worldgen::structure::start::NoiseBlock::*;
        column.resize(-2, 5);
        for (index, block) in [Air, Water, Lava, Solid, Other].into_iter().enumerate() {
            column.set(index, block);
        }
    }
    fn cold_enough_to_snow(
        &mut self,
        _: horizon_world::worldgen::biome::BiomeId,
        _: [i32; 3],
    ) -> bool {
        false
    }
    fn generation(&self) -> horizon_world::worldgen::anchor::WorldGenerationContext {
        horizon_world::worldgen::anchor::WorldGenerationContext::new(-64, 384, 63)
    }
    fn noise_biome(&mut self, _: [i32; 3]) -> horizon_world::worldgen::biome::BiomeId {
        horizon_world::worldgen::biome::BiomeId(0)
    }
    fn noise_biomes(
        &mut self,
        quarts: &[[i32; 3]],
    ) -> Vec<horizon_world::worldgen::biome::BiomeId> {
        vec![horizon_world::worldgen::biome::BiomeId(0); quarts.len()]
    }
    fn set_valid_biomes(
        &mut self,
        _: Option<&std::collections::BTreeSet<horizon_world::worldgen::biome::BiomeId>>,
    ) {
    }
}

/// The piece world of the clipping test: every write recorded, and a
/// write outside the test's expectations fails it.
struct RecordingWorld {
    air: horizon_world::registry::BlockStateId,
    writes: Vec<[i32; 3]>,
    states: Vec<horizon_world::registry::BlockStateId>,
}
impl horizon_world::worldgen::structure::legacy::PieceWorld for RecordingWorld {
    fn block(&mut self, _: [i32; 3]) -> PieceResult<horizon_world::registry::BlockStateId> {
        Ok(self.air)
    }
    fn set(
        &mut self,
        pos: [i32; 3],
        state: horizon_world::registry::BlockStateId,
        _: i32,
    ) -> PieceResult<bool> {
        self.writes.push(pos);
        self.states.push(state);
        Ok(true)
    }
    fn fluid(&mut self, _: [i32; 3]) -> PieceResult<horizon_world::fluid::FluidState> {
        Ok(horizon_world::fluid::FluidState::EMPTY)
    }
    fn height(
        &mut self,
        _: horizon_world::heightmap::HeightmapType,
        _: i32,
        _: i32,
    ) -> PieceResult<i32> {
        Ok(64)
    }
    fn min_y(&mut self) -> PieceResult<i32> {
        Ok(-64)
    }
    fn max_y(&mut self) -> PieceResult<i32> {
        Ok(320)
    }
    fn block_entity(&mut self, _: [i32; 3], _: Compound) -> PieceResult<()> {
        Ok(())
    }
    fn entity(&mut self, _: Compound) -> PieceResult<()> {
        Ok(())
    }
    fn post_process(&mut self, _: [i32; 3]) -> PieceResult<()> {
        Ok(())
    }
    fn fluid_tick(
        &mut self,
        _: [i32; 3],
        _: horizon_world::fluid::FluidKind,
        _: i32,
    ) -> PieceResult<()> {
        Ok(())
    }
    fn seed(&mut self) -> PieceResult<i64> {
        Ok(0)
    }
    fn level_boolean(&mut self) -> PieceResult<bool> {
        Ok(false)
    }
    fn level_float(&mut self) -> PieceResult<f32> {
        Ok(0.0)
    }
    fn level_int(&mut self, _: i32) -> PieceResult<i32> {
        Ok(0)
    }
    fn sea_level(&mut self) -> PieceResult<i32> {
        Ok(63)
    }
    fn face_full(
        &mut self,
        _: [i32; 3],
        _: horizon_core::geometry::Direction,
    ) -> PieceResult<bool> {
        Ok(false)
    }
    fn face_sturdy(
        &mut self,
        _: [i32; 3],
        _: horizon_core::geometry::Direction,
    ) -> PieceResult<bool> {
        Ok(false)
    }
    fn center_sturdy(
        &mut self,
        _: [i32; 3],
        _: horizon_core::geometry::Direction,
    ) -> PieceResult<bool> {
        Ok(false)
    }
    fn biome(&mut self, _: [i32; 3]) -> PieceResult<horizon_world::worldgen::biome::BiomeId> {
        Err(PlacementQueryError::Unavailable)
    }
    fn place_template(
        &mut self,
        _: &horizon_world::worldgen::structure::StructureResources,
        _: &horizon_world::worldgen::structure::StructureTemplate,
        _: [i32; 3],
        _: [i32; 3],
        _: &mut horizon_world::worldgen::structure::processor::PlaceSettings<'_>,
        _: &mut dyn horizon_world::worldgen::structure::processor::ProcessorRandom,
        _: i32,
    ) -> PieceResult<bool> {
        Ok(true)
    }
}

/// The engine's own placement tests build their registry this way
/// (`legacy::tests::registry`): two facts rows, held once for the
/// `&'static` the engine's resources take — no game resources needed.
fn test_resources() -> &'static horizon_world::worldgen::structure::StructureResources {
    mod_platform::__private::part_tests::install_parts_fixture(&[]).unwrap();
    let facts = serde_json::json!({
        "states": [
            ["air", true, false, "", {"id":"minecraft:air","properties":{}}],
            ["stone", false, false, "", {"id":"minecraft:stone","properties":{}}]
        ],
        "defaults":{"minecraft:air":0,"minecraft:stone":1}
    });
    let registry =
        horizon_world::registry::BlockRegistry::from_facts_json(facts.to_string().as_bytes())
            .unwrap();
    crate::game::api_values::resolve_block_definitions(&registry).unwrap();
    resources_for_registry(registry)
}

fn resources_for_registry(
    registry: horizon_world::registry::BlockRegistry,
) -> &'static horizon_world::worldgen::structure::StructureResources {
    Box::leak(Box::new(
        horizon_world::worldgen::structure::StructureResources::new(
            Box::leak(Box::new(registry)),
            horizon_world::worldgen::structure::StateTransforms::default(),
            std::path::Path::new("/nonexistent"),
            std::collections::BTreeMap::new(),
            &std::collections::BTreeMap::new(),
        ),
    ))
}

/// `placeBlock`'s clipping: a write outside `chunk_box` answers `false`
/// and touches nothing, silently, as vanilla's `placeBlock` skips it; one
/// inside writes.
#[test]
fn a_write_outside_the_chunk_box_is_dropped_silently() {
    use horizon_world::worldgen::structure::legacy::{PieceContext as EngineContext, SolidRender};
    let resources = test_resources();
    struct Air;
    impl SolidRender for Air {
        fn solid_render(&self, _: horizon_world::registry::BlockStateId) -> bool {
            false
        }
    }
    let air = resources.registry.default_state("minecraft:air").unwrap();
    let mut world = RecordingWorld {
        air,
        writes: Vec::new(),
        states: Vec::new(),
    };
    let mut random = horizon_core::random::LegacyRandom::new(7);
    {
        let mut engine = EngineContext {
            world: &mut world,
            resources,
            blocks: &Air,
            random: &mut random,
            // Chunk (0, 0)'s writable box.
            chunk_box: BoundingBox::new([0, -64, 0], [15, 320, 15]),
            chunk: ChunkPos::new(0, 0),
            reference: [8, 64, 8],
        };
        let mut context = Placing::new(&mut engine).unwrap();
        let stone = context.state(mod_api::__private::block_key("minecraft:stone").unwrap());
        // Outside on every axis: dropped, silently, nothing recorded.
        for outside in [[16, 64, 0], [0, 64, 16], [-1, 64, 0], [0, 64, -1]] {
            assert!(!context.set_block(outside, stone));
        }
        // Inside: written.
        assert!(context.set_block([15, 64, 15], stone));
    }
    assert_eq!(world.writes, [[15, 64, 15]]);
}

#[test]
fn structure_random_adapter_preserves_vanilla_scalar_draws() {
    assert_eq!(Draws(&mut LegacyRandom::new(0)).next_int_bound(100), 60);
    assert_eq!(
        Draws(&mut LegacyRandom::new(0)).next_long(),
        -4962768465676381896
    );
    assert_eq!(
        Draws(&mut LegacyRandom::new(0)).next_float().to_bits(),
        0.73096776_f32.to_bits()
    );
    assert_eq!(
        Draws(&mut LegacyRandom::new(0)).next_double().to_bits(),
        0.730967787376657_f64.to_bits()
    );
    assert!(Draws(&mut LegacyRandom::new(0)).next_boolean());
    let mut random = LegacyRandom::new(0);
    let mut draws = Draws(&mut random);
    assert_eq!(draws.next_int_bound(0), 0);
    assert_eq!(draws.next_long(), -4962768465676381896);
}

#[test]
fn structure_column_projection_keeps_classes_and_resets_reused_output() {
    use horizon_world::worldgen::structure::start::{NoiseBlock as Engine, NoiseColumnTarget};
    let mut column = Column(NoiseColumn::default());
    column.resize(-2, 5);
    for (index, block) in [
        Engine::Air,
        Engine::Water,
        Engine::Lava,
        Engine::Solid,
        Engine::Other,
    ]
    .into_iter()
    .enumerate()
    {
        column.set(index, block);
    }
    assert_eq!(
        column.0.blocks,
        [
            NoiseBlock::Air,
            NoiseBlock::Water,
            NoiseBlock::Lava,
            NoiseBlock::Solid,
            NoiseBlock::Other
        ]
    );
    column.resize(7, 2);
    assert_eq!(column.0.min, 7);
    assert_eq!(column.0.blocks, [NoiseBlock::Air, NoiseBlock::Air]);
}

struct ConfigObserver(std::sync::Arc<std::sync::Mutex<Vec<Arc<Tag>>>>);
impl StructureType for ConfigObserver {
    fn find_generation_point(&self, context: &mut dyn GenerationContext) -> Option<FoundStructure> {
        self.0.lock().unwrap().push(context.config());
        None
    }
    fn load_piece(
        &self,
        _: &dyn BlockMetadata,
        _: &Compound,
    ) -> Result<Arc<dyn StructurePiece>, StructureError> {
        Err(StructureError("fixture has no saved pieces".into()))
    }
}

#[test]
fn structure_config_is_converted_at_load_and_shared_by_starts() {
    use horizon_world::worldgen::structure::{
        mod_types::ModStructureTypes,
        start::{StructureKind, StructureRegistry},
    };
    use std::collections::{BTreeMap, BTreeSet};
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let types: ModStructureTypes = BTreeMap::from([(
        "probe:kind".to_owned(),
        Adapter::new("probe:kind", Arc::new(ConfigObserver(observed.clone()))).into_engine_type(),
    )]);
    let mut definitions = BTreeMap::from([(
        "probe:tower".to_owned(),
        serde_json::json!({
            "type":"probe:kind", "biomes":[], "size":32768, "wide":2147483648_i64,
            "list":[true,128], "nested":{"enabled":true}
        })
        .to_string(),
    )]);
    let mut registry = StructureRegistry::new(
        &definitions,
        &BTreeMap::new(),
        BTreeMap::new(),
        &|_| Ok(BTreeSet::new()),
        &types,
    )
    .unwrap();
    let StructureKind::Mod(kind) = &mut registry.structures[0].kind else {
        panic!("registered kind");
    };
    let Tag::Compound(config) = kind.config.as_ref() else {
        panic!("compound config");
    };
    assert_eq!(config.get(&"size".into()), Some(&Tag::Int(32768)));
    assert_eq!(config.get(&"wide".into()), Some(&Tag::Long(2147483648)));
    assert_eq!(
        config.get(&"list".into()),
        Some(&Tag::List(vec![Tag::Byte(1), Tag::Short(128)]))
    );
    assert_eq!(
        config.get(&"nested".into()),
        Some(&Tag::compound([("enabled", Tag::Byte(1))]))
    );
    // Discard the source text: starts must use the retained prepared value, not a config map.
    definitions.clear();
    let resources = test_resources();
    let mut random = WorldgenRandom::new(LegacyRandom::new(0));
    for _ in 0..2 {
        assert!(
            kind.ty
                .find_generation_point(
                    &mut StubContext,
                    resources,
                    ChunkPos::new(0, 0),
                    &kind.config,
                    &mut random
                )
                .is_none()
        );
    }
    let observed = observed.lock().unwrap();
    assert_eq!(observed.len(), 2);
    assert!(
        observed
            .iter()
            .all(|config| Arc::ptr_eq(config, &kind.config))
    );
}

#[test]
#[allow(clippy::err_expect)] // TODO(arismoko/horizon#2273): `expect_err` once the Ok type is Debug
fn structure_bad_config_refuses_at_load_but_unknown_types_do_not_convert() {
    use horizon_world::worldgen::structure::{
        mod_types::ModStructureTypes,
        start::{StructureKind, StructureRegistry},
    };
    use std::collections::{BTreeMap, BTreeSet};
    let types: ModStructureTypes = BTreeMap::from([(
        "probe:kind".to_owned(),
        Adapter::new("probe:kind", Arc::new(ConfigObserver(Default::default()))).into_engine_type(),
    )]);
    let definitions = BTreeMap::from([(
        "probe:tower".to_owned(),
        serde_json::json!({"type":"probe:kind","biomes":[],"bad":[null]}).to_string(),
    )]);
    let error = StructureRegistry::new(
        &definitions,
        &BTreeMap::new(),
        BTreeMap::new(),
        &|_| Ok(BTreeSet::new()),
        &types,
    )
    .err()
    .expect("refused during definition load");
    assert_eq!(
        error.to_string(),
        "structure probe:tower: probe:kind: JSON-to-NBT does not accept nested null"
    );
    let registry = StructureRegistry::new(
        &definitions,
        &BTreeMap::new(),
        BTreeMap::new(),
        &|_| Ok(BTreeSet::new()),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(
        matches!(&registry.structures[0].kind, StructureKind::Unsupported(id) if id == "probe:kind")
    );
}

#[test]
fn compiled_book_piece_resolves_on_worker_load_and_refuses_bad_saved_values() {
    use horizon_world::worldgen::structure::legacy::{PieceContext as EngineContext, SolidRender};
    mod_platform::__private::part_tests::install_parts_fixture(&[mod_api::ModEntry::new(
        "mymod",
        Box::new(book_example::structures::StructureMod),
    )])
    .unwrap();
    let registry = horizon_world::registry::BlockRegistry::from_facts_json(br#"{"defaults":{"minecraft:air":0,"minecraft:oak_log":1},"states":[["a",true,false,"","minecraft:air"],["s",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"y"}}],["s",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"x"}}],["s",false,false,"",{"id":"minecraft:oak_log","properties":{"axis":"z"}}]]}"#.as_slice()).unwrap();
    crate::game::api_values::resolve_block_definitions(&registry).unwrap();
    let resources = resources_for_registry(registry);
    let (_, implementation) = mod_api::__private::structure_type_declarations()
        .into_iter()
        .next()
        .unwrap();
    std::thread::spawn(move || {
        assert!(crate::game::api_values::block_state::current().is_err());
        let metadata = WorkerResources::get(resources.registry).unwrap().metadata;
        let adapter = Adapter::new("mymod:tower", implementation.clone());
        let tag = Compound::from([
            ("block".into(), Tag::String("minecraft:oak_log".into())),
            ("axis".into(), Tag::String("x".into())),
            ("min".into(), Tag::IntArray(vec![1, 64, 2])),
            ("max".into(), Tag::IntArray(vec![1, 66, 2])),
        ]);
        for (key, value) in [
            ("block", Tag::String("minecraft:missing".into())),
            ("axis", Tag::String("diagonal".into())),
            ("min", Tag::IntArray(vec![1, 64])),
        ] {
            let mut invalid = tag.clone();
            invalid.insert(key.into(), value);
            assert!(implementation.load_piece(metadata, &invalid).is_err());
            assert!(adapter.load_piece(resources.registry, &invalid).is_none());
        }
        let piece = adapter.load_piece(resources.registry, &tag).unwrap();
        assert_eq!(
            piece.bounding_box(),
            BoundingBox::new([1, 64, 2], [1, 66, 2])
        );
        let saved = piece.save().unwrap();
        for key in ["block", "axis", "min", "max"] {
            assert_eq!(saved.get(&key.into()), tag.get(&key.into()));
        }
        struct Opaque;
        impl SolidRender for Opaque {
            fn solid_render(&self, _: horizon_world::registry::BlockStateId) -> bool {
                true
            }
        }
        let mut world = RecordingWorld {
            air: horizon_world::registry::BlockStateId(0),
            writes: vec![],
            states: vec![],
        };
        let mut random = LegacyRandom::new(9);
        piece
            .post_process(&mut EngineContext {
                world: &mut world,
                resources,
                blocks: &Opaque,
                random: &mut random,
                chunk_box: BoundingBox::new([0, -64, 0], [15, 320, 15]),
                chunk: ChunkPos::new(0, 0),
                reference: [8, 64, 8],
            })
            .unwrap();
        assert_eq!(world.writes, [[1, 64, 2], [1, 65, 2], [1, 66, 2]]);
        assert_eq!(world.states, [horizon_world::registry::BlockStateId(2); 3]);
    })
    .join()
    .unwrap();
}

/// Exercise the actual terrain consumer with generated and saved mod pieces.
#[test]
fn piece_beardifier_shapes_terrain_and_is_resolved_once() {
    use horizon_core::density::{DensitySampler, SamplerContext, TerrainAdjustment as Engine};
    use horizon_world::worldgen::structure::start::{
        Piece, StructureRegistry, StructureStart, beardifier,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TerrainType(Arc<AtomicUsize>);
    #[derive(Debug)]
    struct TerrainPiece {
        mode: i32,
        calls: Arc<AtomicUsize>,
    }
    impl StructurePiece for TerrainPiece {
        fn bounding_box(&self) -> BoundingBox {
            let x = self.mode * 40;
            BoundingBox::new([x, 64, 0], [x + 8, 73, 8])
        }
        fn beardifier(&self) -> Option<mod_api::PieceBeardifier> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            match self.mode {
                0 => Some(mod_api::PieceBeardifier::new(
                    BoundingBox::new([0, 67, 0], [8, 73, 8]),
                    mod_api::TerrainAdjustment::BeardThin,
                    2,
                )),
                1 => None,
                _ => Some(mod_api::PieceBeardifier::new(
                    self.bounding_box(),
                    mod_api::TerrainAdjustment::None,
                    0,
                )),
            }
        }
        fn post_process(&self, _: &mut dyn PieceContext) -> Result<(), StructureError> {
            Ok(())
        }
        fn add_additional_save_data(&self, tag: &mut Compound) {
            tag.insert("Mode".into(), Tag::Int(self.mode));
        }
    }
    impl StructureType for TerrainType {
        fn find_generation_point(&self, _: &mut dyn GenerationContext) -> Option<FoundStructure> {
            Some(FoundStructure {
                position: [0, 64, 0],
                pieces: (0..3)
                    .map(|mode| {
                        Arc::new(TerrainPiece {
                            mode,
                            calls: self.0.clone(),
                        }) as Arc<dyn StructurePiece>
                    })
                    .collect(),
            })
        }
        fn load_piece(
            &self,
            _: &dyn BlockMetadata,
            tag: &Compound,
        ) -> Result<Arc<dyn StructurePiece>, StructureError> {
            let Some(Tag::Int(mode)) = tag.get(&"Mode".into()) else {
                return Err(StructureError("missing fixture mode".into()));
            };
            Ok(Arc::new(TerrainPiece {
                mode: *mode,
                calls: self.0.clone(),
            }))
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let adapter = Adapter::new("probe:terrain", Arc::new(TerrainType(calls.clone())));
    let resources = test_resources();
    let (_, generated) = adapter
        .find_generation_point(
            &mut StubContext,
            resources,
            ChunkPos::new(0, 0),
            &Arc::new(Tag::Compound(Compound::new())),
            &mut WorldgenRandom::new(LegacyRandom::new(0)),
        )
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    let loaded: Vec<_> = generated
        .iter()
        .map(|piece| {
            adapter
                .load_piece(resources.registry, &piece.save().unwrap())
                .unwrap()
        })
        .collect();
    assert_eq!(calls.load(Ordering::SeqCst), 6);
    let mut registry = StructureRegistry::new(
        &BTreeMap::from([(
            "probe:terrain".into(),
            serde_json::json!({"type":"probe:terrain", "biomes":[],
                "terrain_adaptation":"none"})
            .to_string(),
        )]),
        &BTreeMap::new(),
        BTreeMap::new(),
        &|_| Ok(BTreeSet::new()),
        &BTreeMap::new(),
    )
    .unwrap();
    for pieces in [generated, loaded] {
        let start = Arc::new(StructureStart::new(
            0,
            &registry.structures[0],
            ChunkPos::new(0, 0),
            pieces.into_iter().map(Piece::Legacy).collect(),
        ));
        assert!(start.bounding_box.is_inside([-12, 69, 0]));
        for default in [Engine::None, Engine::Bury] {
            registry.structures[0].terrain = default;
            for _ in 0..2 {
                let density =
                    beardifier(&registry, std::slice::from_ref(&start), ChunkPos::new(0, 0))
                        .unwrap();
                let sample = |position| {
                    density
                        .sample_value(&mut SamplerContext::default(), position)
                        .unwrap()
                };
                // Vanilla Beardifier: dy = y - (67 + 2), kernel exp(-r²/16),
                // -dyOffset / sqrt(r²/2) / 2 * kernel * .8. Allow the
                // vanilla fast inverse-square-root approximation, not a second
                // call to Horizon's density helpers.
                for y in [64, 68, 69, 73] {
                    let offset = f64::from(y - 69) + 0.5;
                    let expected = -offset / (offset * offset / 2.0).sqrt() / 2.0
                        * (-offset * offset / 16.0).exp()
                        * 0.8;
                    assert!((f64::from(sample([4, y, 4])) - expected).abs() < 0.002);
                }
                assert_eq!(sample([4, 81, 4]), 0.0);
                let second =
                    beardifier(&registry, std::slice::from_ref(&start), ChunkPos::new(2, 0));
                let second = second.map_or(0.0, |d| {
                    d.sample_value(&mut SamplerContext::default(), [44, 64, 4])
                        .unwrap()
                });
                assert_eq!(second, if default == Engine::None { 0.0 } else { 1.0 });
                assert!(
                    beardifier(&registry, std::slice::from_ref(&start), ChunkPos::new(5, 0))
                        .is_none()
                );
            }
        }
        registry.structures[0].terrain = Engine::None;
    }
    assert_eq!(
        calls.load(Ordering::SeqCst),
        6,
        "terrain chunks must use cached answers"
    );
}

#[test]
fn unordered_beardifier_bounds_add_no_terrain_adjustment() {
    let unordered = mod_api::PieceBeardifier::new(
        BoundingBox::new([1, 0, 0], [0, 0, 0]),
        mod_api::TerrainAdjustment::BeardThin,
        0,
    );
    assert!(engine_beardifier(unordered).is_none());
    let ordered = mod_api::PieceBeardifier::new(
        BoundingBox::new([0, 0, 0], [1, 1, 1]),
        mod_api::TerrainAdjustment::BeardThin,
        2,
    );
    assert_eq!(engine_beardifier(ordered).unwrap().ground_level_delta, 2);
}
