//! An explosion's particles on the client: the one at its centre
//! (`ClientPacketListener.handleExplosion`) and the debris its block particles
//! throw up over the next tick (`ClientExplosionTracker`).
use super::{ParticleEngine, ParticleLevel, ParticleOptions, ParticleType};
use horizon_core::{geometry::Vec3, random::RandomSource};

/// One entry of an explosion's weighted block-particle list
/// (`ExplosionParticleInfo`): the particle, how far out it appears as a share
/// of the distance drawn for it, what its speed is multiplied by, and its
/// weight in the pick.
#[derive(Clone, Debug, PartialEq)]
// exhaustive: the game builds it with a literal; it mirrors vanilla's record
pub struct ExplosionParticle {
    pub options: ParticleOptions,
    pub scaling: f32,
    pub speed: f32,
    pub weight: u32,
}
impl ExplosionParticle {
    /// `Level.DEFAULT_EXPLOSION_BLOCK_PARTICLES`: poof (scaling 0.5) and smoke,
    /// one each.
    pub fn defaults() -> Vec<Self> {
        let entry = |ty, scaling| Self {
            options: ParticleOptions::simple(ty),
            scaling,
            speed: 1.0,
            weight: 1,
        };
        vec![
            entry(ParticleType::Poof, 0.5),
            entry(ParticleType::Smoke, 1.0),
        ]
    }
}

/// `ClientExplosionTracker.ExplosionInfo`.
#[derive(Clone, Debug)]
pub(super) struct TrackedExplosion {
    pub(super) center: [f64; 3],
    pub(super) radius: f32,
    pub(super) block_count: usize,
    pub(super) particles: Vec<ExplosionParticle>,
}

impl ParticleEngine {
    /// `ClientExplosionTracker.track`: queue an explosion's block count,
    /// position and block particles for debris generation during
    /// `ClientLevel.tick`. One with no block particles (a wind charge's) or no
    /// blocks throws none.
    pub fn track_explosion(
        &mut self,
        center: [f64; 3],
        radius: f32,
        block_count: usize,
        particles: &[ExplosionParticle],
    ) {
        if block_count > 0 && particles.iter().any(|p| p.weight > 0) {
            self.tracked_explosions.push(TrackedExplosion {
                center,
                radius,
                block_count,
                particles: particles.to_vec(),
            });
        }
    }
    /// `ClientPacketListener.handleExplosion` + `ClientExplosionTracker`: the
    /// centre particle (`emitter`) and debris tracked for `tick`.
    pub fn explosion(
        &mut self,
        center: [f64; 3],
        emitter: &ParticleOptions,
        block_count: usize,
        radius: f32,
        particles: &[ExplosionParticle],
    ) {
        self.add(emitter, center, [1.0, 0.0, 0.0]);
        self.track_explosion(center, radius, block_count, particles);
    }
    /// `ClientExplosionTracker.tick`: throws up to 512 debris particles across all
    /// explosions tracked in this tick, weighted by each explosion's block count.
    pub(super) fn tick_explosions(&mut self, level: &ParticleLevel) {
        self.tick_explosions_with(&|b| level.facts.is_air(level.world.block(b[0], b[1], b[2])));
    }
    pub(super) fn tick_explosions_with(&mut self, is_air: &dyn Fn([i32; 3]) -> bool) {
        if self.tracked_explosions.is_empty() {
            return;
        }
        let explosions = std::mem::take(&mut self.tracked_explosions);
        if self.particle_level != 0 {
            return;
        }
        let total_blocks: usize = explosions.iter().map(|e| e.block_count).sum();
        if total_blocks == 0 {
            return;
        }
        let total_particles = total_blocks.min(512);
        for _ in 0..total_particles {
            // `WeightedRandom.getRandomItem` draws its `nextInt(totalWeight)`
            // even when one explosion is tracked.
            let mut pick = self.random.next_int_bound(total_blocks as i32).unwrap_or(0) as usize;
            let mut explosion = &explosions[0];
            for exp in &explosions {
                if pick < exp.block_count {
                    explosion = exp;
                    break;
                }
                pick -= exp.block_count;
            }
            let r = &mut self.random;
            let d = Vec3::new(
                f64::from(r.next_float() * 2.0 - 1.0),
                f64::from(r.next_float() * 2.0 - 1.0),
                f64::from(r.next_float() * 2.0 - 1.0),
            )
            .normalize();
            let distance = r.next_float().cbrt() * explosion.radius;
            let local = [
                d.x * f64::from(distance),
                d.y * f64::from(distance),
                d.z * f64::from(distance),
            ];
            let at = [
                explosion.center[0] + local[0],
                explosion.center[1] + local[1],
                explosion.center[2] + local[2],
            ];
            if !is_air(ParticleLevel::block_at(at)) {
                continue;
            }
            let speed =
                0.5 / (distance / explosion.radius + 0.1) * r.next_float() * r.next_float() + 0.3;
            // `WeightedList.getRandomOrThrow`: one `nextInt(totalWeight)`,
            // even for a single entry. A list whose total is past an `int`
            // (which `WeightedList` refuses) throws nothing.
            let total = explosion
                .particles
                .iter()
                .map(|p| u64::from(p.weight))
                .sum::<u64>();
            let Ok(total) = i32::try_from(total) else {
                continue;
            };
            let mut pick = r.next_int_bound(total).unwrap_or(0) as u32;
            let Some(particle) = explosion.particles.iter().find(|p| {
                let here = pick < p.weight;
                pick = pick.saturating_sub(p.weight);
                here
            }) else {
                continue;
            };
            let scaling = f64::from(particle.scaling);
            let p = [
                explosion.center[0] + local[0] * scaling,
                explosion.center[1] + local[1] * scaling,
                explosion.center[2] + local[2] * scaling,
            ];
            let speed = f64::from(speed * particle.speed);
            self.add(
                &particle.options,
                p,
                [d.x * speed, d.y * speed, d.z * speed],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_assets::resource_source::ResourceSource;
    use std::collections::{HashMap, HashSet};

    /// The debris one tick throws in air for a blast of 100 blocks at the
    /// camera whose block particles are smoke with these weights.
    fn debris(weights: &[u32]) -> usize {
        let mut engine = ParticleEngine::new(
            &ResourceSource::indexed(Default::default()),
            &HashMap::new(),
            &HashSet::new(),
        );
        let particles: Vec<_> = weights
            .iter()
            .map(|&weight| ExplosionParticle {
                weight,
                ..ExplosionParticle::defaults().remove(1)
            })
            .collect();
        engine.track_explosion([0.0; 3], 4.0, 100, &particles);
        engine.tick_explosions_with(&|_| true);
        engine.pending.len()
    }

    #[test]
    fn weights_whose_total_is_past_an_int_throw_no_debris() {
        let max = i32::MAX as u32;
        assert_eq!(debris(&[max, max, max]), 0);
        assert_eq!(debris(&[1 << 30, 1 << 30]), 0);
        assert_eq!(debris(&[u32::MAX, 1]), 0);
        assert_eq!(debris(&[max - 1, 1]), 100);
        assert_eq!(debris(&[1, 1]), 100);
    }
}
