//! Durability lost by what the player wears and holds: armour hurt with its wearer
//! (`wear_armor`, `hurt_armor`, `hurt_equipment`, `hurt_player_equipment`) and the held stack's wear
//! (`damage_held`, `damage_held_in_slot`, `hurt_held_without_breaking`).
use super::*;
use horizon_client::motion::MotionEvent;

impl Game<'_> {
    /// Run equipment continuations with the acting player installed on the real host.
    /// The source ID belongs to the world's damage catalog throughout the hit.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn hurt_player_equipment(
        &mut self,
        source: horizon_world::damage_source::DamageTypeId,
        amount: f32,
        blocking: Option<horizon_client::health::Blocking>,
        living_cause: bool,
        causing: i32,
        creative_source: bool,
        attack: &horizon_client::health::Attack,
        armor_weapon: Option<horizon_client::health::ArmorWeapon>,
    ) -> horizon_client::Result<horizon_client::health::BlockOutcome> {
        let rules = self.world.health_rules;
        let helmet = self.armor[0]
            .as_ref()
            .is_some_and(|s| s.count > 0 && &*s.item != "minecraft:air");
        let mut hit = self
            .player
            .as_mut()
            .ok_or("missing damage target")?
            .start_hurt(
                crate::game::entity::damage_level_of!(self, attack),
                &self.health_resources,
                rules,
                source,
                amount,
                blocking,
                living_cause,
                causing,
                creative_source,
                helmet,
                !self.combat.refuse_player_hit,
                armor_weapon,
            )?;
        if let Some(damage) = hit.helmet_damage() {
            let hit_source = hit.source(attack);
            self.hurt_helmet(&hit_source, damage);
            self.refresh_armor_protection();
        }
        let armor_damage = self
            .player
            .as_mut()
            .ok_or("missing damage target")?
            .select_equipment_hurt(
                crate::game::entity::damage_level_of!(self, attack),
                &self.health_resources,
                rules,
                &mut hit,
            )?;
        if let Some(damage) = armor_damage {
            let hit_source = hit.source(attack);
            self.hurt_armor(&hit_source, damage);
            self.refresh_armor_protection();
        }
        self.player
            .as_mut()
            .ok_or("missing damage target")?
            .finish_hurt(
                crate::game::entity::damage_level_of!(self, attack),
                &self.health_resources,
                rules,
                hit,
            )
    }
    /// The armor hits among a tick's motion events (`Player.hurtArmor` from
    /// `getDamageAfterArmorAbsorb`), whatever the damage: a mob, an explosion,
    /// a cactus, lava, a campfire.
    pub(in crate::game) fn wear_armor(&mut self, events: &[MotionEvent]) {
        for event in events {
            if let MotionEvent::Health(horizon_client::health::HealthEvent::ArmorHurt {
                source,
                causing,
                direct,
                position,
                amount,
            }) = event
            {
                let Some(name) = self
                    .health_resources
                    .damage
                    .definition(*source)
                    .map(|d| d.name.clone())
                else {
                    continue;
                };
                let source = horizon_client::health::HurtSource {
                    damage_type: &name,
                    causing: *causing,
                    direct: *direct,
                    position: *position,
                };
                self.hurt_armor(&source, *amount);
            }
        }
    }
    /// `Player.hurtArmor`: every armor slot wears.
    pub(in crate::game) fn hurt_armor(
        &mut self,
        source: &horizon_client::health::HurtSource<'_>,
        damage: f32,
    ) {
        self.hurt_equipment(source, damage, &[3, 2, 1, 0]);
    }

    /// `Player.hurtHelmet`: the head slot alone wears.
    pub(in crate::game) fn hurt_helmet(
        &mut self,
        source: &horizon_client::health::HurtSource<'_>,
        damage: f32,
    ) {
        self.hurt_equipment(source, damage, &[0]);
    }

    /// `LivingEntity.doHurtEquipment` of the `slots` of `armor`: each worn piece that
    /// wears when its wearer is hurt loses max(1, damage / 4) durability, except
    /// to a fire source when the piece resists fire (`ItemStack.canBeHurtBy`).
    pub(in crate::game) fn hurt_equipment(
        &mut self,
        source: &horizon_client::health::HurtSource<'_>,
        damage: f32,
        slots: &[usize],
    ) {
        if damage <= 0.0 {
            return;
        }
        let amount = if damage.is_nan() {
            0
        } else {
            (damage / 4.0).max(1.0) as i32
        };
        if amount <= 0 {
            return;
        }
        let amount = amount as u32;
        let name = source.damage_type;
        // The pieces that may wear (`doHurtEquipment`'s checks).
        let mut eligible: [Option<ItemStack>; 4] = std::array::from_fn(|i| {
            let stack = self.armor[i].clone().filter(|_| slots.contains(&i))?;
            let info = world_items!(self).get(&stack.item)?;
            let component = |id| stack.data().and_then(|data| data.other(id));
            // ItemStack.isDamageableItem: component removal and
            // unbreakable are checked before enchantment RNG or hooks.
            if stack.count == 0
                || !horizon_client::item_sync::damage_on_hurt(&stack, info.equip_damage_on_hurt)
                || component("minecraft:unbreakable").is_some()
                || component("!minecraft:max_damage").is_some()
                || component("!minecraft:damage").is_some()
                || !self.stack_can_be_hurt_by(&stack, name).unwrap_or(true)
            {
                return None;
            }
            let max = match component("minecraft:max_damage") {
                Some(horizon_core::nbt::Tag::Int(value)) => u32::try_from(*value).ok()?,
                _ => info.max_damage,
            };
            if max == 0 {
                return None;
            }
            Some(stack)
        });
        // No equipment can wear: there is no armor damage event to dispatch.
        if eligible.iter().all(Option::is_none) {
            return;
        }
        // ArmorHurtEvent: once for the pieces that would wear, a server player's only.
        let mut amounts = [amount; 4];
        if mod_api::__private::listened(mod_api::HookId::ArmorHurt)
            && let (Some(uuid), Some(player)) = (self.lan_acting_uuid, self.lan_acting_entity)
        {
            let mut entries: Vec<mod_api::ArmorHurt> = eligible
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    Some(mod_api::ArmorHurt {
                        slot: armor_slot(i),
                        stack: e.as_ref()?.try_into_api().ok()?,
                        damage: durability_damage(damage),
                        new_damage: durability_damage(damage),
                    })
                })
                .collect();
            let mut cancel = false;
            self.native_armor_hurt(
                Some(uuid),
                player,
                "minecraft:player",
                source,
                &mut entries,
                &mut cancel,
            );
            if cancel {
                return;
            }
            for (i, amount) in amounts.iter_mut().enumerate() {
                *amount = answered_wear(&entries, armor_slot(i), *amount);
            }
        }
        // Each break is applied before the next slot's hook and RNG draw.
        for &index in slots {
            let Some(before) = eligible[index].take() else {
                continue;
            };
            if amounts[index] == 0 || self.armor[index].as_ref() != Some(&before) {
                continue;
            }
            let (updated, broken) =
                self.damage_stack(&before, amounts[index].min(i32::MAX as u32) as i32, true);
            if updated.as_ref() != Some(&before) && self.armor[index].as_ref() == Some(&before) {
                self.armor[index] = updated;
                if let Some(stack) = broken {
                    self.player_equipment_broke(armor_slot(index), &stack);
                    self.apply_equipment();
                }
            }
        }
    }
    pub(super) fn damage_held(&mut self, amount: u32) {
        self.damage_held_in_slot(
            amount,
            horizon_core::equipment_slot::EquipmentSlot::MainHand,
        );
    }
    /// `InteractionHand.asEquipmentSlot`: the slot of the hand that acts, whose
    /// break (`LivingEntity.onEquippedItemBroken`) the caller reports.
    pub(super) fn acting_hand_slot(&self) -> horizon_core::equipment_slot::EquipmentSlot {
        match self.hand {
            Hand::Main => horizon_core::equipment_slot::EquipmentSlot::MainHand,
            Hand::Off => horizon_core::equipment_slot::EquipmentSlot::OffHand,
        }
    }
    /// Hurt the acting hand's stack through ordinary wear/criterion/break handling,
    /// reporting its break in the caller's explicit equipment slot.
    pub(super) fn damage_held_in_slot(
        &mut self,
        amount: u32,
        break_slot: horizon_core::equipment_slot::EquipmentSlot,
    ) {
        let Some(stack) = self.held().cloned() else {
            return;
        };
        let (updated, broken) = self.damage_stack(&stack, amount.min(i32::MAX as u32) as i32, true);
        if self.held() != Some(&stack) {
            return;
        }
        self.edit_block_use_stack(|slot| {
            if let Some(original) = slot.as_mut() {
                if let Some(updated) = &updated {
                    original.damage = original
                        .damage
                        .saturating_add(updated.damage.saturating_sub(stack.damage))
                        .saturating_sub(stack.damage.saturating_sub(updated.damage));
                    original.count = original
                        .count
                        .saturating_sub(stack.count.saturating_sub(updated.count));
                    if original.count == 0 {
                        *slot = None;
                    }
                } else {
                    *slot = None;
                }
            }
        });
        if let Some(broken) = broken {
            self.player_equipment_broke(break_slot, &broken);
        }
    }
    /// `ItemStack.hurtWithoutBreaking`: Unbreaking applies, but the stack stops at
    /// one point of durability left instead of breaking.
    pub(super) fn hurt_held_without_breaking(&mut self, amount: u32) {
        if self.creative {
            return;
        }
        let Some(held) = self.held().cloned() else {
            return;
        };
        let amount = self.wear(&held, amount);
        let max = self.trident_max_damage(&held);
        if amount == 0 || max == 0 {
            return;
        }
        self.edit_block_use_stack(|slot| {
            if let Some(stack) = slot.as_mut() {
                stack.damage = (stack.damage + amount).min(max - 1);
            }
        });
    }
}
