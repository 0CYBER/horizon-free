//! `Mob.aiStep`'s looting loop: a mob with `canPickUpLoot` takes what lies
//! within its box grown by `getPickupReach` (1, 0, 1) and wears or holds it
//! when it beats what it has (`equipItemIfPossible`), dropping the replaced
//! stack by its drop chance. The species whose `pickUpItem` does something
//! else keep their own loops (`pandas.rs`, `piglins.rs`, `villagers.rs`,
//! `allays.rs`, `sulfur_cubes.rs`).
use super::{
    Game,
    pickups::{Collector, Taken},
};
use horizon_client::{
    entities::ItemEntity,
    items::ItemStack,
    mobs::{
        Mob, Species,
        equip::{self, Facts, ItemTags, Slot},
        finalize::{self, set_drop_chance, set_item_stack, stored_drop_chance},
    },
};
use horizon_core::{geometry::Vec3, random::RandomSource};

impl Game<'_> {
    /// `Mob.aiStep`'s looting block, once per mob per tick.
    pub(super) fn tick_mob_looting(&mut self) {
        let griefing = self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING);
        if self.dropped.is_empty() || (!griefing && !self.grief_hooked()) {
            return;
        }
        let mut near = super::item_areas::ItemAreas::default();
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if !m.can_pick_up_loot()
                || !m.alive()
                || m.removed
                || !self.creatures.ticked.contains(&m.id)
                || !self.entity_grief(m.id, griefing)
            {
                continue;
            }
            // `Mob.ITEM_PICKUP_REACH`.
            let reach = m.bounds().inflate(Vec3::new(1.0, 0.0, 1.0));
            let mut scan = near.scan(&self.dropped, self.next_entity, reach);
            if scan.is_empty() {
                continue;
            }
            let (species, baby, passenger) = (m.species(), m.is_baby(), m.extra.passenger);
            let mut j = 0;
            loop {
                j = scan.next_from(j, self.dropped.len());
                let Some(e) = self.dropped.get(j) else {
                    break;
                };
                if e.pickup_delay > 0 || !reach.intersects(e.bounds()) {
                    j += 1;
                    continue;
                }
                let stack = e.stack.clone();
                let item = &stack.item;
                let tags = ItemTags {
                    spear: self.items().item_in_tag("minecraft:spears", item),
                    egg: self.items().item_in_tag("minecraft:eggs", item),
                    disliked_weapon: self
                        .items()
                        .item_in_tag("minecraft:wither_skeleton_disliked_weapons", item),
                };
                if !equip::wants_to_pick_up(species, baby, passenger, item, tags) {
                    j += 1;
                    continue;
                }
                // `Mob.pickUpItem`: nothing leaves the ground what it cannot use.
                let Some(taken) = self.equip_item_if_possible(i, &stack) else {
                    j += 1;
                    continue;
                };
                let before = self.dropped[j].clone();
                let id = self.mobs[i].id;
                self.add_pickup(Taken::Item(Box::new(before.clone())), Collector::Mob(id));
                self.mob_item_pickup_trigger(i, &before);
                let e = &mut self.dropped[j];
                e.stack.count -= taken;
                if e.stack.count == 0 {
                    self.remove_dropped(j);
                    scan.removed(j);
                } else {
                    j += 1;
                }
            }
        }
    }

    /// `Mob.equipItemIfPossible`: what `slot` would take of `stack`, and how
    /// many items that takes. `None` when the mob has no use for it.
    fn equip_item_if_possible(&mut self, index: usize, stack: &ItemStack) -> Option<u32> {
        // `getEquipmentSlotForItem`, then `isEquippableInSlot`: the stack's own `equippable`
        // component and its allowed entities, then the parts of its item.
        let worn = equip::equippable(self.items(), stack).map(|e| e.slot);
        let mut slot = equip::slot_for(worn)?;
        let m = &self.mobs[index];
        let tags = &self.stations().enchantments.entity_tags;
        let data = equip::equippable_in_slot(self.items(), tags, stack, slot.name(), &m.kind.id);
        let observed = mod_api::__private::item_listens(mod_api::HookId::CanEquip, &stack.item);
        let source = observed
            .then(|| horizon_client::mobs::equip::EquipInput::new(u64::from(m.id), stack.clone()));
        let prepared = self.equip_stack(source.as_deref().unwrap_or(stack), &[&m.kind.id], &[m.id]);
        if !self.can_equip(&prepared, slot.name(), m.id, &m.kind.id, data) {
            return None;
        }
        let mut replace = self.can_replace(index, stack, slot);
        // An armour piece that beats nothing it is worn with falls back to an
        // empty main hand.
        if slot.is_armor() && !replace {
            slot = Slot::MainHand;
            replace = self.mobs[index].extra.main_hand_id().is_none();
        }
        if !replace {
            return None;
        }
        // `EquipmentSlot.limit`: an armour slot takes one item, a hand the
        // whole stack.
        let taken = if slot.is_armor() {
            1
        } else {
            stack.count.max(1)
        };
        if !self.equip_can_apply(&prepared, self.mobs[index].id) {
            return None;
        }
        let m = &mut self.mobs[index];
        let old = finalize::item_stack(m, slot.index());
        let chance = stored_drop_chance(m, slot.index());
        // `DropChances.byEquipment` against the roll the equip makes.
        let drop_old = old.is_some() && (m.random.next_float() - 0.1).max(0.0) < chance;
        // The whole stack comes with it: its enchantments (a Sharpness V sword
        // stays sharp), damage, name and components.
        let mut equipped = stack.clone();
        equipped.count = taken;
        set_item_stack(m, slot.index(), equipped);
        // `setItemSlotAndDropWhenKilled`, then `persistenceRequired`.
        set_drop_chance(m, slot.index(), 2.0);
        m.persistent = true;
        let at = [m.position.x, m.position.y, m.position.z];
        let id = m.id;
        if drop_old && let Some(old) = old {
            self.spawn_item_own(old, at, 10);
        }
        self.mob_equip_event(id, at, true);
        Some(taken)
    }

    /// `Mob.canReplaceCurrentItem` for what `m` wears or holds in `slot`.
    fn can_replace(&self, index: usize, stack: &ItemStack, slot: Slot) -> bool {
        let m = &self.mobs[index];
        let Some(worn_stack) = finalize::item_stack(m, slot.index()) else {
            return true;
        };
        let current = worn_stack.item.clone();
        let species = m.species();
        // `Drowned.canReplaceCurrentItem` never parts with a nautilus shell.
        if species == Species::Drowned && &*current == "minecraft:nautilus_shell" {
            return false;
        }
        let new = self.stack_facts(m, stack, slot);
        let worn = self.stack_facts(m, &worn_stack, slot);
        let preferred = equip::preferred_weapon_tag(species).map(|tag| {
            (
                self.items().item_in_tag(tag, &stack.item),
                self.items().item_in_tag(tag, &current),
            )
        });
        equip::can_replace_current_item(&new, Some(&worn), slot, preferred)
    }

    /// The numbers `Mob.compareArmor` and `compareWeapons` read off `stack`
    /// in `slot` for the mob `m`.
    pub(super) fn stack_facts(&self, m: &Mob, stack: &ItemStack, slot: Slot) -> Facts {
        let (armor, toughness, attack) = self.attributes(m, stack, slot);
        Facts {
            armor,
            armor_toughness: toughness,
            attack_damage: attack,
            enchantments: stack.enchantments().len(),
            damage: stack.damage as i32,
            custom_name: stack.custom_name().is_some(),
            prevents_armor_change: world_stations!(self)
                .enchantments
                .prevents_armor_change(stack),
        }
    }

    /// `Mob.getApproximateAttributeWith` for `stack` in `slot`: each
    /// attribute starts from the mob's own value. (armour, toughness, damage)
    fn attributes(&self, m: &Mob, stack: &ItemStack, slot: Slot) -> (f64, f64, f64) {
        let modifiers = horizon_client::item_attributes::attribute_modifiers(stack, self.items());
        let modifiers = &*modifiers;
        (
            equip::attribute_with(modifiers, "minecraft:armor", f64::from(m.kind.armor), slot),
            equip::attribute_with(
                modifiers,
                "minecraft:armor_toughness",
                f64::from(m.kind.armor_toughness),
                slot,
            ),
            equip::attribute_with(
                modifiers,
                "minecraft:attack_damage",
                f64::from(m.attrs.attack_damage),
                slot,
            ),
        )
    }

    /// `LivingEntity.onItemPickup`: the advancement of whoever threw the item,
    /// whichever mob took it.
    pub(super) fn mob_item_pickup_trigger(&mut self, index: usize, e: &ItemEntity) {
        let Some(owner) = e.thrower else {
            return;
        };
        let stack = e.stack.clone();
        let target = self.mob_entity(&self.mobs[index]);
        self.for_each_player(|g| {
            if g.acting_uuid() == owner {
                g.trigger_item_entity("minecraft:thrown_item_picked_up_by_entity", &stack, &target);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// What the part of an iron helmet answers to `can_equip`: yes for one entity type only,
    /// and only while that entity stands at or above y 64, read through the `Host`.
    #[derive(Clone)]
    struct OnlyFor(&'static str, Arc<Mutex<Vec<(String, u32)>>>);
    #[mod_api::horizon_part]
    impl mod_api::ItemPart for OnlyFor {
        fn can_equip(&self, host: &dyn mod_api::Host, event: &mut mod_api::CanEquipEvent<'_>) {
            let entity = event.entity.entity();
            (self.1.lock().unwrap()).push((event.slot.to_owned(), entity.id()));
            let standing = entity.position(host).is_some_and(|[_, y, _]| y >= 64.0);
            event.can_equip = event.kind == self.0 && standing;
        }
    }
    struct Hatter(OnlyFor);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Hatter {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let helmet = r.item_key("minecraft:iron_helmet");
            r.item_part(helmet, self.0.clone());
        }
    }

    /// `Mob.equipItemIfPossible` asks `isEquippableInSlot` first, and `IItemExtension.canEquip`
    /// is that answer: a part of the iron helmet that accepts only skeletons keeps it off a
    /// zombie that would take it, and puts it on a skeleton, unless the skeleton stands lower
    /// than the part allows. Items without a part follow their component.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_can_equip_part_decides_which_mob_wears_a_picked_up_item() {
        use horizon_core::geometry::Vec3;

        let asked = Arc::default();
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "hatter",
            Box::new(Hatter(OnlyFor("minecraft:skeleton", Arc::clone(&asked)))),
        )]
        .into();
        crate::entry::install_client_hooks();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let mobs = [
                    (1, "minecraft:zombie", 64.0),
                    (2, "minecraft:skeleton", 64.0),
                    (3, "minecraft:skeleton", 60.0),
                ];
                for (id, name, y) in mobs {
                    let kind = game.res.mob_types().get(name).unwrap().clone();
                    game.mobs
                        .push(Mob::new(id, kind, Vec3::new(0.5, y, 0.5), 0.0));
                }
                let worn = |game: &Game<'_>, index: usize, slot: Slot| {
                    finalize::item(&game.mobs[index], slot.index()).map(|item| item.to_string())
                };
                let helmet = ItemStack::new("minecraft:iron_helmet", 1);
                crate::game::equip_answers::take_equip_conversions();
                // The zombie is refused, and nothing it was asked about changed.
                assert_eq!(game.equip_item_if_possible(0, &helmet), None);
                assert_eq!(worn(game, 0, Slot::Head), None);
                assert_eq!(worn(game, 0, Slot::MainHand), None);
                assert_eq!(
                    std::mem::take(&mut *asked.lock().unwrap()),
                    [("head".to_owned(), 1)]
                );
                // The skeleton is accepted.
                assert_eq!(game.equip_item_if_possible(1, &helmet), Some(1));
                assert_eq!(
                    worn(game, 1, Slot::Head).as_deref(),
                    Some("minecraft:iron_helmet")
                );
                assert_eq!(
                    std::mem::take(&mut *asked.lock().unwrap()),
                    [("head".to_owned(), 2)]
                );
                // A skeleton standing below y 64 is refused: the part reads where it is.
                assert_eq!(game.equip_item_if_possible(2, &helmet), None);
                assert_eq!(worn(game, 2, Slot::Head), None);
                assert_eq!(
                    std::mem::take(&mut *asked.lock().unwrap()),
                    [("head".to_owned(), 3)]
                );
                assert_eq!(
                    crate::game::equip_answers::take_equip_conversions(),
                    3,
                    "one preparation per mob pickup attempt"
                );
                // No part for a diamond helmet: the zombie takes it as vanilla.
                let diamond = ItemStack::new("minecraft:diamond_helmet", 1);
                assert_eq!(game.equip_item_if_possible(0, &diamond), Some(1));
                assert_eq!(
                    worn(game, 0, Slot::Head).as_deref(),
                    Some("minecraft:diamond_helmet")
                );
                assert!(asked.lock().unwrap().is_empty());
                assert_eq!(crate::game::equip_answers::take_equip_conversions(), 0);
            });
        });
    }

    /// `getEquipmentSlotForItem` and `isEquippableInSlot` read the stack's own `equippable`, with
    /// no part installed: a zombie holds an iron helmet whose component is removed in its main
    /// hand, wears one overridden for the feet on its feet, and takes none whose
    /// `allowed_entities` leaves zombies out, an explicitly empty list included.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mob_reads_the_picked_up_stacks_own_equippable() {
        use horizon_core::{geometry::Vec3, nbt::Tag};

        let helmet = |fields: Option<Vec<(&str, Tag)>>| {
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
            let (id, value) = match fields {
                Some(fields) => ("minecraft:equippable", Tag::compound(fields)),
                None => ("!minecraft:equippable", Tag::Compound(Default::default())),
            };
            helmet.data_mut().set_other(id, Some(value));
            helmet
        };
        let slot = |name: &str| ("slot", Tag::String(name.into()));
        let allowed = |list: Vec<Tag>| ("allowed_entities", Tag::List(list));
        crate::game::native_entity_refs::tests::with_game(|game| {
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            for id in 1..=4 {
                let at = Vec3::new(0.5 + f64::from(id) * 4.0, 64.0, 0.5);
                game.mobs.push(Mob::new(id, kind.clone(), at, 0.0));
            }
            let worn = |game: &Game<'_>, index: usize| {
                [Slot::MainHand, Slot::Head, Slot::Feet]
                    .into_iter()
                    .find(|slot| finalize::item(&game.mobs[index], slot.index()).is_some())
            };
            assert_eq!(game.equip_item_if_possible(0, &helmet(None)), Some(1));
            assert_eq!(worn(game, 0), Some(Slot::MainHand));
            let feet = helmet(Some(vec![slot("feet")]));
            assert_eq!(game.equip_item_if_possible(1, &feet), Some(1));
            assert_eq!(worn(game, 1), Some(Slot::Feet));
            let skeletons = vec![Tag::String("skeleton".into())];
            let skeletons = helmet(Some(vec![slot("head"), allowed(skeletons)]));
            assert_eq!(game.equip_item_if_possible(2, &skeletons), None);
            assert_eq!(worn(game, 2), None);
            let nobody = helmet(Some(vec![slot("head"), allowed(Vec::new())]));
            assert_eq!(game.equip_item_if_possible(3, &nobody), None);
            assert_eq!(worn(game, 3), None);
        });
    }
}
