use horizon_world::{
    DATA_VERSION,
    chunk::BlockSection,
    collision::CachedCollisionRegistry,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId},
};

fn fixture() -> (Tag, Tag) {
    let states = vec![
        Tag::String("minecraft:air".into()),
        Tag::String("minecraft:stone".into()),
        Tag::String("minecraft:context".into()),
    ];
    let registry = Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("states", Tag::List(states.clone())),
        (
            "defaults",
            Tag::compound([
                ("minecraft:air", Tag::Int(0)),
                ("minecraft:stone", Tag::Int(1)),
                ("minecraft:context", Tag::Int(2)),
            ]),
        ),
    ]);
    let empty = Tag::compound([
        ("x", Tag::LongArray(vec![0])),
        ("y", Tag::LongArray(vec![0])),
        ("z", Tag::LongArray(vec![0])),
        ("occupied", Tag::ByteArray(vec![])),
        ("cube", Tag::Byte(0)),
        ("cube_axes", Tag::Byte(0)),
        ("discrete_min", Tag::IntArray(vec![0; 3])),
        ("discrete_max", Tag::IntArray(vec![0; 3])),
    ]);
    let coords = Tag::LongArray(vec![0, 1.0f64.to_bits() as i64]);
    let block = Tag::compound([
        ("x", coords.clone()),
        ("y", coords.clone()),
        ("z", coords),
        ("occupied", Tag::ByteArray(vec![1])),
        ("cube", Tag::Byte(1)),
        ("cube_axes", Tag::Byte(7)),
        ("discrete_min", Tag::IntArray(vec![0; 3])),
        ("discrete_max", Tag::IntArray(vec![1; 3])),
    ]);
    let collision = Tag::compound([
        ("DataVersion", Tag::Int(DATA_VERSION)),
        ("states", Tag::List(states)),
        ("shapes", Tag::List(vec![empty, block])),
        ("cached_shapes", Tag::IntArray(vec![0, 1, -1])),
    ]);
    (registry, collision)
}

#[test]
fn empty_cached_and_context_states_are_distinct() {
    let (registry_tag, collision_tag) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    let collision = CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap();
    assert_eq!(collision.cached_state_count(), 2);
    assert_eq!(collision.shape_count(), 2);
    assert!(
        collision
            .cached_shape(BlockStateId(0))
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert!(
        !collision
            .cached_shape(BlockStateId(1))
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert!(collision.cached_shape(BlockStateId(2)).unwrap().is_none());
    assert!(collision.cached_shape(BlockStateId(3)).is_err());
    let mut section = BlockSection::uniform(BlockStateId(0), &registry).unwrap();
    section.set(4095, BlockStateId(1)).unwrap();
    assert!(
        !collision
            .section_shape(&section, 4095)
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert!(collision.section_shape(&section, 4096).is_err());
    let different = BlockRegistry::from_nbt(&registry_tag).unwrap();
    let foreign_section = BlockSection::uniform(BlockStateId(0), &different).unwrap();
    assert!(collision.section_shape(&foreign_section, 0).is_err());
}

#[test]
fn invalid_imports_fail_before_collision_queries() {
    let (registry_tag, valid) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    for (field, tag) in [
        ("DataVersion", Tag::Int(0)),
        ("cached_shapes", Tag::IntArray(vec![0, 1])),
        ("cached_shapes", Tag::IntArray(vec![0, 2, -1])),
        ("cached_shapes", Tag::IntArray(vec![0, 1, -2])),
    ] {
        let mut bad = valid.clone();
        *bad.get_mut(field).unwrap() = tag;
        assert!(CachedCollisionRegistry::from_nbt(&bad, &registry).is_err());
    }
    let mut bad = valid.clone();
    let Some(Tag::List(states)) = bad.get_mut("states") else {
        unreachable!()
    };
    states.swap(0, 1);
    assert!(CachedCollisionRegistry::from_nbt(&bad, &registry).is_err());
    for (field, tag) in [
        ("cube", Tag::Byte(2)),
        ("cube_axes", Tag::Byte(8)),
        ("cube_axes", Tag::Byte(0)),
        ("occupied", Tag::ByteArray(vec![2])),
        ("discrete_min", Tag::IntArray(vec![0, 0])),
        ("discrete_min", Tag::IntArray(vec![-1, 0, 0])),
        ("discrete_max", Tag::IntArray(vec![1, 0, 1])),
        ("occupied", Tag::ByteArray(vec![])),
        ("x", Tag::LongArray(vec![0, 0])),
        ("x", Tag::LongArray(vec![0, f64::NAN.to_bits() as i64])),
        ("x", Tag::LongArray(vec![0, 0.5f64.to_bits() as i64])),
    ] {
        let mut bad = valid.clone();
        let Some(Tag::List(shapes)) = bad.get_mut("shapes") else {
            unreachable!()
        };
        *shapes[1].get_mut(field).unwrap() = tag;
        assert!(CachedCollisionRegistry::from_nbt(&bad, &registry).is_err());
    }
}

#[test]
fn equal_grids_share_storage_without_collapsing_shape_identities() {
    let (registry_tag, mut collision_tag) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    let Some(Tag::List(shapes)) = collision_tag.get_mut("shapes") else {
        unreachable!()
    };
    shapes.push(shapes[1].clone());
    *collision_tag.get_mut("cached_shapes").unwrap() = Tag::IntArray(vec![0, 1, 2]);
    let collision = CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap();
    assert_eq!(collision.shape_count(), 3);
    assert_eq!(collision.unique_grid_count(), 2);
    let first = collision.cached_shape(BlockStateId(1)).unwrap().unwrap();
    let second = collision.cached_shape(BlockStateId(2)).unwrap().unwrap();
    assert!(!first.same_identity(second));
    assert!(std::ptr::eq(
        first.boxes().as_ptr(),
        second.boxes().as_ptr()
    ));
    assert!(first.same_identity(collision.cached_shape(BlockStateId(1)).unwrap().unwrap()));
}
