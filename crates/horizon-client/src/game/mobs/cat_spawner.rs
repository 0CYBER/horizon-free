//! `CatSpawner`: every 1200 ticks a cat may appear near a random player,
//! in a village with enough occupied beds or inside a swamp hut.
use super::{
    Dim, Game,
    raids::{section, sections_to_village},
};
use horizon_client::mobs::{Species, spawn_rules};
use horizon_core::geometry::Vec3;
use horizon_gameplay::cat_spawner::{candidate, hut_takes_cat, village_takes_cat};

pub(super) use horizon_gameplay::cat_spawner::CatSpawner;

impl Game<'_> {
    /// `CatSpawner.tick` (the overworld's custom spawner).
    pub(super) fn tick_cat_spawner(&mut self) {
        if self.dim != Dim::Overworld || !self.creatures.cat.due() {
            return;
        }
        // ServerLevel.getRandomPlayer: one of the living players.
        let alive: Vec<Vec3> = self
            .level_players()
            .into_iter()
            .filter(|p| p.alive)
            .map(|p| p.position)
            .collect();
        let Some(pos) = candidate(&mut self.world.random, &alive, &self.world.chunks) else {
            return;
        };
        let ok = spawn_rules::is_spawn_position_ok(
            "minecraft:cat",
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
            if village_takes_cat(&self.mobs, pos) {
                self.spawn_cat(pos, false);
            }
        } else {
            let hut = self.structure_tag("minecraft:cats_spawn_in");
            if self.structures_at(pos).iter().any(|s| hut.contains(s))
                && hut_takes_cat(&self.mobs, pos)
            {
                self.spawn_cat(pos, true);
            }
        }
    }
    /// `spawnCat`: a natural spawn, then `snapTo(spawnPos, 0.0F, 0.0F)`, so
    /// the cat faces yaw 0 and its facing takes no draw.
    fn spawn_cat(&mut self, pos: [i32; 3], persistent: bool) {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        ];
        let Some(id) = self.spawn_mob_facing(
            "minecraft:cat",
            at,
            persistent,
            super::creatures::SpawnSite::Finalizing(
                horizon_client::mobs::spawn_rules::Reason::Natural,
            ),
            0.0,
        ) else {
            return;
        };
        // `Cat.finalizeSpawn` asks `#cats_spawn_as_black` about the entity's
        // own block position, which is still the origin here: `snapTo` runs
        // after it. So the hut the cat spawns in does not make it black.
        let black = self.structure_tag("minecraft:cats_spawn_as_black");
        let at_origin = self.structures_at([0, 0, 0]);
        if at_origin.iter().any(|s| black.contains(s))
            && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
        {
            m.extra.variant = 10;
        }
    }
}

#[cfg(test)]
#[path = "tests/cat_spawner_tests.rs"]
mod cat_spawner_tests;
