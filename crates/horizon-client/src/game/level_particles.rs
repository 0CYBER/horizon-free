//! Level events whose client handler (`LevelEventHandler`) spawns particles
//! on a block: bone meal's growth (1505), dragon egg teleportation (2015), a
//! consumable's teleport (2017), electric sparks (3002), waxing (3003), wax
//! off (3004) and scraping (3005).
use super::Game;
use horizon_client::particles::{self, ParticleOptions, ParticleType};
use horizon_core::{position::BlockPos, random::RandomSource};
use horizon_world::level::LevelEvent;

impl Game<'_> {
    pub(super) fn dragon_egg_particles(&mut self, pos: [i32; 3], data: i32) {
        self.particles
            .block_teleportation(pos, data, [16, 8, 16], self.settings.particles);
    }
    /// Event 2017: `TeleportRandomlyConsumeEffect`'s directional particles.
    pub(super) fn consume_teleport_particles(&mut self, pos: [i32; 3], data: i32) {
        self.particles
            .block_teleportation(pos, data, [127, 127, 127], self.settings.particles);
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
    /// `BoneMealItem.addGrowthParticles` (event 1505, `count` the data):
    /// happy villager sparks over a spreading block's neighbourhood (grass,
    /// nylium, netherrack, moss) or water, or inside a growing block.
    pub(super) fn growth_particles(&mut self, pos: BlockPos, count: i32) {
        let p = [pos.x, pos.y, pos.z];
        let state = self.world.block(p[0], p[1], p[2]);
        let facts = self.res.facts();
        // ParticleUtils.spawnParticles(level, pos, count, xzSpread, ySpread,
        // allowFloatingParticles, particle).
        let (at, count, xz, height, floating) = match self.res.behaviours().bone_meal_spreads(state)
        {
            Some(spreads) => {
                // BonemealableBlock.getParticlePos: below for rooted dirt
                // (its hanging roots) and mangrove leaves (their propagule).
                let at = match &*facts.get(state).name {
                    "minecraft:rooted_dirt" | "minecraft:mangrove_leaves" => [p[0], p[1] - 1, p[2]],
                    _ => p,
                };
                if spreads {
                    (at, count * 3, 3.0, 1.0, false)
                } else {
                    // ParticleUtils.spawnParticleInBlock: up to the
                    // block's shape (a full block when air).
                    let inside = self.world.block(at[0], at[1], at[2]);
                    let height = if facts.is_air(inside) {
                        1.0
                    } else {
                        facts
                            .get(inside)
                            .outline
                            .iter()
                            .map(|b| b[4])
                            .fold(0.0, f64::max)
                    };
                    (at, count, 0.5, height, true)
                }
            }
            None if &*facts.get(state).name == "minecraft:water" => (p, count * 3, 3.0, 1.0, false),
            None => return,
        };
        let options = ParticleOptions::simple(ParticleType::HappyVillager);
        for _ in 0..count.max(0) {
            let r = self.particles.level_random();
            let v = [0.0; 3].map(|_: f64| r.next_gaussian() * 0.02);
            let start = 0.5 - xz;
            let x = f64::from(at[0]) + start + r.next_double() * xz * 2.0;
            let y = f64::from(at[1]) + r.next_double() * height;
            let z = f64::from(at[2]) + start + r.next_double() * xz * 2.0;
            let below = [x.floor() as i32, y.floor() as i32 - 1, z.floor() as i32];
            if floating
                || !self
                    .res
                    .facts()
                    .is_air(self.world.block(below[0], below[1], below[2]))
            {
                self.particles.add(&options, [x, y, z], v);
            }
        }
    }
    /// `ServerLevel.globalLevelEvent`: heard by every player on the server
    /// (an end portal opening, the dragon's death roar): here, and sent to
    /// every guest whatever their dimension.
    pub(super) fn global_level_event(&mut self, id: i32, pos: [i32; 3], data: i32) {
        self.global_level_event_sound(id, pos);
        if self.audio.capture.is_some() {
            self.lan_global_events.push((id, pos, data));
        }
    }
    /// `LevelEventHandler.globalLevelEvent`: the sound two blocks from the
    /// camera towards the event, so it is heard from anywhere.
    pub(super) fn global_level_event_sound(&mut self, id: i32, pos: [i32; 3]) {
        let (sound, volume) = match id {
            1023 => ("minecraft:entity.wither.spawn", 1.0),
            1028 => ("minecraft:entity.ender_dragon.death", 5.0),
            1038 => ("minecraft:block.end_portal.spawn", 1.0),
            _ => return,
        };
        let at = self.eye_position().map(|eye| {
            let d = [
                f64::from(pos[0]) + 0.5 - eye[0],
                f64::from(pos[1]) + 0.5 - eye[1],
                f64::from(pos[2]) + 0.5 - eye[2],
            ];
            let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if length < 1.0e-4 {
                return eye;
            }
            [0, 1, 2].map(|k| eye[k] + d[k] / length * 2.0)
        });
        // Each guest plays it from the global packet.
        self.audio.play_unshared(sound, at, volume, 1.0);
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
