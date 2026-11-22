use super::*;
use crate::mobs::Species;

#[test]
fn carrier_inventory_is_adopted_and_saved_from_the_model() {
    let stack = Tag::compound([
        ("id", string("minecraft:stone")),
        ("count", Tag::Int(3)),
        (
            "components",
            Tag::compound([(
                "minecraft:custom_data",
                Tag::compound([("audit", Tag::Int(137))]),
            )]),
        ),
    ]);
    let offers = Tag::compound([(
        "Recipes",
        Tag::List(vec![Tag::compound([
            (
                "buy",
                Tag::compound([("id", string("minecraft:emerald")), ("count", Tag::Int(1))]),
            ),
            (
                "sell",
                Tag::compound([("id", string("minecraft:dirt")), ("count", Tag::Int(2))]),
            ),
        ])]),
    )]);
    for (species, id) in [
        (Species::Pillager, "minecraft:pillager"),
        (Species::WanderingTrader, "minecraft:wandering_trader"),
        (Species::Piglin, "minecraft:piglin"),
    ] {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        Arc::make_mut(&mut kind).id = id.into();
        Arc::make_mut(&mut kind).species = species;
        let types = HashMap::from([(id.to_string(), kind)]);
        for input in [
            None,
            Some(Tag::List(vec![])),
            Some(Tag::List(vec![stack.clone()])),
        ] {
            let mut raw = Tag::compound([
                ("id", string(id)),
                ("Pos", doubles(&[1.5, 200.0, 4.5])),
                ("CustomName", string("kept")),
                ("Tags", Tag::List(vec![string("kept")])),
            ]);
            if let Some(input) = input.clone() {
                put(&mut raw, "Inventory", input);
            }
            if species == Species::WanderingTrader {
                put(&mut raw, "Offers", offers.clone());
                put(&mut raw, "DespawnDelay", Tag::Int(48000));
            }
            if species == Species::Pillager {
                put(&mut raw, "PatrolLeader", Tag::Byte(1));
                put(&mut raw, "CanJoinRaid", Tag::Byte(0));
            }
            let mut mob = mob_from_nbt(&raw, &types, 1, None).unwrap();
            let has_stack = matches!(&input, Some(Tag::List(v)) if !v.is_empty());
            assert_eq!(
                mob.extra.inventory.len(),
                usize::from(has_stack),
                "{id} adoption"
            );
            let expected = Tag::List(if has_stack {
                vec![stack.clone()]
            } else {
                vec![]
            });
            let mut stale = raw.clone();
            put(
                &mut stale,
                "Inventory",
                Tag::List(vec![Tag::compound([
                    ("id", string("minecraft:dirt")),
                    ("count", Tag::Int(2)),
                ])]),
            );
            for base in [None, Some(&raw), Some(&stale)] {
                let saved = mob_to_nbt(&mob, [137, 0, 0, 1], base, None, |_| None);
                assert_eq!(saved.get("Inventory"), Some(&expected), "{id} save");
                if base.is_some() {
                    assert_eq!(saved.get("CustomName"), raw.get("CustomName"));
                    assert_eq!(saved.get("Tags"), raw.get("Tags"));
                }
                if species == Species::WanderingTrader {
                    assert_eq!(saved.get("Offers"), Some(&offers));
                    assert_eq!(saved.get("DespawnDelay"), Some(&Tag::Int(48000)));
                }
                if species == Species::Pillager {
                    assert_eq!(saved.get("PatrolLeader"), Some(&Tag::Byte(1)));
                    assert_eq!(saved.get("CanJoinRaid"), Some(&Tag::Byte(0)));
                }
            }
            if has_stack {
                mob.extra.inventory[0].count = 4;
                let mut changed = stack.clone();
                put(&mut changed, "count", Tag::Int(4));
                assert_eq!(
                    mob_to_nbt(&mob, [137, 0, 0, 1], Some(&raw), None, |_| None).get("Inventory"),
                    Some(&Tag::List(vec![changed]))
                );
            }
            mob.extra.inventory.clear();
            assert_eq!(
                mob_to_nbt(&mob, [137, 0, 0, 1], Some(&stale), None, |_| None).get("Inventory"),
                Some(&Tag::List(vec![]))
            );
        }
    }
    // This InventoryCarrier projection does not add a pig inventory.
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    Arc::make_mut(&mut kind).id = "minecraft:pig".into();
    Arc::make_mut(&mut kind).species = Species::Pig;
    let mut pig = Mob::new(2, kind, Vec3::ZERO, 0.0);
    pig.extra
        .inventory
        .push(crate::save::item_from_nbt(&stack).unwrap());
    assert!(
        mob_to_nbt(&pig, [137, 0, 0, 2], None, None, |_| None)
            .get("Inventory")
            .is_none()
    );
}
