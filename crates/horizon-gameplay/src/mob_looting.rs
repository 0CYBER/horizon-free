//! `Mob.aiStep`'s looting loop: a mob with `canPickUpLoot` takes what lies
//! within its box grown by `getPickupReach` (1, 0, 1) and wears or holds it
//! when it beats what it has (`equipItemIfPossible`), dropping the replaced
//! stack by its drop chance. The species whose `pickUpItem` does something
//! else keep their own loops (`pandas.rs`, `piglins.rs`, `villagers.rs`,
//! `allays.rs`, `sulfur_cubes.rs`).
use crate::{item_areas::ItemAreas, mob_level::MobLevel};
use horizon_client::{
    items::{ItemStack, Items},
    mobs::{
        Mob, Species,
        equip::{self, Facts, ItemTags, Slot},
        finalize::{self, set_drop_chance, set_item_stack, stored_drop_chance},
    },
    stations::StationData,
};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// What the looting code calls on its level beyond [`MobLevel`]: the game's own
/// methods for the replace check and the attribute numbers, which mods target. Each runs
/// the function of this module of the same name.
pub trait LootingLevel: MobLevel {
    /// `Mob.canReplaceCurrentItem` ([`can_replace`]).
    fn can_replace(&self, index: usize, stack: &ItemStack, slot: Slot) -> bool;
    /// `Mob.getApproximateAttributeWith` ([`attributes`]).
    fn attributes(&self, m: &Mob, stack: &ItemStack, slot: Slot) -> (f64, f64, f64);
}

/// `Mob.aiStep`'s looting block, once per mob per tick.
pub fn tick_mob_looting<L: MobLevel + ?Sized>(l: &mut L) {
    let griefing = l.mob_griefing();
    if l.parts().dropped.is_empty() || (!griefing && !l.grief_hooked()) {
        return;
    }
    let mut near = ItemAreas::default();
    for i in 0..l.parts().mobs.len() {
        let p = l.parts();
        let m = &p.mobs[i];
        if !m.can_pick_up_loot() || !m.alive() || m.removed || !p.ticked.contains(&m.id) {
            continue;
        }
        let id = m.id;
        if !l.entity_grief(id, griefing) {
            continue;
        }
        let p = l.parts();
        let m = &p.mobs[i];
        // `Mob.ITEM_PICKUP_REACH`.
        let reach = m.bounds().inflate(Vec3::new(1.0, 0.0, 1.0));
        let mut scan = near.scan(p.dropped, p.next_entity, reach);
        if scan.is_empty() {
            continue;
        }
        let (species, baby, passenger) = (m.species(), m.is_baby(), m.extra.passenger);
        let mut j = 0;
        loop {
            let p = l.parts();
            j = scan.next_from(j, p.dropped.len());
            let Some(e) = p.dropped.get(j) else {
                break;
            };
            if e.pickup_delay > 0 || !reach.intersects(e.bounds()) {
                j += 1;
                continue;
            }
            let stack = e.stack.clone();
            let item = &stack.item;
            let tags = ItemTags {
                spear: p.items.item_in_tag("minecraft:spears", item),
                egg: p.items.item_in_tag("minecraft:eggs", item),
                disliked_weapon: p
                    .items
                    .item_in_tag("minecraft:wither_skeleton_disliked_weapons", item),
            };
            if !equip::wants_to_pick_up(species, baby, passenger, item, tags) {
                j += 1;
                continue;
            }
            // `Mob.pickUpItem`: nothing leaves the ground what it cannot use.
            let Some(taken) = l.equip_item_if_possible(i, &stack) else {
                j += 1;
                continue;
            };
            let p = l.parts();
            let before = p.dropped[j].clone();
            let id = p.mobs[i].id;
            l.take(id, before.clone());
            l.on_item_pickup(i, &before);
            let e = &mut l.parts().dropped[j];
            e.stack.count -= taken;
            if e.stack.count == 0 {
                l.discard_item(j);
                scan.removed(j);
            } else {
                j += 1;
            }
        }
    }
}

/// `Mob.equipItemIfPossible`: what `slot` would take of `stack`, and how
/// many items that takes. `None` when the mob has no use for it.
pub fn equip_item_if_possible<L: LootingLevel + ?Sized>(
    l: &mut L,
    index: usize,
    stack: &ItemStack,
) -> Option<u32> {
    // `getEquipmentSlotForItem`, then `isEquippableInSlot`: the stack's own `equippable`
    // component and its allowed entities, then the parts of its item.
    let p = l.parts();
    let worn = equip::equippable(p.items, stack).map(|e| e.slot);
    let mut slot = equip::slot_for(worn)?;
    let m = &p.mobs[index];
    let tags = &p.stations.enchantments.entity_tags;
    let data = equip::equippable_in_slot(p.items, tags, stack, slot.name(), &m.kind.id);
    let prepared = l.equip_prepare(index, stack);
    if !l.equip_allows(&prepared, index, slot.name(), data) {
        return None;
    }
    let mut replace = l.can_replace(index, stack, slot);
    let p = l.parts();
    // An armour piece that beats nothing it is worn with falls back to an
    // empty main hand.
    if slot.is_armor() && !replace {
        slot = Slot::MainHand;
        replace = p.mobs[index].extra.main_hand_id().is_none();
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
    let id = p.mobs[index].id;
    if !l.equip_can_apply(&prepared, id) {
        return None;
    }
    let m = &mut l.parts().mobs[index];
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
        l.spawn_item_own(old, at, 10);
    }
    l.on_equip_item(id, at, true);
    Some(taken)
}

/// `Mob.canReplaceCurrentItem` for what the mob `index` wears or holds in `slot`.
pub fn can_replace<L: MobLevel + ?Sized>(
    l: &L,
    items: &Items,
    index: usize,
    stack: &ItemStack,
    slot: Slot,
) -> bool {
    let m = &l.mobs()[index];
    let Some(worn_stack) = finalize::item_stack(m, slot.index()) else {
        return true;
    };
    let current = worn_stack.item.clone();
    let species = m.species();
    // `Drowned.canReplaceCurrentItem` never parts with a nautilus shell.
    if species == Species::Drowned && &*current == "minecraft:nautilus_shell" {
        return false;
    }
    let new = l.stack_facts(m, stack, slot);
    let worn = l.stack_facts(m, &worn_stack, slot);
    let preferred = equip::preferred_weapon_tag(species).map(|tag| {
        (
            items.item_in_tag(tag, &stack.item),
            items.item_in_tag(tag, &current),
        )
    });
    equip::can_replace_current_item(&new, Some(&worn), slot, preferred)
}

/// The numbers `Mob.compareArmor` and `compareWeapons` read off `stack`
/// in `slot` for the mob `m`.
pub fn stack_facts<L: LootingLevel + ?Sized>(
    l: &L,
    stations: &StationData,
    m: &Mob,
    stack: &ItemStack,
    slot: Slot,
) -> Facts {
    let (armor, toughness, attack) = l.attributes(m, stack, slot);
    Facts {
        armor,
        armor_toughness: toughness,
        attack_damage: attack,
        enchantments: stack.enchantments().len(),
        damage: stack.damage as i32,
        custom_name: stack.custom_name().is_some(),
        prevents_armor_change: stations.enchantments.prevents_armor_change(stack),
    }
}

/// `Mob.getApproximateAttributeWith` for `stack` in `slot`: each
/// attribute starts from the mob's own value. (armour, toughness, damage)
pub fn attributes(items: &Items, m: &Mob, stack: &ItemStack, slot: Slot) -> (f64, f64, f64) {
    let modifiers = horizon_client::item_attributes::attribute_modifiers(stack, items);
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
