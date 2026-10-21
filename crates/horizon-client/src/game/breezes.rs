//! Breeze particles (`Breeze.tick`): block dust at its feet while it stands,
//! a burst of twenty while it slides and a short trail along a long jump.
use super::Game;
use horizon_client::{
    mobs::{Species, breeze},
    particles::Particle,
};

impl Game<'_> {
    pub(super) fn tick_breeze_effects(&mut self) {
        let Some(camera) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::Breeze
                || !m.alive()
                || m.position.distance_squared(camera) > 32.0 * 32.0
            {
                continue;
            }
            let pose = breeze::pose(m);
            let (amount, jump_trail) = match pose {
                breeze::SLIDING => (20, false),
                breeze::LONG_JUMPING => (3, true),
                _ => (1, false),
            };
            if jump_trail {
                self.mobs[i].extra.scratch[1] += 1;
                if self.mobs[i].extra.scratch[1] > 5 {
                    continue;
                }
            } else {
                self.mobs[i].extra.scratch[1] = 0;
            }
            let m = &self.mobs[i];
            let (x, y, z) = (
                m.position.x.floor() as i32,
                m.position.y.floor() as i32,
                m.position.z.floor() as i32,
            );
            let mut at = [x, y, z];
            let mut state = self.world.block(x, y, z);
            if self.res.facts().is_air(state) {
                at[1] = (m.position.y - 0.2).floor() as i32;
                state = self.world.block(x, at[1], z);
            }
            let Some((sprite, tint)) = self.particle_sprite(state, at) else {
                continue;
            };
            let at = if jump_trail {
                [
                    m.position.x + m.velocity.x,
                    m.position.y + m.velocity.y + f64::from(0.1_f32),
                    m.position.z + m.velocity.z,
                ]
            } else {
                [m.position.x, m.position.y, m.position.z]
            };
            for _ in 0..amount {
                let particle =
                    Particle::terrain(&mut self.world.random, at, [0.0; 3], sprite.clone(), tint);
                self.particles.push(particle);
            }
        }
    }
}
