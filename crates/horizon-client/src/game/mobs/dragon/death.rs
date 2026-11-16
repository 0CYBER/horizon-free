//! The dying ender dragon (`EnderDragon.tickDeath` and its die event), in its
//! own module so `dragon.rs` keeps its size.
use super::*;
impl Game<'_> {
    /// `tickDeath`: rising for 200 ticks, explosions, experience, then the
    /// exit portal, the egg and a gateway.
    pub(in crate::game) fn tick_dragon_death(&mut self) {
        let Some(d) = self.dragon.as_mut() else {
            return;
        };
        d.death_time += 1;
        let t = d.death_time;
        let p = d.position;
        let r = &mut d.random;
        let mut off = || {
            [
                f64::from((r.next_float() - 0.5) * 8.0),
                f64::from((r.next_float() - 0.5) * 4.0),
                f64::from((r.next_float() - 0.5) * 8.0),
            ]
        };
        if (180..=200).contains(&t) {
            let o = off();
            self.particles.add(
                &ParticleOptions::simple(ParticleType::ExplosionEmitter),
                [p.x + o[0], p.y + 2.0 + o[1], p.z + o[2]],
                [0.0; 3],
            );
        }
        // Only the fight's own dragon has a fight (`dragonFight`): elsewhere
        // (`/summon`), or summoned in the End after the fight is won, a
        // dragon is worth 500 and its death ends nothing.
        let fight = self.is_fight_dragon();
        let first_kill = fight && !self.fight.previously_killed;
        let xp = if first_kill { 12000 } else { 500 };
        let drops = self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::MOB_DROPS);
        if t > 150 && t % 5 == 0 && drops {
            self.spawn_orbs([p.x, p.y, p.z], (xp as f32 * 0.08) as u32);
        }
        if t == 1 && !self.dragon_silent() {
            // EnderDragon.tickDeath: global level event 1028, the death
            // roar, heard everywhere, unless the dragon is silent.
            let at = [p.x, p.y, p.z].map(|v| v.floor() as i32);
            self.global_level_event(1028, at, 0);
        }
        if let Some(d) = self.dragon.as_mut() {
            // deathMove is new Vec3(0.0, 0.1F, 0.0): the float widened.
            let rise = f64::from(0.1_f32);
            d.position.y += rise;
            for part in &mut d.parts {
                part.0.y += rise;
            }
        }
        if t >= 200 {
            if drops {
                self.spawn_orbs([p.x, p.y, p.z], (xp as f32 * 0.2) as u32);
            }
            let (id, at) = self.dragon.as_ref().map_or((0, p), |d| (d.id, d.position));
            self.discard_resident_dragon();
            if fight {
                self.dragon_killed();
            }
            // remove, then gameEvent(ENTITY_DIE): with no health left,
            // `Warden.canTargetEntity` refuses the dragon, so no warden hears it.
            self.dragon_died_event(id, at, false);
        }
    }
    /// `Entity.gameEvent(GameEvent.ENTITY_DIE)` for the removed dragon: sculk
    /// sensors (and, while it has health, wardens) hear it, and the nearest
    /// catalyst blooms (`CatalystListener`). It takes no experience: the
    /// dragon's `xpReward` is 0, its orbs come from `tickDeath`.
    pub(in crate::game) fn dragon_died_event(&mut self, id: u32, at: Vec3, targetable: bool) {
        let at = [at.x, at.y, at.z];
        let ctx = horizon_client::sculk::Context {
            source: Some(Who::Mob(id).into()),
            targetable,
            living: true,
            ..Default::default()
        };
        self.post_game_event("minecraft:entity_die", at, ctx);
        self.catalyst_takes_experience(at, |_| 0);
    }
}
