use super::sensor_scan_tests::kind;
use super::*;
use horizon_core::geometry::Vec3;

fn horse() -> Mob {
    mob("minecraft:horse", Species::Horse)
}

fn mob(id: &str, species: Species) -> Mob {
    let mut kind = kind();
    let k = Arc::make_mut(&mut kind);
    k.id = id.into();
    k.species = species;
    Mob::new(1, kind, Vec3::ZERO, 0.0)
}

#[test]
fn equipment_stacks_reads_full_offhand() {
    let mut horse = horse();
    let mut stack = crate::items::ItemStack::new("minecraft:shield", 3);
    stack.damage = 17;
    finalize::set_item_stack(&mut horse, finalize::OFFHAND, stack.clone());
    assert_eq!(horse.equipment_stacks()[1], Some(stack));
}

#[test]
fn removing_body_equipment_keeps_the_whole_stack() {
    let mut horse = horse();
    let mut stack = crate::items::ItemStack::new("minecraft:red_carpet", 3);
    stack.damage = 7;
    stack.data_mut().custom_name = Some(horizon_core::nbt::Tag::String("kept".into()));
    horse.extra.set_body_stack(Some(stack.clone()));
    assert_eq!(horse.take_body_equipment(), Some(stack));
    assert_eq!(horse.take_body_equipment(), None);
}

#[test]
fn replacing_armor_with_same_item_forgets_old_stack() {
    let mut horse = horse();
    let mut old = crate::items::ItemStack::new("minecraft:iron_helmet", 1);
    old.damage = 8;
    horse.set_equipment_stack(5, Some(old));
    let mut replacement = crate::items::ItemStack::new("minecraft:iron_helmet", 2);
    replacement.damage = 3;
    finalize::set_item_stack(&mut horse, finalize::HEAD, replacement.clone());
    assert_eq!(horse.equipment_stacks()[5], Some(replacement));
}

#[test]
fn saddle_stack_keeps_count() {
    let mut horse = horse();
    let stack = crate::items::ItemStack::new("minecraft:saddle", 3);
    horse.set_saddle(Some(stack.clone()));
    assert_eq!(horse.saddle_item(), Some(stack));
}

/// `Entity.hurt` on a client level does nothing: the horse a rider's
/// client drives takes no fall of its own, and the server's health stays
/// (#15294: the client's copy died and lay on the ground, still ridable).
#[test]
fn a_client_copy_takes_no_fall_damage() {
    let context = crate::test_resources::server_loop_context();
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let mut m = horse();
    m.extra.client_side = true;
    let health = m.health;
    let mut events = Vec::new();
    assert!(!m.damage(level, 20.0, Damage::of(DamageKind::Fall)));
    m.cause_fall_damage_at(
        context,
        level,
        30.0,
        1.0,
        &mut events,
        None,
        DamageKind::Fall,
    );
    assert_eq!(m.health, health);
    assert_eq!(m.hurt_time, 0);
    assert!(m.alive());
    // The server's copy of the same fall is hurt.
    let mut server = horse();
    server.cause_fall_damage_at(
        context,
        level,
        30.0,
        1.0,
        &mut events,
        None,
        DamageKind::Fall,
    );
    assert!(server.health < health);
}

/// `LivingEntity.hurtArmor` (`doHurtEquipment`): a hit that lands on a mob
/// wearing body armour leaves the piece owing a wear, which the host
/// applies with the item's own maximum and fire rules.
#[test]
fn a_hurt_horse_reports_the_body_armour_wear() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let hit = Damage {
        kind: DamageKind::MobAttack,
        attacker: None,
        from: None,
        attacker_species: None,
    };
    let mut m = horse();
    m.extra
        .set_body(Some("minecraft:diamond_horse_armor".into()));
    assert!(m.damage(level, 6.0, hit), "the hit landed");
    let owed = m.outbox.iter().find_map(|e| match e {
        MobEvent::ArmorHurt { amount, .. } => Some(*amount),
        _ => None,
    });
    assert!(
        owed.is_some_and(|amount| amount > 0.0),
        "the worn armour owes a wear: {owed:?}"
    );
    // A horse with no body armour owes nothing.
    let mut bare = horse();
    assert!(bare.damage(level, 6.0, hit));
    assert!(
        !bare
            .outbox
            .iter()
            .any(|e| matches!(e, MobEvent::ArmorHurt { .. }))
    );
}

/// `Horse` is the only mob that overrides `LivingEntity.hurtArmor` (the
/// base one is empty, `LivingEntity.java:1887`), so a donkey wearing a body
/// item — `/item replace entity @e[type=minecraft:donkey,limit=1]
/// armor.body with minecraft:wolf_armor` — wears nothing.
#[test]
fn a_donkey_wearing_a_body_item_wears_nothing() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let mut donkey = mob("minecraft:donkey", Species::Donkey);
    donkey.extra.set_body(Some("minecraft:wolf_armor".into()));
    assert!(
        donkey.damage(level, 6.0, Damage::of(DamageKind::MobAttack)),
        "the hit landed"
    );
    assert!(
        !donkey
            .outbox
            .iter()
            .any(|e| matches!(e, MobEvent::ArmorHurt { .. })),
        "only a horse's own override wears its body piece"
    );
}

/// Vanilla holds a head item and a body item at once on the mobs with a
/// body slot (`LivingEntity.setItemSlot` over `EntityEquipment`): the
/// packet keeps the helmet on 5 and the armour on 6, `/item` reads each
/// back, and the armour value is the body piece's.
#[test]
fn a_horse_holds_a_helmet_and_its_body_armour_at_once() {
    use horizon_core::equipment_slot::EquipmentSlot;
    let mut m = horse();
    m.extra.set_slot_id(
        horizon_items::equipment::HEAD,
        Some("minecraft:iron_helmet".into()),
    );
    m.extra
        .set_body(Some("minecraft:diamond_horse_armor".into()));
    let gear = m.equipment_stacks();
    assert_eq!(
        gear[5].as_ref().map(|s| s.item.as_ref().to_owned()),
        Some("minecraft:iron_helmet".to_owned())
    );
    assert_eq!(
        gear[6].as_ref().map(|s| s.item.as_ref().to_owned()),
        Some("minecraft:diamond_horse_armor".to_owned())
    );
    let slot = |s| m.item_by_slot(s).map(|i| i.item.as_ref().to_owned());
    assert_eq!(
        slot(EquipmentSlot::Head),
        Some("minecraft:iron_helmet".to_owned())
    );
    assert_eq!(
        slot(EquipmentSlot::Body),
        Some("minecraft:diamond_horse_armor".to_owned())
    );
    // Death keeps both (`Mob.dropCustomDeathLoot` by slot).
    let body = crate::mobs::drops::slot_item(&m, "body").unwrap();
    let head = crate::mobs::drops::slot_item(&m, "head").unwrap();
    assert_eq!(body.item, "minecraft:diamond_horse_armor");
    assert_eq!(head.item, "minecraft:iron_helmet");
}

/// `getDamageAfterArmorAbsorb` asks `hurtArmor` only when the damage type
/// is not in `#bypasses_armor` (`LivingEntity.java:1907-1909`), so a horse
/// that falls keeps its body piece whole.
#[test]
fn a_fall_does_not_wear_the_body_piece() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let mut m = horse();
    m.extra
        .set_body(Some("minecraft:diamond_horse_armor".into()));
    assert!(
        m.damage(level, 6.0, Damage::of(DamageKind::Fall)),
        "the fall landed"
    );
    assert!(
        !m.outbox
            .iter()
            .any(|e| matches!(e, MobEvent::ArmorHurt { .. })),
        "a #bypasses_armor hit asks for no wear"
    );
}

/// The wear is measured by the damage `hurtArmor` is handed, which is the
/// one the armour, toughness and enchantment reductions have not touched
/// yet: 8 raw damage is `8 / 4 = 2` durability even though plate leaves far
/// less than 8 through to the health.
#[test]
fn the_wear_is_the_damage_before_the_armour_reduction() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let mut m = horse();
    let k = Arc::make_mut(&mut m.kind);
    k.armor = 20.0;
    m.extra
        .set_body(Some("minecraft:diamond_horse_armor".into()));
    let before = m.health;
    assert!(m.damage(level, 8.0, Damage::of(DamageKind::MobAttack)));
    assert!(m.health < before, "the hit reached the health");
    let owed = m.outbox.iter().find_map(|e| match e {
        MobEvent::ArmorHurt { amount, .. } => Some(*amount),
        _ => None,
    });
    assert_eq!(
        owed,
        Some(8.0),
        "the pre-reduction damage, not the damage the health took"
    );
}
