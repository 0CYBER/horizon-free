//! `CatSpawner`: every 1200 ticks a cat may appear near a random player,
//! in a village with enough occupied beds or inside a swamp hut.
use super::{
    Dim, Game,
    raids::{section, sections_to_village},
};
use horizon_client::mobs::{Species, spawn_rules};
use horizon_core::{geometry::Vec3, random::RandomSource};

const TICK_DELAY: i32 = 1200;

impl Game<'_> {
    /// `CatSpawner.tick` (the overworld's custom spawner).
    pub(super) fn tick_cat_spawner(&mut self) {
        if self.dim != Dim::Overworld {
            return;
        }
        self.creatures.cat_tick -= 1;
        if self.creatures.cat_tick > 0 {
            return;
        }
        self.creatures.cat_tick = TICK_DELAY;
        // ServerLevel.getRandomPlayer: one of the living players.
        let alive: Vec<Vec3> = self
            .level_players()
            .into_iter()
            .filter(|p| p.alive)
            .map(|p| p.position)
            .collect();
        if alive.is_empty() {
            return;
        }
        let pick = self
            .world
            .random
            .next_int_bound(alive.len() as i32)
            .unwrap_or(0);
        let p = alive[pick as usize];
        let mut offset = || {
            let n = 8 + self.world.random.next_int_bound(24).unwrap_or(0);
            if self.world.random.next_boolean() {
                -n
            } else {
                n
            }
        };
        let x = offset();
        let z = offset();
        let pos = [
            p.x.floor() as i32 + x,
            p.y.floor() as i32,
            p.z.floor() as i32 + z,
        ];
        // hasChunksAt(pos +- 10)
        let loaded = ((pos[0] - 10) >> 4..=(pos[0] + 10) >> 4).all(|cx| {
            ((pos[2] - 10) >> 4..=(pos[2] + 10) >> 4)
                .all(|cz| self.world.chunks.contains_key(&(cx, cz)))
        });
        if !loaded {
            return;
        }
        let ok = spawn_rules::is_spawn_position_ok(
            Species::Cat,
            &self.mob_world_base(&[]),
            pos[0],
            pos[1],
            pos[2],
        );
        if !ok {
            return;
        }
        if sections_to_village(&self.village_centers(), section(pos)) <= 2 {
            self.spawn_cat_in_village(pos);
        } else {
            let hut = self.structure_tag("minecraft:cats_spawn_in");
            if self.structures_at(pos).iter().any(|s| hut.contains(s)) {
                self.spawn_cat_in_hut(pos);
            }
        }
    }
    /// `spawnInVillage`: more than four occupied beds within 48 blocks and
    /// fewer than five cats around.
    fn spawn_cat_in_village(&mut self, pos: [i32; 3]) {
        // PoiManager.getCountInRange(HOME, IS_OCCUPIED): beds villagers have claimed.
        let mut beds: Vec<[i32; 3]> = self
            .mobs
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
        if beds.len() > 4 && self.cats_near(pos, 48.0) < 5 {
            self.spawn_cat(pos, false);
        }
    }
    /// `spawnInHut`: no cat within 16 blocks; the cat stays.
    fn spawn_cat_in_hut(&mut self, pos: [i32; 3]) {
        if self.cats_near(pos, 16.0) == 0 {
            self.spawn_cat(pos, true);
        }
    }
    /// Cats in `AABB(pos).inflate(radius, 8, radius)`.
    fn cats_near(&self, pos: [i32; 3], radius: f64) -> usize {
        let [x, y, z] = pos.map(f64::from);
        let min = Vec3::new(x - radius, y - 8.0, z - radius);
        let max = Vec3::new(x + 1.0 + radius, y + 9.0, z + 1.0 + radius);
        self.mobs
            .iter()
            .filter(|m| {
                m.species() == Species::Cat && m.alive() && m.bounds().intersects_corners(min, max)
            })
            .count()
    }
    /// `spawnCat`: finalized for a natural spawn, black inside
    /// `#cats_spawn_as_black` structures (`Cat.finalizeSpawn`).
    fn spawn_cat(&mut self, pos: [i32; 3], persistent: bool) {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        ];
        let Some(id) = self.spawn_mob("minecraft:cat", at, persistent) else {
            return;
        };
        let black = self.structure_tag("minecraft:cats_spawn_as_black");
        let in_black = self.structures_at(pos).iter().any(|s| black.contains(s));
        if in_black && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.extra.variant = 10;
        }
    }
}
