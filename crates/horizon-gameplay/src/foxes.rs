//! Foxes and dolphins on the game side: what they take from the ground (`Fox.pickUpItem`,
//! `Fox.canHoldItem`, `Dolphin.pickUpItem`) and where `FoxSearchForItemsGoal` and
//! `Dolphin.MoveToItemGoal` walk to.
use crate::{item_areas::ItemAreas, mob_level::MobLevel};
use horizon_client::{
    items::Items,
    mobs::{
        Species, dolphin,
        equip::Slot,
        finalize::{self, set_drop_chance, set_item_stack},
        fox,
    },
};
use horizon_core::geometry::Vec3;

/// What fox code calls on its level beyond [`MobLevel`]: the game's own fox
/// methods, which mods target. Each runs the function of this module of the same name.
pub trait FoxLevel: MobLevel {
    /// `Fox.isConsumableFood` ([`fox_food`]).
    fn fox_food(&self, item: &str) -> bool;
    /// `Fox.pickUpItem` ([`fox_pick_up`]).
    fn fox_pick_up(&mut self, fox_index: usize, item_index: usize);
}

/// `Fox.ALLOWED_ITEMS` and the looting block of `Mob.aiStep` for foxes, once a tick.
pub fn tick_foxes<L: FoxLevel + ?Sized>(l: &mut L) {
    let griefing = l.mob_griefing();
    let mut near = ItemAreas::default();
    for i in 0..l.parts().mobs.len() {
        let p = l.parts();
        let m = &p.mobs[i];
        if m.species() != Species::Fox || !m.alive() || m.removed || !p.ticked.contains(&m.id) {
            continue;
        }
        // `Mob.aiStep`: the items in the box grown by (1, 0, 1), in order.
        let reach = m.bounds().inflate(Vec3::new(1.0, 0.0, 1.0));
        let (id, loots) = (m.id, m.can_pick_up_loot());
        let may_grief = loots && l.entity_grief(id, griefing);
        let p = l.parts();
        if may_grief && near.may_meet(p.dropped, p.next_entity, reach) {
            let mut j = 0;
            while j < l.parts().dropped.len() {
                let e = &l.dropped()[j];
                if e.pickup_delay > 0 || e.stack.count == 0 || !reach.intersects(e.bounds()) {
                    j += 1;
                    continue;
                }
                let item_food = l.fox_food(&e.stack.item);
                let held_food = l.mobs()[i]
                    .extra
                    .main_hand_id()
                    .is_some_and(|held| l.fox_food(held));
                if !fox::can_hold_item(&l.mobs()[i], item_food, held_food) {
                    j += 1;
                    continue;
                }
                // The entity is discarded: the next one is at `j` now.
                l.fox_pick_up(i, j);
            }
        }
        // `FoxSearchForItemsGoal`: the first allowed item within 8 blocks.
        let p = l.parts();
        let m = &p.mobs[i];
        let area = m.bounds().inflate(Vec3::new(8.0, 8.0, 8.0));
        let at = if near.may_meet(p.dropped, p.next_entity, area) {
            p.dropped
                .iter()
                .find(|e| e.pickup_delay == 0 && e.health > 0 && area.intersects(e.bounds()))
                .map(|e| [e.position.x, e.position.y, e.position.z])
        } else {
            None
        };
        fox::set_item_at(&mut p.mobs[i], at);
    }
}

/// `Dolphin.pickUpItem` in `Mob.aiStep`'s looting block, and the item
/// `Dolphin.MoveToItemGoal` swims to (`Dolphin.ALLOWED_ITEMS`: no pickup delay, alive, in water).
pub fn tick_dolphins<L: MobLevel + ?Sized>(l: &mut L) {
    let griefing = l.mob_griefing();
    let mut near = ItemAreas::default();
    for i in 0..l.parts().mobs.len() {
        let p = l.parts();
        let m = &p.mobs[i];
        if m.species() != Species::Dolphin || !m.alive() || m.removed || !p.ticked.contains(&m.id) {
            continue;
        }
        // `Mob.aiStep`: the items in the box grown by (1, 0, 1); `Dolphin.pickUpItem`
        // takes the whole stack when the mouth is empty (`Mob.canHoldItem` is true).
        let reach = m.bounds().inflate(Vec3::new(1.0, 0.0, 1.0));
        let (id, loots) = (m.id, m.can_pick_up_loot());
        let may_grief = loots && l.entity_grief(id, griefing);
        let p = l.parts();
        if may_grief && near.may_meet(p.dropped, p.next_entity, reach) {
            let mut j = 0;
            while j < l.parts().dropped.len() {
                let p = l.parts();
                let e = &p.dropped[j];
                if e.pickup_delay > 0
                    || e.stack.count == 0
                    || !reach.intersects(e.bounds())
                    || p.mobs[i].extra.main_hand_id().is_some()
                {
                    j += 1;
                    continue;
                }
                let before = p.dropped[j].clone();
                l.on_item_pickup(i, &before);
                let m = &mut l.parts().mobs[i];
                set_item_stack(m, Slot::MainHand.index(), before.stack.clone());
                set_drop_chance(m, Slot::MainHand.index(), 2.0);
                let id = m.id;
                l.take(id, before);
                l.discard_item(j);
            }
        }
        let p = l.parts();
        let area = p.mobs[i].bounds().inflate(Vec3::new(8.0, 8.0, 8.0));
        let at = if near.may_meet(p.dropped, p.next_entity, area) {
            let (facts, world) = (p.facts, &*p.world);
            p.dropped
                .iter()
                .find(|e| {
                    e.pickup_delay == 0
                        && e.health > 0
                        && area.intersects(e.bounds())
                        && facts
                            .get(world.block(
                                e.position.x.floor() as i32,
                                e.position.y.floor() as i32,
                                e.position.z.floor() as i32,
                            ))
                            .fluid
                            == horizon_client::blocks::Fluid::Water
                })
                .map(|e| [e.position.x, e.position.y, e.position.z])
        } else {
            None
        };
        dolphin::set_item_at(&mut p.mobs[i], at);
    }
}

/// `Fox.isConsumableFood`.
pub fn fox_food(items: &Items, item: &str) -> bool {
    items
        .get(item)
        .is_some_and(|i| i.food.is_some() && i.consumable.is_some())
}

/// `Fox.pickUpItem`: the extra items drop where it stands, what it held is spat out
/// ahead of it, and it holds one of the item.
pub fn fox_pick_up<L: MobLevel + ?Sized>(l: &mut L, fox_index: usize, item_index: usize) {
    let p = l.parts();
    let before = p.dropped[item_index].clone();
    let mut stack = before.stack.clone();
    let count = stack.count;
    let at = {
        let m = &p.mobs[fox_index];
        [m.position.x, m.position.y, m.position.z]
    };
    // `dropItemStack(itemStack.split(count - 1))`: a default ItemEntity at its feet.
    if count > 1 {
        let mut rest = stack.clone();
        rest.count = count - 1;
        stack.count = 1;
        l.spawn_item_own(rest, at, 10);
    }
    // `spitOutItem(getItemBySlot(MAINHAND))`.
    let held = finalize::item_stack(&l.parts().mobs[fox_index], Slot::MainHand.index());
    if let Some(held) = held {
        let p = l.parts();
        let look = p.mobs[fox_index].view_vector();
        let from = [at[0] + look[0], at[1] + 1.0, at[2] + look[2]];
        if let Some(thrown_id) = l.spawn_item_own(held, from, 40) {
            let fox_id = l.parts().mobs[fox_index].id;
            let id = l.mob_uuid(fox_id);
            if let Some(thrown) = l.item_entity_mut(thrown_id) {
                thrown.thrower = Some(id);
            }
        }
        l.play_sound("minecraft:entity.fox.spit", at, 1.0, 1.0);
    }
    // `onItemPickup`, then `setItemSlot(MAINHAND, split(1))`, `setGuaranteedDrop`.
    l.on_item_pickup(fox_index, &before);
    let m = &mut l.parts().mobs[fox_index];
    set_item_stack(m, Slot::MainHand.index(), stack);
    set_drop_chance(m, Slot::MainHand.index(), 2.0);
    fox::took_item(m);
    let id = m.id;
    // `take(entity, count)` and `entity.discard()`.
    l.take(id, before);
    l.discard_item(item_index);
}
