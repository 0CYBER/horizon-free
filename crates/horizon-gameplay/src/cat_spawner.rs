//! `CatSpawner`: every 1200 ticks a cat may appear near a random player,
//! in a village with enough occupied beds or inside a swamp hut.
use crate::custom_spawner::{CustomSpawnerLevel, section, sections_to_village};
use horizon_client::chunk_map::ChunkMap;
use horizon_client::mobs::{Mob, Species, spawn_rules};
use horizon_core::{geometry::Vec3, random::RandomSource};

pub const TICK_DELAY: i32 = 1200;

/// What the cat spawner calls on its level beyond [`CustomSpawnerLevel`]: the game's own
/// `spawn_cat`, which mods target. It runs [`spawn_cat`].
pub trait CatLevel: CustomSpawnerLevel {
    /// `spawnCat`.
    fn spawn_cat(&mut self, pos: [i32; 3], persistent: bool);
}

/// `CatSpawner.nextTick`.
#[derive(Default)]
pub struct CatSpawner {
    pub next_tick: i32,
}
impl CatSpawner {
    /// Counts down one tick; true when a spawn attempt is due.
    pub fn due(&mut self) -> bool {
        self.next_tick -= 1;
        if self.next_tick > 0 {
            return false;
        }
        self.next_tick = TICK_DELAY;
        true
    }

    /// `CatSpawner.tick` (the overworld's custom spawner).
    pub fn tick(&mut self, l: &mut impl CatLevel) {
        if !l.overworld() || !self.due() {
            return;
        }
        // ServerLevel.getRandomPlayer: one of the living players.
        let alive: Vec<Vec3> = l
            .players()
            .into_iter()
            .filter(|p| p.alive)
            .map(|p| p.position)
            .collect();
        let p = l.parts();
        let Some(pos) = candidate(&mut p.world.random, &alive, &p.world.chunks) else {
            return;
        };
        if !l.is_spawn_position_ok("minecraft:cat", Species::Cat, pos) {
            return;
        }
        if sections_to_village(&l.village_centers(), section(pos)) <= 2 {
            if village_takes_cat(l.parts().mobs, pos) {
                l.spawn_cat(pos, false);
            }
        } else {
            let hut = l.structure_tag("minecraft:cats_spawn_in");
            if l.structures_at(pos).iter().any(|s| hut.contains(s))
                && hut_takes_cat(l.parts().mobs, pos)
            {
                l.spawn_cat(pos, true);
            }
        }
    }
}

/// `spawnCat`: a natural spawn, then `snapTo(spawnPos, 0.0F, 0.0F)`, so
/// the cat faces yaw 0 and its facing takes no draw.
pub fn spawn_cat(l: &mut impl CustomSpawnerLevel, pos: [i32; 3], persistent: bool) {
    let at = [
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]),
        f64::from(pos[2]) + 0.5,
    ];
    let Some(id) = l.spawn_mob_facing(
        "minecraft:cat",
        at,
        persistent,
        spawn_rules::Reason::Natural,
        0.0,
    ) else {
        return;
    };
    // `Cat.finalizeSpawn` asks `#cats_spawn_as_black` about the entity's
    // own block position, which is still the origin here: `snapTo` runs
    // after it. So the hut the cat spawns in does not make it black.
    let black = l.structure_tag("minecraft:cats_spawn_as_black");
    let at_origin = l.structures_at([0, 0, 0]);
    if at_origin.iter().any(|s| black.contains(s))
        && let Some(m) = l.parts().mobs.iter_mut().find(|m| m.id == id)
    {
        m.extra.variant = 10;
    }
}

/// `getRandomPlayer` plus two offsets of 8 to 31 blocks, if `hasChunksAt(pos +- 10)`.
pub fn candidate(
    random: &mut impl RandomSource,
    alive: &[Vec3],
    chunks: &ChunkMap,
) -> Option<[i32; 3]> {
    if alive.is_empty() {
        return None;
    }
    let pick = random.next_int_bound(alive.len() as i32).unwrap_or(0);
    let p = alive[pick as usize];
    let mut offset = || {
        let n = 8 + random.next_int_bound(24).unwrap_or(0);
        if random.next_boolean() { -n } else { n }
    };
    let x = offset();
    let z = offset();
    let pos = [
        p.x.floor() as i32 + x,
        p.y.floor() as i32,
        p.z.floor() as i32 + z,
    ];
    let loaded = ((pos[0] - 10) >> 4..=(pos[0] + 10) >> 4).all(|cx| {
        ((pos[2] - 10) >> 4..=(pos[2] + 10) >> 4).all(|cz| chunks.contains_key(&(cx, cz)))
    });
    loaded.then_some(pos)
}

/// `spawnInVillage`: more than four occupied beds within 48 blocks and fewer than five cats around.
pub fn village_takes_cat(mobs: &[Mob], pos: [i32; 3]) -> bool {
    // PoiManager.getCountInRange(HOME, IS_OCCUPIED): beds villagers have claimed.
    let mut beds: Vec<[i32; 3]> = mobs
        .iter()
        .filter(|m| m.species() == Species::Villager && m.alive())
        .filter_map(|m| m.extra.bed)
        .filter(|b| {
            let d = |i: usize| i64::from(b[i] - pos[i]);
            d(0).abs() <= 48
                && d(2).abs() <= 48
                && d(0).pow(2) + d(1).pow(2) + d(2).pow(2) <= 48 * 48
        })
        .collect();
    beds.sort_unstable();
    beds.dedup();
    beds.len() > 4 && cats_near(mobs, pos, 48.0) < 5
}

/// `spawnInHut`: no cat within 16 blocks.
pub fn hut_takes_cat(mobs: &[Mob], pos: [i32; 3]) -> bool {
    cats_near(mobs, pos, 16.0) == 0
}

/// Cats in `AABB(pos).inflate(radius, 8, radius)`.
pub fn cats_near(mobs: &[Mob], pos: [i32; 3], radius: f64) -> usize {
    let [x, y, z] = pos.map(f64::from);
    let min = Vec3::new(x - radius, y - 8.0, z - radius);
    let max = Vec3::new(x + 1.0 + radius, y + 9.0, z + 1.0 + radius);
    mobs.iter()
        .filter(|m| {
            m.species() == Species::Cat && m.alive() && m.bounds().intersects_corners(min, max)
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::LegacyRandom;

    #[test]
    fn due_fires_on_the_first_tick_then_every_1200() {
        let mut c = CatSpawner::default();
        assert!(c.due());
        assert!((0..1199).all(|_| !c.due()));
        assert!(c.due());
    }

    #[test]
    fn no_player_draws_nothing() {
        let mut random = LegacyRandom::new(7);
        let mut fresh = LegacyRandom::new(7);
        assert_eq!(candidate(&mut random, &[], &ChunkMap::default()), None);
        assert_eq!(random.next_int(), fresh.next_int());
    }
}
