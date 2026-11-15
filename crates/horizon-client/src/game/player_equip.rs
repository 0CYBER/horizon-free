//! `LivingEntity.onEquipItem` for players: the equip sound and the equip
//! game event when an armour or off-hand slot takes a different item, from
//! every path that calls it (use, the inventory menu, the creative screen,
//! the swap-hands key, dispensers), for the host and for LAN guests.
use super::{Game, equipment_modifier_applies};
use horizon_client::items::ItemStack;

/// The equipment slots `onEquipItem` can act on, as `EquipmentSlot` names:
/// the armour (`Player.doesEmitEquipEvent`: these emit the game event) and
/// the off hand.
const ARMOR: [&str; 4] = ["head", "chest", "legs", "feet"];
const OFFHAND: &str = "offhand";

/// `ItemStack.isSameItemSameComponents` over optional stacks.
fn same(a: Option<&ItemStack>, b: Option<&ItemStack>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.same_item_same_components(b),
        _ => false,
    }
}

impl Game<'_> {
    /// The armour (head first) and the off hand, in `EquipmentSlot` order of
    /// the inventory menu's slots.
    pub(super) fn player_equipment(&self) -> [Option<ItemStack>; 5] {
        let [head, chest, legs, feet] = self.armor.clone();
        [head, chest, legs, feet, self.offhand.clone()]
    }

    /// `onEquipItem` for every slot that changed since `before`: what the
    /// server does for the creative screen, which sends each changed slot
    /// (`handleSetCreativeModeSlot` sets it by player).
    pub(super) fn player_equipment_changed(&mut self, before: &[Option<ItemStack>; 5]) {
        let after = self.player_equipment();
        for (i, slot) in ARMOR.into_iter().chain([OFFHAND]).enumerate() {
            self.player_on_equip(slot, before[i].as_ref(), after[i].as_ref());
        }
    }

    /// `LivingEntity.onEquipItem(slot, old, new)` for the player in place
    /// (the host, or a guest inside `as_guest`).
    pub(super) fn player_on_equip(
        &mut self,
        slot: &str,
        old: Option<&ItemStack>,
        new: Option<&ItemStack>,
    ) {
        if !same(old, new) {
            self.player_equipped(slot, new);
        }
    }

    /// The body of `onEquipItem` once the item changed: the item's
    /// `Equippable.equipSound` when it is equipped in its own slot, and the
    /// `equip` / `unequip` game event for armour. Nothing on a client (the
    /// server plays it) or for a spectator. The player's `firstTick` never
    /// sees an equip here: loading a save sets the slots directly.
    pub(super) fn player_equipped(&mut self, slot: &str, new: Option<&ItemStack>) {
        if self.remote.is_some() || self.spectator() {
            return;
        }
        self.native_on_equip(slot, new);
        let Some(at) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        let at = [at.x, at.y, at.z];
        let equippable = new.and_then(|stack| {
            let equippable = horizon_client::item_sync::equippable(stack, self.items())?;
            Some((
                equippable.slot == slot,
                horizon_client::item_sync::equip_sound(stack, self.items()),
            ))
        });
        if let Some((true, sound)) = &equippable {
            let sound = sound
                .as_deref()
                .unwrap_or("minecraft:item.armor.equip_generic");
            // Player.getSoundSource: PLAYERS.
            let players = horizon_client::settings::SOUND_SOURCES
                .iter()
                .position(|s| *s == "player")
                .unwrap_or(0);
            self.audio.play_on(players, sound, Some(at), 1.0, 1.0);
        }
        if ARMOR.contains(&slot) {
            let event = if equippable.is_some() {
                "minecraft:equip"
            } else {
                "minecraft:unequip"
            };
            let ctx = self.player_context(None);
            self.post_game_event(event, at, ctx);
        }
    }

    /// `LivingEntity.detectEquipmentUpdates` runs in the player's tick: until a guest's tick has seen
    /// the item it switched to (`attack_item`), its attributes keep the last item's modifiers, so a
    /// swap and a spear's jab in one tick charge on the old item's attack speed (#15524).
    pub(super) fn held_item_ticked(&self) -> bool {
        super::held_items::held_attack_item(&self.inventory, self.selected)
            == self.attack_item.as_ref()
    }

    /// Equipment attribute modifiers (LivingEntity.collectEquipmentChanges).
    pub(super) fn apply_equipment(&mut self) {
        let Some(player) = self.player.as_mut() else {
            return;
        };
        for (attribute, id) in std::mem::take(&mut self.applied_modifiers) {
            let _ = player.set_modifier_unclamped(&attribute, &id, 0.0, "add_value", false);
        }
        if self.remote.is_some() {
            for (attr, id, _, op) in std::mem::take(&mut self.potions.enchant_modifiers) {
                let _ = player.set_modifier_unclamped(&attr, &id, 0.0, &op, false);
            }
            return;
        }
        let slots = ["head", "chest", "legs", "feet"];
        let mut equipped: Vec<(Option<&ItemStack>, &str)> = self
            .armor
            .iter()
            .zip(slots)
            .map(|(s, n)| (s.as_ref(), n))
            .collect();
        equipped.push((self.inventory[self.selected].as_ref(), "mainhand"));
        equipped.push((self.offhand.as_ref(), "offhand"));
        for (stack, slot) in equipped {
            // ItemStack.forEachModifier: the stack's own ATTRIBUTE_MODIFIERS
            // (its item's unless a command or a mod replaced them, #14641).
            let Some(stack) = stack else {
                continue;
            };
            for (attribute, id, amount, op, group) in world_items!(self).attribute_modifiers(stack)
            {
                if !equipment_modifier_applies(&group, slot) {
                    continue;
                }
                if player
                    .set_modifier_unclamped(&attribute, &id, amount, &op, true)
                    .is_ok()
                {
                    self.applied_modifiers.push((attribute, id));
                }
            }
        }
        let _ = player.clamp_health_to_max();
        self.apply_enchantment_attributes();
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_pos_guest_tests::game;
    use super::*;
    use horizon_protocol::packets::play::{self, Serverbound as S};

    /// #15524: a guest that switches from its empty hand (a 5-tick charge, which the server's 5 ticks
    /// of tolerance always meet) to a spear and jabs in the same tick charges on the hand's attack
    /// speed, as on vanilla's server, where `detectEquipmentUpdates` gives it the spear's modifiers
    /// only in its next tick.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_swap_and_a_jab_in_one_tick_charge_on_the_last_item() {
        let mut game = game();
        let mut host = game.lan.take().unwrap();
        host.hub.guests[0].inventory[1] = Some(ItemStack::new("minecraft:iron_spear", 1));
        // It has played a tick with its empty hand.
        game.tick_guests(&mut host);
        let swap = S::SetCarriedItem(play::SetCarriedItem { slot: 1 });
        game.lan_guest_packet(&mut host, 2, swap);
        let charge = |game: &mut Game<'_>, host: &mut super::super::lan::LanHost| {
            game.as_guest(host, 2, |g| {
                (
                    g.main_hand_attack_speed(),
                    !g.cannot_attack_with_item(g.attack_ticks as i32, 5),
                )
            })
            .unwrap()
        };
        assert_eq!(
            charge(&mut game, &mut host),
            (4.0, true),
            "the hand's charge"
        );
        // Its tick sees the spear (and starts its charge again): the spear's own speed counts.
        game.tick_guests(&mut host);
        let (speed, _) = charge(&mut game, &mut host);
        assert!(speed < 4.0, "the spear's attack speed: {speed}");
    }
    /// The other side of #15524: what a guest switches to or puts on applies by its next tick, as
    /// vanilla's `detectEquipmentUpdates` does: a mace's and an iron axe's attack speed (4 plus the
    /// `float` modifiers `-3.4F` and `-3.1F`), Efficiency V's mining efficiency (5² + 1) and a diamond
    /// chestplate's 8 armour, worn in the tick it switched hands or on its own (vanilla 26.3's data).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn what_a_guest_switches_to_applies_by_its_next_tick() {
        let mut pickaxe = ItemStack::new("minecraft:diamond_pickaxe", 1);
        pickaxe.data_mut().enchantments = vec![("minecraft:efficiency".into(), 5)];
        let chestplate = || Some(ItemStack::new("minecraft:diamond_chestplate", 1));
        // (what slot 1 holds, armour put on with the swap, swaps, the attribute, before its tick, after)
        let cases = [
            (
                Some(ItemStack::new("minecraft:mace", 1)),
                None,
                true,
                "minecraft:attack_speed",
                Some(4.0),
                4.0 + f64::from(-3.4f32),
            ),
            (
                Some(ItemStack::new("minecraft:iron_axe", 1)),
                None,
                true,
                "minecraft:attack_speed",
                Some(4.0),
                4.0 + f64::from(-3.1f32),
            ),
            (
                Some(pickaxe),
                None,
                true,
                "minecraft:mining_efficiency",
                Some(0.0),
                26.0,
            ),
            (
                Some(ItemStack::new("minecraft:stick", 1)),
                chestplate(),
                true,
                "minecraft:armor",
                Some(0.0),
                8.0,
            ),
            (None, chestplate(), false, "minecraft:armor", None, 8.0),
        ];
        let read = |game: &mut Game<'_>, host: &mut super::super::lan::LanHost, attribute: &str| {
            game.as_guest(host, 2, |g| {
                g.player.as_mut().unwrap().attribute(attribute).unwrap()
            })
            .unwrap()
        };
        for (held, worn, swaps, attribute, before, after) in cases {
            let mut game = game();
            let mut host = game.lan.take().unwrap();
            host.hub.guests[0].inventory[1] = held;
            // It has played a tick with its empty hand and nothing on.
            game.tick_guests(&mut host);
            host.hub.guests[0].armor[1] = worn; // the chest (head, chest, legs, feet)
            if swaps {
                let swap = S::SetCarriedItem(play::SetCarriedItem { slot: 1 });
                game.lan_guest_packet(&mut host, 2, swap);
            }
            if let Some(before) = before {
                let value = read(&mut game, &mut host, attribute);
                assert!(
                    (value - before).abs() < 1e-9,
                    "{attribute} before its tick: {value}"
                );
            }
            game.tick_guests(&mut host);
            let value = read(&mut game, &mut host, attribute);
            assert!(
                (value - after).abs() < 1e-9,
                "{attribute} after its tick: {value}"
            );
        }
    }
}
