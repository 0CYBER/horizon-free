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

fn canonical_marker(shape: &mut Tag, value: Tag) {
    let Tag::Compound(fields) = shape else {
        unreachable!()
    };
    fields.insert("canonical".into(), value);
}

#[test]
fn canonical_shapes_restore_global_identity_without_normalizing_equal_grids() {
    use horizon_core::geometry::VoxelShape;
    let (registry_tag, mut collision_tag) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    assert!(
        !CachedCollisionRegistry::from_nbt(&collision_tag, &registry)
            .unwrap()
            .has_canonical_identities()
    );
    let Some(Tag::List(shapes)) = collision_tag.get_mut("shapes") else {
        unreachable!()
    };
    canonical_marker(&mut shapes[0], Tag::Byte(1));
    canonical_marker(&mut shapes[1], Tag::Byte(2));
    let mut equal_block = shapes[1].clone();
    canonical_marker(&mut equal_block, Tag::Byte(0));
    shapes.push(equal_block);
    *collision_tag.get_mut("cached_shapes").unwrap() = Tag::IntArray(vec![0, 1, 2]);
    let collision = CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap();
    assert!(collision.has_canonical_identities());
    assert_eq!(collision.unique_grid_count(), 2);
    let empty = collision.cached_shape(BlockStateId(0)).unwrap().unwrap();
    let block = collision.cached_shape(BlockStateId(1)).unwrap().unwrap();
    let separate = collision.cached_shape(BlockStateId(2)).unwrap().unwrap();
    assert!(empty.same_identity(&VoxelShape::empty()));
    assert!(block.same_identity(&VoxelShape::block()));
    assert!(!separate.same_identity(&VoxelShape::block()));
    assert_eq!(block.boxes(), separate.boxes());
    assert!(std::ptr::eq(
        block.boxes().as_ptr(),
        separate.boxes().as_ptr()
    ));
}

#[test]
fn canonical_identity_claims_require_exact_geometry_and_unique_exports() {
    let (registry_tag, valid) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    for (index, marker) in [
        (0, Tag::Byte(2)),
        (1, Tag::Byte(1)),
        (1, Tag::Byte(3)),
        (1, Tag::Int(2)),
    ] {
        let mut data = valid.clone();
        let Some(Tag::List(shapes)) = data.get_mut("shapes") else {
            unreachable!()
        };
        canonical_marker(&mut shapes[index], marker);
        assert!(CachedCollisionRegistry::from_nbt(&data, &registry).is_err());
    }
    let mut duplicate = valid.clone();
    let Some(Tag::List(shapes)) = duplicate.get_mut("shapes") else {
        unreachable!()
    };
    canonical_marker(&mut shapes[1], Tag::Byte(2));
    shapes.push(shapes[1].clone());
    assert!(CachedCollisionRegistry::from_nbt(&duplicate, &registry).is_err());
    let mut noncanonical_zero = valid;
    let Some(Tag::List(shapes)) = noncanonical_zero.get_mut("shapes") else {
        unreachable!()
    };
    canonical_marker(&mut shapes[1], Tag::Byte(2));
    let Some(Tag::LongArray(coords)) = shapes[1].get_mut("x") else {
        unreachable!()
    };
    coords[0] = (-0.0f64).to_bits() as i64;
    assert!(CachedCollisionRegistry::from_nbt(&noncanonical_zero, &registry).is_err());
}

#[test]
fn registry_scan_binding_requires_complete_canonical_metadata() {
    use horizon_world::{
        block_collision::BlockCollisionRegistry,
        registry_collision_scan::RegistryCollisionWorld,
        suffocation::{SuffocationDefinition, SuffocationRegistry},
    };
    use std::{collections::BTreeSet, sync::Arc};
    let (registry_tag, mut collision_tag) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    let definitions = ["minecraft:air", "minecraft:stone", "minecraft:context"]
        .map(|name| (name.to_owned(), SuffocationDefinition::Never))
        .into_iter()
        .collect();
    for marked in [false, true] {
        if marked {
            let Some(Tag::List(shapes)) = collision_tag.get_mut("shapes") else {
                unreachable!()
            };
            canonical_marker(&mut shapes[0], Tag::Byte(1));
            canonical_marker(&mut shapes[1], Tag::Byte(2));
        }
        let cached =
            Arc::new(CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap());
        let collisions = Arc::new(BlockCollisionRegistry::new(cached).unwrap());
        assert!(collisions.collision_block(BlockStateId(3)).is_err());
        let binding = SuffocationRegistry::new(collisions, &definitions, &BTreeSet::new()).unwrap();
        assert_eq!(
            RegistryCollisionWorld::new(&binding, &mut ()).is_ok(),
            marked
        );
    }
}

/// A table read from its bytes checks its state identities while reading and
/// keeps none of them, yet loads the same shapes as the tree; a nested table
/// (`block-properties.nbt`'s `occlusion`) is checked the same way.
#[test]
fn streamed_tables_check_states_while_reading() {
    use horizon_world::{
        nbt::{self, Limits, NamedTag},
        registry::{BlockTable, BlockTableFile},
    };
    let (registry_tag, collision_tag) = fixture();
    let registry = BlockRegistry::from_nbt(&registry_tag).unwrap();
    let bytes = |tag: &Tag| {
        let root = NamedTag {
            name: "".into(),
            tag: tag.clone(),
        };
        nbt::write_named(&root, Limits::tables()).unwrap()
    };
    let file = BlockTableFile::read(&bytes(&collision_tag), &registry).unwrap();
    let table = file.table("collision").unwrap();
    assert_eq!(table.get("states"), Some(&Tag::List(Vec::new())));
    let streamed = CachedCollisionRegistry::from_table(table).unwrap();
    let tree = CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap();
    for id in 0..3 {
        let id = BlockStateId(id);
        assert_eq!(
            streamed.cached_shape(id).unwrap().map(|s| s.boxes()),
            tree.cached_shape(id).unwrap().map(|s| s.boxes())
        );
    }

    let mut outer = Tag::compound([("occlusion", collision_tag.clone())]);
    let Tag::Compound(map) = &mut outer else {
        unreachable!()
    };
    map.insert(
        "states".into(),
        collision_tag.get("states").unwrap().clone(),
    );
    let file = BlockTableFile::read(&bytes(&outer), &registry).unwrap();
    let nested = file
        .table("outer")
        .unwrap()
        .nested("occlusion", "occlusion")
        .unwrap();
    assert_eq!(nested.get("states"), Some(&Tag::List(Vec::new())));
    CachedCollisionRegistry::from_table(nested).unwrap();
    // Checked only as a tree, an emptied list is refused.
    assert!(BlockTable::check(nested.tag(), &registry, "occlusion").is_err());

    // Other states, a different count or no states at all are refused.
    let with_states = |edit: &dyn Fn(&mut Vec<Tag>)| {
        let mut tag = collision_tag.clone();
        let Some(Tag::List(states)) = tag.get_mut("states") else {
            unreachable!()
        };
        edit(states);
        tag
    };
    let swapped = with_states(&|s| s.swap(1, 2));
    assert!(BlockTableFile::read(&bytes(&swapped), &registry).is_err());
    let short = with_states(&|s| drop(s.pop()));
    assert!(BlockTableFile::read(&bytes(&short), &registry).is_err());
    let mut swapped = swapped;
    let Tag::Compound(map) = &mut swapped else {
        unreachable!()
    };
    map.remove(&"states".into());
    let file = BlockTableFile::read(&bytes(&swapped), &registry).unwrap();
    assert!(file.table("collision").is_err());
}
