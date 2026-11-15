//! Who may hurt a mob: `LivingEntity.isInvulnerableTo`'s `Entity.isInvulnerableToBase` term,
//! and the dead-or-removed guard every damage hook runs after (`LivingIncomingDamageEvent`).
use super::{Damage, DamageKind, Mob, Who};

impl Mob {
    /// A player returning a ghast's fireball gets past its fire immunity.
    pub(super) fn reflected_fireball(&self, source: &Damage) -> bool {
        &*self.kind.id == "minecraft:ghast"
            && source.kind == DamageKind::Fireball
            && source.attacker == Some(Who::Player)
    }

    /// `Entity.isInvulnerableToBase`'s invulnerability term: `Invulnerable` or
    /// `invulnerable_time` left, which only #bypasses_invulnerability or a blow a creative
    /// player caused (`DamageSource.isCreativePlayer`) gets past.
    pub fn is_invulnerable_to(&self, source: &Damage) -> bool {
        self.is_invulnerable() && !self.creative_attacker && !source.kind.bypasses_invulnerability()
    }
    /// `LivingEntity.isInvulnerableTo` and dead or removed: only #bypasses_invulnerability or a
    /// creative player's blow gets past the flag. The guard every damage hook runs after
    /// (`LivingIncomingDamageEvent`).
    pub fn refuses_damage(&self, source: &Damage) -> bool {
        self.extra.client_side
            || !self.alive()
            || self.removed
            || self.is_invulnerable_to(source)
            || (source.is_fire() && self.kind.flags.fire_immune && !self.reflected_fireball(source))
    }
}
