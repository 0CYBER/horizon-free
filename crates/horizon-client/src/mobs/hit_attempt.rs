//! Observe the complete species invulnerability predicate once per living hit attempt.
use super::*;

impl Mob {
    pub(super) fn remember_accepted_hit(
        &mut self,
        level: &DamageLevel<'_>,
        source: &Damage,
        accepted: bool,
    ) {
        if accepted {
            self.death_canceled = false;
            self.accepted_hit = level.accepted_hit(crate::health::Hurting::Mob(self, source));
        }
    }
    /// WitherBoss refuses its friends after asking isInvulnerableTo, not at the caller.
    pub(super) fn attacker_allows_damage(&self, source: &Damage) -> bool {
        !source
            .attacker_species
            .is_some_and(|attacker| wither::shielded_from(self.species(), attacker))
    }

    pub(super) fn invulnerable_to_attempt(
        &self,
        level: &mut DamageLevel<'_>,
        source: &Damage,
    ) -> bool {
        let bypasses = source.kind.bypasses_invulnerability();
        let reflected = self.reflected_fireball(source);
        let flagged = self.is_invulnerable_to(source);
        let digging =
            self.species() == Species::Warden && warden::digging_or_emerging(self) && !bypasses;
        // Ghast's override can short-circuit the base predicate. Its final answer is still
        // the one observation for this attempt, including a reflected fireball.
        let vanilla = digging
            || if self.species() == Species::Ghast && (flagged || reflected) {
                self.removed || flagged
            } else {
                self.removed
                    || flagged
                    || (source.is_fire() && self.kind.flags.fire_immune && !reflected)
            };
        level.invulnerability_check(crate::health::Hurting::Mob(self, source), vanilla)
    }
}
