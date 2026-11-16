use super::*;

#[test]
fn loot_removed_enchantable_preserves_random() {
    let mut items = crate::enchant::tests::sample_items();
    items.enchantments = crate::enchant::tests::sample().into();
    let props = horizon_world::chunk::BlockProperties::new();
    let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
        "name":"minecraft:diamond_sword","modifier":[
        {"type":"minecraft:set_components","components":{"!minecraft:enchantable":{}}},
        {"type":"minecraft:enchant_with_levels","levels":30}]}]}]});
    let mut random = LegacyRandom::new(11);
    let mut expected = random.clone();
    let drops = ctx(&items, None, &props, "").evaluate(&table, &mut random);
    assert_eq!(drops.len(), 1);
    assert!(drops[0].enchantments().is_empty());
    assert_eq!(random.next_long(), expected.next_long());
}

#[test]
fn plain_sword_control() {
    let mut items = crate::enchant::tests::sample_items();
    items.enchantments = crate::enchant::tests::sample().into();
    let props = horizon_world::chunk::BlockProperties::new();
    let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
        "name":"minecraft:diamond_sword","modifier":[
        {"type":"minecraft:enchant_with_levels","levels":30}]}]}]});
    let drops = ctx(&items, None, &props, "").evaluate(&table, &mut LegacyRandom::new(11));
    assert_eq!(
        drops[0].enchantments(),
        &[
            (Arc::from("minecraft:sharpness"), 3),
            (Arc::from("minecraft:unbreaking"), 3)
        ]
    );
}

/// Loot `enchant_with_levels` selects inside the roll, holding its random: it reads the
/// `is_primary_item_for` answers stored on the snapshot it rolls with.
#[test]
fn loot_selection_reads_the_snapshots_primary_answers() {
    let mut items = crate::enchant::tests::sample_items();
    let enchantments = crate::enchant::tests::sample();
    let sharpness: Arc<str> = "minecraft:sharpness".into();
    enchantments.primary_answers().install(
        [(
            Arc::from("minecraft:diamond_sword"),
            [(sharpness.clone(), false)].into_iter().collect(),
        )]
        .into_iter()
        .collect(),
    );
    items.enchantments = enchantments.into();
    let props = horizon_world::chunk::BlockProperties::new();
    let table = json!({"pools":[{"rolls":1,"entries":[{"type":"minecraft:item",
        "name":"minecraft:diamond_sword","modifier":[
        {"type":"minecraft:enchant_with_levels","levels":30}]}]}]});
    let drops = ctx(&items, None, &props, "").evaluate(&table, &mut LegacyRandom::new(11));
    // The same roll as `plain_sword_control`, without sharpness to choose.
    assert_eq!(drops.len(), 1);
    assert!(
        drops[0]
            .enchantments()
            .iter()
            .all(|(id, _)| *id != sharpness)
    );
    assert!(!drops[0].enchantments().is_empty());
}
