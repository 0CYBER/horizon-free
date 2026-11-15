//! The benchmark's level: a stone floor at y 63 over a square of otherwise empty chunks.
use super::super::Game;
use horizon_client::{Result, chunk::Chunk};
use horizon_world::level::Ticks;
use std::sync::Arc;

/// The floor's height; entities stand at `FLOOR + 1`.
pub(super) const FLOOR: i32 = 63;

impl Game<'_> {
    /// Replace the level with a fresh one seeded `seed`: the chunks within `radius` of the
    /// origin, plains, lit by the sky, with the floor, and no entity or block entity.
    pub(super) fn hot_level(&mut self, seed: i64, radius: i32) -> Result<()> {
        self.world = Game::world_for_mods(self.res.facts().air, seed);
        let plains = u32::from(world_biomes!(self).id("minecraft:plains"));
        for cx in -radius..=radius {
            for cz in -radius..=radius {
                let mut chunk = Chunk::empty(cx, cz);
                for section in &mut chunk.sections {
                    let section = Arc::make_mut(section);
                    section.biomes = horizon_core::paletted::PalettedContainer::new(
                        horizon_client::chunk::BIOME_STRATEGY,
                        plains,
                    );
                    section.light = horizon_client::chunk::Light::Uniform(15 << 4);
                }
                self.world.chunks.insert((cx, cz), Arc::new(chunk));
            }
        }
        self.block_ticks = Ticks::new(0);
        self.tick_chunks.clear();
        self.sync_tick_chunks();
        self.mobs.clear();
        self.dropped.clear();
        self.orbs.clear();
        self.hoppers.clear();
        self.level_events.clear();
        self.ui.rules = Default::default();
        self.dim = super::super::Dim::Overworld;
        self.world.shape = self.dim.shape();
        self.ui.difficulty = 2;
        self.level.seed = seed;
        self.rand_sequences = Default::default();
        self.world.game_time = 0;
        self.world.day_time = 6000;
        for cx in -radius..=radius {
            for cz in -radius..=radius {
                let (x, z) = (cx * 16, cz * 16);
                self.hot_command(&format!(
                    "fill {x} {FLOOR} {z} {} {FLOOR} {} minecraft:stone",
                    x + 15,
                    z + 15
                ))?;
            }
        }
        // A server ticks only the chunks someone holds: no player is here, so they are forced.
        // The level before this one may have left its own forced (the command fails with none).
        self.run_command_quietly("forceload remove all");
        for cx in -radius..=radius {
            let x = cx * 16;
            self.hot_command(&format!(
                "forceload add {x} {} {x} {}",
                -radius * 16,
                radius * 16
            ))?;
        }
        for rule in ["spawn_mobs", "advance_time", "advance_weather"] {
            self.hot_command(&format!("gamerule {rule} false"))?;
        }
        Ok(())
    }

    /// A setup command that must succeed; its failure message is the error.
    pub(super) fn hot_command(&mut self, command: &str) -> Result<()> {
        let outer = self.chat_capture.replace(Vec::new());
        let ran = self.run_command(command);
        let said = std::mem::replace(&mut self.chat_capture, outer).unwrap_or_default();
        if ran {
            return Ok(());
        }
        let said: Vec<String> = said.iter().map(|text| text.plain()).collect();
        Err(format!(
            "the benchmark's setup command failed: {command}: {}",
            said.join("; ")
        )
        .into())
    }
}
