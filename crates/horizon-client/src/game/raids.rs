//! Pillager patrols (`PatrolSpawner`) and the village sections they avoid.
use super::{Dim, Game};
use horizon_client::mobs::{Species, data, raider};
use horizon_core::random::RandomSource;
use std::collections::HashSet;

/// Patrol and raid state owned by the game.
#[derive(Default)]
pub(super) struct Raiding {
    /// `PatrolSpawner.nextTick`.
    patrol_next_tick: i32,
}

/// `SectionPos` of a block position.
pub(super) fn section(p: [i32; 3]) -> (i32, i32, i32) {
    (p[0] >> 4, p[1] >> 4, p[2] >> 4)
}
/// `PoiManager.DistanceTracker`: sections from the nearest village centre,
/// 7 when farther than 6.
pub(super) fn sections_to_village(centers: &HashSet<(i32, i32, i32)>, at: (i32, i32, i32)) -> i32 {
    centers
        .iter()
        .map(|c| {
            (c.0 - at.0)
                .abs()
                .max((c.1 - at.1).abs())
                .max((c.2 - at.2).abs())
        })
        .min()
        .map_or(7, |d| d.min(7))
}

impl Game<'_> {
    /// Sections holding a claimed village point of interest (a job site,
    /// bed or bell a villager has taken): `PoiManager.isVillageCenter`.
    pub(super) fn village_centers(&self) -> HashSet<(i32, i32, i32)> {
        let mut out = HashSet::new();
        for m in &self.mobs {
            if m.species() != Species::Villager || !m.alive() {
                continue;
            }
            for p in [m.extra.home, m.extra.bed, m.extra.meeting]
                .into_iter()
                .flatten()
            {
                out.insert(section(p));
            }
        }
        out
    }
    /// `ServerLevel.isCloseToVillage`.
    pub(super) fn close_to_village(&self, at: [i32; 3], distance: i32) -> bool {
        distance <= 6 && sections_to_village(&self.village_centers(), section(at)) <= distance
    }
    /// `PatrolSpawner.tick`: every 10 minutes, a fifth of the time in
    /// daylight, a patrol of pillagers spawns 24 to 48 blocks from a player
    /// who is not near a village.
    pub(super) fn tick_patrol_spawner(&mut self) {
        if self.dim != Dim::Overworld
            || self.world.health_rules.difficulty == horizon_world::food::Difficulty::Peaceful
            || !self.ui.rules.bool("spawn_monsters")
            || !self.ui.rules.bool("spawn_patrols")
        {
            return;
        }
        self.raiding.patrol_next_tick -= 1;
        if self.raiding.patrol_next_tick > 0 {
            return;
        }
        let delay = self.world.random.next_int_bound(1200).unwrap_or(0);
        self.raiding.patrol_next_tick += 12000 + delay;
        let bright = self.sky_darken() < 4;
        let r = &mut self.world.random;
        if !bright || r.next_int_bound(5).unwrap_or(1) != 0 {
            return;
        }
        // One player: `players.get(random.nextInt(1))`.
        let _ = r.next_int_bound(1);
        let Some(p) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        if self.spectator() {
            return;
        }
        let block = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
        if self.close_to_village(block, 2) {
            return;
        }
        let r = &mut self.world.random;
        let x = (24 + r.next_int_bound(24).unwrap_or(0)) * if r.next_boolean() { -1 } else { 1 };
        let z = (24 + r.next_int_bound(24).unwrap_or(0)) * if r.next_boolean() { -1 } else { 1 };
        let mut at = [block[0] + x, block[1], block[2] + z];
        // hasChunksAt(±10).
        for (dx, dz) in [(-10, -10), (10, -10), (-10, 10), (10, 10)] {
            if !self
                .world
                .chunks
                .contains_key(&((at[0] + dx) >> 4, (at[2] + dz) >> 4))
            {
                return;
            }
        }
        // The `can_pillager_patrol_spawn` attribute: the early game timeline
        // (first five days) and the mushroom fields forbid it.
        let biome = self.biome_facts(at[0], at[1], at[2]);
        if self.world.day_time < 120_000 || biome.mushroom {
            return;
        }
        let f = self.local_difficulty([f64::from(at[0]), f64::from(at[1]), f64::from(at[2])]);
        let group = f.effective.ceil() as i32 + 1;
        for i in 0..group {
            at[1] = self.motion_top(at[0], at[2]);
            if !self.spawn_patrol_member(at, i == 0) && i == 0 {
                break;
            }
            let r = &mut self.world.random;
            at[0] += r.next_int_bound(5).unwrap_or(0) - r.next_int_bound(5).unwrap_or(0);
            at[2] += r.next_int_bound(5).unwrap_or(0) - r.next_int_bound(5).unwrap_or(0);
        }
    }
    /// A raider's ominous banner (drop chance 2.0: always dropped) and a
    /// patrol captain's ominous bottle (`entities/pillager`).
    pub(super) fn raider_death_loot(&mut self, m: &horizon_client::mobs::Mob, pos: [f64; 3]) {
        if !raider::is_raider(m.species()) {
            return;
        }
        if m.extra.head.as_deref() == Some(raider::BANNER) {
            self.spawn_item(raider::ominous_banner(), pos, [0.0, 0.2, 0.0], 10);
        }
        if m.species() == Species::Pillager && raider::is_captain(m) {
            let amplifier = self.world.random.next_int_bound(5).unwrap_or(0) as u8;
            let amplifier = (amplifier > 0).then_some(amplifier);
            let mut bottle = horizon_client::items::ItemStack::new("minecraft:ominous_bottle", 1);
            bottle.data_mut().ominous_amplifier = amplifier;
            self.spawn_item(bottle, pos, [0.0, 0.2, 0.0], 10);
        }
    }
    /// `OminousBottleAmplifier.onConsume`: Bad Omen for 100 minutes at the
    /// bottle's amplifier, then the dispose sound; the bottle is used up.
    pub(super) fn drink_ominous_bottle(
        &mut self,
        slot: usize,
        stack: &horizon_client::items::ItemStack,
    ) {
        let amplifier = stack.data().and_then(|d| d.ominous_amplifier).unwrap_or(0);
        let mut effect =
            horizon_client::effects::EffectInstance::new("bad_omen", 120_000, i32::from(amplifier));
        effect.visible = false;
        self.potions.effects.add(effect);
        let eye = self.eye_position().unwrap_or_default();
        self.audio
            .play("minecraft:item.ominous_bottle.dispose", Some(eye), 1.0, 1.0);
        if !self.creative
            && let Some(s) = self.inventory[slot].as_mut()
        {
            s.count -= 1;
            if s.count == 0 {
                self.inventory[slot] = None;
            }
        }
    }
    /// `PatrolSpawner.spawnPatrolMember`: a pillager on a valid empty block
    /// that may spawn there; the leader picks a target and wears the banner.
    fn spawn_patrol_member(&mut self, at: [i32; 3], leader: bool) -> bool {
        let state = self.world.block(at[0], at[1], at[2]);
        let below = self.world.block(at[0], at[1] - 1, at[2]);
        let empty = self.res.mob_data.state(state).flags & data::VALID_EMPTY_SPAWN != 0;
        let valid = self.res.mob_data.state(below).flags & data::VALID_SPAWN != 0;
        // PatrollingMonster.checkPatrollingMonsterSpawnRules.
        if !empty || !valid || self.world.light(at[0], at[1], at[2]) & 15 > 8 {
            return false;
        }
        let pos = [f64::from(at[0]), f64::from(at[1]), f64::from(at[2])];
        let Some(id) = self.spawn_mob_for("minecraft:pillager", pos, false, true) else {
            return false;
        };
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) else {
            return false;
        };
        if leader {
            raider::set_patrol_leader(m, true);
            // findPatrolTarget runs before the mob is placed, so the target
            // is around the world origin.
            raider::find_patrol_target_from(m, [0, 0, 0]);
        }
        raider::finalize(m, raider::Reason::Patrol);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_to_village_is_a_chebyshev_distance_capped_at_seven() {
        let mut centers = HashSet::new();
        assert_eq!(sections_to_village(&centers, (0, 0, 0)), 7);
        centers.insert(section([40, 64, 40]));
        assert_eq!(sections_to_village(&centers, (2, 4, 2)), 0);
        assert_eq!(sections_to_village(&centers, (4, 4, 2)), 2);
        assert_eq!(sections_to_village(&centers, (-10, 4, 2)), 7);
        assert_eq!(section([-1, -17, 15]), (-1, -2, 0));
    }
}
