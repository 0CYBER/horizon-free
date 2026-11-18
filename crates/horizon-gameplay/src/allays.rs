//! Allays on the server side: spotting and picking up matching drops, and
//! throwing their load to the liked player or note block (`AllayAi`).
use crate::mob_level::MobLevel;
use horizon_client::{
    items::ItemStack,
    mobs::{Species, allay},
};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// What allay code calls on its level beyond [`MobLevel`]: the game's own allay
/// methods, which mods target. Each runs the function of this module of the same name.
pub trait AllayLevel: MobLevel {
    /// `GoAndGiveItemsToTarget` / `AllayAi.throwItem` ([`deliver`]).
    fn allay_deliver(&mut self, i: usize);
    /// `AllayAi.getLikedPlayer` ([`liked_player`]).
    fn allay_liked_player(&self, i: usize) -> Option<Vec3>;
    /// The UUID of the player an allay likes ([`liked_uuid`]).
    fn allay_liked_uuid(&self, i: usize) -> [i32; 4];
}

/// `Allay.aiStep`'s looting and the brain's item behaviours, once a tick.
pub fn tick_allays<L: AllayLevel + ?Sized>(l: &mut L) {
    let griefing = l.mob_griefing();
    for i in 0..l.parts().mobs.len() {
        let p = l.parts();
        if p.mobs[i].species() != Species::Allay
            || !p.mobs[i].alive()
            || !p.ticked.contains(&p.mobs[i].id)
        {
            continue;
        }
        let pos = p.mobs[i].position;
        // The dance follows the jukebox events the allay hears
        // (`mob_listen`), `Allay.tick` and `aiStep`.
        let Some(held) = p.mobs[i].extra.main_hand_stack().cloned() else {
            p.mobs[i].extra.point = None;
            continue;
        };
        let (mobs, dropped) = (l.mobs(), l.dropped());
        // The held filter chooses item types; the pocket independently
        // decides whether the candidate's full components can fit.
        // `Mob.aiStep`'s looting loop asks for no AI: `canPickUpLoot` and
        // mob_griefing are its gates, the latter through `entity_grief`.
        let wants = mobs[i].can_pick_up_loot() && l.entity_grief(mobs[i].id, griefing);
        let room = |stack: &ItemStack| {
            mobs[i].extra.allay.as_ref().map_or(0, |st| {
                st.room(stack, mobs[i].kind.inventory_stack_limit(stack))
            })
        };
        let reach = allay::pickup_reach(mobs[i].bounds());
        // GoToWantedItem: a stack it considers equal within 32 that the
        // allay sees (NearestItemSensor's hasLineOfSight, eye to eye). The
        // sensor is the brain's, so without AI only `Mob.aiStep`'s looting
        // loop runs: it takes an equal drop already in `getPickupReach`.
        let wanted = if !wants {
            None
        } else if mobs[i].no_ai {
            dropped.iter().position(|e| {
                e.pickup_delay == 0
                    && allay::considers_item_equal(&held, &e.stack)
                    && room(&e.stack) > 0
                    && reach.contains(e.position)
            })
        } else {
            let w = l.mob_world();
            let eye = Vec3::new(pos.x, mobs[i].eye_y(), pos.z);
            // The sensor looks in the allay's box grown by (32, 16, 32).
            let b = mobs[i].bounds();
            let (low, high) = (b.min_position().y - 16.0, b.max_position().y + 16.0);
            dropped
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.pickup_delay == 0
                        && allay::considers_item_equal(&held, &e.stack)
                        && room(&e.stack) > 0
                })
                .map(|(j, e)| (j, e.position.distance_squared(pos)))
                .filter(|(_, d)| *d < 32.0 * 32.0)
                .filter(|(j, _)| {
                    let y = dropped[*j].position.y;
                    y <= high && y + 0.25 >= low
                })
                .filter(|(j, _)| {
                    // ItemEntity's eye height is 0.2125.
                    let e = &dropped[*j];
                    let at = Vec3::new(e.position.x, e.position.y + 0.2125, e.position.z);
                    w.line_of_sight(eye, at)
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(j, _)| j)
        };
        let Some(j) = wanted else {
            let p = l.parts();
            p.mobs[i].extra.point = None;
            // `GiveInventoryToLookTargetPlayer` is the brain's too.
            if !p.mobs[i].no_ai {
                l.allay_deliver(i);
            }
            continue;
        };
        let p = l.parts();
        let e = &p.dropped[j];
        if reach.contains(e.position) {
            let (item, id) = (e.clone(), p.mobs[i].id);
            let max = p.mobs[i].kind.inventory_stack_limit(&e.stack);
            let take = p.mobs[i]
                .extra
                .allay
                .as_mut()
                .unwrap()
                .add(e.stack.clone(), max);
            l.take(id, item);
            let e = &mut l.parts().dropped[j];
            e.stack.count -= take;
            if e.stack.count == 0 {
                l.discard_item(j);
            }
            let at = [pos.x, pos.y, pos.z];
            let random = &mut l.parts().world.random;
            let pitch = (random.next_float() - random.next_float()) * 1.4 + 2.0;
            l.play_sound("minecraft:entity.item.pickup", at, 0.2, pitch);
            l.parts().mobs[i].extra.point = None;
        } else {
            p.mobs[i].extra.point = Some([e.position.x, e.position.y, e.position.z]);
        }
    }
}

/// The UUID of the player an allay likes (`LIKED_PLAYER`).
pub fn liked_uuid<L: MobLevel + ?Sized>(l: &L, i: usize) -> [i32; 4] {
    // An allay saved before its player was kept belongs to the host.
    l.mobs()[i]
        .extra
        .owner
        .filter(|u| *u != [0, 0, 0, 1])
        .or(l.host_uuid())
        .unwrap_or([0, 0, 0, 1])
}

/// `AllayAi.getLikedPlayer`: the player the allay was given an item by,
/// found by UUID among the players of the level, when they are not a
/// spectator and within 64 blocks.
pub fn liked_player<L: AllayLevel + ?Sized>(l: &L, i: usize) -> Option<Vec3> {
    let m = &l.mobs()[i];
    if !m.extra.tame {
        return None;
    }
    let uuid = l.allay_liked_uuid(i);
    let found = l.player_by_uuid(uuid);
    // The 64-block search uses the feet; the deposit point is the eyes
    // (`new EntityTracker(player, true)`).
    found
        .filter(|(feet, _)| feet.distance_squared(m.position) < 64.0 * 64.0)
        .map(|(_, eye)| Vec3::new(eye[0], eye[1], eye[2]))
}

/// `GoAndGiveItemsToTarget` / `AllayAi.throwItem`: within 3 blocks of
/// the deposit, one item a tick, each restarting the three-second pause
/// before it picks anything up again.
pub fn deliver<L: AllayLevel + ?Sized>(l: &mut L, i: usize) {
    if allay::inventory(&l.mobs()[i]).is_none() {
        return;
    }
    // AllayAi.getItemDepositPosition: the liked note block, else the player.
    let dimension = l.dimension();
    let p = l.parts();
    let (facts, world) = (p.facts, &*p.world);
    let noteblock = allay::noteblock_deposit(&mut p.mobs[i], dimension, |p| {
        &*facts.get(world.block(p[0], p[1], p[2])).name == "minecraft:note_block"
    });
    let player = l.allay_liked_player(i);
    let m = &l.mobs()[i];
    let target = match noteblock {
        Some(p) => Vec3::new(p[0], p[1], p[2]),
        None => match player.filter(|_| m.extra.tame) {
            Some(p) => p,
            None => return,
        },
    };
    let eye = Vec3::new(m.position.x, m.eye_y(), m.position.z);
    if target.distance_squared(eye) >= 9.0 {
        return;
    }
    let Some(item) = allay::inventory(m).cloned() else {
        return;
    };
    let from = [
        m.position.x,
        m.position.y + m.eye_height() - 0.2,
        m.position.z,
    ];
    // BehaviorUtils.throwItem: the vector runs from the allay's feet.
    let d = [
        target.x - m.position.x,
        target.y + 1.0 - m.position.y,
        target.z - m.position.z,
    ];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
    let v = [d[0] / len * 0.2, d[1] / len * 0.3, d[2] / len * 0.2];
    // AllayAi.throwItem: the liked player hears of the block below the deposit.
    let below = [
        target.x.floor() as i32,
        (target.y - 1.0).floor() as i32,
        target.z.floor() as i32,
    ];
    let uuid = l.allay_liked_uuid(i);
    // AllayAi.throwItem: one item of the pocket stack, its components and all.
    let stack = ItemStack { count: 1, ..item };
    l.trigger_block_for(uuid, "minecraft:allay_drop_item_on_block", below, &stack);
    let m = &mut l.parts().mobs[i];
    m.extra.allay.as_mut().unwrap().take(1);
    allay::set_pickup_cooldown(m, Some(60));
    // ItemEntity.setDefaultPickUpDelay.
    l.spawn_item(stack, from, v, 10);
    // BehaviorUtils.throwItem: ItemEntity.setThrower(the allay).
    let id = l.mobs()[i].id;
    let allay = l.mob_uuid(id);
    if let Some(thrown) = l.just_spawned_item() {
        thrown.thrower = Some(allay);
    }
    let world = l.parts().world;
    if world.game_time % 7 == 0 && world.random.next_double() < 0.9 {
        const PITCHES: [f32; 16] = [
            0.5625, 0.625, 0.75, 0.9375, 1.0, 1.0, 1.125, 1.25, 1.5, 1.875, 2.0, 2.25, 2.5, 3.0,
            3.75, 4.0,
        ];
        let pitch = PITCHES[world.random.next_int_bound(16).unwrap_or(4) as usize];
        l.play_mob_sound(
            i,
            "minecraft:entity.allay.item_thrown",
            6,
            1.0,
            pitch,
            false,
        );
    }
}
