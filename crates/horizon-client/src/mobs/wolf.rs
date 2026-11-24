//! `Wolf`'s body armour: `Wolf.canArmorAbsorb`, the `Wolf.actuallyHurt` that
//! takes a hit instead of the wolf's health, and the `Wolf.hurtArmor` that
//! wears the piece on a hit it did not take.
use super::{Damage, DamageKind, Mob, MobEvent, Species, WOLF_ARMOR_DURABILITY};
use horizon_core::equipment_slot::EquipmentSlot;

/// `Wolf.canArmorAbsorb`: it wears wolf armor and the damage type is not in
/// `#bypasses_wolf_armor` (drowning, magic, thorns, in a wall, out of the
/// world and the untyped kills).
pub fn armor_absorbs(m: &Mob, source: &Damage) -> bool {
    m.species() == Species::Wolf
        && m.extra.body_id() == Some("minecraft:wolf_armor")
        && !matches!(
            source.kind,
            DamageKind::Drown
                | DamageKind::Magic
                | DamageKind::IndirectMagic
                | DamageKind::InWall
                | DamageKind::Thorns
                | DamageKind::OutOfWorld
                | DamageKind::Cramming
                | DamageKind::Generic
                | DamageKind::GenericKill
                | DamageKind::OutsideBorder
                | DamageKind::Starve
                | DamageKind::Wither
        )
}

/// `Wolf.hurtArmor` -> `doHurtEquipment`: the body piece wears by
/// `max(1, damage / 4)` on a hit the armour did not take, which
/// `getDamageAfterArmorAbsorb` asks for every hit outside `#bypasses_armor`.
/// `Wolf.actuallyHurt` skips `LivingEntity.actuallyHurt`, and with it that
/// call, for the hits the armour does take — [`hurt_armor`] below wears those
/// by `ceil(damage)` instead.
///
/// The wear is put on the piece here, inside the hit, as vanilla's
/// `doHurtEquipment` does: `canArmorAbsorb` reads the slot again on the next
/// hit, so a wear still owed at the end of the tick would let armour this hit
/// broke take another one.
pub fn wears_body(m: &Mob, absorbed: bool) -> bool {
    m.species() == Species::Wolf && !absorbed && m.extra.body_id() == Some("minecraft:wolf_armor")
}

/// The `Wolf.hurtArmor` wear of a hit the armour did not take:
/// `(int)Math.max(1.0F, damage / 4.0F)`. `doHurtEquipment` only wears the
/// stack: the crack sound and scutes belong to `Wolf.actuallyHurt`'s
/// absorbed branch alone.
pub fn wear_body(m: &mut Mob, damage: f32) {
    wear(m, ((damage / 4.0).max(1.0)) as i32, false);
}

/// `Wolf.actuallyHurt`: the armor loses `ceil(damage)` durability, cracks at
/// `Crackiness.WOLF_ARMOR` steps and breaks at its maximum.
pub fn hurt_armor(m: &mut Mob, damage: f32) {
    wear(m, damage.ceil() as i32, true);
}

/// `ItemStack.hurtAndBreak`: the piece loses `amount` durability, cracks at the
/// `Crackiness.WOLF_ARMOR` steps and breaks at its maximum, where
/// `LivingEntity.onEquippedItemBroken` reports the slot's break. `cracks`:
/// whether a crackiness step plays the crack sound and scutes, which only
/// `Wolf.actuallyHurt` checks.
fn wear(m: &mut Mob, amount: i32, cracks: bool) {
    let crack = |lost: i32| {
        let left = (WOLF_ARMOR_DURABILITY - lost) as f32 / WOLF_ARMOR_DURABILITY as f32;
        [0.95_f32, 0.69, 0.32].iter().filter(|t| left < **t).count()
    };
    let before = m.extra.body_damage();
    let after = before + amount;
    let at = [m.position.x, m.position.y, m.position.z];
    if after >= WOLF_ARMOR_DURABILITY {
        let broken = m.extra.body_stack().cloned();
        m.extra.set_body_stack(None);
        // `LivingEntity.onEquippedItemBroken`: the body slot's break event,
        // which each client's `breakItem` turns into the break sound and five
        // crumbs. The stack goes along: a window that is its own client has
        // already lost it from the slot.
        if let Some(stack) = broken {
            m.outbox.push(MobEvent::EquipmentBroke {
                mob: m.id,
                slot: EquipmentSlot::Body,
                stack,
                at,
            });
        }
    } else {
        m.extra.set_body_damage(after);
        if cracks && crack(before) != crack(after) {
            m.outbox
                .push(MobEvent::Sound("item.wolf_armor.crack", at, 1.0, 1.0));
            // Twenty scutes fly off, a block above the wolf's feet.
            m.outbox.push(MobEvent::ItemParticles {
                item: "minecraft:armadillo_scute",
                at: [at[0], at[1] + 1.0, at[2]],
                spread: [0.2, 0.1, 0.2],
                speed: 0.1,
                count: 20,
            });
        }
    }
}

#[cfg(test)]
#[path = "wolf_armor_tests.rs"]
mod wolf_armor_tests;
