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
    /// The egg item at the parent (`Sniffer.spawnChildFromBreeding`; the
    /// brain already played the plop and dropped the experience).
    pub(super) fn lay_sniffer_egg(&mut self, parent: &Mob) {
        let at = [parent.position.x, parent.position.y, parent.position.z];
        // new ItemEntity(level, x, y, z, stack): a little random push.
        let vx = self.world.random.next_double() * 0.2 - 0.1;
        let vz = self.world.random.next_double() * 0.2 - 0.1;
        self.spawn_item(
            ItemStack::new("minecraft:sniffer_egg", 1),
            at,
            [vx, 0.2, vz],
            10,
        );
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
    /// The sniffer's client-side effects: `SnifferSoundInstance` when it
    /// starts digging, `Sniffer.emitDiggingParticles` (block dust and the
    /// block's hit sound between the 1.7th and the 6th second of the
    /// digging animation) and `playSearchingSound`.
    pub(super) fn tick_sniffer_effects(&mut self) {
        let Some(camera) = self.player.as_ref().map(|p| p.position) else {
            return;
        };
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::Sniffer
                || !m.alive()
                || m.silent
                || m.position.distance_squared(camera) > 32.0 * 32.0
            {
                continue;
            }
            let at = [m.position.x, m.position.y, m.position.z];
            if sniffer::state(m) == sniffer::SEARCHING && m.age.is_multiple_of(20) {
                self.audio
                    .play("minecraft:entity.sniffer.searching", Some(at), 1.0, 1.0);
                continue;
            }
            if !sniffer::digging(m) {
                continue;
            }
            let Some(ms) = m.anim.states[3].millis(m.age as f32) else {
                continue;
            };
            if ms == 0 {
                self.audio
                    .play("minecraft:entity.sniffer.digging", Some(at), 1.0, 1.0);
            }
            if ms <= 1700 || ms >= 6000 {
                continue;
            }
            let head = sniffer::head_block(m);
            let below = self.world.block(head[0], head[1] - 1, head[2]);
            let Some((sprite, tint)) = self.particle_sprite(below, [head[0], head[1] - 1, head[2]])
            else {
                continue;
            };
            let dust = [
                f64::from(head[0]) + 0.5,
                f64::from(head[1]) + 0.5 - f64::from(0.65_f32),
                f64::from(head[2]) + 0.5,
            ];
            for _ in 0..30 {
                let particle =
                    Particle::terrain(&mut self.world.random, dust, [0.0; 3], sprite.clone(), tint);
                self.particles.push(particle);
            }
            if self.mobs[i].age.is_multiple_of(10) {
                let hit = self.res.facts().get(below).sounds.hit.clone();
                self.audio.play(&hit, Some(at), 0.5, 0.5);
            }
        }
    }
}
