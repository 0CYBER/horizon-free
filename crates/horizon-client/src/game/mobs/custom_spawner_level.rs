//! `CustomSpawnerLevel` for the game: the custom spawners' level services,
//! forwarded to the game's own.
use super::{Dim, Game, creatures::SpawnSite};
use horizon_client::mobs::{Species, raider, spawn_rules};
use horizon_core::random::{LegacyRandom, LevelRandom};
use horizon_gameplay::custom_spawner::{CustomSpawnerLevel, SpawnerParts, SpawnerPlayer};
use std::collections::HashSet;

impl CustomSpawnerLevel for Game<'_> {
    fn parts(&mut self) -> SpawnerParts<'_> {
        SpawnerParts {
            world: &mut self.world,
            mobs: &mut self.mobs,
            facts: self.res.facts(),
        }
    }
    fn overworld(&self) -> bool {
        self.dim == Dim::Overworld
    }
    fn spawn_mobs_and_monsters(&self) -> bool {
        self.ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::SPAWN_MOBS)
            && self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::SPAWN_MONSTERS)
    }
    fn players(&self) -> Vec<SpawnerPlayer> {
        self.level_players_iter()
            .map(|p| SpawnerPlayer {
                uuid: p.uuid,
                position: p.position,
                alive: p.alive,
                spectator: p.spectator,
            })
            .collect()
    }
    fn village_centers(&self) -> HashSet<(i32, i32, i32)> {
        Game::village_centers(self)
    }
    fn sky_darken(&self) -> i32 {
        Game::sky_darken(self)
    }
    fn surface_height(&self, x: i32, z: i32) -> i32 {
        Game::surface_height(self, x, z)
    }
    fn motion_top(&self, x: i32, z: i32) -> i32 {
        Game::motion_top(self, x, z)
    }
    fn biome_tag(&self, tag: &str, pos: [i32; 3]) -> bool {
        spawn_rules::SpawnLevel::biome_tag(&self.natural_level(), tag, pos[0], pos[1], pos[2])
    }
    fn chunk_biome_has(&self, tag: &str, pos: [i32; 3]) -> bool {
        let biome = self.world.chunks.get(&(pos[0] >> 4, pos[2] >> 4)).map(|c| {
            self.res
                .biomes()
                .get(c.biome(pos[0], pos[1], pos[2]))
                .name
                .clone()
        });
        biome.is_some_and(|b| self.res.mob_data().biome_has(tag, &b))
    }
    fn structure_tag(&self, tag: &str) -> Vec<String> {
        Game::structure_tag(self, tag)
    }
    fn structures_at(&self, pos: [i32; 3]) -> Vec<String> {
        Game::structures_at(self, pos)
    }
    fn is_spawn_position_ok(&self, entity: &str, species: Species, pos: [i32; 3]) -> bool {
        spawn_rules::is_spawn_position_ok(
            entity,
            species,
            &self.mob_world_base(&[]),
            pos[0],
            pos[1],
            pos[2],
        )
    }
    fn check_spawn_rules(
        &mut self,
        entity: &str,
        species: Species,
        reason: spawn_rules::Reason,
        pos: [i32; 3],
    ) -> bool {
        // The level random is lent to the rules while the level is read.
        let mut random = std::mem::replace(
            &mut self.world.random,
            LevelRandom::for_level(LegacyRandom::new(0)),
        );
        let ok = spawn_rules::check_spawn_rules(
            entity,
            species,
            &self.natural_level(),
            reason,
            &mut random,
            pos,
        );
        self.world.random = random;
        ok
    }
    fn spawn_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        reason: spawn_rules::Reason,
    ) -> Option<u32> {
        Game::spawn_mob(self, kind, at, persistent, SpawnSite::Finalizing(reason))
    }
    fn spawn_mob_facing(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        reason: spawn_rules::Reason,
        yaw: f32,
    ) -> Option<u32> {
        Game::spawn_mob_facing(
            self,
            kind,
            at,
            persistent,
            SpawnSite::Finalizing(reason),
            yaw,
        )
    }
    fn spawn_event_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        leader: bool,
        reason: raider::Reason,
    ) -> Option<u32> {
        Game::spawn_event_mob(self, kind, at, leader, reason)
    }
    fn village_siege_cancelled(&mut self, player: u128, attempted: [f64; 3]) -> bool {
        let mut canceled = false;
        self.native_village_siege(self.dim.id(), player, attempted, &mut canceled);
        canceled
    }
}
