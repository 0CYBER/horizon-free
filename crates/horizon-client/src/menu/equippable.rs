//! `DataComponents.EQUIPPABLE` as a menu reads it from a stack: its slot and the entity types that
//! may wear it (`LivingEntity.isEquippableInSlot`), then the parts of its item
//! (`IItemExtension.canEquip`).
use super::{Env, Menu};
use crate::items::ItemStack;
use crate::mobs::equip::equippable_in_slot;

/// The equipment slots of `Role::Armor`, by `EquipmentSlot.getIndex` (feet first).
pub(super) const ARMOR_SLOTS: [&str; 4] = ["feet", "legs", "chest", "head"];

pub(super) struct EquipPlacement {
    pub(super) allowed: bool,
    pub(super) max: u32,
    menu: super::MutationStamp,
    source: Option<crate::mobs::equip::EquipStamp>,
    observed: bool,
}
impl EquipPlacement {
    pub(super) fn can_apply(&self, menu: &Menu, env: &Env, stack: &ItemStack) -> bool {
        if !self.allowed {
            return false;
        }
        if !self.observed {
            return true;
        }
        self.menu == menu.mutation_stamp()
            && self.source.is_some()
            && self.source == crate::mobs::equip::source_stamp(stack)
            && match (env.can_equip, env.player_entity_id) {
                (Some(parts), Some(entity)) => parts.can_apply(stack, entity as u32),
                _ => false,
            }
    }
}
impl Menu {
    pub(super) fn equip_placement(
        &self,
        env: &Env,
        slot: usize,
        stack: &ItemStack,
    ) -> EquipPlacement {
        let observed = matches!(self.state.slots[slot].role, super::Role::Armor(_))
            && env.player_entity_id.is_some()
            && env.can_equip.is_some_and(|parts| parts.listens(stack));
        let menu = self.mutation_stamp();
        let source = observed
            .then(|| crate::mobs::equip::source_stamp(stack))
            .flatten();
        let (allowed, max) = if self.state.slots[slot].role == super::Role::Hidden {
            (false, 0)
        } else {
            self.slot_rules(
                env,
                slot,
                stack,
                self.vanilla_may_place(env, slot, stack),
                self.vanilla_slot_max(slot, stack),
            )
        };
        EquipPlacement {
            menu,
            source,
            observed,
            allowed,
            max: self.limit_slot_max(slot, max),
        }
    }
    /// Apply the backing container's bound without asking placement again.
    pub(super) fn limit_slot_max(&self, s: usize, max: u32) -> u32 {
        let slot = self.state.slots[s];
        if slot.container == super::MAIN {
            self.state
                .container_limits
                .get(slot.index)
                .copied()
                .flatten()
                .map_or(max, |limit| max.min(limit))
        } else {
            max
        }
    }

    /// `ArmorSlot.mayPlace`: `LivingEntity.isEquippableInSlot` for the player of `env`, armour
    /// slot `eq` of [`ARMOR_SLOTS`]. The stack's `equippable` component answers first and the
    /// parts of its item may change that (`IItemExtension.canEquip`, asked through the
    /// `env.can_equip` the game lent the click).
    pub(super) fn armor_may_place(env: &Env, eq: usize, stack: &ItemStack) -> bool {
        let tags = &env.enchantments.entity_tags;
        let data = equippable_in_slot(env.items, tags, stack, ARMOR_SLOTS[eq], "minecraft:player");
        match (env.can_equip, env.player_entity_id) {
            (Some(parts), Some(player)) => {
                if !parts.listens(stack) {
                    return data;
                }
                let ask = |stack: &ItemStack| {
                    parts.prepare(stack, player as u32, "minecraft:player")
                        && parts.can_equip(
                            stack,
                            ARMOR_SLOTS[eq],
                            player as u32,
                            "minecraft:player",
                            data,
                        )
                };
                match crate::mobs::equip::source_stamp_at(stack as *const _ as usize) {
                    Some(Some(_)) => ask(stack),
                    Some(None) => false,
                    None => {
                        let source = crate::mobs::equip::EquipInput::new(0, stack.clone());
                        ask(&source)
                    }
                }
            }
            _ => data,
        }
    }

    /// A mount's equipment slot `slot` (`body`, `saddle`): `LivingEntity.isEquippableInSlot`
    /// for the mount the menu belongs to.
    pub(super) fn mount_may_place(&self, env: &Env, slot: &str, stack: &ItemStack) -> bool {
        let tags = &env.enchantments.entity_tags;
        (self.state.mount.as_deref())
            .is_some_and(|mount| equippable_in_slot(env.items, tags, stack, slot, mount))
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::tests::{stack, with_menu};
    use super::super::{Input, Kind};
    use horizon_core::nbt::Tag;

    /// A stone whose `equippable` component has `fields`.
    pub(in crate::menu) fn equippable(fields: Vec<(&str, Tag)>) -> Option<crate::items::ItemStack> {
        let mut stone = stack("stone", 1)?;
        stone
            .data_mut()
            .set_other("minecraft:equippable", Some(Tag::compound(fields)));
        Some(stone)
    }

    /// A menu reads a stack's `EQUIPPABLE.slot`: a patched stone can go to
    /// the head slot and is equipped there.
    #[test]
    fn a_menu_reads_a_stacks_equippable_slot() {
        with_menu(Kind::Inventory, |env, menu| {
            let stone = equippable(vec![("slot", Tag::String("head".into()))]);
            menu.set_slot(env, 9, stone);
            menu.click(env, 9, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(5).map(|s| &*s.item), Some("minecraft:stone"));
            assert_eq!(menu.slot(9), None);
            assert_eq!(menu.equipped.last().map(|(s, _)| *s), Some("head"));
        });
    }

    /// `allowed_entities` comes from the same stack component: a head item
    /// only a horse may wear stays out of the player's head slot.
    #[test]
    fn a_stacks_allowed_entities_keep_it_off_the_player() {
        with_menu(Kind::Inventory, |env, menu| {
            let stone = equippable(vec![
                ("slot", Tag::String("head".into())),
                ("allowed_entities", Tag::String("horse".into())),
            ]);
            menu.set_slot(env, 9, stone);
            menu.click(env, 9, 0, Input::QuickMove).unwrap();
            assert_eq!(menu.slot(5), None);
        });
    }

    /// An iron helmet whose `equippable` is removed (`fields` None) or replaced by one with
    /// `fields`.
    fn helmet(fields: Option<Vec<(&str, Tag)>>) -> Option<crate::items::ItemStack> {
        let mut helmet = stack("iron_helmet", 1)?;
        let (id, value) = match fields {
            Some(fields) => ("minecraft:equippable", Tag::compound(fields)),
            None => ("!minecraft:equippable", Tag::Compound(Default::default())),
        };
        helmet.data_mut().set_other(id, Some(value));
        Some(helmet)
    }

    /// The armour slots read the stack's own component before the item's: a removed one keeps
    /// an iron helmet off the head, an override for the feet moves it there, and an explicitly
    /// empty `allowed_entities` allows nobody while a component with none allows the player.
    #[test]
    fn the_armour_slots_read_the_stacks_own_equippable() {
        with_menu(Kind::Inventory, |env, menu| {
            let head = |slot: &str| ("slot", Tag::String(slot.into()));
            // Menu slots 5 to 8 are the head, chest, legs and feet.
            let takes = |stack: Option<crate::items::ItemStack>| {
                let stack = stack.unwrap();
                (5..=8)
                    .filter(|s| menu.may_place(env, *s, &stack))
                    .collect::<Vec<_>>()
            };
            assert_eq!(takes(stack("iron_helmet", 1)), [5]);
            assert_eq!(takes(helmet(None)), [] as [usize; 0]);
            assert_eq!(takes(helmet(Some(vec![head("feet")]))), [8]);
            let nobody = ("allowed_entities", Tag::List(Vec::new()));
            assert_eq!(
                takes(helmet(Some(vec![head("head"), nobody]))),
                [] as [usize; 0]
            );
            let player = ("allowed_entities", Tag::String("player".into()));
            assert_eq!(takes(helmet(Some(vec![head("head"), player]))), [5]);
        });
    }

    /// What a lent `CanEquip` was asked, answering the opposite of the data.
    #[derive(Default)]
    struct Contrary(std::cell::RefCell<Vec<(String, u32, String, bool)>>);
    impl crate::mobs::equip::CanEquip for Contrary {
        fn can_equip(
            &self,
            _: &crate::items::ItemStack,
            slot: &str,
            entity: u32,
            kind: &str,
            data: bool,
        ) -> bool {
            (self.0.borrow_mut()).push((slot.to_owned(), entity, kind.to_owned(), data));
            !data
        }
    }

    /// `ArmorSlot.mayPlace` asks the `CanEquip` the game lent the click, for the menu's player
    /// with the component's answer, and takes its answer either way; with none lent, the
    /// component decides.
    #[test]
    fn the_armour_slots_ask_the_lent_can_equip() {
        with_menu(Kind::Inventory, |env, menu| {
            let helmet = stack("iron_helmet", 1).unwrap();
            let parts = Contrary::default();
            let lent = super::Env {
                can_equip: Some(&parts),
                player_entity_id: Some(41),
                ..*env
            };
            assert!(!menu.may_place(&lent, 5, &helmet));
            assert!(menu.may_place(&lent, 8, &helmet));
            let player =
                |slot: &str, data| (slot.to_owned(), 41, "minecraft:player".to_owned(), data);
            assert_eq!(
                *parts.0.borrow(),
                [player("head", true), player("feet", false)]
            );
            assert!(menu.may_place(env, 5, &helmet));
            assert!(!menu.may_place(env, 8, &helmet));
        });
    }
}
