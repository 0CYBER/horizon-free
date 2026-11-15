//! Independent fixtures for the registry lookup boundary, copied by move.py.
use horizon_mod_runtime::registry_keys::{
    EffectResolutionError, Names, dimension_id, dimension_slot,
};

#[test]
fn requests_canonicalize_deduplicate_and_report_each_requesting_mod_once() {
    let mut names = Names::default();
    assert_eq!(names.request("first", "stone"), Ok(0));
    assert_eq!(names.request("first", "minecraft:stone"), Ok(0));
    assert_eq!(names.request("second", "minecraft:stone"), Ok(0));
    assert_eq!(names.request("second", "test:machine/shaft"), Ok(1));
    assert_eq!(names.id(0), Some("minecraft:stone"));
    assert_eq!(names.id(2), None);
    let missing = names.resolve(|_| None::<u32>, |owner, id| format!("{owner}: {id}"));
    assert_eq!(
        missing,
        Err(vec![
            "first: minecraft:stone".to_owned(),
            "second: minecraft:stone".to_owned(),
            "second: test:machine/shaft".to_owned(),
        ])
    );
    for bad in [
        "",
        ":stone",
        "test:",
        "TEST:stone",
        "a:b:c",
        "a/b:stone",
        "test:bad space",
    ] {
        assert!(names.request("bad", bad).is_err(), "{bad:?}");
    }
    assert_eq!(names.request("third", "new:value"), Ok(2));
}

#[test]
fn effect_registry_order_is_distinct_from_requested_slot_order() {
    let mut names = Names::default();
    names.request("mod", "test:later").unwrap();
    let resolved = horizon_mod_runtime::registry_keys::resolve_effects(
        &names,
        ["minecraft:speed", "test:unrequested", "test:later"],
        |id| match id {
            "minecraft:speed" => Some(0),
            "test:later" => Some(2),
            "test:unrequested" => Some(1),
            _ => None,
        },
    )
    .unwrap();
    assert_eq!(resolved.indices, [2, 0, 1]);
    assert_eq!(resolved.keys, [1, 2, 0]);
    assert_eq!(resolved.names.id(0), Some("test:later"));
    assert_eq!(resolved.names.id(1), Some("minecraft:speed"));
    assert_eq!(resolved.names.id(2), Some("test:unrequested"));
    let errors = horizon_mod_runtime::registry_keys::resolve_effects(&names, [], |_| None);
    assert_eq!(
        errors.err().unwrap(),
        [EffectResolutionError::UnknownEffect {
            mod_id: "mod",
            id: "test:later".to_owned(),
        }]
    );
}

#[test]
fn effect_resolution_refuses_indices_outside_the_registry() {
    for index in [1, usize::MAX] {
        let result = horizon_mod_runtime::registry_keys::resolve_effects(
            &Names::default(),
            ["minecraft:speed"],
            |_| Some(index),
        );
        assert_eq!(
            result.err().unwrap(),
            [EffectResolutionError::InvalidIndex {
                id: "minecraft:speed".to_owned(),
                index,
                registry_len: 1,
            }]
        );
    }
}

#[test]
fn block_entity_resolution_checks_requests_before_appending_registry_names() {
    let mut names = Names::default();
    names.request("mod", "test:machine").unwrap();
    let resolved = horizon_mod_runtime::registry_keys::resolve_block_entity_types(
        &names,
        ["minecraft:chest", "test:machine"],
        |id| (id == "test:machine").then_some(17),
    )
    .unwrap();
    assert_eq!(resolved.id(0), Some("test:machine"));
    assert_eq!(resolved.id(1), Some("minecraft:chest"));
    assert_eq!(resolved.id(2), None);
    let errors = horizon_mod_runtime::registry_keys::resolve_block_entity_types(
        &names,
        ["test:machine"],
        |_| None::<u32>,
    );
    assert_eq!(
        errors.err().unwrap(),
        ["mod mod requested unknown block entity type test:machine"]
    );
}

#[test]
fn completed_names_keep_requested_slots_and_sort_unrequested_entries() {
    let mut requests = horizon_mod_runtime::registry_keys::Requests::default();
    requests.particles.request("mod", "test:spark").unwrap();
    let names = requests
        .particles
        .complete(
            [
                "minecraft:smoke",
                "test:spark",
                "minecraft:flame",
                "minecraft:smoke",
            ],
            |owner, id| format!("{owner}: {id}"),
        )
        .unwrap();
    assert_eq!(names.id(0), Some("test:spark"));
    assert_eq!(names.id(1), Some("minecraft:flame"));
    assert_eq!(names.id(2), Some("minecraft:smoke"));
    assert_eq!(names.entries.len(), 3);
    assert_eq!(requests.particles.entries.len(), 1);
    let again = requests
        .particles
        .complete(
            ["minecraft:flame", "test:spark", "minecraft:smoke"],
            |owner, id| format!("{owner}: {id}"),
        )
        .unwrap();
    assert_eq!(names.slots, again.slots);
}

#[test]
fn incomplete_names_report_each_owner_without_publishing_a_partial_table() {
    let mut names = Names::default();
    names.request("first", "test:absent").unwrap();
    names.request("second", "test:absent").unwrap();
    assert_eq!(
        names
            .complete(["minecraft:flame"], |owner, id| format!("{owner}: {id}"))
            .err(),
        Some(vec![
            "first: test:absent".into(),
            "second: test:absent".into()
        ])
    );
    assert_eq!(names.entries.len(), 1);
    assert_eq!(names.id(0), Some("test:absent"));
    assert_eq!(names.id(1), None);
}

#[test]
fn extra_dimension_slots_are_shared_and_stable() {
    let slot = dimension_slot("test:custom");
    let other = std::thread::spawn(|| dimension_slot("test:custom"))
        .join()
        .unwrap();
    assert_eq!(slot, other);
    assert_eq!(dimension_id(slot), Some("test:custom"));
    let second = dimension_slot("test:second");
    assert_ne!(slot, second);
    assert_eq!(dimension_id(slot), Some("test:custom"));
    assert_eq!(dimension_id(u32::MAX), None);
}
