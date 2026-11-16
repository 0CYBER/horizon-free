//! Enchanting what mobs spawn with (`Mob.populateDefaultEquipmentEnchantments`
//! through the `mob_spawn_equipment` provider) and the raid buffs
//! (`Raider.applyRaidBuffs`).
use super::Game;

#[cfg(test)]
#[path = "tests/drowned_spawn_tests.rs"]
mod drowned_spawn_tests;
use horizon_client::{
    mobs::{
        drops::{self, SLOTS},
        finalize::equip_stack,
        gear::{self, MAIN},
        spawner::equipment_table,
    },
    raid::{self, RaiderType},
};
use horizon_core::random::RandomSource;
use {horizon_core::nbt::Tag, horizon_world::loot::EntityLootParams};

/// `EnchantmentsByCostWithDifficulty` of `mob_spawn_equipment`.
const MIN_COST: i32 = 5;
const MAX_COST_SPAN: f32 = 17.0;

/// The mobs that call `populateDefaultEquipmentEnchantments` when they spawn.
impl Game<'_> {
    /// `Mob.equip(EquipmentTable)` for a spawner's `equipment`: the loot table
    /// (the mob as `this`, its random sequence) is rolled and each stack goes
    /// in the slot `EquipmentUser.resolveSlot` picks, with the table's drop
    /// chance for that slot.
    pub(super) fn equip_from_table(&mut self, id: u32, table: &Tag) {
        let Some((name, chances)) = equipment_table(table) else {
            return;
        };
        let Some(loot) = self.server_loot.get() else {
            return;
        };
        let Some(m) = self.mobs.iter().find(|m| m.id == id) else {
            return;
        };
        if !loot.has_table(&name) {
            return;
        }
        let params = EntityLootParams {
            this: drops::loot_entity(m, None),
            ..EntityLootParams::default()
        };
        let seed = self.level.seed;
        let stacks = match loot.random_sequence(&name) {
            Some(sequence) => {
                let random = self.rand_sequences.get(sequence, seed);
                loot.roll_entity(&name, random, &params)
            }
            None => loot.roll_entity(&name, &mut self.world.random, &params),
        };
        let mut inserted: Vec<usize> = Vec::new();
        for stack in stacks.iter().filter(|s| !s.is_empty()) {
            // `resolveSlot`: an `equippable` item goes in its own slot, any
            // other in the main hand, each slot once.
            let slot = match self
                .res
                .items()
                .get(&stack.item)
                .and_then(|i| i.equip_slot.as_deref())
            {
                Some(name) => SLOTS.iter().position(|s| *s == name),
                None => Some(0),
            };
            let Some(slot) = slot.filter(|s| !inserted.contains(s)) else {
                continue;
            };
            if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                equip_stack(m, slot, stack, chances[slot]);
            }
            inserted.push(slot);
        }
    }
    /// `EnchantmentsByCostWithDifficulty.enchant` on the stack the mob holds
    /// in a `finalize` slot: nothing is drawn when the slot is empty.
    pub(super) fn enchant_item(&mut self, id: u32, slot: usize, special: f32) {
        let Some(stack) = self
            .mobs
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| horizon_client::mobs::finalize::item_stack(m, slot))
        else {
            return;
        };
        let random = &mut self.world.random;
        let top = MIN_COST + (special * MAX_COST_SPAN) as i32;
        let cost = random
            .next_int_between_inclusive(MIN_COST, top)
            .unwrap_or(MIN_COST);
        let e = &world_stations!(self).enchantments;
        let pool = e.tag("minecraft:on_mob_spawn_equipment");
        let picked = e.select(world_items!(self), random, &stack, cost, pool);
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            let slot = horizon_client::mobs::finalize::gear_slot(slot);
            for (enchantment, level) in picked {
                gear::upgrade(m, slot, &enchantment, level);
            }
        }
    }
    /// `Pillager` / `Vindicator.applyRaidBuffs`: Quick Charge on a fresh
    /// crossbow after waves 3 and 5, Sharpness on a fresh axe.
    pub(super) fn apply_raid_buffs(&mut self, id: u32, kind: RaiderType, wave: i32, odds: f32) {
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return;
        };
        let enchant = m.random.next_float() <= odds;
        match kind {
            RaiderType::Pillager if enchant => {
                let level = if wave > raid::num_groups(2) {
                    2
                } else if wave > raid::num_groups(1) {
                    1
                } else {
                    0
                };
                if level > 0 {
                    horizon_client::mobs::finalize::set_item(
                        m,
                        horizon_client::mobs::finalize::MAINHAND,
                        "minecraft:crossbow",
                    );
                    gear::upgrade(m, MAIN, "minecraft:quick_charge", level);
                }
            }
            RaiderType::Vindicator => {
                horizon_client::mobs::finalize::set_item(
                    m,
                    horizon_client::mobs::finalize::MAINHAND,
                    "minecraft:iron_axe",
                );
                if enchant {
                    let level = if wave > raid::num_groups(2) { 2 } else { 1 };
                    gear::upgrade(m, MAIN, "minecraft:sharpness", level);
                }
            }
            _ => {}
        }
    }
}
