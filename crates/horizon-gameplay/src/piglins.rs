//! The game side of piglins (`PiglinAi`): spotting and picking up gold,
//! bartering when the admiring is over, and the hand-over of currency.
use crate::piglin_level::PiglinLevel;
use horizon_client::{
    first_person::SwingAnimation,
    items::ItemStack,
    mobs::{
        Mob, Species,
        equip::{self, Slot},
        finalize::{
            OFFHAND, SLOT_NAMES, item_stack, set_drop_chance, set_item_stack,
            set_slot_enchantments, stored_drop_chance,
        },
        piglin::{ADMIRE, eat, has_eaten_recently},
    },
};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// Whether `item` is in the item tag `tag` (the game's own tables).
pub fn item_in_tag<L: PiglinLevel + ?Sized>(l: &L, tag: &str, item: &str) -> bool {
    l.res_items()
        .item_tags
        .get(tag)
        .is_some_and(|t| t.contains(item))
}
pub fn piglin_loved<L: PiglinLevel + ?Sized>(l: &L, item: &str) -> bool {
    l.item_in_tag("minecraft:piglin_loved", item)
}
/// `ItemStack.has(DataComponents.BLOCKS_ATTACKS)` for an item id.
pub fn item_blocks_attacks<L: PiglinLevel + ?Sized>(l: &L, item: &str) -> bool {
    l.res_items()
        .get(item)
        .is_some_and(|i| i.blocks_attacks.is_some())
}
/// `PiglinAi.isLovedItem` or a preferred weapon (`Piglin.canReplaceCurrentItem`).
pub fn piglin_wanted<L: PiglinLevel + ?Sized>(l: &L, m: &Mob, item: &str) -> bool {
    l.piglin_loved(item)
        || !m.is_baby() && l.item_in_tag("minecraft:piglin_preferred_weapons", item)
}
/// The `SLOT_NAMES` index `getEquipmentSlotForItem` picks: the
/// `equippable` slot (armour, a shield's off hand), else the main hand.
pub fn piglin_slot<L: PiglinLevel + ?Sized>(l: &L, item: &str) -> usize {
    let slot = l
        .res_items()
        .get(item)
        .and_then(|i| i.equip_slot.as_deref())
        .filter(|s| matches!(*s, "head" | "chest" | "legs" | "feet" | "offhand"))
        .unwrap_or("mainhand");
    SLOT_NAMES.iter().position(|n| *n == slot).unwrap_or(0)
}
/// `Piglin.canReplaceCurrentItem`: an empty slot takes anything, a
/// `PREVENT_ARMOR_CHANGE` piece stays, a wanted item replaces an unwanted
/// one and never the other way round; when both or neither are wanted,
/// `Mob.canReplaceCurrentItem` compares armour or weapons.
pub fn piglin_can_replace<L: PiglinLevel + ?Sized>(
    l: &L,
    m: &Mob,
    stack: &ItemStack,
    slot: usize,
) -> bool {
    let Some(current) = item_stack(m, slot) else {
        return true;
    };
    let new_wanted = l.piglin_wanted(m, &stack.item);
    let current_wanted = l.piglin_wanted(m, &current.item);
    let slot = [
        Slot::MainHand,
        Slot::OffHand,
        Slot::Feet,
        Slot::Legs,
        Slot::Chest,
        Slot::Head,
    ]
    .get(slot)
    .copied()
    .unwrap_or(Slot::MainHand);
    let new = l.stack_facts(m, stack, slot);
    let worn = l.stack_facts(m, &current, slot);
    // `getPreferredWeaponType` is the preferred-weapons tag for adults.
    let preferred = (!m.is_baby()).then(|| {
        (
            l.item_in_tag("minecraft:piglin_preferred_weapons", &stack.item),
            l.item_in_tag("minecraft:piglin_preferred_weapons", &current.item),
        )
    });
    piglin_replace(new_wanted, current_wanted, &new, &worn, slot, preferred)
}
/// `SimpleContainer.canAddItem` on the piglin's eight slots.
pub fn piglin_can_add<L: PiglinLevel + ?Sized>(l: &L, m: &Mob, stack: &ItemStack) -> bool {
    m.extra.inventory.len() < 8
        || m.extra
            .inventory
            .iter()
            .any(|s| s.same_item_same_components(stack) && s.count < l.piglin_max_stack(&s.item))
}
pub fn piglin_max_stack<L: PiglinLevel + ?Sized>(l: &L, item: &str) -> u32 {
    l.world_items().get(item).map_or(64, |i| i.max_stack)
}
/// `PiglinAi.wantsToPickup`: babies ignore their own item tag, but can
/// pick up currency and other loved items just like adults.
pub fn piglin_wants<L: PiglinLevel + ?Sized>(l: &L, m: &Mob, stack: &ItemStack) -> bool {
    let item = &*stack.item;
    if m.is_baby() && l.item_in_tag("minecraft:ignored_by_piglin_babies", item)
        || l.item_in_tag("minecraft:piglin_repellents", item)
    {
        return false;
    }
    if m.extra.piglin.admire_disabled > 0 && m.ai.target.is_some() {
        return false;
    }
    let holds_loved = m.extra.off_hand_id().is_some_and(|o| l.piglin_loved(o));
    if l.piglin_currency(stack) {
        return !holds_loved;
    }
    let space = l.piglin_can_add(m, stack);
    if item == "minecraft:gold_nugget" {
        space
    } else if l.item_in_tag("minecraft:piglin_food", item) {
        !has_eaten_recently(m) && space
    } else if l.piglin_loved(item) {
        !holds_loved && space
    } else {
        l.piglin_can_replace(m, stack, l.piglin_slot(item))
    }
}
/// `Mob.equipItemIfPossible` for a piglin: put it in its slot when that
/// slot takes it (an armour piece that cannot replace what is worn goes
/// to an empty main hand), dropping what it replaces by drop chance.
pub fn piglin_equip<L: PiglinLevel + ?Sized>(l: &mut L, index: usize, stack: &ItemStack) -> bool {
    let item = &*stack.item;
    let mut slot = l.piglin_slot(item);
    let m = &l.mobs()[index];
    let mut replace = l.piglin_can_replace(m, stack, slot);
    if slot > 1 && !replace {
        slot = 0;
        replace = m.extra.main_hand_id().is_none();
    }
    if !replace {
        return false;
    }
    let m = &mut l.parts().mobs[index];
    let old = item_stack(m, slot);
    let chance = stored_drop_chance(m, slot);
    let drop_old = old.is_some() && (m.random.next_float() - 0.1).max(0.0) < chance;
    // The piglin keeps the stack whole: its components, enchantments and trim.
    set_item_stack(
        m,
        slot,
        ItemStack {
            count: 1,
            ..stack.clone()
        },
    );
    // setItemSlotAndDropWhenKilled, then persistenceRequired.
    set_drop_chance(m, slot, 2.0);
    m.persistent = true;
    let at = [m.position.x, m.position.y, m.position.z];
    if let (true, Some(old)) = (drop_old, old) {
        l.spawn_item_own(old, at, 10);
    }
    true
}
/// `PiglinAi.putInInventory`: what does not fit is thrown at a random
/// spot nearby.
pub fn piglin_stash<L: PiglinLevel + ?Sized>(l: &mut L, index: usize, mut stack: ItemStack) {
    let max = l.piglin_max_stack(&stack.item);
    let p = l.parts();
    let m = &mut p.mobs[index];
    for s in &mut m.extra.inventory {
        if stack.count > 0 && s.same_item_same_components(&stack) && s.count < max {
            let n = (max - s.count).min(stack.count);
            s.count += n;
            stack.count -= n;
        }
    }
    while stack.count > 0 && m.extra.inventory.len() < 8 {
        let n = stack.count.min(max);
        m.extra.inventory.push(ItemStack {
            count: n,
            ..stack.clone()
        });
        stack.count -= n;
    }
    if stack.count == 0 {
        return;
    }
    let r = &mut p.world.random;
    let to = [
        m.position.x + f64::from(r.next_float() * 8.0 - 4.0),
        m.position.y + 1.0,
        m.position.z + f64::from(r.next_float() * 8.0 - 4.0),
    ];
    l.piglin_throw(index, to, stack);
}
/// `PiglinAi.stopHoldingOffHandItem` for something that is not
/// bartering currency: wear it if it can, else keep it in the inventory.
pub fn piglin_keep<L: PiglinLevel + ?Sized>(l: &mut L, index: usize, stack: ItemStack) {
    // Nuggets keep their whole stack in the inventory (a hand slot here
    // holds one item id only).
    if &*stack.item == "minecraft:gold_nugget" || !l.piglin_equip(index, &stack) {
        l.piglin_stash(index, stack);
    }
}
/// `BehaviorUtils.throwItem`: toss a stack from a piglin toward `target_pos`.
/// `targetPos` is elevated by 1.0 block per `throwItemsTowardPos`.
/// Throw vector is `(targetPos - thrower.position()).normalize() * 0.30000001192092896`.
/// Pickup delay is default (10 ticks). Hand Y pos is `eyeY - 0.3`.
pub fn piglin_throw<L: PiglinLevel + ?Sized>(
    l: &mut L,
    index: usize,
    target_pos: [f64; 3],
    stack: ItemStack,
) {
    let m = &l.mobs()[index];
    let from = [m.position.x, m.eye_y() - 0.3, m.position.z];
    let d = [
        target_pos[0] - m.position.x,
        target_pos[1] - m.position.y,
        target_pos[2] - m.position.z,
    ];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    const VELOCITY: f64 = 0.30000001192092896;
    let v = [
        d[0] / len * VELOCITY,
        d[1] / len * VELOCITY,
        d[2] / len * VELOCITY,
    ];
    l.spawn_item(stack, from, v, 10);
}
/// Piglins notice wanted items on the ground within 9 blocks (`PiglinAi.wantsToPickup`:
/// gold and other loved items, nuggets, food, gear), walk to them and
/// take them (`GoToWantedItem`, `Piglin.pickUpItem`).
pub fn piglin_pickups<L: PiglinLevel + ?Sized>(l: &mut L) {
    let griefing = l.mob_griefing();
    if !griefing && !l.grief_hooked() {
        return;
    }
    for i in 0..l.mobs().len() {
        let p = l.parts();
        let m = &p.mobs[i];
        if m.species() != Species::Piglin || !m.alive() || !p.ticked.contains(&m.id) {
            continue;
        }
        let id = m.id;
        // `Piglin.wantsToPickUp`: NeoForge's `canEntityGrief` for the piglin.
        if !l.entity_grief(id, griefing) {
            continue;
        }
        let m = &l.mobs()[i];
        let pos = m.position;
        // NearestItemSensor: the nearest wanted item within 32 that the piglin
        // sees (eye to eye); GoToWantedItem then walks to it within 9 blocks.
        let w = l.mob_world();
        let eye = Vec3::new(pos.x, m.eye_y(), pos.z);
        let dropped = l.dropped();
        let nearest = dropped
            .iter()
            .enumerate()
            .map(|(j, e)| (j, e.position.distance_squared(pos)))
            .filter(|(_, d)| *d < 32.0 * 32.0)
            .filter(|&(j, _)| l.piglin_wants(m, &dropped[j].stack))
            .filter(|&(j, _)| {
                // ItemEntity's eye height is 0.2125.
                let e = &dropped[j];
                let at = Vec3::new(e.position.x, e.position.y + 0.2125, e.position.z);
                w.line_of_sight(eye, at)
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let loved = nearest.is_some_and(|(j, _)| l.piglin_loved(&dropped[j].stack.item));
        l.admire_seen_item(i, nearest.map(|(_, d)| d), loved);
        let best = nearest.filter(|(_, d)| *d <= 81.0);
        let Some((j, _)) = best else {
            l.parts().mobs[i].extra.point = None;
            continue;
        };
        let p = l.parts();
        let e = &p.dropped[j];
        let close = (e.position.x - pos.x).abs() <= 1.3
            && (e.position.z - pos.z).abs() <= 1.3
            && (e.position.y - pos.y).abs() <= 1.0;
        if !close || e.pickup_delay > 0 {
            p.mobs[i].extra.point = Some([e.position.x, e.position.y, e.position.z]);
            continue;
        }
        // Nuggets come whole, anything else one at a time.
        let (taken_item, id) = (e.clone(), p.mobs[i].id);
        let item = e.stack.item.clone();
        l.take(id, taken_item.clone());
        // LivingEntity.onItemPickup, before PiglinAi.pickUpItem splits the stack.
        l.on_item_pickup(i, &taken_item);
        let stack = if &*item == "minecraft:gold_nugget" {
            l.discard_item(j).stack
        } else {
            let e = &mut l.parts().dropped[j];
            e.stack.count -= 1;
            let one = ItemStack {
                count: 1,
                ..e.stack.clone()
            };
            if e.stack.count == 0 {
                l.discard_item(j);
            }
            one
        };
        if l.piglin_loved(&item) {
            l.start_admiring(i, stack);
        } else if l.item_in_tag("minecraft:piglin_food", &item) && !has_eaten_recently(&l.mobs()[i])
        {
            // PiglinAi.eat: the food is gone and it will not eat for a while.
            let m = &mut l.parts().mobs[i];
            eat(m);
            m.nav_stop();
        } else {
            l.parts().mobs[i].nav_stop();
            l.piglin_keep(i, stack);
        }
    }
}
/// `StartAdmiringItemIfSeen` and the admire activity's stop behaviours, from the
/// distance squared to `NEAREST_VISIBLE_WANTED_ITEM` and whether that item is loved.
pub fn admire_seen_item<L: PiglinLevel + ?Sized>(
    l: &mut L,
    index: usize,
    nearest: Option<f64>,
    loved: bool,
) {
    let m = &mut l.parts().mobs[index];
    let holding = m.extra.off_hand_id().is_some();
    // ADMIRING_ITEM remains present while the off hand is admired.
    if holding && m.extra.using_item > 0 {
        return;
    }
    m.extra.piglin.admire_seen(holding, nearest, loved);
}
/// `PiglinAi.holdInOffhand` and `admireGoldItem`: the piglin holds `stack`, the one item it
/// took (`split(1)` from the ground, `consumeAndReturn(1)` from a hand), with its components.
pub fn start_admiring<L: PiglinLevel + ?Sized>(l: &mut L, index: usize, stack: ItemStack) {
    // PiglinAi.holdInOffhand: what the off hand held is dropped, whole.
    if let Some(old) = item_stack(&l.mobs()[index], OFFHAND) {
        let m = &mut l.parts().mobs[index];
        m.extra.set_slot_stack(OFFHAND, None);
        set_slot_enchantments(m, OFFHAND, Vec::new());
        let at = [m.position.x, m.position.y, m.position.z];
        l.spawn_item_own(old, at, 10);
    }
    let m = &mut l.parts().mobs[index];
    // The memory is set again, for the item it holds now.
    m.extra.piglin.seen_admire = 0;
    horizon_client::mobs::piglin::hold_in_offhand(m, stack);
    m.extra.using_item = ADMIRE;
    m.extra.point = None;
    m.nav_stop();
    // The admiring grunt is `PiglinAi.updateActivity`'s: `piglin::brain_step`
    // plays it when the activity changes.
}
/// `PiglinAi.mobInteract`: hand a piglin the held `stack` (`canAdmire`: only
/// piglin currency is accepted from the hand; other loved items are passed over).
pub fn give_piglin<L: PiglinLevel + ?Sized>(l: &mut L, index: usize, stack: &ItemStack) -> bool {
    let m = &l.mobs()[index];
    if m.species() != Species::Piglin
        || m.is_baby()
        || m.extra.using_item > 0
        || m.extra.piglin.seen_admire > 0
        || m.extra.piglin.admire_disabled > 0
        || !l.piglin_currency(stack)
    {
        return false;
    }
    l.start_admiring(
        index,
        ItemStack {
            count: 1,
            ..stack.clone()
        },
    );
    true
}
/// `PiglinAi.stopHoldingOffHandItem`, when the admiring ran out or a hit
/// stopped it (`piglin::stop_admiring`, which bars the barter). An adult
/// barters currency after admiring it, loses it to a hit, and equips or keeps
/// anything else. Babies equip the admired item, or hold it in the main hand
/// after stashing or throwing the old item. An empty off hand (a second stop
/// of the same hit) does nothing.
pub fn finish_admiring<L: PiglinLevel + ?Sized>(l: &mut L, mob: u32) {
    let Some(i) = l.mobs().iter().position(|m| m.id == mob) else {
        return;
    };
    let barter = !std::mem::take(&mut l.parts().mobs[i].extra.piglin.hit_stop);
    if let Some(item) = l.mobs()[i].extra.off_hand_id()
        && l.item_blocks_attacks(item)
    {
        return;
    }
    let Some(stack) = item_stack(&l.mobs()[i], OFFHAND) else {
        return;
    };
    let m = &mut l.parts().mobs[i];
    m.extra.set_slot_stack(OFFHAND, None);
    set_slot_enchantments(m, OFFHAND, Vec::new());
    if l.mobs()[i].is_baby() {
        if !l.piglin_equip(i, &stack) {
            if let Some(old) = item_stack(&l.mobs()[i], 0) {
                if l.piglin_loved(&old.item) {
                    l.piglin_stash(i, old);
                } else {
                    l.piglin_throw_items(i, vec![old]);
                }
            }
            let m = &mut l.parts().mobs[i];
            set_item_stack(m, 0, stack);
            set_drop_chance(m, 0, 2.0);
            m.persistent = true;
        }
        return;
    }
    if !l.piglin_currency(&stack) {
        l.piglin_keep(i, stack);
        return;
    }
    if !barter {
        return;
    }
    let p = l.parts();
    let items = p.items.loot_table(
        "minecraft:gameplay/piglin_bartering",
        false,
        None,
        &mut p.world.random,
    );
    l.piglin_throw_items(i, items);
}
/// `PiglinAi.throwItems`: barters and a baby's unwanted main-hand item
/// use the same player-directed throw.
pub fn piglin_throw_items<L: PiglinLevel + ?Sized>(l: &mut L, i: usize, items: Vec<ItemStack>) {
    if items.is_empty() {
        return;
    }
    // `PiglinAi.throwItemsTowardPos` swings the off hand once before the
    // throws, so the barters leave the hand the piglin admires them in.
    l.parts().mobs[i].swing_hand_modified(true, SwingAnimation::DEFAULT);
    let at = l.mobs()[i].position;
    let target = l.piglin_throw_target(at);
    for stack in items {
        let to = target.unwrap_or_else(|| {
            let r = &mut l.parts().world.random;
            [
                at.x + f64::from(r.next_float() * 8.0 - 4.0),
                at.y + 1.0,
                at.z + f64::from(r.next_float() * 8.0 - 4.0),
            ]
        });
        l.piglin_throw(i, to, stack);
    }
}

/// The decision of `Piglin.canReplaceCurrentItem` once both stacks are known.
fn piglin_replace(
    new_wanted: bool,
    current_wanted: bool,
    new: &equip::Facts,
    worn: &equip::Facts,
    slot: Slot,
    preferred: Option<(bool, bool)>,
) -> bool {
    if worn.prevents_armor_change {
        return false;
    }
    if new_wanted != current_wanted {
        return new_wanted;
    }
    equip::can_replace_current_item(new, Some(worn), slot, preferred)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn armor(points: f64, binding: bool) -> equip::Facts {
        equip::Facts {
            armor: points,
            armor_toughness: 0.0,
            attack_damage: 0.0,
            enchantments: 0,
            damage: 0,
            custom_name: false,
            prevents_armor_change: binding,
        }
    }

    #[test]
    fn better_helmet_replaces_worse_one() {
        // Diamond (3) over iron (2), neither loved.
        let (diamond, iron) = (armor(3.0, false), armor(2.0, false));
        assert!(piglin_replace(
            false,
            false,
            &diamond,
            &iron,
            Slot::Head,
            None
        ));
        assert!(!piglin_replace(
            false,
            false,
            &iron,
            &diamond,
            Slot::Head,
            None
        ));
    }

    #[test]
    fn binding_curse_keeps_the_worn_piece() {
        let (diamond, cursed) = (armor(3.0, false), armor(1.0, true));
        assert!(!piglin_replace(
            false,
            false,
            &diamond,
            &cursed,
            Slot::Head,
            None
        ));
        assert!(!piglin_replace(
            true,
            false,
            &diamond,
            &cursed,
            Slot::Head,
            None
        ));
    }

    #[test]
    fn wanted_beats_comparison() {
        let (gold, diamond) = (armor(2.0, false), armor(3.0, false));
        assert!(piglin_replace(
            true,
            false,
            &gold,
            &diamond,
            Slot::Head,
            None
        ));
        assert!(!piglin_replace(
            false,
            true,
            &diamond,
            &gold,
            Slot::Head,
            None
        ));
    }

    #[test]
    fn preferred_weapon_comparison_by_damage() {
        let mut strong = armor(0.0, false);
        strong.attack_damage = 7.0;
        let mut weak = armor(0.0, false);
        weak.attack_damage = 4.0;
        assert!(piglin_replace(
            false,
            false,
            &strong,
            &weak,
            Slot::MainHand,
            None
        ));
        assert!(!piglin_replace(
            false,
            false,
            &weak,
            &strong,
            Slot::MainHand,
            None
        ));
    }
}
