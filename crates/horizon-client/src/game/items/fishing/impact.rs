//! The flying bobber's cancellable impact, before vanilla's deflection decision.
use super::*;
use horizon_client::{mobs::Who, projectiles::Hit};

impl Game<'_> {
    /// Expose the actual cast hook to the listener, then resume only that same
    /// cast. Cancellation skips hooking and redirection, never Entity.move.
    pub(super) fn fishing_impact(&mut self, h: Hook, hit: Hit) -> Option<(Hook, bool)> {
        if matches!(hit, Hit::None) || !self.native_hears_impacts() {
            return Some((h, false));
        }
        let (id, join, position, velocity) = (h.id, h.join, h.position, h.velocity);
        let target = self.native_impact_target(hit);
        self.hook.put_back(h);
        let canceled = self.native_separate_projectile_impact(
            id,
            "minecraft:fishing_bobber",
            Some(Who::Player),
            hit,
        );
        if !self
            .hook
            .as_ref()
            .is_some_and(|h| h.id == id && h.join == join)
        {
            return None;
        }
        let h = self.hook.extract()?;
        if self
            .persist
            .removals
            .contains(&super::super::commands::Target::Hook(id))
        {
            // The deferred removal already made the handle stale. Consume the
            // extracted body now, clearing the owner's line without retrieval.
            return None;
        }
        let changed = h.position != position
            || h.velocity != velocity
            || h.state != State::Flying
            || target != self.native_impact_target(hit);
        Some((h, canceled || changed))
    }
}

#[cfg(test)]
#[path = "impact_tests.rs"]
mod tests;
