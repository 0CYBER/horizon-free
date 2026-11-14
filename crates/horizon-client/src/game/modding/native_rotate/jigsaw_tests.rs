//! Real assembler and template-placement regressions for position-sensitive rotate Parts.
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use horizon_core::{
    bounding_box::BoundingBox,
    nbt::{self, Compound, Limits, NamedTag, Tag},
    position::ChunkPos,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{
    compression::Compression,
    heightmap::HeightmapType,
    native_load_context::{DataLoadOrigin, LoadContext, LoadInputs},
    registry::BlockRegistry,
    worldgen::{
        anchor::WorldGenerationContext,
        biome::BiomeId,
        block_feature::FeatureBlockBindings,
        feature::FeatureWorld,
        placement::{PlacementContext, PlacementQueryError},
        structure::{
            StateTransforms, StructureResources,
            jigsaw::{self, JigsawConfig, JigsawContext, PoolPiece, Pools},
            piece_limited,
            start::{NoiseColumnTarget, StartContext},
        },
    },
};
use mod_api::{BlockPart, Hooks, ModEntry, Mods, RotateEvent, WorldgenHost};
use serde_json::json;
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, PoisonError},
};

/// The child faces down only at its actual placed height, never at its sampled local Y=2.
struct AtHeight {
    position_sensitive: bool,
}
#[mod_api::horizon_part]
impl BlockPart for AtHeight {
    fn rotate(&self, context: &dyn WorldgenHost, event: &mut RotateEvent<'_>) {
        if context.state_with_property(event.state, "orientation", "down_south")
            == Some(event.state)
        {
            *event.rotated = context
                .state_with_property(
                    event.state,
                    "orientation",
                    if !self.position_sensitive || event.pos[1] == 80 {
                        "down_south"
                    } else {
                        "up_south"
                    },
                )
                .unwrap();
        } else if self.position_sensitive && matches!(event.pos[1], 9 | 89) {
            *event.rotated = context
                .state_with_property(event.state, "orientation", "down_south")
                .unwrap();
        }
    }
}
struct Turns(bool);
#[mod_api::__subscriptions]
impl Hooks for Turns {
    fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        let jigsaw = r.block_key("minecraft:jigsaw");
        r.block_part(
            jigsaw,
            AtHeight {
                position_sensitive: self.0,
            },
        );
    }
}

/// A complete flat-world column: first free Y=80, stone below, air above.
struct Flat;
impl JigsawContext for Flat {
    fn min_y(&self) -> i32 {
        -64
    }
    fn max_y(&self) -> i32 {
        319
    }
    fn base_height(&mut self, _: i32, _: i32, _: HeightmapType) -> i32 {
        80
    }
    fn column_has_valid_biome(&mut self, _: i32, _: i32, _: i32, _: i32) -> bool {
        true
    }
}
impl StartContext for Flat {
    fn seed(&self) -> i64 {
        0
    }
    fn sea_level(&self) -> i32 {
        63
    }
    fn base_column(&mut self, _: i32, _: i32, _: HeightmapType) -> Vec<bool> {
        (-64..320).map(|y| y < 80).collect()
    }
    fn base_columns_by(
        &mut self,
        x: i32,
        z: i32,
        count: usize,
        kinds: &dyn Fn(usize) -> HeightmapType,
    ) -> Vec<Vec<bool>> {
        (0..count)
            .map(|i| self.base_column(x, z, kinds(i)))
            .collect()
    }
    fn noise_column_into(&mut self, _: i32, _: i32, column: &mut dyn NoiseColumnTarget) {
        use horizon_world::worldgen::structure::start::NoiseBlock;
        column.resize(-64, 384);
        for i in 0..384 {
            column.set(
                i,
                if i < 144 {
                    NoiseBlock::Solid
                } else {
                    NoiseBlock::Air
                },
            );
        }
    }
    fn cold_enough_to_snow(&mut self, _: BiomeId, _: [i32; 3]) -> bool {
        false
    }
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(-64, 384, 63)
    }
    fn noise_biome(&mut self, _: [i32; 3]) -> BiomeId {
        BiomeId(0)
    }
    fn noise_biomes(&mut self, positions: &[[i32; 3]]) -> Vec<BiomeId> {
        vec![BiomeId(0); positions.len()]
    }
    fn set_valid_biomes(&mut self, _: Option<&std::collections::BTreeSet<BiomeId>>) {}
}

fn ints(values: &[i32]) -> Tag {
    Tag::List(values.iter().copied().map(Tag::Int).collect())
}
fn template_tag(child: bool) -> Tag {
    Tag::compound([
        ("size", ints(if child { &[1, 3, 1] } else { &[1, 1, 1] })),
        (
            "palette",
            Tag::List(vec![Tag::compound([
                ("id", Tag::String("minecraft:jigsaw".into())),
                (
                    "properties",
                    Tag::compound([(
                        "orientation",
                        Tag::String(if child { "down_south" } else { "up_south" }.into()),
                    )]),
                ),
            ])]),
        ),
        (
            "blocks",
            Tag::List(vec![Tag::compound([
                ("pos", ints(if child { &[0, 2, 0] } else { &[0, 0, 0] })),
                ("state", Tag::Int(0)),
                (
                    "nbt",
                    Tag::compound([
                        (
                            "name",
                            Tag::String(if child { "test:child" } else { "test:start" }.into()),
                        ),
                        (
                            "target",
                            Tag::String(
                                if child {
                                    "minecraft:empty"
                                } else {
                                    "test:child"
                                }
                                .into(),
                            ),
                        ),
                        (
                            "pool",
                            Tag::String(
                                if child {
                                    "minecraft:empty"
                                } else {
                                    "test:children"
                                }
                                .into(),
                            ),
                        ),
                        ("joint", Tag::String("rollable".into())),
                        ("final_state", Tag::String("minecraft:air".into())),
                    ]),
                ),
            ])]),
        ),
        ("entities", Tag::List(Vec::new())),
    ])
}
fn compressed(tag: Tag) -> Arc<[u8]> {
    let raw = nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag,
        },
        Limits::templates(),
    )
    .unwrap();
    Compression::Gzip.encode(&raw).unwrap().into()
}
fn resources(registry: &'static BlockRegistry, root: &Path) -> StructureResources {
    let transforms = StateTransforms::from_json(
        &std::fs::read_to_string(root.join("worldgen/blocks/structure-blocks.json")).unwrap(),
        registry.len(),
    )
    .unwrap();
    StructureResources::new(
        registry,
        transforms,
        root,
        BTreeMap::new(),
        &BTreeMap::new(),
    )
    .with_pack_templates(
        [
            ("test:start".into(), compressed(template_tag(false))),
            ("test:child".into(), compressed(template_tag(true))),
        ]
        .into(),
    )
}
fn pools() -> Pools {
    let pool = |location, projection| {
        json!({
            "fallback": "minecraft:empty", "elements": [{ "weight": 1, "element": {
                "element_type": "minecraft:single_pool_element", "location": location,
                "processors": [], "projection": projection,
            }}],
        })
        .to_string()
    };
    Pools::new(
        [
            ("test:start".into(), pool("test:start", "terrain_matching")),
            ("test:children".into(), pool("test:child", "rigid")),
            (
                "minecraft:empty".into(),
                json!({"fallback": "minecraft:empty", "elements": []}).to_string(),
            ),
        ]
        .into(),
    )
}
fn config(aliases: &BTreeMap<String, String>, max_depth: i32) -> JigsawConfig<'_> {
    JigsawConfig {
        start_pool: "test:start",
        start_jigsaw: Some("test:start"),
        max_depth,
        use_expansion_hack: false,
        project_to: None,
        max_distance: [96, 96],
        aliases,
        padding: [0, 0],
        waterlogging: false,
    }
}

/// An in-memory world, not a callback oracle: the real processors and placement write its blocks.
struct Blocks {
    air: EngineState,
    blocks: BTreeMap<[i32; 3], EngineState>,
    entities: BTreeMap<[i32; 3], Compound>,
}
impl FeatureWorld for Blocks {
    type Error = PlacementQueryError;
    fn block(&mut self, pos: [i32; 3]) -> Result<EngineState, Self::Error> {
        Ok(self.blocks.get(&pos).copied().unwrap_or(self.air))
    }
    fn set_block(
        &mut self,
        pos: [i32; 3],
        state: EngineState,
        _: i32,
    ) -> Result<bool, Self::Error> {
        self.blocks.insert(pos, state);
        Ok(true)
    }
}
impl PlacementContext for Blocks {
    fn generation(&self) -> WorldGenerationContext {
        WorldGenerationContext::new(-64, 384, 63)
    }
    fn min_y(&mut self) -> Result<i32, PlacementQueryError> {
        Ok(-64)
    }
    fn height(&mut self, _: HeightmapType, _: i32, _: i32) -> Result<i32, PlacementQueryError> {
        Ok(80)
    }
    fn block_state(&mut self, pos: [i32; 3]) -> Result<EngineState, PlacementQueryError> {
        self.block(pos)
    }
    fn set_block_entity(
        &mut self,
        pos: [i32; 3],
        nbt: Compound,
    ) -> Result<(), PlacementQueryError> {
        self.entities.insert(pos, nbt);
        Ok(())
    }
}
fn assert_built(pieces: &[PoolPiece], resources: &StructureResources) {
    assert_eq!(
        pieces.len(),
        2,
        "the local-origin orientation must not reject the child"
    );
    assert_eq!(pieces[0].position, [10, 9, 20]);
    assert_eq!(pieces[1].position, [10, 78, 20]);
    let mut world = Blocks {
        air: resources.registry.default_state("minecraft:air").unwrap(),
        blocks: BTreeMap::new(),
        entities: BTreeMap::new(),
    };
    let bounds = BoundingBox::new([-64, -64, -64], [128, 319, 128]);
    for piece in pieces {
        piece
            .place(
                &mut world,
                resources,
                &mut LegacyRandom::new(0),
                bounds,
                [10, 10, 20],
                true,
                &mut |_, _, _, _| panic!("the fixture contains no feature elements"),
            )
            .unwrap();
    }
    // These literal expected heights come from the flat surface and vanilla placement
    // equations, independently of the callback's answer and the assembler's output.
    for (pos, orientation) in [([10, 79, 20], "up"), ([10, 80, 20], "down_south")] {
        let state = resources.registry.state(world.blocks[&pos]).unwrap();
        let actual = state.properties["orientation"].as_str();
        if pos[1] == 79 {
            assert!(actual.starts_with("up_"), "{actual}");
        } else {
            assert_eq!(actual, orientation);
        }
    }
}

fn assert_connector_reads(
    pieces: &[PoolPiece],
    resources: &StructureResources,
    child_top: horizon_core::geometry::Direction,
) {
    use horizon_core::geometry::Direction::{Down, North, Up};
    let read = |piece: &PoolPiece, position| {
        let connectors = piece.element.shuffled_jigsaws(
            resources,
            position,
            piece.rotation,
            &mut LegacyRandom::new(0),
        );
        assert_eq!(connectors.len(), 1);
        (connectors[0].pos, connectors[0].front, connectors[0].top)
    };
    assert_eq!(read(&pieces[0], [10, 10, 20]), ([10, 10, 20], Up, North));
    assert_eq!(read(&pieces[0], [10, 9, 20]), ([10, 9, 20], Up, North));
    assert_eq!(read(&pieces[1], [0; 3]), ([0, 2, 0], Down, child_top));
    assert_eq!(
        read(&pieces[1], [10, 78, 20]),
        ([10, 80, 20], Down, child_top)
    );
}

fn publish(game: &mut crate::game::Game<'_>) {
    game.options.dedicated = Some(Vec::new());
    game.resolve_parts().unwrap();
    let load = LoadContext::new(
        DataLoadOrigin::ServerDataLoad,
        Arc::new(LoadInputs::new(
            BTreeMap::new(),
            game.options.resources.clone(),
        )),
        [],
    );
    horizon_world::native_worldgen_hooks::publish_context(Arc::new(load.worldgen_context([])));
    let _ = horizon_world::native_worldgen_hooks::install(
        horizon_world::native_worldgen_hooks::NativeWorldgenHooks {
            alter_ground: crate::game::api_values::alter_ground,
            on_tree_grow: crate::game::native_tree_grow::on_tree_grow,
            rotate: super::rotate,
            feature: native_worldgen_parts::feature,
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn position_sensitive_part_connects_at_built_height_in_both_assemblers() {
    let _publishing = native_worldgen_parts::PUBLISHING
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mods: Mods = vec![ModEntry::new("placed_connector", Box::new(Turns(true)))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            publish(game);
            let resources = resources(game.pack.assets.registry, &game.options.resources);
            let bindings =
                FeatureBlockBindings::new(resources.registry, Default::default()).unwrap();
            let aliases = BTreeMap::new();
            let config = config(&aliases, 1);
            let pools = pools();
            let (_, normal) = jigsaw::assemble(
                &config,
                &mut Flat,
                &pools,
                &resources,
                &bindings,
                &mut LegacyRandom::new(0),
                [10, 10, 20],
            )
            .unwrap();
            assert_built(&normal, &resources);
            assert_connector_reads(&normal, &resources, horizon_core::geometry::Direction::East);
            let (_, limited) = piece_limited::assemble(
                &config,
                &piece_limited::Config::decode(&json!({})),
                &mut Flat,
                &pools,
                &resources,
                &bindings,
                ChunkPos::new(0, 0),
                [10, 10, 20],
            )
            .unwrap();
            assert_built(&limited, &resources);
            assert_connector_reads(
                &limited,
                &resources,
                horizon_core::geometry::Direction::West,
            );
            let projected = JigsawConfig {
                max_depth: 0,
                project_to: Some(HeightmapType::WorldSurfaceWg),
                ..config
            };
            let (position, start) = jigsaw::assemble(
                &projected,
                &mut Flat,
                &pools,
                &resources,
                &bindings,
                &mut LegacyRandom::new(0),
                [10, 10, 20],
            )
            .unwrap();
            assert_eq!(position, [10, 90, 20]);
            assert_eq!(start.len(), 1);
            assert_eq!(start[0].position, [10, 89, 20]);
            let connectors = start[0].element.shuffled_jigsaws(
                &resources,
                start[0].position,
                start[0].rotation,
                &mut LegacyRandom::new(0),
            );
            assert_eq!(
                (connectors[0].pos, connectors[0].front, connectors[0].top),
                (
                    [10, 89, 20],
                    horizon_core::geometry::Direction::Up,
                    horizon_core::geometry::Direction::North
                )
            );
        })
    });
}

fn junction(y: i32, delta: i32, projection: &str) -> Tag {
    Tag::compound([
        ("source_x", Tag::Int(10)),
        ("source_ground_y", Tag::Int(y)),
        ("source_z", Tag::Int(20)),
        ("delta_y", Tag::Int(delta)),
        ("dest_proj", Tag::String(projection.into())),
    ])
}
fn expected_piece(child: bool, rotation: &str, connected: bool) -> Tag {
    // Literal fixed-seed fixture, independently calculated from the vanilla placement equations
    // and java.util.Random's documented recurrence (seed 0), not from the assembler's output.
    Tag::compound([
        ("id", Tag::String("minecraft:jigsaw".into())),
        ("O", Tag::Int(-1)),
        ("GD", Tag::Int(0)),
        (
            "BB",
            Tag::IntArray(if child {
                vec![10, 78, 20, 10, 80, 20]
            } else {
                vec![10, 9, 20, 10, 9, 20]
            }),
        ),
        ("PosX", Tag::Int(10)),
        ("PosY", Tag::Int(if child { 78 } else { 9 })),
        ("PosZ", Tag::Int(20)),
        ("rotation", Tag::String(rotation.into())),
        ("ground_level_delta", Tag::Int(if child { 2 } else { 1 })),
        ("liquid_settings", Tag::String("ignore_waterlogging".into())),
        (
            "pool_element",
            Tag::compound([
                (
                    "element_type",
                    Tag::String("minecraft:single_pool_element".into()),
                ),
                (
                    "location",
                    Tag::String(if child { "test:child" } else { "test:start" }.into()),
                ),
                ("processors", Tag::List(Vec::new())),
                (
                    "projection",
                    Tag::String(if child { "rigid" } else { "terrain_matching" }.into()),
                ),
            ]),
        ),
        (
            "junctions",
            Tag::List(if connected {
                vec![if child {
                    junction(80, 1, "terrain_matching")
                } else {
                    junction(81, -1, "rigid")
                }]
            } else {
                Vec::new()
            }),
        ),
    ])
}
fn bytes(tags: Vec<Tag>) -> Vec<u8> {
    nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag: Tag::compound([("pieces", Tag::List(tags))]),
        },
        Limits::templates(),
    )
    .unwrap()
}
fn saved(pieces: &[PoolPiece]) -> Vec<u8> {
    bytes(
        pieces
            .iter()
            .map(|piece| Tag::Compound(piece.save()))
            .collect(),
    )
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn no_part_fixed_seed_layout_bytes_and_draws_stay_unchanged() {
    let _publishing = native_worldgen_parts::PUBLISHING
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    for modded in [false, true] {
        let mods: Mods = if modded {
            vec![ModEntry::new("constant_connector", Box::new(Turns(false)))].into()
        } else {
            Vec::new().into()
        };
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                publish(game);
                let resources = resources(game.pack.assets.registry, &game.options.resources);
                let bindings =
                    FeatureBlockBindings::new(resources.registry, Default::default()).unwrap();
                let pools = pools();
                let aliases = BTreeMap::new();
                let config = config(&aliases, 1);
                let mut random = LegacyRandom::new(0);
                let (position, pieces) = jigsaw::assemble(
                    &config,
                    &mut Flat,
                    &pools,
                    &resources,
                    &bindings,
                    &mut random,
                    [10, 10, 20],
                )
                .unwrap();
                assert_eq!(position, [10, 10, 20]);
                assert_eq!(
                    saved(&pieces),
                    bytes(vec![
                        expected_piece(false, "CLOCKWISE_180", true),
                        expected_piece(true, "COUNTERCLOCKWISE_90", true)
                    ])
                );
                assert_eq!(random.next_long(), 5_700_976_833_288_827_063);
                let (_, limited) = piece_limited::assemble(
                    &config,
                    &piece_limited::Config::decode(&json!({})),
                    &mut Flat,
                    &pools,
                    &resources,
                    &bindings,
                    ChunkPos::new(0, 0),
                    [10, 10, 20],
                )
                .unwrap();
                assert_eq!(
                    saved(&limited),
                    bytes(vec![
                        expected_piece(false, "CLOCKWISE_180", true),
                        expected_piece(true, "CLOCKWISE_90", true)
                    ])
                );
                let start_only = JigsawConfig {
                    max_depth: 0,
                    ..config
                };
                let mut random = LegacyRandom::new(0);
                let (_, start) = jigsaw::assemble(
                    &start_only,
                    &mut Flat,
                    &pools,
                    &resources,
                    &bindings,
                    &mut random,
                    [10, 10, 20],
                )
                .unwrap();
                assert_eq!(
                    saved(&start),
                    bytes(vec![expected_piece(false, "CLOCKWISE_180", false)])
                );
                assert_eq!(random.next_long(), 4_437_113_781_045_784_766);
                let projected = JigsawConfig {
                    project_to: Some(HeightmapType::WorldSurfaceWg),
                    ..start_only
                };
                let mut random = LegacyRandom::new(0);
                let (position, start) = jigsaw::assemble(
                    &projected,
                    &mut Flat,
                    &pools,
                    &resources,
                    &bindings,
                    &mut random,
                    [10, 10, 20],
                )
                .unwrap();
                assert_eq!(position, [10, 90, 20]);
                let Tag::Compound(mut expected) = expected_piece(false, "CLOCKWISE_180", false)
                else {
                    unreachable!()
                };
                expected.insert("PosY".into(), Tag::Int(89));
                expected.insert("BB".into(), Tag::IntArray(vec![10, 89, 20, 10, 89, 20]));
                assert_eq!(saved(&start), bytes(vec![Tag::Compound(expected)]));
                assert_eq!(random.next_long(), 4_437_113_781_045_784_766);
            })
        });
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn placed_palette_keeps_connector_identity_and_named_start_requirement() {
    let _publishing = native_worldgen_parts::PUBLISHING
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mods: Mods = Vec::new().into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            publish(game);
            let resources = resources(game.pack.assets.registry, &game.options.resources);
            let bindings =
                FeatureBlockBindings::new(resources.registry, Default::default()).unwrap();
            let pools = pools();
            let aliases = BTreeMap::new();
            let config = config(&aliases, 1);
            let stone = Tag::compound([("id", Tag::String("minecraft:stone".into()))]);
            let Tag::Compound(mut child) = template_tag(true) else {
                unreachable!()
            };
            let Some(Tag::List(jigsaw)) = child.remove(&"palette".into()) else {
                unreachable!()
            };
            let jigsaw = jigsaw[0].clone();
            child.insert(
                "palettes".into(),
                Tag::List(vec![
                    Tag::List(vec![stone.clone(), jigsaw.clone()]),
                    Tag::List(vec![jigsaw.clone(), stone.clone()]),
                ]),
            );
            let Some(Tag::List(blocks)) = child.get_mut(&"blocks".into()) else {
                unreachable!()
            };
            let Tag::Compound(mut other) = blocks[0].clone() else {
                unreachable!()
            };
            other.insert("pos".into(), ints(&[0, 0, 0]));
            other.insert("state".into(), Tag::Int(1));
            blocks.push(Tag::Compound(other));
            // Independent position-seeded palette literals: ZERO selects palette 1;
            // the placed child origin (10,78,20) selects palette 0. The connector at
            // local (0,2,0) is used for matching; expansion instead sees local (0,0,0).
            let resources = resources.with_pack_templates(
                [
                    ("test:start".into(), compressed(template_tag(false))),
                    ("test:child".into(), compressed(Tag::Compound(child))),
                ]
                .into(),
            );
            let (_, pieces) = jigsaw::assemble(
                &config,
                &mut Flat,
                &pools,
                &resources,
                &bindings,
                &mut LegacyRandom::new(0),
                [10, 10, 20],
            )
            .unwrap();
            assert_eq!(pieces.len(), 2);
            let child = &pieces[1];
            let sampled = child.element.shuffled_jigsaws(
                &resources,
                [0; 3],
                child.rotation,
                &mut LegacyRandom::new(0),
            );
            let expanded = child.element.shuffled_jigsaws(
                &resources,
                child.position,
                child.rotation,
                &mut LegacyRandom::new(0),
            );
            assert_eq!(
                sampled.iter().map(|j| j.pos).collect::<Vec<_>>(),
                [[0, 2, 0]]
            );
            assert_eq!(
                expanded.iter().map(|j| j.pos).collect::<Vec<_>>(),
                [[10, 78, 20]]
            );
            let (_, pieces) = piece_limited::assemble(
                &config,
                &piece_limited::Config::decode(&json!({})),
                &mut Flat,
                &pools,
                &resources,
                &bindings,
                ChunkPos::new(0, 0),
                [10, 10, 20],
            )
            .unwrap();
            assert_eq!(pieces.len(), 2);
            let Tag::Compound(mut start) = template_tag(false) else {
                unreachable!()
            };
            start.remove(&"palette".into());
            start.insert(
                "palettes".into(),
                Tag::List(vec![Tag::List(vec![jigsaw]), Tag::List(vec![stone])]),
            );
            let resources = resources.with_pack_templates(
                [("test:start".into(), compressed(Tag::Compound(start)))].into(),
            );
            let start_pools = self::pools();
            // The start requirement is checked in palette 0 at the sampled origin
            // (10,10,20). Expansion uses palette 1 at (10,9,20), with no connector;
            // vanilla does not resample the anchor or substitute the origin palette.
            let (_, pieces) = jigsaw::assemble(
                &config,
                &mut Flat,
                &start_pools,
                &resources,
                &bindings,
                &mut LegacyRandom::new(0),
                [10, 10, 20],
            )
            .unwrap();
            assert_eq!(pieces.len(), 1);
            assert_eq!(pieces[0].position, [10, 9, 20]);
            assert!(
                pieces[0]
                    .element
                    .shuffled_jigsaws(
                        &resources,
                        pieces[0].position,
                        pieces[0].rotation,
                        &mut LegacyRandom::new(0)
                    )
                    .is_empty()
            );
        })
    });
}
