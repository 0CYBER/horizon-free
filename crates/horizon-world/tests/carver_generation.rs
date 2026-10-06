use horizon_core::position::ChunkPos;
use horizon_world::worldgen::{
    anchor::WorldGenerationContext,
    carver::{CarverCatalog, MaskGenerationError, generate_carving_mask},
};
use std::{collections::BTreeMap, sync::Arc};

#[test]
fn origin_order_wraps_coordinates_and_upgrading_changes_the_ceiling() {
    let target = ChunkPos::new(i32::MAX, i32::MIN);
    for upgrading in [false, true] {
        let mut visited = Vec::new();
        let mask = generate_carving_mask(
            0,
            target,
            WorldGenerationContext::new(-64, 384, 63),
            upgrading,
            |origin| {
                visited.push(origin);
                Ok::<_, ()>(&[])
            },
        )
        .unwrap();
        assert!(mask.is_empty());
        assert_eq!(mask.min_y(), -63);
        assert_eq!(mask.max_y(), if upgrading { 319 } else { 312 });
        let expected: Vec<_> = (-8..=8)
            .flat_map(|dx| {
                (-8..=8).map(move |dz| {
                    ChunkPos::new(target.x.wrapping_add(dx), target.z.wrapping_add(dz))
                })
            })
            .collect();
        assert_eq!(visited, expected);
    }
}

#[test]
fn invalid_bounds_skip_the_source_and_source_failures_stop_immediately() {
    let target = ChunkPos::new(0, 0);
    let invalid = generate_carving_mask(
        0,
        target,
        WorldGenerationContext::new(0, 8, 0),
        false,
        |_| -> Result<_, ()> { panic!("invalid bounds must precede biome queries") },
    );
    assert!(matches!(invalid, Err(MaskGenerationError::Mask(_))));
    let mut calls = 0;
    let failed = generate_carving_mask(
        0,
        target,
        WorldGenerationContext::new(0, 16, 0),
        false,
        |_| {
            calls += 1;
            if calls == 19 {
                Err("biome failure")
            } else {
                Ok(&[])
            }
        },
    );
    assert!(matches!(
        failed,
        Err(MaskGenerationError::Source("biome failure"))
    ));
    assert_eq!(calls, 19);
}

#[test]
fn holder_lists_preserve_duplicate_identity_and_reject_unknown_names() {
    let definition = r#"{"type":"cave","probability":1,"y":{"absolute":8},"count":0,"thickness":1,"room_vertical_radius_multiplier":1,"horizontal_radius_multiplier":1,"vertical_radius_multiplier":1,"floor_level":-1}"#;
    let catalog =
        CarverCatalog::from_json(&BTreeMap::from([("test".into(), definition.into())])).unwrap();
    let entries = catalog.decode_set(r#"["test","minecraft:test"]"#).unwrap();
    assert_eq!(entries.len(), 2);
    assert!(Arc::ptr_eq(&entries[0], &entries[1]));
    assert_eq!(catalog.decode_set(definition).unwrap().len(), 1);
    assert!(catalog.decode_set("[]").unwrap().is_empty());
    assert!(catalog.decode_set(r#"["test","missing"]"#).is_err());
    assert!(
        CarverCatalog::from_json(&BTreeMap::from([
            ("test".into(), definition.into()),
            ("minecraft:test".into(), definition.into()),
        ]))
        .is_err()
    );
}
