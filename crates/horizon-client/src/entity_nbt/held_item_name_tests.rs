use super::*;

#[test]
fn horse_head_and_body_survive_save_load_and_drop_independently() {
    use crate::mobs::drops::{self, Death, Drop};
    use horizon_world::loot::{LootData, PackOverlay};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() || !root.join("data").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<_, _> = data
        .types
        .into_iter()
        .map(|(id, kind)| (id, Arc::new(kind)))
        .collect();
    let mut zombie = Mob::new(3, Arc::clone(&types["minecraft:zombie"]), Vec3::ZERO, 0.0);
    let saddle = crate::items::ItemStack::new("minecraft:saddle", 3);
    zombie.set_saddle(Some(saddle.clone()));
    let mut offhand = crate::items::ItemStack::new("minecraft:shield", 2);
    offhand.damage = 11;
    offhand.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("Shield".into()));
    crate::mobs::finalize::set_item_stack(
        &mut zombie,
        crate::mobs::finalize::OFFHAND,
        offhand.clone(),
    );
    let saved = mob_to_nbt(&zombie, [0; 4], None, None, |_| None);
    let zombie = mob_from_nbt(&saved, &types, 4, None).unwrap();
    assert_eq!(zombie.saddle_item(), Some(saddle));
    assert_eq!(zombie.equipment_stacks()[1], Some(offhand));
    let mut horse = Mob::new(1, Arc::clone(&types["minecraft:horse"]), Vec3::ZERO, 0.0);
    horse.extra.set_slot_id(
        horizon_items::equipment::HEAD,
        Some("minecraft:iron_helmet".into()),
    );
    let mut armor = crate::items::ItemStack::new("minecraft:diamond_horse_armor", 3);
    armor.damage = 7;
    armor.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("Body".into()));
    horse.extra.set_body_stack(Some(armor.clone()));
    let stacks = horse.equipment_stacks();
    assert_eq!(
        stacks[5].as_ref().unwrap().item.as_ref(),
        "minecraft:iron_helmet"
    );
    assert_eq!(
        stacks[6].as_ref().unwrap().item.as_ref(),
        "minecraft:diamond_horse_armor"
    );
    let raw = mob_to_nbt(&horse, [1, 2, 3, 4], None, None, |_| None);
    let mut horse = mob_from_nbt(&raw, &types, 2, None).unwrap();
    assert_eq!(horse.extra.head_id(), Some("minecraft:iron_helmet"));
    assert_eq!(horse.extra.body_id(), Some("minecraft:diamond_horse_armor"));
    assert_eq!(horse.equipment_stacks()[6], Some(armor.clone()));
    let mut chances = [0.0; 8];
    chances[drops::SLOTS.iter().position(|s| *s == "head").unwrap()] = 2.0;
    chances[drops::SLOTS.iter().position(|s| *s == "body").unwrap()] = 2.0;
    horse.extra.drop_chances = Some(Box::new(chances));
    let defaults = horizon_world::item_defaults::read(&root).unwrap();
    let loot = LootData::load(&root, &PackOverlay::new(&[]), &defaults).unwrap();
    let death = Death {
        loot: &loot,
        damage: None,
        player: None,
        player_killed: false,
        luck: 0.0,
        vehicle: None,
        mob_drops: true,
        world_seed: 0,
    };
    let (items, _) = drops::die(
        &mut horse,
        &death,
        &mut crate::random_sequences::RandomSequences::default(),
        &mut horizon_core::random::LevelRandom::for_level(horizon_core::random::LegacyRandom::new(
            7,
        )),
    );
    assert!(
        items
            .iter()
            .any(|drop| matches!(drop, Drop::Stack(s) if s == &armor))
    );
    let ids: Vec<_> = items
        .into_iter()
        .filter_map(|drop| match drop {
            Drop::Item(s) => Some(s.item.to_string()),
            Drop::Stack(s) => Some(s.item.to_string()),
            _ => None,
        })
        .collect();
    assert!(ids.iter().any(|id| id == "minecraft:iron_helmet"));
    assert!(ids.iter().any(|id| id == "minecraft:diamond_horse_armor"));
}

#[test]
fn a_worn_piece_keeps_its_damage_and_name_through_the_save() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<_, _> = data
        .types
        .into_iter()
        .map(|(id, kind)| (id, Arc::new(kind)))
        .collect();
    let mut mob = Mob::new(1, Arc::clone(&types["minecraft:zombie"]), Vec3::ZERO, 0.0);
    let mut helmet = crate::items::ItemStack::new("minecraft:iron_helmet", 1);
    helmet.damage = 40;
    helmet.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("Hat".into()));
    helmet.data_mut().trim = Some(("minecraft:gold".into(), "minecraft:coast".into()));
    assert!(mob.set_equipment_stack(5, Some(helmet.clone())));
    assert!(mob.set_equipment_stack(0, Some(helmet)));
    let raw = mob_to_nbt(&mob, [1, 2, 3, 4], None, None, |_| None);
    let loaded = mob_from_nbt(&raw, &types, 2, None).unwrap();
    let worn = crate::mobs::finalize::item_stack(&loaded, crate::mobs::finalize::HEAD).unwrap();
    assert_eq!(worn.damage, 40);
    assert_eq!(worn.custom_name().as_deref(), Some("Hat"));
    let held = crate::mobs::finalize::item_stack(&loaded, crate::mobs::finalize::MAINHAND).unwrap();
    assert_eq!(
        held.data().unwrap().trim,
        Some(("minecraft:gold".into(), "minecraft:coast".into()))
    );
    assert_eq!(
        worn.data().unwrap().trim,
        Some(("minecraft:gold".into(), "minecraft:coast".into()))
    );
    // Replacing the item forgets the old stack.
    let mut loaded = loaded;
    crate::mobs::finalize::set_item(
        &mut loaded,
        crate::mobs::finalize::HEAD,
        "minecraft:iron_helmet",
    );
    let fresh = crate::mobs::finalize::item_stack(&loaded, crate::mobs::finalize::HEAD).unwrap();
    assert_eq!(fresh.damage, 0);
    assert!(fresh.custom_name().is_none());
}

#[test]
fn replacing_a_named_item_does_not_restore_its_old_name_on_save() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/horizon-client")
        .join(horizon_core::VANILLA_VERSION);
    if !root.join("items.json").exists() {
        return;
    }
    let data = crate::mobs::MobTypes::load(&root).unwrap();
    let types: HashMap<_, _> = data
        .types
        .into_iter()
        .map(|(id, kind)| (id, Arc::new(kind)))
        .collect();
    let mut mob = Mob::new(1, Arc::clone(&types["minecraft:zombie"]), Vec3::ZERO, 0.0);
    let mut sword = crate::items::ItemStack::new("minecraft:diamond_sword", 1);
    sword.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("Old Sword".into()));
    mob.extra.set_main_hand_stack(Some(sword));
    let raw = mob_to_nbt(&mob, [1, 2, 3, 4], None, None, |_| None);
    let mut loaded = mob_from_nbt(&raw, &types, 2, None).unwrap();
    assert_eq!(
        loaded
            .extra
            .main_hand_stack()
            .unwrap()
            .custom_name()
            .as_deref(),
        Some("Old Sword")
    );
    loaded
        .extra
        .set_main_hand(Some("minecraft:diamond_sword".into()));
    let saved = mob_to_nbt(&loaded, [1, 2, 3, 4], Some(&raw), None, |_| None);
    let restored = mob_from_nbt(&saved, &types, 3, None).unwrap();
    assert_eq!(
        restored.extra.main_hand_id(),
        Some("minecraft:diamond_sword")
    );
    assert!(
        restored
            .extra
            .main_hand_stack()
            .unwrap()
            .custom_name()
            .is_none()
    );
}
