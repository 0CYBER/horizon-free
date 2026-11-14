//! Rocket impacts dispatch only after the vanilla deflection decision.
use super::*;

impl Game<'_> {
    /// Let listeners inspect and remove the live rocket, preserving its identity
    /// across the temporary tick extraction. Refuse a stale hit after writes.
    pub(super) fn rocket_impact(
        &mut self,
        index: usize,
        r: Rocket,
        hit: ProjectileHit,
        hears: bool,
    ) -> Option<(Rocket, bool)> {
        if !hears || matches!(hit, ProjectileHit::None) {
            return Some((r, false));
        }
        let (id, join, position, velocity) = (r.id, r.join, r.position, r.velocity);
        let owner = self.rocket_owner(&r);
        let target = self.native_impact_target(hit);
        self.rockets.put_back(index, r);
        let canceled =
            self.native_separate_projectile_impact(id, "minecraft:firework_rocket", owner, hit);
        let r = self.rockets.extract(join)?;
        if self
            .persist
            .removals
            .contains(&super::super::commands::Target::Rocket(id))
        {
            self.rockets.retire_extracted(id, join);
            return None;
        }
        let changed = r.position != position
            || r.velocity != velocity
            || target != self.native_impact_target(hit);
        Some((r, canceled || changed))
    }
}

#[cfg(test)]
#[path = "impact_tests.rs"]
mod tests;
