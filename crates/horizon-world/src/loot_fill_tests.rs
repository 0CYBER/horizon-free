use super::*;

fn fixture(empty: bool) -> LootData {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir().join(format!(
        "loot-fill-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir(&root).unwrap();
    let dir = root.as_path().join("data/minecraft/loot_table");
    std::fs::create_dir_all(&dir).unwrap();
    let pools = if empty {
        serde_json::json!([])
    } else {
        serde_json::json!([
            {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond","modifier":[{"type":"minecraft:set_count","count":16}]}]},
            {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:emerald"}]},
            {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:gold_ingot"}]},
            {"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot"}]}
        ])
    };
    std::fs::write(
        dir.join("test.json"),
        serde_json::json!({"pools":pools}).to_string(),
    )
    .unwrap();
    std::fs::write(root.as_path().join("items.json"), "{}").unwrap();
    let defaults = crate::item_defaults::read(root.as_path()).unwrap();
    let data = LootData::load(root.as_path(), &PackOverlay::NONE, &defaults).unwrap();
    std::fs::remove_dir_all(root).unwrap();
    data
}

fn summary(slots: &[Option<LootStack>]) -> Vec<Option<(&str, i32)>> {
    slots
        .iter()
        .map(|s| s.as_ref().map(|s| (s.item.as_str(), s.count)))
        .collect()
}

// These literals were recorded from fill_with before this pure refactor:
// its contract is unchanged slot output and RNG advancement.
const EXPECTED: [Option<(&str, i32)>; 5] = [
    Some(("minecraft:iron_ingot", 1)),
    Some(("minecraft:diamond", 10)),
    Some(("minecraft:gold_ingot", 1)),
    Some(("minecraft:emerald", 1)),
    Some(("minecraft:diamond", 6)),
];

#[test]
fn fill_and_split_preserve_recorded_draws() {
    let data = fixture(false);
    let mut random = LegacyRandom::new(37);
    assert_eq!(
        summary(&data.fill_with("minecraft:test", &mut random, 5, None)),
        EXPECTED
    );
    assert_eq!(random.next_int_bound(1000).unwrap(), 706);

    let mut random = LegacyRandom::new(37);
    let items = data
        .roll_container_with(
            "minecraft:test",
            &mut random,
            ContainerLootContext::default(),
        )
        .unwrap();
    assert_eq!(
        items
            .iter()
            .map(|s| (s.item.as_str(), s.count))
            .collect::<Vec<_>>(),
        [
            ("minecraft:diamond", 16),
            ("minecraft:emerald", 1),
            ("minecraft:gold_ingot", 1),
            ("minecraft:iron_ingot", 1),
        ]
    );
    let sentinel = LootStack::new("test:sentinel");
    let mut slots = vec![None; 6];
    slots[2] = Some(sentinel.clone());
    data.place_container_loot(items, &mut slots, vec![0, 1, 3, 4, 5], &mut random);
    assert_eq!(
        summary(&slots),
        [
            EXPECTED[0],
            EXPECTED[1],
            Some(("test:sentinel", 1)),
            EXPECTED[2],
            EXPECTED[3],
            EXPECTED[4]
        ]
    );
    assert_eq!(slots[2], Some(sentinel));
    assert_eq!(random.next_int_bound(1000).unwrap(), 706);
}

#[test]
fn missing_table_does_not_draw() {
    let data = fixture(false);
    let mut random = LegacyRandom::new(37);
    assert_eq!(
        summary(&data.fill_with("minecraft:missing", &mut random, 5, None)),
        [None; 5]
    );
    assert_eq!(random.next_int_bound(1000).unwrap(), 505);
    let mut random = LegacyRandom::new(37);
    assert!(
        data.roll_container_with(
            "minecraft:missing",
            &mut random,
            ContainerLootContext::default()
        )
        .is_none()
    );
    assert_eq!(
        random.next_int_bound(1000).unwrap(),
        LegacyRandom::new(37).next_int_bound(1000).unwrap()
    );
}

#[test]
fn full_container_preserves_recorded_draws() {
    let data = fixture(false);
    let mut random = LegacyRandom::new(37);
    assert!(
        data.fill_with("minecraft:test", &mut random, 0, None)
            .is_empty()
    );
    assert_eq!(random.next_int_bound(1000).unwrap(), 18);
    let mut random = LegacyRandom::new(37);
    let items = data
        .roll_container_with(
            "minecraft:test",
            &mut random,
            ContainerLootContext::default(),
        )
        .unwrap();
    let mut slots = vec![Some(LootStack::new("test:sentinel"))];
    let before = slots.clone();
    data.place_container_loot(items, &mut slots, vec![], &mut random);
    assert_eq!(slots, before);
    assert_eq!(random.next_int_bound(1000).unwrap(), 18);
}

#[test]
fn empty_roll_still_shuffles_slots() {
    let data = fixture(true);
    let mut random = LegacyRandom::new(37);
    assert_eq!(
        summary(&data.fill_with("minecraft:test", &mut random, 5, None)),
        [None; 5]
    );
    assert_eq!(random.next_int_bound(1000).unwrap(), 230);
    let mut random = LegacyRandom::new(37);
    let items = data
        .roll_container_with(
            "minecraft:test",
            &mut random,
            ContainerLootContext::default(),
        )
        .unwrap();
    assert!(items.is_empty());
    let mut slots = vec![None; 5];
    data.place_container_loot(items, &mut slots, (0..5).collect(), &mut random);
    assert_eq!(summary(&slots), [None; 5]);
    assert_eq!(random.next_int_bound(1000).unwrap(), 230);
}

#[test]
fn container_context_supplies_luck_and_this_entity() {
    let mut data = fixture(true);
    data.tables = Arc::new(
        [(
            "test:context".to_owned(),
            serde_json::json!({
                "pools": [{"rolls": 1, "bonus_rolls": 1, "entries": [{
                    "type": "minecraft:item", "name": "minecraft:diamond",
                    "condition": {"type": "minecraft:entity_properties", "entity": "this",
                        "predicate": {"minecraft:entity_type": "minecraft:player"}}
                }]}]
            }),
        )]
        .into_iter()
        .collect(),
    );
    let player = LootEntity::new("minecraft:player");
    for (luck, entity, expected) in [
        (2.0, Some(&player), 3),
        (0.0, Some(&player), 1),
        (0.0, None, 0),
    ] {
        let stacks = data
            .roll_container_with(
                "test:context",
                &mut LegacyRandom::new(37),
                ContainerLootContext {
                    luck,
                    entity,
                    biome: None,
                },
            )
            .unwrap();
        assert_eq!(stacks.iter().map(|s| s.count).sum::<i32>(), expected);
        assert!(stacks.iter().all(|s| s.item == "minecraft:diamond"));
    }
}
