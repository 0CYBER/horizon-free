//! `CommonPlayerSpawnInfo` for a guest: the login, respawn and travel packets share it.

use super::Hub;
use horizon_protocol::packets::play;

impl Hub {
    /// The levels open on the server, in MinecraftServer.levelKeys order: the Login packet's
    /// `levels`. A runtime level is among them while it is open.
    pub fn level_keys(&self) -> Vec<String> {
        ["overworld", "the_end", "the_nether"]
            .map(|l| format!("minecraft:{l}"))
            .into_iter()
            .chain(
                self.config
                    .levels
                    .extra
                    .iter()
                    .map(|level| level.id.to_owned()),
            )
            .collect()
    }

    /// `CommonPlayerSpawnInfo`, with the player's `portalCooldown`.
    pub(in crate::lan) fn spawn_info(
        &self,
        i: usize,
        portal_cooldown: i32,
        sea_level: i32,
    ) -> play::SpawnInfo {
        let g = &self.guests[i];
        play::SpawnInfo {
            dimension_type: self.dimension_type,
            dimension: self.config.dimension.clone(),
            seed: horizon_core::biome::obfuscate_seed(
                self.config
                    .levels
                    .seed_of(&self.config.dimension)
                    .unwrap_or(self.config.seed),
            ),
            game_type: g.game_mode,
            previous_game_type: None,
            debug: self.debug_world,
            flat: self.flat_world,
            // Player.getLastDeathLocation.
            last_death: g
                .last_death
                .as_ref()
                .map(|(dimension, pos)| play::GlobalPos {
                    dimension: dimension.clone(),
                    pos: *pos,
                }),
            portal_cooldown,
            sea_level,
        }
    }
}
