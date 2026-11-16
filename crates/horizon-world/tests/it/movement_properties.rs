use horizon_core::position::BlockPos;
use {
    horizon_core::nbt::Tag,
    horizon_world::{
        DATA_VERSION,
        movement_properties::{
            BlockFactorEdits, MovementBlockError, MovementBlockProvider, MovementProperties,
            MovementTags, NoFactorEdits, RegistryMovementWorld,
        },
        movement_support,
        movement_support::{MovementBlockWorld, MovementLocation},
        registry::{BlockRegistry, BlockStateId},
    },
};
fn fixture() -> (Tag, Tag) {
    let states = Tag::List(vec![
        Tag::String("minecraft:stone".into()),
        Tag::String("minecraft:water".into()),
    ]);
    let registry = Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("states", states.clone()),
        (
            "defaults",
            Tag::compound([
                ("minecraft:stone", Tag::Int(0)),
                ("minecraft:water", Tag::Int(1)),
            ]),
        ),
    ]);
    let definition = Tag::compound([
        ("friction", Tag::Float(0.6f32.to_bits())),
        ("speedFactor", Tag::Float((-0.0f32).to_bits())),
        ("jumpFactor", Tag::Float(1f32.to_bits())),
        ("bounceRestitution", Tag::Float(0)),
        ("fallDistanceReduction", Tag::Float(0)),
        ("fenceGate", Tag::Byte(0)),
    ]);
    let movement = Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("states", states),
        (
            "blocks",
            Tag::compound([
                ("minecraft:stone", definition.clone()),
                ("minecraft:water", definition),
            ]),
        ),
    ]);
    (registry, movement)
}
#[test]
fn rejects_foreign_incomplete_and_malformed_definitions() {
    let (r, valid) = fixture();
    let registry = BlockRegistry::from_nbt(&r).unwrap();
    for (key, value) in [
        ("DataVersion", Tag::Int(0)),
        ("states", Tag::List(vec![])),
        (
            "states",
            Tag::List(vec![
                Tag::String("minecraft:water".into()),
                Tag::String("minecraft:stone".into()),
            ]),
        ),
        ("blocks", Tag::compound([] as [(&str, Tag); 0])),
    ] {
        let mut bad = valid.clone();
        *bad.get_mut(key).unwrap() = value;
        assert!(MovementProperties::from_nbt(&bad, &registry).is_err());
    }
    for (key, value) in [("friction", Tag::Int(1)), ("fenceGate", Tag::Byte(2))] {
        let mut bad = valid.clone();
        *bad.get_mut("blocks")
            .unwrap()
            .get_mut("minecraft:stone")
            .unwrap()
            .get_mut(key)
            .unwrap() = value;
        assert!(MovementProperties::from_nbt(&bad, &registry).is_err());
    }
    let mut bad = valid.clone();
    let Tag::Compound(blocks) = bad.get_mut("blocks").unwrap() else {
        unreachable!()
    };
    blocks.insert(
        "minecraft:unknown".into(),
        blocks.values().next().unwrap().clone(),
    );
    assert!(MovementProperties::from_nbt(&bad, &registry).is_err());
}
#[test]
fn reloaded_tags_preserve_previous_binding_and_coefficients() {
    let (r, data) = fixture();
    let registry = BlockRegistry::from_nbt(&r).unwrap();
    let properties = MovementProperties::from_nbt(&data, &registry).unwrap();
    let empty = properties.bind(&MovementTags::default()).unwrap();
    let mut tags = MovementTags::default();
    tags.fences.insert("minecraft:stone".into());
    tags.suppresses_bounce.insert("minecraft:water".into());
    let changed = properties.bind(&tags).unwrap();
    assert!(!empty.get(BlockStateId(0)).unwrap().fence);
    assert!(changed.get(BlockStateId(0)).unwrap().fence);
    let water = changed.get(BlockStateId(1)).unwrap();
    assert!(water.water && water.suppresses_bounce);
    assert_eq!(
        water.coefficients.speed_factor.to_bits(),
        (-0.0f32).to_bits()
    );
    tags.walls.insert("minecraft:missing".into());
    assert!(properties.bind(&tags).is_err());
    assert!(empty.get(BlockStateId(2)).is_err());
}
#[test]
fn provider_errors_and_unknown_states_remain_distinct() {
    struct Provider(bool);
    impl MovementBlockProvider for Provider {
        type Error = &'static str;
        fn movement_block_state(&mut self, _: BlockPos) -> Result<BlockStateId, Self::Error> {
            if self.0 {
                Err("missing chunk")
            } else {
                Ok(BlockStateId(2))
            }
        }
    }
    let (r, data) = fixture();
    let registry = BlockRegistry::from_nbt(&r).unwrap();
    let properties = MovementProperties::from_nbt(&data, &registry)
        .unwrap()
        .bind(&MovementTags::default())
        .unwrap();
    let mut provider = Provider(true);
    let mut world = RegistryMovementWorld {
        properties: &properties,
        world: &mut provider,
        edits: &NoFactorEdits,
    };
    assert!(matches!(
        world.block_state(BlockPos::default()),
        Err(MovementBlockError::World("missing chunk"))
    ));
    world.world.0 = false;
    assert!(matches!(
        world.block_state(BlockPos::default()),
        Err(MovementBlockError::Data(_))
    ));
}

/// An edit to a block's factor is made where the block is read, and the entity then combines the
/// edited values as vanilla does: the block at its feet, unless that gives 1.0, then the one below.
#[test]
fn edited_block_factors_combine_as_the_blocks_own() {
    struct Stone;
    impl MovementBlockProvider for Stone {
        type Error = ();
        fn movement_block_state(&mut self, _: BlockPos) -> Result<BlockStateId, ()> {
            Ok(BlockStateId(0))
        }
    }
    struct Double;
    impl BlockFactorEdits for Double {
        fn jump(&self, state: BlockStateId, factor: f32) -> f32 {
            assert_eq!(state, BlockStateId(0));
            factor * 2.0
        }
        fn speed(&self, _: BlockStateId, factor: f32) -> f32 {
            factor + 0.5
        }
    }
    let (r, data) = fixture();
    let registry = BlockRegistry::from_nbt(&r).unwrap();
    let properties = MovementProperties::from_nbt(&data, &registry)
        .unwrap()
        .bind(&MovementTags::default())
        .unwrap();
    let location = MovementLocation {
        position: horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5),
        block_position: BlockPos::new(0, 64, 0),
        supporting_block: None,
    };
    let read = |edits: &dyn BlockFactorEdits| {
        let mut world = RegistryMovementWorld {
            properties: &properties,
            world: &mut Stone,
            edits,
        };
        (
            movement_support::block_jump_factor(&mut world, location).unwrap(),
            movement_support::block_speed_factor(&mut world, location).unwrap(),
        )
    };
    let (jump, speed) = read(&NoFactorEdits);
    assert_eq!(
        (jump.to_bits(), speed.to_bits()),
        (1f32.to_bits(), (-0.0f32).to_bits())
    );
    assert_eq!(read(&Double), (2.0, 0.5));
}
