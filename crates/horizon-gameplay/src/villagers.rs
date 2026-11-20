//! What villagers do with each other and with the player: gossip and
//! reputation, sharing food and wares, picking up dropped items, making
//! babies, calling iron golems and the prices they ask.
use crate::{item_areas::ItemAreas, mob_level::MobLevel, villager_level::VillagerLevel};
use horizon_client::{
    gossip::{self, GossipType},
    items::{ItemStack, Items},
    mobs::{Mob, Species, spawning, villager, villager_brain},
};
use horizon_core::{geometry::Vec3, position::BlockPos};

fn block_of(p: Vec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}

pub fn villager_index<L: MobLevel + ?Sized>(l: &L, id: u32) -> Option<usize> {
    l.mobs()
        .iter()
        .position(|m| m.id == id && m.species() == Species::Villager)
}

pub fn item_max_stack(items: &Items, item: &str) -> u32 {
    items.get(item).map_or(64, |i| i.max_stack)
}

/// `UseBonemeal.tick`'s `BoneMealItem.growCrop`: villager `id`'s first
/// bone meal on `pos`, with level event 1505 and the next crop to work
/// on when it grew.
pub fn villager_bonemeal<L: VillagerLevel + ?Sized>(l: &mut L, id: u32, pos: BlockPos) {
    let Some(i) = l.villager_index(id) else {
        return;
    };
    let p = l.parts();
    let inventory = &villager::ext_mut(&mut p.mobs[i]).inventory;
    let Some(slot) = inventory
        .iter()
        .position(|s| s.count > 0 && &*s.item == "minecraft:bone_meal")
    else {
        return;
    };
    let mut stack = inventory[slot].clone();
    let (grown, crop) = l.bonemeal_grow(pos, &mut stack);
    // A canceled `BonemealEvent` that succeeded uses none of the meal.
    if grown && crop != Some(true) {
        stack.count = stack.count.saturating_sub(1);
    }
    // The event's host may have moved the villager in `mobs`.
    let Some(i) = l.villager_index(id) else {
        return;
    };
    let p = l.parts();
    let m = &mut p.mobs[i];
    let inventory = &mut villager::ext_mut(m).inventory;
    if slot < inventory.len() {
        if stack.count == 0 {
            inventory.remove(slot);
        } else {
            inventory[slot] = stack;
        }
    }
    if grown {
        villager_brain::bonemeal_grown(m, p.world.game_time);
        l.level_event(horizon_world::level::LevelEvent::LevelEvent {
            id: 1505,
            pos,
            data: 15,
            source: None,
        });
    }
}

/// `WorkAtComposter.useWorkstation`: at a composter the farmer makes
/// bread, then composts seeds.
pub fn villager_work_at_composter<L: VillagerLevel + ?Sized>(l: &mut L, id: u32, pos: [i32; 3]) {
    let Some(i) = l.villager_index(id) else {
        return;
    };
    if l.composter_level(pos).is_none() {
        return;
    }
    l.villager_make_bread(i);
    l.compost_items(i, pos);
}

/// `WorkAtComposter.makeBread`: up to three loaves from nine wheat while
/// the villager holds at most 36 bread; what does not fit drops.
pub fn villager_make_bread<L: VillagerLevel + ?Sized>(l: &mut L, i: usize) {
    let max_stack = l.item_max_stack("minecraft:bread");
    let p = l.parts();
    let e = villager::ext_mut(&mut p.mobs[i]);
    if e.count_item("minecraft:bread") > 36 {
        return;
    }
    let loaves = (e.count_item("minecraft:wheat") / 3).min(3);
    if loaves == 0 {
        return;
    }
    e.remove_item("minecraft:wheat", loaves * 3);
    let left = e.add_item(ItemStack::new("minecraft:bread", loaves), max_stack);
    if let Some(rest) = left {
        // spawnAtLocation(level, stack, 0.5F)
        let p = l.mobs()[i].position;
        l.spawn_item(rest, [p.x, p.y + 0.5, p.z], [0.0, 0.2, 0.0], 10);
    }
}

/// `BehaviorUtils.throwItem`: toss a stack from a villager's hand toward
/// a point.
pub fn villager_throw<L: VillagerLevel + ?Sized>(
    l: &mut L,
    from: Vec3,
    eye: f64,
    to: Vec3,
    stack: ItemStack,
) {
    // The hand drop and the velocity are the float 0.3F, widened.
    let k = f64::from(0.3_f32);
    let at = [from.x, eye - k, from.z];
    let d = Vec3::new(to.x - from.x, to.y - from.y, to.z - from.z).normalize();
    l.spawn_item(stack, at, [d.x * k, d.y * k, d.z * k], 10);
}

/// `TradeWithVillager.tick` for one pair: gossip (at most once in 1200
/// ticks for both), then surplus food and wanted wares tossed over.
pub fn villager_meet<L: VillagerLevel + ?Sized>(l: &mut L, a: u32, b: u32) -> usize {
    let (Some(ia), Some(ib)) = (l.villager_index(a), l.villager_index(b)) else {
        return 0;
    };
    let professions = l.stations().professions.clone();
    let p = l.parts();
    let now = p.world.game_time;
    let meeting = villager::meet(p.mobs, ia, ib, now, p.items, &professions);
    for stack in meeting.thrown {
        l.villager_throw(meeting.from, meeting.eye, meeting.to, stack);
    }
    if meeting.gossiped {
        l.villager_golem_check(a, 5);
    }
    meeting.new_gossip
}

/// `Villager.spawnGolemIfNeeded`: when enough villagers nearby that
/// slept recently and saw no golem agree, an iron golem appears.
pub fn villager_golem_check<L: VillagerLevel + ?Sized>(l: &mut L, id: u32, needed: usize) {
    let Some(i) = l.villager_index(id) else {
        return;
    };
    let now = l.parts().world.game_time;
    let Some(near) = villager::golem_summoners(l.mobs(), i, now, needed) else {
        return;
    };
    let start = block_of(l.mobs()[i].position);
    if l.try_spawn_iron_golem(start) {
        for j in near {
            villager_brain::golem_detected(&mut l.parts().mobs[j], now);
        }
    }
}

/// `SpawnUtil.trySpawnMob(IRON_GOLEM, ...)`: a golem near `start`, with
/// the ambient sound `trySpawnMob` plays.
pub fn try_spawn_iron_golem<L: VillagerLevel + ?Sized>(l: &mut L, start: [i32; 3]) -> bool {
    let Some(kind) = l.mob_type("minecraft:iron_golem") else {
        return false;
    };
    let id = l.parts().next_entity + 1;
    let mut random = l.parts().world.random.clone();
    let golem = villager::summon_golem(&l.mob_world(), start, &mut random, |random, at, yaw| {
        let mut m = Mob::new(id, kind.clone(), at, yaw);
        villager::finalize_summoned_golem(&mut m, random);
        m
    });
    l.parts().world.random = random;
    let Some(golem) = golem else {
        return false;
    };
    l.set_next_entity(id);
    let sound = golem.kind.sounds[0];
    if !sound.is_empty() {
        let at = golem.center();
        l.play_sound(&format!("minecraft:{sound}"), at, 1.0, 1.0);
    }
    l.parts().mobs.push(golem);
    let joined = l.mobs().len() - 1;
    l.mob_joined(joined, false);
    true
}

/// `VillagerMakeLove.tryToGiveBirth`: both eat, a free bed within 48
/// blocks is taken for the child, who is a baby villager of one of
/// three villager types.
pub fn villager_birth<L: VillagerLevel + ?Sized>(
    l: &mut L,
    parent: u32,
    partner: u32,
    child: Option<([i32; 3], Option<u8>)>,
) {
    let (Some(ia), Some(ib)) = (l.villager_index(parent), l.villager_index(partner)) else {
        return;
    };
    let p = l.parts();
    villager::ext_mut(&mut p.mobs[ib]).eat_and_digest(p.items);
    let from = p.mobs[ia].position;
    let Some((bed, variant)) = child else {
        // VillagerMakeLove: entity event 13 for the partner, then the parent.
        for i in [ib, ia] {
            let id = l.mobs()[i].id;
            l.mob_event_particles(id, 13);
        }
        return;
    };
    let variant = variant.unwrap_or_else(|| {
        let facts = l.biome_facts(
            from.x.floor() as i32,
            from.y.floor() as i32,
            from.z.floor() as i32,
        );
        spawning::villager_type(&facts)
    });
    l.parts().mobs[ib].extra.age = 6000;
    let Some(id) = l.spawn_mob_raw("minecraft:villager", [from.x, from.y, from.z], true) else {
        // `VillagerMakeLove.tryToGiveBirth` releases the child's bed when
        // `breed` fails. Here POI tickets derive from resident villagers'
        // beds: selection has not claimed one, and a rejected join leaves
        // no child to own it. Keep both parents' already-set breeding ages.
        return;
    };
    if let Some(child) = l.parts().mobs.iter_mut().find(|m| m.id == id) {
        child.extra.variant = variant;
        child.extra.profession = Default::default();
        child.extra.age = -24000;
        child.extra.bed = Some(bed);
    }
    // VillagerMakeLove.giveBirth: entity event 12 for the child.
    l.mob_event_particles(id, 12);
}

/// `NearestItemSensor.doTick` for the villagers whose sensor is due: the
/// nearest dropped item within 32 blocks that `Villager.wantsToPickUp`
/// accepts and the villager sees, or none.
pub fn villager_item_sense<L: VillagerLevel + ?Sized>(l: &mut L) {
    if l.dropped().is_empty() {
        let p = l.parts();
        for m in p.mobs.iter_mut() {
            if m.species() == Species::Villager
                && p.ticked.contains(&m.id)
                && villager_brain::take_item_scan(m)
            {
                villager_brain::set_wanted_item(m, None);
            }
        }
        return;
    }
    let picks_up = l.villager_picks_up();
    let mut due = Vec::new();
    let p = l.parts();
    for (i, m) in p.mobs.iter_mut().enumerate() {
        if m.species() == Species::Villager
            && p.ticked.contains(&m.id)
            && villager_brain::take_item_scan(m)
        {
            due.push(i);
        }
    }
    if due.is_empty() {
        return;
    }
    // Reuse the existing item-cell index only for large, spread-out item
    // sets with many due sensors. Small and clustered scenes retain the
    // original full scan without building an index.
    let mut nearby = ItemAreas::worthwhile(l.dropped(), due.len()).then(ItemAreas::default);
    let next_entity = l.parts().next_entity;
    let w = l.mob_world();
    let mut found = Vec::new();
    for i in due {
        let m = &l.mobs()[i];
        let area = m.bounds().inflate(Vec3::new(32.0, 16.0, 32.0));
        let professions = l.stations().professions.clone();
        let requested = villager::requested_items(m, &professions);
        let eye = Vec3::new(m.position.x, m.eye_y(), m.position.z);
        let mut best: Option<(f64, Vec3)> = None;
        let mut scan = nearby
            .as_mut()
            .map(|index| index.scan(l.dropped(), next_entity, area));
        let mut j = 0;
        loop {
            if let Some(indexed) = &mut scan {
                j = indexed.next_from(j, l.dropped().len());
            }
            let Some(e) = l.dropped().get(j) else {
                break;
            };
            j += 1;
            let d = e.position.distance_squared(m.position);
            if best.is_some_and(|(b, _)| d >= b) || !e.bounds().intersects(area) || d >= 32.0 * 32.0
            {
                continue;
            }
            let item = &e.stack.item;
            let wants = villager::food_points(l.items(), item) > 0
                || picks_up.as_ref().is_some_and(|t| t.contains(&**item))
                || requested.iter().any(|key| key == &**item);
            let fits = villager::ext(m).is_none_or(|x| x.can_add(&e.stack, l.item_max_stack(item)));
            // ItemEntity's eye height is 0.2125.
            let at = Vec3::new(e.position.x, e.position.y + 0.2125, e.position.z);
            if wants && fits && w.line_of_sight(eye, at) {
                best = Some((d, e.position));
            }
        }
        found.push((i, best.map(|(_, p)| [p.x, p.y, p.z])));
    }
    for (i, at) in found {
        villager_brain::set_wanted_item(&mut l.parts().mobs[i], at);
    }
}

/// `Mob.aiStep` item pickup for villagers (`InventoryCarrier`): food,
/// seeds and what their profession wants, from within a block.
pub fn villager_pickups<L: VillagerLevel + ?Sized>(l: &mut L) {
    let griefing = l.mob_griefing();
    if l.dropped().is_empty() || (!griefing && !l.grief_hooked()) {
        return;
    }
    let picks_up = l.villager_picks_up();
    // A villager with no item near skips its scan; one near an item scans
    // the items of the cells it meets as before, testing the cheap reach
    // first: a server's villagers and dropped items each run into the
    // hundreds or thousands.
    let mut near_items = ItemAreas::default();
    for i in 0..l.mobs().len() {
        let p = l.parts();
        let m = &p.mobs[i];
        if m.species() != Species::Villager || !m.alive() || !p.ticked.contains(&m.id) {
            continue;
        }
        let (id, pos, half) = (m.id, m.position, f64::from(m.width()) / 2.0 + 1.0);
        let top = f64::from(m.height());
        if !l.entity_grief(id, griefing) {
            continue;
        }
        let reach = horizon_core::geometry::Aabb::new(
            Vec3::new(pos.x - half, pos.y - 0.25, pos.z - half),
            Vec3::new(pos.x + half, pos.y + top, pos.z + half),
        );
        let next_entity = l.parts().next_entity;
        let mut scan = near_items.scan(l.dropped(), next_entity, reach);
        if scan.is_empty() {
            continue;
        }
        let professions = l.stations().professions.clone();
        let requested = villager::requested_items(&l.mobs()[i], &professions);
        let mut j = 0;
        loop {
            j = scan.next_from(j, l.dropped().len());
            let Some(e) = l.dropped().get(j) else {
                break;
            };
            let near = (e.position.x - pos.x).abs() <= half
                && (e.position.z - pos.z).abs() <= half
                && e.position.y >= pos.y - 0.25
                && e.position.y <= pos.y + top;
            if !near || e.pickup_delay > 0 {
                j += 1;
                continue;
            }
            let item = e.stack.item.clone();
            let wants = villager::food_points(l.items(), &item) > 0
                || picks_up.as_ref().is_some_and(|t| t.contains(&*item))
                || requested.iter().any(|key| key == &*item);
            if !wants {
                j += 1;
                continue;
            }
            let max = l.item_max_stack(&item);
            let stack = l.dropped()[j].stack.clone();
            if !villager::ext_mut(&mut l.parts().mobs[i]).can_add(&stack, max) {
                j += 1;
                continue;
            }
            let before = l.dropped()[j].clone();
            let rest = villager::ext_mut(&mut l.parts().mobs[i]).add_item(stack, max);
            if rest.as_ref().is_none_or(|r| r.count < before.stack.count) {
                let id = l.mobs()[i].id;
                l.take(id, before);
            }
            match rest {
                None => {
                    l.discard_item(j);
                    scan.removed(j);
                }
                Some(rest) => {
                    l.parts().dropped[j].stack.count = rest.count;
                    j += 1;
                }
            }
        }
    }
}

/// `Villager.setLastHurtByMob` and `ServerLevel.onReputationEvent`: a hit
/// from a player counts against that player with the villager that was hit.
pub fn villager_reputation_tick<L: VillagerLevel + ?Sized>(l: &mut L) {
    let mut angry = Vec::new();
    for i in 0..l.mobs().len() {
        let m = &l.mobs()[i];
        if m.species() != Species::Villager
            || m.ai.last_hurt_by != Some(horizon_client::mobs::Who::Player)
        {
            continue;
        }
        let time = m.ai.last_hurt_by_time;
        let alive = m.alive();
        let at = m.id;
        if villager::ext(m).is_some_and(|e| e.hurt_seen == time) {
            continue;
        }
        let Some(uuid) = l.hurt_by_player_uuid(at) else {
            continue;
        };
        let p = l.parts();
        let e = villager::ext_mut(&mut p.mobs[i]);
        e.hurt_seen = time;
        e.gossips.add(
            uuid,
            GossipType::MinorNegative,
            gossip::REPUTATION_CHANGE_PER_EVENT,
        );
        if alive {
            angry.push(at);
        }
    }
    // Villager.setLastHurtByMob: entity event 13.
    for id in angry {
        l.mob_event_particles(id, 13);
    }
}

/// `Villager.tellWitnessesThatIWasMurdered`: the villagers that saw a
/// player kill one remember it as a major wrong.
/// `killer` is the guest connection that last hurt it (None: the host).
pub fn villager_murdered<L: VillagerLevel + ?Sized>(
    l: &mut L,
    victim: &Mob,
    by_player: bool,
    killer: Option<u32>,
) {
    if victim.species() != Species::Villager || !by_player {
        return;
    }
    let Some(uuid) = l.player_uuid_of(killer) else {
        return;
    };
    let snapshot: Vec<_> = l.mobs().iter().map(|m| m.seen(None, l.items())).collect();
    let witnesses =
        villager_brain::murder_witnesses(&mut victim.clone(), &l.mob_world_seen(&snapshot));
    for id in witnesses {
        let p = l.parts();
        let Some(m) = p.mobs.iter_mut().find(|m| m.id == id) else {
            continue;
        };
        villager::ext_mut(m).gossips.add(
            uuid,
            GossipType::MajorNegative,
            gossip::REPUTATION_CHANGE_PER_EVENT,
        );
    }
}

/// Retain the accepted trade's actual player until the eligible mob AI
/// phase, including across menu close. XP and rewards stay in the session.
pub fn villager_trade_pending<L: VillagerLevel + ?Sized>(l: &mut L, id: u32) {
    let Some(player) = l.player_uuid_of(l.lan_acting()) else {
        return;
    };
    let Some(i) = l.villager_index(id) else {
        return;
    };
    villager::ext_mut(&mut l.parts().mobs[i]).last_traded_player = Some(player);
}

/// `Villager.getPlayerReputation` of the local player.
pub fn villager_reputation<L: VillagerLevel + ?Sized>(l: &L, id: u32) -> i32 {
    let Some(uuid) = l.host_uuid() else {
        return 0;
    };
    l.villager_reputation_of(id, uuid)
}

/// `Villager.getPlayerReputation` of the player `uuid`.
pub fn villager_reputation_of<L: VillagerLevel + ?Sized>(l: &L, id: u32, uuid: [i32; 4]) -> i32 {
    l.villager_index(id)
        .and_then(|i| villager::ext(&l.mobs()[i]))
        .map_or(0, |e| e.gossips.reputation(uuid))
}

/// `AbstractBedBlock.kickVillagerOutOfBed`: the first sleeping villager
/// whose box touches the bed's head block wakes up
/// (`Villager.stopSleeping`). Whether one did.
pub fn kick_villager_out_of_bed<L: VillagerLevel + ?Sized>(l: &mut L, head: [i32; 3]) -> bool {
    let min = Vec3::new(f64::from(head[0]), f64::from(head[1]), f64::from(head[2]));
    let max = Vec3::new(min.x + 1.0, min.y + 1.0, min.z + 1.0);
    let Some(i) = l.mobs().iter().position(|m| {
        m.species() == Species::Villager
            && villager::sleeping(m)
            && m.bounds().intersects_corners(min, max)
    }) else {
        return false;
    };
    let id = l.mobs()[i].id;
    l.wake_villager(id);
    true
}
