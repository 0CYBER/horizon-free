//! Sniffers on the game side: the egg a bred pair lays
//! (`Sniffer.spawnChildFromBreeding`), the snifflet that hatches from an
//! egg block (`SnifferEggBlock.tick`) and the dust and sound of digging.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mobs::{Mob, Species, sniffer},
    particles::Particle,
};
use horizon_core::random::RandomSource;

impl Game<'_> {
    /// An egg item at the parent, a plop and some experience.
    pub(super) fn lay_sniffer_egg(&mut self, parent: &Mob) {
        let at = [parent.position.x, parent.position.y, parent.position.z];
        self.spawn_item(
            ItemStack::new("minecraft:sniffer_egg", 1),
            at,
            [0.0, 0.2, 0.0],
            10,
        );
        let pitch = (self.world.random.next_float() - self.world.random.next_float()) * 0.2 + 0.5;
        self.audio
            .play("minecraft:block.sniffer_egg.plop", Some(at), 1.0, pitch);
        let xp = 1 + self.world.random.next_int_bound(7).unwrap_or(0) as u32;
        self.spawn_orbs(at, xp);
    }
    /// A snifflet where the egg was.
    pub(super) fn hatch_sniffer(&mut self, pos: [i32; 3]) {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        if let Some(id) = self.spawn_mob("minecraft:sniffer", at, true)
            && let Some(m) = self.mobs.iter_mut().find(|m| m.id == id)
        {
            m.extra.age = sniffer::BABY_START_AGE;
            let yaw = self.world.random.next_float() * 360.0 - 180.0;
            m.yaw = yaw;
            m.body_yaw = yaw;
            m.head_yaw = yaw;
        }
    }
    /// `Sniffer.emitDiggingParticles`: block dust and the digging sound
    /// between the 1.7th and the 6th second of the digging animation.
    pub(super) fn tick_sniffer_effects(&mut self) {
        let Some(camera) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::Sniffer
                || !m.alive()
                || !sniffer::digging(m)
                || m.position.distance_squared(camera) > 32.0 * 32.0
            {
                continue;
            }
            let Some(ms) = m.anim.states[3].millis(m.age as f32) else {
                continue;
            };
            if ms <= 1700 || ms >= 6000 {
                continue;
            }
            let head = sniffer::head_block(m);
            let below = self.world.block(head[0], head[1] - 1, head[2]);
            let Some((sprite, tint)) = self.particle_sprite(below) else {
                continue;
            };
            let at = [
                f64::from(head[0]) + 0.5,
                f64::from(head[1]) + 0.5 - f64::from(0.65_f32),
                f64::from(head[2]) + 0.5,
            ];
            for _ in 0..30 {
                let particle =
                    Particle::terrain(&mut self.world.random, at, [0.0; 3], sprite.clone(), tint);
                self.particles.push(particle);
            }
            if self.mobs[i].age.is_multiple_of(10) {
                self.audio
                    .play("minecraft:entity.sniffer.digging", Some(at), 0.5, 0.5);
            }
        }
    }
}
