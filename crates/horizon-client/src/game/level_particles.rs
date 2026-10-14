//! Level events whose client handler (`LevelEventHandler`) spawns particles
//! on a block: dragon egg teleportation (2015), electric sparks (3002),
//! waxing (3003), wax off (3004) and scraping (3005).
use super::Game;
use horizon_client::particles::{self, ParticleOptions, ParticleType};
use horizon_core::position::BlockPos;
use horizon_world::level::LevelEvent;

impl Game<'_> {
    pub(super) fn dragon_egg_particles(&mut self, pos: [i32; 3], data: i32) {
        self.particles
            .block_teleportation(pos, data, self.settings.particles);
    }
    /// `LevelEventHandler.levelEvent` 3002 to 3005, drawing from the client
    /// level's random.
    pub(super) fn block_particle_event(&mut self, id: i32, pos: BlockPos, data: i32) {
        let ty = match id {
            3002 => ParticleType::ElectricSpark,
            3003 => ParticleType::WaxOn,
            3004 => ParticleType::WaxOff,
            3005 => ParticleType::Scrape,
            _ => return,
        };
        let p = [pos.x, pos.y, pos.z];
        let mut spawned = Vec::new();
        let random = self.particles.level_random();
        // 3002 with an axis: a lightning rod; otherwise copper cleaned by lightning.
        match usize::try_from(data) {
            Ok(axis) if id == 3002 && axis < 3 => {
                particles::along_axis(random, axis, p, |p, v| spawned.push((p, v)));
            }
            _ => particles::on_block_faces(random, p, |p, v| spawned.push((p, v))),
        }
        let options = ParticleOptions::simple(ty);
        for (p, v) in spawned {
            self.particles.add(&options, p, v);
        }
    }
    /// Queue and play level event `id` at `pos` (`Level.levelEvent`).
    pub(super) fn level_event_at(&mut self, id: i32, pos: [i32; 3], data: i32) {
        self.level_events.push(LevelEvent::LevelEvent {
            id,
            pos: BlockPos::new(pos[0], pos[1], pos[2]),
            data,
        });
        let _ = self.process_level_events();
    }
}
