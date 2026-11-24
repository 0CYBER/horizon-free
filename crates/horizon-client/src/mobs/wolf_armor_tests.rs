//! A wolf's body armour: a hit the armour does not take still wears it
//! (`Wolf.hurtArmor`), and a piece that breaks goes out as the body slot's
//! break event, which each client answers with `LivingEntity.breakItem`.
use super::*;
use crate::mobs::Who;
use crate::mobs::sensor_scan_tests::kind;
use horizon_core::geometry::Vec3;
use std::sync::Arc;

fn wolf() -> Mob {
    let mut kind = kind();
    let k = Arc::make_mut(&mut kind);
    k.id = "minecraft:wolf".into();
    k.species = Species::Wolf;
    let mut m = Mob::new(7, kind, Vec3::ZERO, 0.0);
    m.extra.set_body(Some("minecraft:wolf_armor".into()));
    m
}

fn thorns() -> Damage {
    Damage {
        kind: DamageKind::Thorns,
        attacker: Some(Who::Player),
        from: None,
        attacker_species: None,
    }
}

/// The wear `LivingEntity.doHurtEquipment` owes, before it divides it.
fn owed(m: &Mob) -> Option<f32> {
    m.outbox.iter().find_map(|e| match e {
        MobEvent::ArmorHurt { amount, .. } => Some(*amount),
        _ => None,
    })
}

/// `Wolf.actuallyHurt` takes a `#bypasses_wolf_armor` hit itself, but
/// `LivingEntity.actuallyHurt` still asks `getDamageAfterArmorAbsorb`, and
/// `Wolf.hurtArmor` hands `doHurtEquipment` the body slot: 24 thorns wear the
/// piece by `max(1, 24 / 4) = 6`, and the wolf keeps the hit on its health.
/// The wear is on the piece before the hit returns, as vanilla's
/// `doHurtEquipment` runs inside `actuallyHurt`.
/// A hit the armour does take skips that: `Wolf.actuallyHurt` wears it by
/// `ceil(damage)` itself.
#[test]
fn a_thorns_hit_wears_the_body_piece_it_does_not_absorb() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let mut m = wolf();
    let before = m.health;
    assert!(m.damage(level, 24.0, thorns()), "the hit landed");
    assert_eq!(m.extra.body_damage(), 6, "max(1, 24 / 4)");
    assert_eq!(
        owed(&m),
        None,
        "the host owes nothing at the end of the tick"
    );
    assert!(m.health < before, "the wolf took the hit: {}", m.health);

    let mut m = wolf();
    assert!(m.damage(level, 24.0, Damage::of(DamageKind::MobAttack)));
    assert_eq!(m.extra.body_damage(), 24, "ceil(24) on the armour itself");
    assert_eq!(
        owed(&m),
        None,
        "the armour took it, so hurtArmor is not asked"
    );
}

/// The wear a hit owes is on the piece before the next hit reads the slot:
/// 60 of 64 points and a thorns hit's six break it, so the bigger hit that
/// follows in the same tick — inside the hurt cooldown, it only adds the
/// difference — lands on the wolf instead of on armour that is gone.
#[test]
fn a_wear_that_breaks_the_piece_is_on_it_before_the_next_hit() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let mut m = wolf();
    m.health = 40.0;
    m.extra.set_body_damage(60);
    assert!(m.damage(level, 24.0, thorns()), "the thorns hit landed");
    assert_eq!(m.extra.body_id(), None, "60 + max(1, 24 / 4) breaks it");
    let before = m.health;
    assert!(
        m.damage(level, 40.0, Damage::of(DamageKind::MobAttack)),
        "the second hit of the tick landed"
    );
    assert!(
        m.health < before,
        "no armour is left to absorb it: {} -> {}",
        before,
        m.health
    );
}

/// `ItemStack.hurtAndBreak` at the piece's maximum calls
/// `LivingEntity.onEquippedItemBroken`, which reports
/// `entityEventForEquipmentBreak(BODY)` — 65 — with the stack that broke,
/// instead of a raw break sound: each client's `breakItem` plays it at 0.8
/// volume and 0.8–1.2 pitch, gated on silence, and throws five crumbs.
#[test]
fn a_broken_body_piece_sends_the_body_break_event() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let mut m = wolf();
    m.extra.set_body_damage(63);
    assert!(
        m.damage(level, 1.0, Damage::of(DamageKind::MobAttack)),
        "the hit landed"
    );
    assert_eq!(m.extra.body_id(), None, "the piece broke");
    assert_eq!(m.extra.body_damage(), 0);
    let events: Vec<_> = m
        .outbox
        .iter()
        .filter(|e| {
            matches!(
                e,
                MobEvent::EquipmentBroke {
                    mob: 7,
                    slot: horizon_core::equipment_slot::EquipmentSlot::Body,
                    stack,
                    at: [0.0, 0.0, 0.0],
                } if &*stack.item == "minecraft:wolf_armor"
            )
        })
        .collect();
    assert_eq!(
        events.len(),
        1,
        "the body slot's break event: {:?}",
        m.outbox
    );
    assert!(
        !m.outbox
            .iter()
            .any(|e| matches!(e, MobEvent::Sound("item.wolf_armor.break", ..))),
        "no raw break sound: {:?}",
        m.outbox
    );
}

/// `Wolf.hurtArmor` -> `doHurtEquipment` has no `Crackiness` check: a thorns
/// hit that wears the piece past the 95% step (4 of 64 points) plays no crack
/// sound, which only the absorbed branch of `Wolf.actuallyHurt` plays.
#[test]
fn a_hit_the_armour_does_not_take_cracks_it_silently() {
    let mut test_level = crate::health::TestLevel::new(0);
    let level = &mut test_level.level();
    let mut m = wolf();
    assert!(m.damage(level, 24.0, thorns()), "the hit landed");
    assert_eq!(m.extra.body_damage(), 6, "past the first crack step");
    assert!(
        !m.outbox
            .iter()
            .any(|e| matches!(e, MobEvent::Sound("item.wolf_armor.crack", ..))),
        "no crack sound: {:?}",
        m.outbox
    );
}
