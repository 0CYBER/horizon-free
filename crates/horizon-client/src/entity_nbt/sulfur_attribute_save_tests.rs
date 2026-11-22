use super::*;
use horizon_world::attributes::{Modifier, Operation};

#[test]
fn the_save_writes_base_and_permanent_modifiers_and_replaces_stale_entries() {
    let mut m = crate::mobs::sulfur_cube::adopt_body_tests::cube(None);
    let map = m.extra.armor_attributes.as_deref_mut().unwrap();
    let kr = map.catalog().id("minecraft:knockback_resistance").unwrap();
    let friction = map.catalog().id("minecraft:friction_modifier").unwrap();
    map.edit(kr, |i, o| {
        i.set_base_value(0.5, o).unwrap();
        i.add_permanent(
            Modifier::new(
                "minecraft:bouncy_add_knockback_resistance",
                0.25,
                Operation::AddValue,
            )
            .unwrap(),
            o,
        )
        .unwrap();
        // The archetype's transient modifier of the same id shadows it.
        i.add_or_update_transient(
            Modifier::new(
                "minecraft:bouncy_add_knockback_resistance",
                -2.0,
                Operation::AddValue,
            )
            .unwrap(),
            o,
        )
        .unwrap();
    });
    map.edit(friction, |i, o| {
        i.add_or_update_transient(
            Modifier::new(
                "minecraft:bouncy_mul_friction_modifier",
                -0.7,
                Operation::AddMultipliedTotal,
            )
            .unwrap(),
            o,
        )
        .unwrap();
    });
    let stale =
        |id: &str| Tag::compound([("id", string(id)), ("base", Tag::Double(9.0f64.to_bits()))]);
    let mut nbt = Compound::new();
    nbt.insert(
        "attributes".into(),
        Tag::List(vec![
            stale("minecraft:max_health"),
            stale("minecraft:knockback_resistance"),
        ]),
    );
    put_sulfur_attributes(&mut nbt, &m);
    let Some(Tag::List(list)) = nbt.get(&"attributes".into()) else {
        panic!("no attributes");
    };
    let ids: Vec<_> = list.iter().map(|t| text(t.get("id")).unwrap()).collect();
    assert_eq!(
        ids,
        [
            "minecraft:max_health",
            "minecraft:knockback_resistance",
            "minecraft:friction_modifier"
        ]
    );
    let base = |t: &Tag| match t.get("base") {
        Some(Tag::Double(b)) => f64::from_bits(*b),
        other => panic!("base {other:?}"),
    };
    assert_eq!(base(&list[0]), 9.0);
    assert_eq!(base(&list[1]), 0.5);
    let Some(Tag::List(modifiers)) = list[1].get("modifiers") else {
        panic!("no modifiers");
    };
    assert_eq!(modifiers.len(), 1);
    assert_eq!(
        modifiers[0].get("amount"),
        Some(&Tag::Double(0.25f64.to_bits()))
    );
    assert_eq!(
        text(modifiers[0].get("operation")).as_deref(),
        Some("add_value")
    );
    // Friction holds only a transient modifier: its base, no modifiers.
    assert_eq!(base(&list[2]), 1.0);
    assert!(list[2].get("modifiers").is_none());
}

#[test]
fn another_species_map_is_not_saved_by_the_bridge() {
    let mut m = crate::mobs::sulfur_cube::adopt_body_tests::cube(None);
    std::sync::Arc::make_mut(&mut m.kind).species = crate::mobs::Species::Slime;
    let map = m.extra.armor_attributes.as_deref_mut().unwrap();
    let kr = map.catalog().id("minecraft:knockback_resistance").unwrap();
    map.edit(kr, |i, o| i.set_base_value(0.5, o).unwrap());
    let mut nbt = Compound::new();
    put_sulfur_attributes(&mut nbt, &m);
    assert!(!nbt.contains_key(&"attributes".into()));
}
