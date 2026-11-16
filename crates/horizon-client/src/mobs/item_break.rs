//! `LivingEntity.breakItem` as a client sees it. A server never plays the
//! break itself: `LivingEntity.onEquippedItemBroken` broadcasts the entity
//! event `entityEventForEquipmentBreak` gives for the slot, and each client's
//! `LivingEntity.handleEntityEvent` breaks the item it still draws in that
//! slot (`getItemBySlot`), with the sound and the crumbs.
use super::Mob;
use crate::items::ItemStack;
use horizon_core::equipment_slot::EquipmentSlot;

/// `LivingEntity.handleEntityEvent` 47 to 52, 65 and 68: the slot whose
/// item broke (the inverse of `entityEventForEquipmentBreak`).
pub fn equipment_break_slot(event: i8) -> Option<EquipmentSlot> {
    Some(match event {
        47 => EquipmentSlot::MainHand,
        48 => EquipmentSlot::OffHand,
        49 => EquipmentSlot::Head,
        50 => EquipmentSlot::Chest,
        51 => EquipmentSlot::Legs,
        52 => EquipmentSlot::Feet,
        65 => EquipmentSlot::Body,
        68 => EquipmentSlot::Saddle,
        _ => return None,
    })
}

/// `LivingEntity.entityEventForEquipmentBreak`: the event whose client side
/// breaks the item in `slot` (the inverse of `equipment_break_slot`).
pub fn equipment_break_event(slot: EquipmentSlot) -> i8 {
    match slot {
        EquipmentSlot::MainHand => 47,
        EquipmentSlot::OffHand => 48,
        EquipmentSlot::Head => 49,
        EquipmentSlot::Chest => 50,
        EquipmentSlot::Legs => 51,
        EquipmentSlot::Feet => 52,
        EquipmentSlot::Body => 65,
        EquipmentSlot::Saddle => 68,
    }
}

impl Mob {
    /// `Mob.sunProtectionSlot`, overridden by zombie horses and zombie nautiluses.
    pub fn sun_protection_slot(&self) -> EquipmentSlot {
        if matches!(
            self.species(),
            super::Species::ZombieHorse | super::Species::ZombieNautilus
        ) {
            EquipmentSlot::Body
        } else {
            EquipmentSlot::Head
        }
    }
    /// `LivingEntity.getItemBySlot` for a mob or another player a server
    /// sends: the stack it wears or holds in `slot`, with the components
    /// `ClientboundSetEquipmentPacket` gave it where they are kept.
    pub fn item_by_slot(&self, slot: EquipmentSlot) -> Option<ItemStack> {
        self.equipment_stacks()[slot as usize].clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::Species;
    use horizon_core::geometry::Vec3;
    use std::sync::Arc;

    fn mob(id: &str, species: Species) -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let k = Arc::make_mut(&mut kind);
        k.id = id.into();
        k.species = species;
        Mob::new(1, kind, Vec3::ZERO, 0.0)
    }

    /// `LivingEntity.entityEventForEquipmentBreak` and its inverse agree for
    /// every slot a mob or another player can wear something in.
    #[test]
    fn the_break_event_and_its_slot_are_inverses() {
        for slot in EquipmentSlot::ALL {
            let event = equipment_break_event(slot);
            assert_eq!(equipment_break_slot(event), Some(slot), "event {event}");
        }
        assert_eq!(equipment_break_event(EquipmentSlot::Body), 65);
        assert_eq!(equipment_break_event(EquipmentSlot::Saddle), 68);
    }

    /// `LivingEntity.entityEventForEquipmentBreak`: 47 main hand, 48 off hand,
    /// 49 to 52 head to feet, 65 body, 68 saddle; other events break nothing.
    #[test]
    fn each_equipment_break_event_names_its_slot() {
        let slots = [
            (47, EquipmentSlot::MainHand),
            (48, EquipmentSlot::OffHand),
            (49, EquipmentSlot::Head),
            (50, EquipmentSlot::Chest),
            (51, EquipmentSlot::Legs),
            (52, EquipmentSlot::Feet),
            (65, EquipmentSlot::Body),
            (68, EquipmentSlot::Saddle),
        ];
        for (event, slot) in slots {
            assert_eq!(equipment_break_slot(event), Some(slot), "event {event}");
        }
        for event in [2, 3, 35, 46, 53, 54, 55, 60, 64, 66, 67, 69] {
            assert_eq!(equipment_break_slot(event), None, "event {event}");
        }
    }

    /// The stack another player (an avatar, as a mannequin) holds or wears
    /// is the one that breaks.
    #[test]
    fn a_player_breaks_what_it_holds_and_wears() {
        let mut p = mob("minecraft:player", Species::Mannequin);
        let mut pick = ItemStack::new("minecraft:diamond_pickaxe", 1);
        pick.damage = 1560;
        p.extra.set_main_hand_stack(Some(pick.clone()));
        p.extra.set_slot_id(
            horizon_items::equipment::OFFHAND,
            Some("minecraft:shield".into()),
        );
        p.extra.set_slot_id(
            horizon_items::equipment::HEAD,
            Some("minecraft:iron_helmet".into()),
        );
        let mut boots = ItemStack::new("minecraft:leather_boots", 1);
        boots.damage = 64;
        p.extra
            .set_slot_stack(horizon_items::equipment::FEET, Some(boots.clone()));
        let item = |slot| p.item_by_slot(slot).map(|s: ItemStack| (s.item, s.damage));
        assert_eq!(
            item(EquipmentSlot::MainHand),
            Some((pick.item.clone(), 1560))
        );
        assert_eq!(
            item(EquipmentSlot::OffHand),
            Some(("minecraft:shield".into(), 0))
        );
        assert_eq!(
            item(EquipmentSlot::Head),
            Some(("minecraft:iron_helmet".into(), 0))
        );
        assert_eq!(item(EquipmentSlot::Feet), Some((boots.item.clone(), 64)));
        assert_eq!(item(EquipmentSlot::Chest), None);
        assert_eq!(item(EquipmentSlot::Body), None);
        assert_eq!(item(EquipmentSlot::Saddle), None);
    }

    #[test]
    fn undead_sun_protection_reads_the_species_slot() {
        for (name, species) in [
            ("minecraft:zombie_horse", Species::ZombieHorse),
            ("minecraft:zombie_nautilus", Species::ZombieNautilus),
        ] {
            let mut m = mob(name, species);
            m.extra.set_slot_id(
                horizon_items::equipment::HEAD,
                Some("minecraft:iron_helmet".into()),
            );
            assert!(m.item_by_slot(m.sun_protection_slot()).is_none());
            m.extra
                .set_body(Some("minecraft:leather_horse_armor".into()));
            assert_eq!(
                m.item_by_slot(m.sun_protection_slot())
                    .unwrap()
                    .item
                    .as_ref(),
                "minecraft:leather_horse_armor"
            );
        }
        let mut zombie = mob("minecraft:zombie", Species::Zombie);
        zombie
            .extra
            .set_body(Some("minecraft:leather_horse_armor".into()));
        assert!(zombie.item_by_slot(zombie.sun_protection_slot()).is_none());
        zombie.extra.set_slot_id(
            horizon_items::equipment::HEAD,
            Some("minecraft:iron_helmet".into()),
        );
        assert!(zombie.item_by_slot(zombie.sun_protection_slot()).is_some());
    }

    /// A wolf's armour is its body slot, and a horse's
    /// saddle its saddle slot.
    #[test]
    fn body_armour_and_saddles_break_from_their_slots() {
        let mut wolf = mob("minecraft:wolf", Species::Wolf);
        wolf.extra.set_body(Some("minecraft:wolf_armor".into()));
        wolf.extra.set_body_damage(63);
        let armour = wolf.item_by_slot(EquipmentSlot::Body).unwrap();
        assert_eq!((&*armour.item, armour.damage), ("minecraft:wolf_armor", 63));
        assert_eq!(wolf.item_by_slot(EquipmentSlot::Head), None);

        let mut horse = mob("minecraft:horse", Species::Horse);
        horse.set_saddle(Some(ItemStack::new("minecraft:saddle", 1)));
        assert_eq!(
            horse.item_by_slot(EquipmentSlot::Saddle).map(|s| s.item),
            Some("minecraft:saddle".into())
        );
    }

    /// `Wolf.actuallyHurt`: each time the armor's crackiness level changes it
    /// plays the crack sound and sends twenty armadillo scute particles.
    #[test]
    fn a_wolfs_armor_throws_scutes_as_it_cracks() {
        use crate::mobs::MobEvent;
        let mut wolf = mob("minecraft:wolf", Species::Wolf);
        wolf.extra.set_body(Some("minecraft:wolf_armor".into()));
        // Levels change below 95%, 69% and 32% of the 64 points left, that is
        // after 4, 20 and 44 points of wear.
        let hurt = |wolf: &mut Mob, damage: f32| {
            wolf.hurt_wolf_armor(damage);
            std::mem::take(&mut wolf.outbox)
        };
        let scutes = |events: &[MobEvent]| {
            events
                .iter()
                .filter(|e| {
                    matches!(
                        e,
                        MobEvent::ItemParticles {
                            item: "minecraft:armadillo_scute",
                            at,
                            spread,
                            speed,
                            count: 20,
                        } if *at == [0.0, 1.0, 0.0] && *spread == [0.2, 0.1, 0.2] && *speed == 0.1
                    )
                })
                .count()
        };
        let cracks = |events: &[MobEvent]| {
            events
                .iter()
                .filter(|e| matches!(e, MobEvent::Sound("item.wolf_armor.crack", ..)))
                .count()
        };
        // Three points: still the first level.
        let events = hurt(&mut wolf, 3.0);
        assert_eq!((cracks(&events), scutes(&events)), (0, 0));
        // The fourth point cracks it, with the sound and the scutes together.
        let events = hurt(&mut wolf, 1.0);
        assert_eq!((cracks(&events), scutes(&events)), (1, 1));
        // Within the level: nothing.
        let events = hurt(&mut wolf, 10.0);
        assert_eq!((cracks(&events), scutes(&events)), (0, 0));
        // Over 69%: the next level.
        let events = hurt(&mut wolf, 6.0);
        assert_eq!((cracks(&events), scutes(&events)), (1, 1));
    }
}
