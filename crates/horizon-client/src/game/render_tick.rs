//! Per-tick client visuals: the particle engine, `ClientLevel.animateTick`,
//! campfire smoke and rain splashes (`ClientLevel.tickWeatherEffects`).
use super::{Dim, Game};
use horizon_client::{
    animate_tick::{AnimateContext, animate_tick, campfire_particles},
    biome::Biomes,
    blocks::{BlockFacts, Fluid},
    chunk::{HEIGHT, MIN_Y},
    particles::{ParticleLevel, ParticleOptions, ParticleType},
    world::World,
};
use horizon_core::random::{LegacyRandom, RandomSource};

/// The first motion-blocking block at or below `from` (Heightmap MOTION_BLOCKING), plus one.
fn motion_blocking(world: &World, facts: &BlockFacts, x: i32, z: i32, from: i32) -> i32 {
    let mut y = from.min(MIN_Y + HEIGHT - 1);
    while y > MIN_Y {
        let f = facts.get(world.block(x, y, z));
        if f.full_collision || f.fluid != Fluid::None || f.name.contains("leaves") {
            return y + 1;
        }
        y -= 1;
    }
    MIN_Y
}
/// `Biome.getPrecipitationAt` is rain (not snow) at this position.
fn rains_at(world: &World, biomes: &Biomes, x: i32, y: i32, z: i32) -> bool {
    let Some(chunk) = world.chunks.get(&(x >> 4, z >> 4)) else {
        return false;
    };
    let b = biomes.get(chunk.biome(x, y, z));
    if !b.precipitation {
        return false;
    }
    let t = if y > 80 {
        b.temperature - (y - 80) as f32 * 0.05 / 40.0
    } else {
        b.temperature
    };
    t >= 0.15
}

impl Game<'_> {
    pub(super) fn tick_particles(&mut self) {
        let eye = self.eye_position();
        if let Some(e) = eye {
            self.particles.camera = e;
        }
        let (rain, _) = if self.dim == Dim::Overworld {
            self.world.weather.levels(1.0)
        } else {
            (0.0, 0.0)
        };
        let world = &self.world;
        let facts = &self.res.facts;
        let biomes = &self.res.biomes;
        let raining = rain > 0.2;
        let raining_at = |p: [i32; 3]| {
            raining
                && motion_blocking(world, facts, p[0], p[2], p[1] + 64) <= p[1]
                && rains_at(world, biomes, p[0], p[1], p[2])
        };
        let ctx = AnimateContext {
            world,
            facts,
            collision: self.collision,
            biomes,
            registry: self.res.assets.registry,
            raining_at: &raining_at,
        };
        if let Some(e) = eye {
            let block = [
                e[0].floor() as i32,
                e[1].floor() as i32,
                e[2].floor() as i32,
            ];
            animate_tick(&self.res.animate, &mut self.particles, &ctx, block);
            if rain > 0.0 {
                // ClientLevel.tickWeatherEffects: rain splashes (weather radius 10).
                let mut random = LegacyRandom::new(self.world.game_time.wrapping_mul(312_987_231));
                let radius = 10;
                let diameter = 2 * radius + 1;
                let count = (0.225 * (diameter * diameter) as f32 * rain * rain) as i32;
                for _ in 0..count {
                    let x = block[0] + random.next_int_bound(diameter).unwrap_or(0) - radius;
                    let z = block[2] + random.next_int_bound(diameter).unwrap_or(0) - radius;
                    let top = motion_blocking(world, facts, x, z, block[1] + 64);
                    if top <= MIN_Y
                        || top > block[1] + 10
                        || top < block[1] - 10
                        || !rains_at(world, biomes, x, top, z)
                    {
                        continue;
                    }
                    let below = top - 1;
                    let (bx, bz) = (random.next_double(), random.next_double());
                    let state = world.block(x, below, z);
                    let f = facts.get(state);
                    let block_top = match self.collision.cached_shapes().cached_shape(state) {
                        Ok(Some(shape)) if !shape.is_empty() => {
                            shape.max_at(horizon_core::geometry::Axis::Y, bx, bz)
                        }
                        _ => 0.0,
                    };
                    let fluid_top = if f.fluid == Fluid::None {
                        0.0
                    } else {
                        f64::from(f.fluid_level) / 9.0
                    };
                    let smoke = f.fluid == Fluid::Lava
                        || &*f.name == "minecraft:magma_block"
                        || (f.name.ends_with("campfire")
                            && self.res.animate.is_lit_campfire(state.0));
                    let ty = if smoke {
                        ParticleType::Smoke
                    } else {
                        ParticleType::Rain
                    };
                    let p = [
                        f64::from(x) + bx,
                        f64::from(below) + block_top.max(fluid_top),
                        f64::from(z) + bz,
                    ];
                    self.particles
                        .add(&ParticleOptions::simple(ty), p, [0.0; 3]);
                }
            }
        }
        if let Some(r) = &self.renderer {
            campfire_particles(
                &self.res.animate,
                &mut self.particles,
                &ctx,
                r.block_entities(),
            );
        }
        let level = ParticleLevel {
            world,
            facts,
            collision: self.collision,
            player: self
                .player
                .as_ref()
                .map(|p| ([p.position.x, p.position.y, p.position.z], p.velocity.y)),
        };
        self.particles.tick(&level);
        for (name, at, volume, pitch) in std::mem::take(&mut self.particles.sounds) {
            self.audio.play(name, Some(at), volume, pitch);
        }
    }
}
